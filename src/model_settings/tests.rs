//! Model settings persistence, discovery and selection regressions.
use super::*;
#[test]
fn display_name_and_model_type_change_picker_without_changing_wire_id() {
    let mut file = SettingsFile::default(); file.seed_presets();
    file.providers[0].enabled = true;
    let endpoint = &mut file.providers[0].endpoints[0];
    endpoint.models.push(ModelEntry { id: "wire-chat".into(), manual: true, capabilities: unknown_capabilities(),
        settings: lumakit_ai::model_config::ModelSettings { name: Some("显示名称".into()), group_name: Some("自定义分组".into()), ..Default::default() } });
    endpoint.models.push(ModelEntry { id: "image-model".into(), manual: true, capabilities: unknown_capabilities(),
        settings: lumakit_ai::model_config::ModelSettings { model_type: ModelType::Image, ..Default::default() } });
    let options = model_options(&file);
    assert_eq!(options.len(), 1); assert_eq!(options[0].name, "显示名称"); assert_eq!(options[0].model.model, "wire-chat");
    assert_eq!(file.providers[0].endpoints[0].models[0].settings.group_name("wire-chat"), "自定义分组");
    let images = model_options_for(&file, ModelType::Image);
    assert_eq!(images.iter().map(|option| option.model.model.as_str()).collect::<Vec<_>>(), ["image-model"]);
}

#[cfg(windows)]
#[test]
fn key_reveal_replace_and_failure_preserve_secret_contract() {
    let root = std::env::temp_dir().join(format!("lumadesk-inline-key-test-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    let key = SavedKey::new("inline-original").unwrap(); let id = key.id.clone();
    let endpoint = settings.draft.providers[0].endpoints[0].id.clone();
    settings.draft.providers[0].endpoints[0].keys.push(key); settings.save().unwrap();
    settings.reveal_key(0, 0, &id);
    assert_eq!(settings.key_drafts[&endpoint].revealed.as_ref().unwrap().as_str(), "inline-original");
    settings.reveal_key(0, 0, "missing");
    assert!(settings.key_drafts[&endpoint].revealed.is_none());
    let draft = settings.key_drafts.get_mut(&endpoint).unwrap(); draft.editing = Some(id.clone()); *draft.value = "replacement".into();
    let original_url = settings.saved.providers[0].endpoints[0].base_url.clone();
    settings.draft.providers[0].endpoints[0].base_url = "unfinished address".into(); settings.dirty = true;
    settings.save_key(0, 0);
    assert!(settings.key_drafts[&endpoint].value.is_empty() && settings.key_drafts[&endpoint].revealed.is_none());
    let restored = ModelSettings::load(&root).unwrap();
    assert_eq!(restored.saved.providers[0].endpoints[0].keys[0].id, id);
    assert_eq!(restored.saved.providers[0].endpoints[0].keys[0].open().unwrap().as_str(), "replacement");
    assert_eq!(restored.saved.providers[0].endpoints[0].base_url, original_url);
    assert_eq!(settings.draft.providers[0].endpoints[0].base_url, "unfinished address"); assert!(settings.dirty);
    let draft = settings.key_drafts.get_mut(&endpoint).unwrap(); draft.editing = Some(id.clone()); *draft.value = "uncommitted".into();
    let db = rusqlite::Connection::open(root.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_key BEFORE UPDATE ON endpoint_keys BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
    settings.save_key(0, 0);
    assert_eq!(settings.draft.providers[0].endpoints[0].keys[0].open().unwrap().as_str(), "replacement");
    assert_eq!(settings.key_drafts[&endpoint].value.as_str(), "uncommitted");
    settings.clear_drafts(); assert!(settings.key_drafts.is_empty());
    drop(db); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn editor_cancel_and_save_keep_other_endpoint() {
    let root = std::env::temp_dir().join(format!("lumadesk-editor-test-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    let provider = settings.saved.providers[0].id.clone();
    let endpoint = settings.saved.providers[0].endpoints[0].id.clone();
    let original = serde_json::to_value(settings.store.load().unwrap().0).unwrap();
    settings.editor = Some(EditorDialog::Endpoint { provider: provider.clone(), id: Some(endpoint.clone()), name: "edited".into(), url: "https://example.com/v1".into(), protocol: Protocol::OpenAiResponses });
    settings.clear_drafts();
    assert_eq!(serde_json::to_value(settings.store.load().unwrap().0).unwrap(), original);
    let before_other = serde_json::to_value(&settings.saved.providers[1]).unwrap();
    let model = EditorDialog::Model { provider, endpoint, original: None, value: ModelEntry { id: "manual-model".into(), capabilities: ModelCapabilities { streaming: true, tools: false, vision: true }, manual: true, settings: Default::default() } };
    settings.apply_editor(&model).unwrap(); settings.save().unwrap();
    let restored = ModelSettings::load(&root).unwrap();
    assert_eq!(restored.saved.providers[0].endpoints[0].models[0].id, "manual-model");
    assert!(restored.saved.providers[0].endpoints[0].models[0].capabilities.vision);
    assert_eq!(serde_json::to_value(&restored.saved.providers[1]).unwrap(), before_other);
    assert!(settings.apply_editor(&model).is_err());
    drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn enable_is_independent_of_invalid_address_and_restores_on_write_failure() {
    let root = std::env::temp_dir().join(format!("lumadesk-enable-test-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    let original_url = settings.saved.providers[0].endpoints[0].base_url.clone();
    settings.draft.providers[0].endpoints[0].base_url = "unfinished".into(); settings.dirty = true;
    settings.set_enabled(0, true);
    assert!(settings.saved.providers[0].enabled);
    assert_eq!(settings.saved.providers[0].endpoints[0].base_url, original_url);
    assert_eq!(settings.draft.providers[0].endpoints[0].base_url, "unfinished");
    let restored = ModelSettings::load(&root).unwrap(); assert!(restored.saved.providers[0].enabled);
    let db = rusqlite::Connection::open(root.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_enable BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
    settings.set_enabled(0, false);
    assert!(settings.error && settings.saved.providers[0].enabled && settings.draft.providers[0].enabled);
    drop(db); drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_infers_new_types_but_preserves_saved_model_settings() {
    let root = std::env::temp_dir().join(format!("lumadesk-sync-types-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    let mut existing = ModelEntry { id: "gpt-image-custom".into(), capabilities: unknown_capabilities(), manual: true, settings: Default::default() };
    existing.settings.name = Some("用户自定义对话模型".into());
    settings.draft.providers[0].endpoints[0].models.push(existing);
    settings.save().unwrap();
    let endpoint = settings.saved.providers[0].endpoints[0].clone();
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(vec!["gpt-image-2".into(), "text-embedding-3-large".into(), "gpt-image-custom".into()])).unwrap();
    settings.job = Some(Job {
        cancel: CancellationToken::new(), thread: std::thread::spawn(|| {}), rx,
        config: DetectConfig { provider: settings.saved.providers[0].id.clone(), endpoint }, sync: true,
    });
    assert!(settings.poll());
    let dialog = settings.sync_dialog.as_ref().unwrap();
    assert_eq!(dialog.types["gpt-image-2"], ModelType::Image);
    assert_eq!(dialog.types["text-embedding-3-large"], ModelType::Embedding);
    assert_eq!(dialog.types["gpt-image-custom"], ModelType::Chat);
    assert_eq!(dialog.selected, HashSet::from(["gpt-image-custom".to_owned()]));
    assert_eq!(settings.saved.providers[0].endpoints[0].models[0].settings.name.as_deref(), Some("用户自定义对话模型"));
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_preserves_manual_identity_and_capabilities() {
    let mut models = vec![ModelEntry { id: "manual".into(), capabilities: ModelCapabilities { streaming: true, tools: true, vision: false }, manual: true, settings: Default::default() }];
    merge_models(&mut models, &["manual".into(), "remote".into()], &HashSet::from(["manual".into(), "remote".into()]));
    assert_eq!(models.len(), 2); assert_eq!(models[0].id, "manual"); assert!(models[0].capabilities.tools); assert!(!models[1].capabilities.streaming);
    assert!(parse_models(br#"{"data":[{"id":"a"},{"id":"a"},{"id":"b"}]}"#).is_ok_and(|ids| ids.len() == 2));
    assert!(parse_models(br#"{"data":[{"id":""}]}"#).is_err());
    assert!(validate_model_id(" bad ").is_err()); assert!(validate_model_id("line\nbreak").is_err());
}
#[test]
fn empty_save_reload_and_url_conventions() {
    let root = std::env::temp_dir().join(format!("lumadesk-model-test-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    assert_eq!(settings.saved.providers.len(), 3);
    assert!(settings.saved.providers.iter().all(|p| !p.enabled && p.endpoints.len() == 1 && p.endpoints[0].keys.is_empty() && p.endpoints[0].models.is_empty()));
    settings.draft.providers.clear(); settings.save().unwrap();
    let restored = ModelSettings::load(&root).unwrap(); assert!(restored.saved.providers.is_empty()); assert!(restored.available_models().is_empty());
    assert_eq!(model_url("http://127.0.0.1:1234/v1", Protocol::OpenAiChat).unwrap(), "http://127.0.0.1:1234/v1/models");
    assert_eq!(model_url("http://[::1]:1234", Protocol::AnthropicMessages).unwrap(), "http://[::1]:1234/v1/models");
    assert!(validate_url("https://user:password@example.com").is_err()); assert!(validate_url("http://example.com").is_err());
    assert!(validate_url("https://:password@example.com").is_err());
    assert!(validate_url("https://example.com?key=hidden").is_err());
    let original = serde_json::to_value(settings.store.load().unwrap().0).unwrap();
    settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: String::new(), enabled: true, endpoints: Vec::new() });
    assert!(settings.save().is_err()); assert_eq!(serde_json::to_value(settings.store.load().unwrap().0).unwrap(), original); assert!(settings.saved.providers.is_empty());
    drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn existing_provider_survives_preset_initialization_and_restart() {
    let root = std::env::temp_dir().join(format!("lumadesk-preset-test-{}", new_id().unwrap()));
    let original = serde_json::json!({"providers": [{"id": "user-fenno", "name": "fenno", "enabled": true, "endpoints": []}]});
    lumakit_support::save_versioned(&root.join("model-settings.json"), VERSION, &original).unwrap();
    let mut settings = ModelSettings::load(&root).unwrap();
    assert_eq!(settings.saved.providers.len(), 3);
    assert_eq!(serde_json::to_value(&settings.saved.providers[0]).unwrap(), original["providers"][0]);
    settings.draft.providers.retain(|p| p.id != "preset-qiniu");
    settings.save().unwrap();
    let restored = ModelSettings::load(&root).unwrap();
    assert!(!restored.saved.providers.iter().any(|p| p.id == "preset-qiniu"));
    assert_eq!(restored.saved.providers.iter().filter(|p| p.name.eq_ignore_ascii_case("fenno")).count(), 1);
    drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}
#[cfg(windows)]
#[test]
fn actual_loopback_detection_auth_failover_and_encrypted_reload() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in [("401 Unauthorized", "{}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}"), ("200 OK", "{\"data\":[{\"id\":\"loopback-model\"}]}")] {
            let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap(); let mut bytes = Vec::new();
            loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
            requests.push(String::from_utf8(bytes).unwrap());
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let first = SavedKey::new("first-key").unwrap(); let second = SavedKey::new("second-key").unwrap();
    let endpoint = EndpointEntry { id: new_id().unwrap(), name: "回环".into(), base_url: format!("http://{address}/v1"), protocol: Protocol::OpenAiChat, selected_key: Some(first.id.clone()), keys: vec![first, second], models: Vec::new(), selected_model: None };
    assert_eq!(detect_models(&endpoint, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
    let mut messages = endpoint.clone(); messages.protocol = Protocol::AnthropicMessages; messages.base_url = format!("http://{address}"); messages.keys.remove(0); messages.selected_key = None;
    assert_eq!(detect_models(&messages, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
    let mut responses = messages.clone(); responses.protocol = Protocol::OpenAiResponses; responses.base_url = format!("http://{address}/custom/v1");
    assert_eq!(detect_models(&responses, CancellationToken::new()).unwrap(), vec!["loopback-model"]);
    let cancelled = CancellationToken::new(); cancelled.cancel(); assert!(detect_models(&responses, cancelled).is_err());
    let requests = server.join().unwrap(); assert!(requests[0].starts_with("GET /v1/models ")); assert!(requests[0].to_ascii_lowercase().contains("authorization: bearer first-key")); assert!(requests[1].to_ascii_lowercase().contains("authorization: bearer second-key"));
    assert!(requests[2].starts_with("GET /v1/models ")); assert!(requests[2].to_ascii_lowercase().contains("x-api-key: second-key")); assert!(requests[2].to_ascii_lowercase().contains("anthropic-version: 2023-06-01"));
    assert!(requests[3].starts_with("GET /custom/v1/models ")); assert!(requests[3].to_ascii_lowercase().contains("authorization: bearer second-key"));
    let encoded = serde_json::to_string(&endpoint).unwrap(); assert!(!encoded.contains("first-key")); let restored: EndpointEntry = serde_json::from_str(&encoded).unwrap(); assert_eq!(restored.keys[0].id, endpoint.keys[0].id); assert_eq!(restored.keys[0].open().unwrap().as_str(), "first-key");
    let root = std::env::temp_dir().join(format!("lumadesk-model-keys-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&root).unwrap();
    settings.draft.providers.push(ProviderEntry { id: new_id().unwrap(), name: "回环测试".into(), enabled: true, endpoints: vec![endpoint] });
    settings.save().unwrap();
    let restored = ModelSettings::load(&root).unwrap();
    let restored_endpoint = &restored.saved.providers.last().unwrap().endpoints[0];
    assert_eq!(restored_endpoint.keys[0].id, settings.saved.providers.last().unwrap().endpoints[0].keys[0].id);
    let selected = lumakit_ai::ModelRef { provider: settings.saved.providers.last().unwrap().id.clone(), endpoint: restored_endpoint.id.clone(), model: "loopback-model".into(), protocol: restored_endpoint.protocol };
    assert!(settings.connection(&selected).is_err());
    settings.draft.providers.last_mut().unwrap().endpoints[0].models.push(ModelEntry { id: selected.model.clone(), capabilities: unknown_capabilities(), manual: true, settings: Default::default() });
    assert!(settings.available_models().is_empty());
    settings.save().unwrap();
    assert_eq!(settings.available_models().len(), 1);
    assert_eq!(settings.connection(&selected).unwrap().api_key.unwrap().as_str(), "first-key");
    settings.draft.providers.last_mut().unwrap().enabled = false;
    settings.save().unwrap();
    assert!(settings.connection(&selected).is_err());
    let bytes = std::fs::read(root.join("providers.db")).unwrap(); assert!(!bytes.windows(b"first-key".len()).any(|part| part == b"first-key"));
    drop(restored); drop(settings); std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn bounded_response_and_pending_request_cancellation() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = [0u8; 2048]; let _ = socket.read(&mut request).unwrap();
            if attempt == 0 {
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BODY + 1).unwrap();
            } else {
                accepted_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(5));
            }
        }
    });
    let endpoint = EndpointEntry { id: "test".into(), name: "回环".into(), base_url: format!("http://{address}"), protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None };
    assert!(detect_models(&endpoint, CancellationToken::new()).unwrap_err().contains("2 MiB"));
    let cancel = CancellationToken::new(); let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || detect_models(&endpoint, worker_cancel));
    accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    cancel.cancel();
    assert!(worker.join().unwrap().unwrap_err().contains("取消"));
    release_tx.send(()).unwrap(); server.join().unwrap();
}
#[cfg(windows)]
#[test]
fn anthropic_two_page_catalog_uses_same_origin_and_auth() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for body in [r#"{"data":[{"id":"page-one"}],"has_more":true,"last_id":"page-one"}"#, r#"{"data":[{"id":"page-two"}],"has_more":false,"last_id":"page-two"}"#] {
            let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
            requests.push(String::from_utf8(bytes).unwrap());
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let endpoint = EndpointEntry { id: "pagination-test".into(), name: "分页".into(), base_url: format!("http://{address}"), protocol: Protocol::AnthropicMessages, keys: vec![SavedKey::new("page-key").unwrap()], models: Vec::new(), selected_key: None, selected_model: None };
    assert_eq!(detect_models(&endpoint, CancellationToken::new()).unwrap(), vec!["page-one", "page-two"]);
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models HTTP/1.1"));
    assert!(requests[1].starts_with("GET /v1/models?after_id=page-one HTTP/1.1"));
    for request in requests { let request = request.to_ascii_lowercase(); assert!(request.contains("x-api-key: page-key")); assert!(request.contains("anthropic-version: 2023-06-01")); assert!(request.contains(&format!("host: {address}"))); }
    assert!(model_page(br#"{"data":[],"has_more":true,"last_id":"a"}"#, Protocol::AnthropicMessages).is_err());
    assert!(model_page(br#"{"data":[{"id":"a"}],"has_more":true}"#, Protocol::AnthropicMessages).is_err());
}
#[test]
fn unauthorized_without_saved_key_says_save_the_key_first() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap(); socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = Vec::new();
        loop { let mut part = [0u8; 2048]; let size = socket.read(&mut part).unwrap(); bytes.extend_from_slice(&part[..size]); if size == 0 || bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; } }
        write!(socket, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let endpoint = EndpointEntry { id: "no-key".into(), name: "无密钥".into(), base_url: format!("http://{address}/v1"), protocol: Protocol::OpenAiChat, keys: Vec::new(), models: Vec::new(), selected_key: None, selected_model: None };
    let error = detect_models(&endpoint, CancellationToken::new()).unwrap_err();
    assert!(error.contains("没有已保存的密钥") && error.contains("401"), "{error}");
    assert!(!server.join().unwrap().to_ascii_lowercase().contains("authorization:"));
}
