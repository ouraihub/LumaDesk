use super::*;

fn state() -> (std::path::PathBuf, ModelSettings, DetectConfig) {
    let path = std::env::temp_dir().join(format!("lumadesk-refresh-outcome-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&path).unwrap();
    settings.draft.providers[0].enabled = true;
    settings.save().unwrap();
    let config = DetectConfig {
        provider: settings.saved.providers[0].id.clone(),
        endpoint: settings.saved.providers[0].endpoints[0].clone(),
    };
    (path, settings, config)
}

fn inject(settings: &mut ModelSettings, results: Discovery, cancelled: bool, panic: bool) {
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        tx.send(results).unwrap();
        assert!(!panic, "synthetic worker panic");
    });
    let cancel = CancellationToken::new();
    if cancelled {
        cancel.cancel();
    }
    let (trace, root) = TraceContext::root("test.catalog.refresh", "ui");
    settings.catalog_refresh = Some(CatalogRefreshJob {
        cancel,
        thread,
        rx,
        trace,
        root,
    });
    // Wait for delivery without polling or changing the completion boundary under test.
    while !settings
        .catalog_refresh
        .as_ref()
        .unwrap()
        .thread
        .is_finished()
    {
        std::thread::yield_now();
    }
}

#[test]
fn cancellation_preserves_failure_and_panic_never_applies_partial_results() {
    for (cancelled, panic) in [(true, false), (false, true), (true, true)] {
        let (path, mut settings, config) = state();
        let before = serde_json::to_value(&settings.saved.providers).unwrap();
        inject(
            &mut settings,
            vec![
                (config.clone(), Ok(vec!["must-not-apply".into()])),
                (config, Err(Failure::Http(503))),
            ],
            cancelled,
            panic,
        );
        assert!(settings.poll_catalog_refresh());
        let notice = settings.catalog_refresh_notice();
        assert!(if panic {
            notice.contains("程序错误")
        } else {
            notice.contains("503")
        });
        assert!(settings.catalog_refresh_trace_id().is_some());
        assert_eq!(
            serde_json::to_value(&settings.saved.providers).unwrap(),
            before
        );
        assert_eq!(
            serde_json::to_value(settings.store.load().unwrap().0).unwrap(),
            before
        );
        drop(settings);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn all_stale_results_are_discarded_without_saving_or_claiming_application() {
    let (path, mut settings, config) = state();
    settings.draft.providers[0].endpoints[0].base_url = "https://changed.example".into();
    settings.save().unwrap();
    let db = rusqlite::Connection::open(path.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_stale BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'private-sql-marker'); END;").unwrap();
    inject(
        &mut settings,
        vec![(config, Ok(vec!["stale-model".into()]))],
        false,
        false,
    );
    assert!(settings.poll_catalog_refresh());
    assert!(settings.catalog_refresh_notice().contains("没有线路应用"));
    assert!(settings.catalog_refresh_notice().contains("1 条结果已失效"));
    assert!(settings.catalog_refresh_trace_id().is_none());
    assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
    drop(db);
    drop(settings);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn apply_read_and_write_failures_keep_distinct_codes_and_saved_state() {
    let (path, mut settings, config) = state();
    let db = rusqlite::Connection::open(path.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_save BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'private-sql-marker'); END;").unwrap();
    let (trace, root) = TraceContext::root("test.catalog.apply", "ui");
    let error = match settings
        .apply_discovery(vec![(config.clone(), Ok(vec!["new-model".into()]))], &trace)
    {
        Ok(_) => panic!("write unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "SETTINGS_STORE_WRITE_FAILED");
    assert!(!error.to_string().contains("private-sql-marker"));
    assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
    db.execute_batch("DROP TRIGGER deny_save; DROP TABLE providers;")
        .unwrap();
    let error = match settings.apply_discovery(vec![(config, Ok(vec!["new-model".into()]))], &trace)
    {
        Ok(_) => panic!("read unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "SETTINGS_STORE_READ_FAILED");
    assert!(!error.to_string().contains("SQL"));
    assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
    root.finish("CHECKED");
    drop(db);
    drop(settings);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn closed_refresh_preserves_received_failure_and_discards_success() {
    let (path, mut settings, config) = state();
    let error = refresh_completed(
        Ok(()),
        Ok(vec![(config.clone(), Err(Failure::Http(429)))]),
        true,
    )
    .err()
    .unwrap();
    assert!(matches!(error, Failure::Http(429)));
    inject(
        &mut settings,
        vec![(config.clone(), Err(Failure::Http(403)))],
        false,
        false,
    );
    settings.cancel_catalog_refresh();
    assert!(!settings.refreshing_catalog());
    inject(
        &mut settings,
        vec![(config, Ok(vec!["closed-model".into()]))],
        false,
        false,
    );
    settings.cancel_catalog_refresh();
    assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
    drop(settings);
    std::fs::remove_dir_all(path).unwrap();
}
