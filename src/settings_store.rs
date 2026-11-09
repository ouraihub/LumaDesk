//! SQLite-backed appearance and network-search settings storage.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DATABASE: &str = "settings.db";
const APPEARANCE_MARKER: &str = "appearance-v1";
const SEARCH_MARKER: &str = "search-v1";
const MCP_MARKER: &str = "mcp-v1";
const MIGRATIONS: [lumakit_support::sqlite::Migration; 2] = [
    lumakit_support::sqlite::Migration {
        version: 1,
        sql: "CREATE TABLE IF NOT EXISTS prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL CHECK (json_valid(value)), updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
   CREATE TABLE IF NOT EXISTS search_configs (provider TEXT PRIMARY KEY, position INTEGER NOT NULL, endpoint TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)));
   CREATE TABLE IF NOT EXISTS search_keys (provider TEXT NOT NULL REFERENCES search_configs(provider) ON DELETE CASCADE, id TEXT NOT NULL, masked TEXT NOT NULL, ciphertext BLOB NOT NULL, PRIMARY KEY(provider,id));
   CREATE TABLE IF NOT EXISTS settings_import_markers (domain TEXT PRIMARY KEY, imported_at INTEGER NOT NULL DEFAULT (unixepoch()));",
    },
    // The preferred key a provider's requests start from; NULL means its first key.
    lumakit_support::sqlite::Migration {
        version: 2,
        sql: "ALTER TABLE search_configs ADD COLUMN selected_key TEXT;",
    },
];

#[derive(Clone)]
pub(crate) struct SearchKeyRow {
    pub id: String,
    pub masked: String,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct SearchConfigRow {
    pub provider: String,
    pub endpoint: String,
    pub username: String,
    pub keys: Vec<SearchKeyRow>,
    pub selected_key: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SearchData {
    pub default_search: String,
    pub default_fetch: String,
    pub configs: Vec<SearchConfigRow>,
    pub preferences: Value,
    pub compression: String,
}
#[derive(Clone)]
pub(crate) struct McpData {
    pub servers: Vec<lumakit_ai::mcp::McpServerConfig>,
}

fn database(root: &Path) -> PathBuf {
    root.join(DATABASE)
}
fn open(root: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("无法创建设置目录：{e}"))?;
    lumakit_support::sqlite::open_and_migrate(database(root), &MIGRATIONS)
        .map_err(|e| format!("无法打开设置数据库：{e}"))
}
fn marker(connection: &Connection, domain: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT 1 FROM settings_import_markers WHERE domain=?1",
            [domain],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|v| v.is_some())
        .map_err(|e| format!("无法读取设置迁移标记：{e}"))
}
fn json(value: &Value) -> Result<String, String> {
    let encoded = serde_json::to_string(value).map_err(|e| format!("设置序列化失败：{e}"))?;
    Ok(encoded)
}

/// Reads appearance preferences. `None` means the legacy import has not run.
pub(crate) fn load_appearance(root: &Path) -> Result<Option<Value>, String> {
    let connection = open(root)?;
    if !marker(&connection, APPEARANCE_MARKER)? {
        return Ok(None);
    }
    let raw = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='appearance'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取外观设置：{e}"))?
        .ok_or_else(|| "外观迁移标记存在但设置缺失".to_owned())?;
    Ok(Some(
        serde_json::from_str(&raw).map_err(|e| format!("外观设置 JSON 无效：{e}"))?,
    ))
}

/// Atomically saves appearance preferences and records its import marker.
pub(crate) fn save_appearance(root: &Path, value: &Value) -> Result<(), String> {
    let encoded = json(value)?;
    let mut connection = open(root)?;
    let transaction = connection
        .transaction()
        .map_err(|e| format!("无法开始外观事务：{e}"))?;
    transaction.execute("INSERT INTO prefs(key,value,updated_at) VALUES('appearance',?1,unixepoch()) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", [encoded]).map_err(|e| format!("保存外观失败：{e}"))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO settings_import_markers(domain) VALUES(?1)",
            [APPEARANCE_MARKER],
        )
        .map_err(|e| format!("写入外观迁移标记失败：{e}"))?;
    transaction
        .commit()
        .map_err(|e| format!("提交外观失败：{e}"))
}

/// Reads relational search settings. `None` means the legacy import has not run.
pub(crate) fn load_search(root: &Path) -> Result<Option<SearchData>, String> {
    let connection = open(root)?;
    if !marker(&connection, SEARCH_MARKER)? {
        return Ok(None);
    }
    let defaults = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='search.defaults'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取搜索默认项：{e}"))?
        .ok_or_else(|| "搜索迁移标记存在但默认项缺失".to_owned())?;
    let compression = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='search.compression'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取搜索压缩设置：{e}"))?
        .ok_or_else(|| "搜索迁移标记存在但压缩设置缺失".to_owned())?;
    let preferences = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='search.preferences'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取搜索偏好：{e}"))?
        .ok_or_else(|| "搜索迁移标记存在但搜索偏好缺失".to_owned())?;
    let defaults = serde_json::from_str::<Value>(&defaults)
        .map_err(|e| format!("搜索默认项 JSON 无效：{e}"))?;
    let default_search = defaults
        .get("default_search")
        .and_then(Value::as_str)
        .ok_or_else(|| "搜索默认项无效".to_owned())?
        .to_owned();
    let default_fetch = defaults
        .get("default_fetch")
        .and_then(Value::as_str)
        .ok_or_else(|| "搜索默认项无效".to_owned())?
        .to_owned();
    let preferences =
        serde_json::from_str(&preferences).map_err(|e| format!("搜索偏好 JSON 无效：{e}"))?;
    let compression = serde_json::from_str::<Value>(&compression)
        .map_err(|e| format!("搜索压缩设置 JSON 无效：{e}"))?
        .as_str()
        .ok_or_else(|| "搜索压缩设置无效".to_owned())?
        .to_owned();
    let mut configs = Vec::new();
    let usernames = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='search.usernames'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取搜索用户名：{e}"))?;
    let usernames: std::collections::HashMap<String, String> = usernames
        .map(|raw| serde_json::from_str(&raw))
        .transpose()
        .map_err(|_| "搜索用户名配置无效".to_owned())?
        .unwrap_or_default();
    let mut statement = connection
        .prepare("SELECT provider,endpoint,selected_key FROM search_configs WHERE enabled=1 ORDER BY position")
        .map_err(|e| format!("无法读取搜索配置：{e}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| format!("无法读取搜索配置：{e}"))?;
    for row in rows {
        let (provider, endpoint, selected_key) =
            row.map_err(|e| format!("无法读取搜索配置：{e}"))?;
        let mut keys = Vec::new();
        let mut key_statement = connection
            .prepare(
                "SELECT id,masked,ciphertext FROM search_keys WHERE provider=?1 ORDER BY rowid",
            )
            .map_err(|e| format!("无法读取搜索密钥：{e}"))?;
        let key_rows = key_statement
            .query_map([&provider], |row| {
                Ok(SearchKeyRow {
                    id: row.get(0)?,
                    masked: row.get(1)?,
                    ciphertext: row.get(2)?,
                })
            })
            .map_err(|e| format!("无法读取搜索密钥：{e}"))?;
        for key in key_rows {
            keys.push(key.map_err(|e| format!("无法读取搜索密钥：{e}"))?);
        }
        let username = usernames.get(&provider).cloned().unwrap_or_default();
        configs.push(SearchConfigRow {
            provider,
            endpoint,
            username,
            keys,
            selected_key,
        });
    }
    Ok(Some(SearchData {
        default_search,
        default_fetch,
        configs,
        preferences,
        compression,
    }))
}

/// Atomically replaces relational search settings and records its import marker.
pub(crate) fn save_search(root: &Path, data: &SearchData) -> Result<(), String> {
    let defaults = serde_json::json!({"default_search": data.default_search, "default_fetch": data.default_fetch});
    let defaults = json(&defaults)?;
    let preferences = json(&data.preferences)?;
    let compression = json(&Value::String(data.compression.clone()))?;
    let usernames = json(
        &serde_json::to_value(
            data.configs
                .iter()
                .filter(|c| !c.username.is_empty())
                .map(|c| (&c.provider, &c.username))
                .collect::<std::collections::HashMap<_, _>>(),
        )
        .map_err(|_| "搜索用户名序列化失败".to_owned())?,
    )?;
    let mut connection = open(root)?;
    let transaction = connection
        .transaction()
        .map_err(|e| format!("无法开始搜索事务：{e}"))?;
    for (key, value) in [
        ("search.defaults", defaults),
        ("search.preferences", preferences),
        ("search.compression", compression),
        ("search.usernames", usernames),
    ] {
        transaction.execute("INSERT INTO prefs(key,value,updated_at) VALUES(?1,?2,unixepoch()) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", params![key, value]).map_err(|e| format!("保存搜索偏好失败：{e}"))?;
    }
    transaction
        .execute("DELETE FROM search_keys", [])
        .map_err(|e| format!("保存搜索密钥失败：{e}"))?;
    transaction
        .execute("DELETE FROM search_configs", [])
        .map_err(|e| format!("保存搜索配置失败：{e}"))?;
    for (position, config) in data.configs.iter().enumerate() {
        transaction
     .execute(
  "INSERT INTO search_configs(provider,position,endpoint,selected_key) VALUES(?1,?2,?3,?4)",
         params![config.provider, position as i64, config.endpoint, config.selected_key],
     )
     .map_err(|e| format!("保存搜索配置失败：{e}"))?;
        for key in &config.keys {
            transaction
                .execute(
                    "INSERT INTO search_keys(provider,id,masked,ciphertext) VALUES(?1,?2,?3,?4)",
                    params![config.provider, key.id, key.masked, key.ciphertext],
                )
                .map_err(|e| format!("保存搜索密钥失败：{e}"))?;
        }
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO settings_import_markers(domain) VALUES(?1)",
            [SEARCH_MARKER],
        )
        .map_err(|e| format!("写入搜索迁移标记失败：{e}"))?;
    transaction
        .commit()
        .map_err(|e| format!("提交搜索设置失败：{e}"))
}

pub(crate) fn load_mcp(root: &Path) -> Result<Option<McpData>, String> {
    let connection = open(root)?;
    if !marker(&connection, MCP_MARKER)? {
        return Ok(None);
    }
    let raw = connection
        .query_row(
            "SELECT value FROM prefs WHERE key='mcp.servers'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取 MCP 设置：{e}"))?
        .ok_or_else(|| "MCP 迁移标记存在但设置缺失".to_owned())?;
    Ok(Some(McpData {
        servers: serde_json::from_str(&raw).map_err(|e| format!("MCP 设置 JSON 无效：{e}"))?,
    }))
}

pub(crate) fn save_mcp(root: &Path, data: &McpData) -> Result<(), String> {
    let encoded = json(
        &serde_json::to_value(&data.servers).map_err(|e| format!("MCP 设置序列化失败：{e}"))?,
    )?;
    let mut connection = open(root)?;
    let transaction = connection
        .transaction()
        .map_err(|e| format!("无法开始 MCP 设置事务：{e}"))?;
    transaction.execute("INSERT INTO prefs(key,value,updated_at) VALUES('mcp.servers',?1,unixepoch()) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", [encoded]).map_err(|e| format!("保存 MCP 设置失败：{e}"))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO settings_import_markers(domain) VALUES(?1)",
            [MCP_MARKER],
        )
        .map_err(|e| format!("写入 MCP 迁移标记失败：{e}"))?;
    transaction
        .commit()
        .map_err(|e| format!("提交 MCP 设置失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn future_schema_is_not_overwritten_or_imported_again() {
        let root = std::env::temp_dir().join(format!(
            "lumadesk-settings-future-{}",
            lumakit_support::new_id().unwrap()
        ));
        save_appearance(&root, &serde_json::json!({"kept":true})).unwrap();
        let connection = Connection::open(database(&root)).unwrap();
        connection.execute_batch("PRAGMA user_version=3;").unwrap();
        assert!(load_appearance(&root).is_err());
        assert!(load_search(&root).is_err());
        assert!(save_appearance(&root, &Value::Null).is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM prefs WHERE key='appearance'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            r#"{"kept":true}"#
        );
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}
