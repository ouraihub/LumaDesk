use super::*;
use lumakit_observability::{LogHandle, LogLevel, parse_line};

#[test]
fn catalog_failures_log_one_root_error_with_safe_storage_children() {
    const CHILD: &str = "LUMADESK_MODEL_LOG_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let path = std::env::temp_dir().join(format!("lumadesk-model-logs-{}", new_id().unwrap()));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "model_settings::logging_tests::catalog_failures_log_one_root_error_with_safe_storage_children", "--test-threads=1"])
            .env(CHILD, &path)
            .status().unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(path).unwrap();
        return;
    }
    let path = std::path::PathBuf::from(std::env::var_os(CHILD).unwrap());
    let log_path = path.join("logs");
    let logs = LogHandle::init(&log_path, LogLevel::Debug, 1024 * 1024).unwrap();
    let mut settings = ModelSettings::load(&path).unwrap();
    let provider = settings.saved.providers[0].id.clone();
    let endpoint = settings.saved.providers[0].endpoints[0].id.clone();
    let db = rusqlite::Connection::open(path.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_sync BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'SQL_SECRET_MARKER'); END;").unwrap();
    settings.sync_dialog = Some(SyncDialog {
        provider,
        endpoint,
        ids: vec!["MODEL_SECRET_MARKER".into()],
        selected: HashSet::new(),
        search: String::new(),
        types: HashMap::new(),
        filter: None,
        folded: HashSet::new(),
    });
    settings.save_sync(
        HashSet::from(["MODEL_SECRET_MARKER".into()]),
        HashMap::new(),
    );
    let failed_trace = settings.diagnostic_trace.clone().unwrap();
    assert!(settings.notice.contains("SETTINGS_STORE_WRITE_FAILED"));
    assert!(!settings.notice.contains("SQL_SECRET_MARKER"));
    let lines = logs.tail(500).unwrap();
    let parsed: Vec<_> = lines.iter().map(|line| parse_line(line)).collect();
    let failure: Vec<_> = parsed
        .iter()
        .filter(|line| line.trace_id.as_deref() == Some(failed_trace.as_str()))
        .collect();
    assert_eq!(
        failure.iter().filter(|line| line.level == "ERROR").count(),
        1
    );
    assert!(failure.iter().any(|line| line.level == "INFO"
        && line.event.as_deref() == Some("ENTRY")
        && line.operation.as_deref() == Some("settings.models.apply")));
    assert!(failure.iter().any(|line| line.level == "WARN"
        && line.event.as_deref() == Some("ERROR")
        && line.operation.as_deref() == Some("settings.models.store.save")));
    assert!(
        failure
            .iter()
            .all(|line| line.event.as_deref() != Some("UNFINISHED"))
    );
    assert!(
        lines.iter().all(
            |line| !line.contains("SQL_SECRET_MARKER") && !line.contains("MODEL_SECRET_MARKER")
        )
    );
    assert!(parsed.iter().any(|line| line.level == "INFO"
        && line.event.as_deref() == Some("EXIT")
        && line.operation.as_deref() == Some("settings.models.load")));
    let (trace, root) = lumakit_observability::TraceContext::root("test.models.panic", "ui");
    let panic_trace = trace.trace_id().to_owned();
    let span = root.span().clone();
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _entered = span.entered();
        tx.send(Ok(vec!["PANIC_SECRET_MARKER".into()])).unwrap();
        panic!("PANIC_SECRET_MARKER");
    });
    settings.job = Some(Job {
        cancel: CancellationToken::new(),
        thread,
        rx,
        config: DetectConfig {
            provider: settings.saved.providers[0].id.clone(),
            endpoint: settings.saved.providers[0].endpoints[0].clone(),
        },
        sync: true,
        trace,
        root,
    });
    while !settings.job.as_ref().unwrap().thread.is_finished() {
        std::thread::yield_now();
    }
    assert!(settings.poll());
    assert!(settings.notice.contains("SETTINGS_WORKER_PANIC"));
    let panic_lines = logs.tail(500).unwrap();
    let panic_events: Vec<_> = panic_lines
        .iter()
        .map(|line| parse_line(line))
        .filter(|line| line.trace_id.as_deref() == Some(panic_trace.as_str()))
        .collect();
    assert_eq!(
        panic_events
            .iter()
            .filter(|line| line.level == "ERROR")
            .count(),
        1
    );
    assert!(
        panic_events
            .iter()
            .any(|line| line.raw.contains("RUST_PANIC"))
    );
    assert!(
        !panic_events
            .iter()
            .any(|line| line.event.as_deref() == Some("CANCEL"))
    );
    assert!(
        panic_lines
            .iter()
            .all(|line| !line.contains("PANIC_SECRET_MARKER"))
    );
    drop(db);
    drop(settings);
    // The process-wide writer owns its file until child exit; the parent test removes it.
    drop(logs);
}
