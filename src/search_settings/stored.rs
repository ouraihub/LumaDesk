//! Stored catalog defaults, database decoding and transaction snapshot encoding.
use super::*;
impl Default for Stored {
    fn default() -> Self {
        Self {
            default_search: Provider::Tavily,
            default_fetch: Provider::Jina,
            configs: PROVIDERS
                .iter()
                .map(|&provider| Config {
                    provider,
                    endpoint: provider.endpoint().into(),
                    keys: Vec::new(),
                    username: String::new(),
                })
                .collect(),
            preferences: SearchPreferences::default(),
            compression: Compression::None,
        }
    }
}
impl Stored {
    pub(super) fn from_database(data: SearchData) -> Result<Self, String> {
        let provider = |name: String| {
            serde_json::from_value(serde_json::Value::String(name))
                .map_err(|_| "搜索数据库包含未知服务商".to_owned())
        };
        let value = Self {
            default_search: provider(data.default_search)?,
            default_fetch: provider(data.default_fetch)?,
            configs: data
                .configs
                .into_iter()
                .map(|row| {
                    Ok(Config {
                        provider: provider(row.provider)?,
                        endpoint: row.endpoint,
                        username: row.username,
                        keys: row
                            .keys
                            .into_iter()
                            .map(|key| SavedKey {
                                id: key.id,
                                masked: key.masked,
                                ciphertext: key.ciphertext,
                            })
                            .collect(),
                    })
                })
                .collect::<Result<_, String>>()?,
            preferences: serde_json::from_value(data.preferences)
                .map_err(|_| "搜索数据库偏好无效".to_owned())?,
            compression: serde_json::from_value(serde_json::Value::String(data.compression))
                .map_err(|_| "搜索压缩方式无效".to_owned())?,
        };
        validate_stored(&value)?;
        Ok(value)
    }
    pub(super) fn save(&self, root: &Path) -> Result<(), String> {
        validate_stored(self)?;
        settings_store::save_search(
            root,
            &SearchData {
                default_search: self.default_search.id().into(),
                default_fetch: self.default_fetch.id().into(),
                configs: self
                    .configs
                    .iter()
                    .map(|config| SearchConfigRow {
                        provider: config.provider.id().into(),
                        endpoint: config.endpoint.clone(),
                        username: config.username.clone(),
                        keys: config
                            .keys
                            .iter()
                            .map(|key| SearchKeyRow {
                                id: key.id.clone(),
                                masked: key.masked.clone(),
                                ciphertext: key.ciphertext.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
                preferences: serde_json::to_value(&self.preferences)
                    .map_err(|_| "搜索偏好序列化失败".to_owned())?,
                compression: if self.compression == Compression::None {
                    "none"
                } else {
                    "cutoff"
                }
                .into(),
            },
        )
    }
}
