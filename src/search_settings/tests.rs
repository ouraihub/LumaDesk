use super::*;
use reqwest::Url;
#[cfg(windows)]
#[test]
fn sqlite_import_preserves_ciphertext_and_rolls_back_failed_save() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-import-{}",
        lumakit_support::new_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(FILE);
    std::fs::write(&path, b"broken").unwrap();
    assert!(SearchSettings::load(&root).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    let mut old = Stored::default();
    old.configs[0]
        .keys
        .push(SavedKey::new("migration-secret").unwrap());
    lumakit_support::save_versioned(&path, 1, &old).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut settings = SearchSettings::load(&root).unwrap();
    assert_eq!(
        settings.stored.configs[0].keys[0].ciphertext,
        old.configs[0].keys[0].ciphertext
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::write(&path, b"not used after import").unwrap();
    let connection = rusqlite::Connection::open(root.join("settings.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_search_write BEFORE INSERT ON search_configs BEGIN SELECT RAISE(ABORT,'reject'); END;").unwrap();
    settings.drafts[0].endpoint = "https://changed.example".into();
    assert!(settings.save(0, None).is_err());
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(restored.stored.configs[0].endpoint, old.configs[0].endpoint);
    assert_eq!(
        restored.stored.configs[0].keys[0].open().unwrap().as_str(),
        "migration-secret"
    );
    connection
        .execute_batch("DROP TRIGGER reject_search_write; DELETE FROM search_configs;")
        .unwrap();
    assert!(SearchSettings::load(&root).is_err());
    drop(connection);
    drop(restored);
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

use std::io::{Read, Write};
#[test]
fn invalid_blacklist_is_atomic_and_url_path_is_case_sensitive() {
    let original = SearchPreferences {
        blacklist: vec!["kept.example".into()],
        ..Default::default()
    };
    assert!(preferences_from_draft(&original, "valid.example\nhttps://bad user/path").is_err());
    assert_eq!(original.blacklist, ["kept.example"]);
    let rules = vec![normalize_rule("https://EXAMPLE.COM/Private/*").unwrap()];
    assert!(blocked(
        &Url::parse("https://example.com/Private/A").unwrap(),
        &rules
    ));
    assert!(!blocked(
        &Url::parse("https://example.com/private/A").unwrap(),
        &rules
    ));
    assert!(normalize_rule("https://bad_host.test/path").is_err());
    assert_eq!(
        html_text("<title>A &amp; B</title><script>secret()</script><p>你好</p>").0,
        "A & B"
    );
    assert!(
        !html_text("<script>secret()</script><p>你好</p>")
            .1
            .contains("secret")
    );
}
#[test]
fn direct_fetch_uses_real_loopback_http_and_unicode_cutoff() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/page", server.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let n = stream.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /page "));
        let body = format!(
            "<title>本地网页</title><script>hidden()</script><p>{}</p>",
            "文".repeat(200)
        );
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let config = Config {
        provider: Provider::Direct,
        endpoint: String::new(),
        keys: Vec::new(),
        username: String::new(),
        selected_key: None,
    };
    let prefs = SearchPreferences {
        cutoff: 100,
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let page = rt
        .block_on(fetch_page(
            config,
            endpoint.clone(),
            prefs,
            CancellationToken::new(),
            &test_trace(),
        ))
        .unwrap();
    assert_eq!(page.title, "本地网页");
    assert_eq!(page.url, endpoint);
    assert_eq!(page.body.chars().count(), 100);
    assert!(!page.body.contains("hidden"));
    worker.join().unwrap();
}
#[cfg(windows)]
#[test]
fn search_http_and_encrypted_restart_readback() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..n]);
            if request.windows(4).any(|x| x == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(text.starts_with("post /search "));
        assert!(text.contains("authorization: bearer local-test-key"));
        let body = r#"{"results":[{"title":"真实回环","url":"https://example.org/Private/A","content":"本地 HTTP 响应"}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    settings.drafts[0].endpoint = endpoint;
    settings.save(0, None).unwrap();
    settings.drafts[0].key = "local-test-key".into();
    settings.save(0, Some(KeyAction::Add)).unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(
        restored.stored.configs[0].keys[0].open().unwrap().as_str(),
        "local-test-key"
    );
    assert!(restored.stored.configs[4].keys.is_empty());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt
        .block_on(search_query(
            restored.stored.configs[0].clone(),
            "本地查询".into(),
            restored.preferences.clone(),
            CancellationToken::new(),
            &test_trace(),
        ))
        .unwrap();
    assert_eq!(result.hits[0].title, "真实回环");
    worker.join().unwrap();
    assert!(
        !std::fs::read(root.join("settings.db"))
            .unwrap()
            .windows(b"local-test-key".len())
            .any(|bytes| bytes == b"local-test-key")
    );
    drop(restored);
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(windows)]
#[test]
fn search_fails_over_only_auth_status_and_filters_before_result_limit() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let len = headers
                        .lines()
                        .find_map(|s| {
                            s.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            assert!(request.contains(if attempt == 0 {
                "failover-first"
            } else {
                "failover-second"
            }));
            if attempt == 0 {
                write!(
                    stream,
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            } else {
                let body = r#"{"results":[{"title":"blocked","url":"https://example.org/Private/A","content":"blocked"},{"title":"allowed","url":"https://example.org/private/A","content":"unicode 文本"}]}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        }
    });
    // The preferred key is the second one: requests start there and wrap to the first.
    let keys = vec![
        SavedKey::new("failover-second").unwrap(),
        SavedKey::new("failover-first").unwrap(),
    ];
    let config = Config {
        provider: Provider::Tavily,
        endpoint,
        selected_key: Some(keys[1].id.clone()),
        keys,
        username: String::new(),
    };
    let prefs = SearchPreferences {
        max_results: 1,
        blacklist: vec!["https://example.org/Private/*".into()],
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt
        .block_on(search_query(
            config,
            "实际查询".into(),
            prefs,
            CancellationToken::new(),
            &test_trace(),
        ))
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].title, "allowed");
    server.join().unwrap();
}

#[test]
fn provider_save_and_key_manager_do_not_commit_other_drafts() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-scope-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    settings.drafts[0].endpoint = "https://search.example/v1".into();
    settings.drafts[4].endpoint = "unfinished address".into();
    settings.preferences.max_results = 0;
    settings.drafts[0].key = "card-key".into();
    settings.save(0, None).unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(
        restored.stored.configs[0].endpoint,
        "https://search.example/v1"
    );
    assert_eq!(
        restored.stored.configs[4].endpoint,
        Provider::Jina.endpoint()
    );
    assert_eq!(restored.stored.preferences.max_results, 5);
    assert!(restored.stored.configs[0].keys.is_empty());
    settings.drafts[0].endpoint = "invalid new address".into();
    settings.drafts[0].key = "manager-key".into();
    settings.save(0, Some(KeyAction::Add)).unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(
        restored.stored.configs[0].endpoint,
        "https://search.example/v1"
    );
    assert_eq!(
        restored.stored.configs[0].keys[0].open().unwrap().as_str(),
        "manager-key"
    );
    drop(restored);
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_five_provider_catalog_upgrades_without_losing_credentials() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-catalog-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut old = Stored::default();
    old.configs.truncate(5);
    old.default_search = Provider::Exa;
    old.configs[2]
        .keys
        .push(SavedKey::new("original-secret").unwrap());
    let ciphertext = old.configs[2].keys[0].ciphertext.clone();
    old.save(&root).unwrap();
    let settings = SearchSettings::load(&root).unwrap();
    assert_eq!(settings.stored.default_search, Provider::Exa);
    assert_eq!(settings.stored.configs[2].keys[0].ciphertext, ciphertext);
    for provider in catalog::SEARCH_PROVIDERS
        .into_iter()
        .chain(catalog::FETCH_PROVIDERS)
    {
        assert_eq!(
            settings
                .stored
                .configs
                .iter()
                .filter(|config| config.provider == provider)
                .count(),
            1
        );
    }
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn key_edit_delete_and_basic_username_survive_restart_atomically() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-keys-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    let index = settings
        .stored
        .configs
        .iter()
        .position(|c| c.provider == Provider::Searxng)
        .unwrap();
    settings.drafts[index].username = "local-user".into();
    settings.save(index, None).unwrap();
    settings.drafts[index].key = "initial-password".into();
    settings.save(index, Some(KeyAction::Add)).unwrap();
    let id = settings.stored.configs[index].keys[0].id.clone();
    settings
        .save(
            index,
            Some(KeyAction::Replace(
                id.clone(),
                "replacement-password".into(),
            )),
        )
        .unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(restored.stored.configs[index].username, "local-user");
    assert_eq!(restored.stored.configs[index].keys[0].id, id);
    assert_eq!(
        restored.stored.configs[index].keys[0]
            .open()
            .unwrap()
            .as_str(),
        "replacement-password"
    );
    let db = rusqlite::Connection::open(root.join("settings.db")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_search_keys BEFORE INSERT ON search_keys BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(
        settings
            .save(
                index,
                Some(KeyAction::Replace(id.clone(), "must-not-save".into()))
            )
            .is_err()
    );
    assert_eq!(
        settings.stored.configs[index].keys[0]
            .open()
            .unwrap()
            .as_str(),
        "replacement-password"
    );
    db.execute_batch("DROP TRIGGER reject_search_keys;")
        .unwrap();
    settings.save(index, Some(KeyAction::Delete(id))).unwrap();
    let after_delete = SearchSettings::load(&root).unwrap();
    assert!(after_delete.stored.configs[index].keys.is_empty());
    assert_eq!(after_delete.stored.configs[index].username, "local-user");
    drop(db);
    drop(after_delete);
    drop(restored);
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_card_preserves_saved_credentials_and_empty_address_restores_default() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-card-failure-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    settings.drafts[0].key = "saved-key".into();
    settings.save(0, Some(KeyAction::Add)).unwrap();
    let ciphertext = settings.stored.configs[0].keys[0].ciphertext.clone();
    settings.drafts[0].endpoint = "not a URL".into();
    assert!(settings.save(0, None).is_err());
    assert_eq!(settings.stored.configs[0].keys[0].ciphertext, ciphertext);
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(restored.stored.configs[0].keys.len(), 1);
    assert_eq!(
        restored.stored.configs[0].endpoint,
        Provider::Tavily.endpoint()
    );
    settings.drafts[0].endpoint.clear();
    settings.save(0, None).unwrap();
    assert_eq!(
        settings.stored.configs[0].endpoint,
        Provider::Tavily.endpoint()
    );
    drop(restored);
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_uses_saved_snapshot_and_does_not_replace_manual_results() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-check-{}",
        lumakit_support::new_id().unwrap()
    ));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&bytes).contains("web search"));
        assert!(!String::from_utf8_lossy(&bytes).contains("unsaved-query"));
        let body =
            r#"{"results":[{"title":"checked","url":"https://example.org","content":"body"}]}"#;
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let mut settings = SearchSettings::load(&root).unwrap();
    settings.drafts[0].endpoint = endpoint;
    settings.save(0, None).unwrap();
    settings.drafts[0].key = "check-key".into();
    settings.save(0, Some(KeyAction::Add)).unwrap();
    settings.query = "unsaved-query".into();
    settings.result = Some(SearchResult {
        query: "previous".into(),
        provider: "tavily".into(),
        hits: Vec::new(),
    });
    settings.launch(false, Some(0), None);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while settings.is_busy() && std::time::Instant::now() < deadline {
        settings.poll();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!settings.is_busy());
    assert!(!settings.card_error[0]);
    assert_eq!(settings.result.as_ref().unwrap().query, "previous");
    server.join().unwrap();
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn preferred_key_survives_restart_delete_and_v1_database_upgrade() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-preferred-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    for value in ["key-a", "key-b", "key-c"] {
        settings.drafts[0].key = value.into();
        settings.save(0, Some(KeyAction::Add)).unwrap();
    }
    let ids: Vec<String> = settings.stored.configs[0]
        .keys
        .iter()
        .map(|key| key.id.clone())
        .collect();
    // The first saved key becomes preferred; choosing another persists across restarts.
    assert_eq!(
        settings.stored.configs[0].selected_key.as_ref(),
        Some(&ids[0])
    );
    assert!(
        settings
            .save(0, Some(KeyAction::Select("missing".into())))
            .is_err()
    );
    settings
        .save(0, Some(KeyAction::Select(ids[1].clone())))
        .unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(
        restored.stored.configs[0].selected_key.as_ref(),
        Some(&ids[1])
    );
    // Deleting the preferred key falls back to the first remaining key.
    settings
        .save(0, Some(KeyAction::Delete(ids[1].clone())))
        .unwrap();
    let restored = SearchSettings::load(&root).unwrap();
    assert_eq!(
        restored.stored.configs[0].selected_key.as_ref(),
        Some(&ids[0])
    );
    assert_eq!(restored.stored.configs[0].keys.len(), 2);
    drop(restored);
    drop(settings);
    // A schema-1 database has no preference column: it upgrades with keys kept and none chosen.
    let db = rusqlite::Connection::open(root.join("settings.db")).unwrap();
    db.execute_batch("ALTER TABLE search_configs DROP COLUMN selected_key; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let upgraded = SearchSettings::load(&root).unwrap();
    assert_eq!(upgraded.stored.configs[0].keys.len(), 2);
    assert_eq!(upgraded.stored.configs[0].selected_key, None);
    assert_eq!(
        upgraded.stored.configs[0].keys[0].open().unwrap().as_str(),
        "key-a"
    );
    drop(upgraded);
    // A preference naming no saved key is corruption, not something to repair silently.
    let db = rusqlite::Connection::open(root.join("settings.db")).unwrap();
    db.execute_batch("UPDATE search_configs SET selected_key='missing' WHERE position=0;")
        .unwrap();
    drop(db);
    assert!(SearchSettings::load(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

pub(super) fn test_trace() -> lumakit_observability::TraceContext {
    let (trace, root) = lumakit_observability::TraceContext::root("test.search", "ui");
    root.finish("REGISTERED");
    trace
}

#[test]
fn failed_read_exposes_same_diagnostic_for_status_and_copy_action() {
    let root = std::env::temp_dir().join(format!(
        "lumadesk-search-diagnostic-{}",
        lumakit_support::secrets::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    settings.fetch_index = settings
        .stored
        .configs
        .iter()
        .position(|config| config.provider == Provider::Direct)
        .unwrap();
    settings.target = format!("http://{address}/private-query");
    settings.launch(true, None, None);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while settings.is_busy() {
        assert!(std::time::Instant::now() < deadline);
        settings.poll();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(settings.page.is_none());
    let diagnostic = settings.diagnostic_trace.as_deref().unwrap();
    assert_eq!(diagnostic.len(), 32);
    assert!(settings.status.contains(diagnostic));
    assert!(!settings.status.contains("private-query"));
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}
