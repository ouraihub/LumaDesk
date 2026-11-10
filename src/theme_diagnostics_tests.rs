use super::*;

#[test]
fn appearance_failure_keeps_preferences_and_records_one_safe_final_error() {
    const CHILD: &str = "LUMADESK_APPEARANCE_LOG_CHILD";
    let root = if let Some(path) = std::env::var_os(CHILD) {
        PathBuf::from(path)
    } else {
        let root = std::env::temp_dir().join(format!(
            "appearance-diagnostic-{}",
            lumakit_support::new_id().unwrap()
        ));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "theme::diagnostics_tests::appearance_failure_keeps_preferences_and_records_one_safe_final_error", "--nocapture"])
            .env(CHILD, &root).output().unwrap();
        assert!(
            output.status.success(),
            "status={} stdout={} stderr={} log={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            std::fs::read_to_string(root.join("logs/app.log")).unwrap_or_default()
        );
        std::fs::remove_dir_all(root).unwrap();
        return;
    };
    let logs = lumakit_observability::LogHandle::init(
        &root.join("logs"),
        lumakit_observability::LogLevel::Debug,
        4 * 1024 * 1024,
    )
    .unwrap();
    let mut appearance = Appearance::load(&root).unwrap();
    let before = appearance.saved.clone();
    let database = rusqlite::Connection::open(root.join("settings.db")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_appearance BEFORE UPDATE ON prefs BEGIN SELECT RAISE(ABORT,'PRIVATE_SQL_PAYLOAD'); END;").unwrap();
    let error = appearance
        .save(Preferences {
            theme_id: "light".into(),
            ..before.clone()
        })
        .unwrap_err();
    assert_eq!(appearance.saved, before);
    assert!(error.contains("SETTINGS_APPEARANCE_SAVE_FAILED"));
    assert!(!error.contains("PRIVATE_SQL_PAYLOAD"));
    logs.flush().unwrap();
    let log = std::fs::read_to_string(logs.status().path).unwrap();
    let lines: Vec<_> = log.lines().map(lumakit_observability::parse_line).collect();
    let errors: Vec<_> = lines.iter().filter(|line| line.level == "ERROR").collect();
    assert_eq!(errors.len(), 1);
    let trace = errors[0].trace_id.as_deref().unwrap();
    assert!(error.contains(trace));
    assert!(
        lines
            .iter()
            .any(|line| line.trace_id.as_deref() == Some(trace)
                && line.level == "WARN"
                && line.event.as_deref() == Some("ERROR"))
    );
    assert!(!log.contains("PRIVATE_SQL_PAYLOAD"));
}
