use super::*;

#[test]
fn cancellation_does_not_hide_failure_or_program_error() {
    assert!(matches!(
        completed(Ok(()), Err::<(), _>(Failure::Http(403)), true),
        Err(Failure::Http(403))
    ));
    assert!(matches!(
        completed(Ok(()), Ok(()), true),
        Err(Failure::Cancelled)
    ));
    let panic = std::thread::spawn(|| panic!("synthetic worker panic")).join();
    let failure = completed(panic, Err::<(), _>(Failure::Http(503)), true).unwrap_err();
    assert_eq!(failure.code(), "SETTINGS_WORKER_PANIC");
}

#[test]
fn poll_keeps_failure_after_cancel_and_does_not_open_sync_for_panicked_worker() {
    for panic in [false, true] {
        let path =
            std::env::temp_dir().join(format!("lumadesk-probe-outcome-{}", new_id().unwrap()));
        let mut settings = ModelSettings::load(&path).unwrap();
        let config = DetectConfig {
            provider: settings.saved.providers[0].id.clone(),
            endpoint: settings.saved.providers[0].endpoints[0].clone(),
        };
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(if panic {
                Ok(vec!["must-not-apply".into()])
            } else {
                Err(Failure::Http(401))
            })
            .unwrap();
            assert!(!panic, "synthetic worker panic");
        });
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (trace, root) = TraceContext::root("test.models.outcome", "ui");
        settings.job = Some(Job {
            cancel,
            thread,
            rx,
            config,
            sync: true,
            trace,
            root,
        });
        while !settings.job.as_ref().unwrap().thread.is_finished() {
            std::thread::yield_now();
        }
        assert!(settings.poll());
        assert!(settings.error);
        assert!(settings.diagnostic_trace.is_some());
        assert!(settings.sync_dialog.is_none());
        assert!(if panic {
            settings.notice.contains("程序错误")
        } else {
            settings.notice.contains("401")
        });
        assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
        drop(settings);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn sync_write_failure_retains_dialog_and_rolls_back_draft_without_raw_sql() {
    let path = std::env::temp_dir().join(format!("lumadesk-sync-failure-{}", new_id().unwrap()));
    let mut settings = ModelSettings::load(&path).unwrap();
    settings.sync_dialog = Some(SyncDialog {
        provider: settings.saved.providers[0].id.clone(),
        endpoint: settings.saved.providers[0].endpoints[0].id.clone(),
        ids: vec!["new-model".into()],
        selected: HashSet::new(),
        search: String::new(),
        types: HashMap::new(),
        filter: None,
        folded: HashSet::new(),
    });
    let db = rusqlite::Connection::open(path.join("providers.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_sync BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'synthetic-private-sql'); END;").unwrap();
    settings.save_sync(HashSet::from(["new-model".into()]), HashMap::new());
    assert!(settings.error);
    assert!(settings.diagnostic_trace.is_some());
    assert!(!settings.notice.contains("synthetic-private-sql"));
    assert!(settings.draft.providers[0].endpoints[0].models.is_empty());
    assert!(settings.saved.providers[0].endpoints[0].models.is_empty());
    assert!(settings.sync_dialog.as_ref().unwrap().selected.is_empty());
    drop(db);
    drop(settings);
    std::fs::remove_dir_all(path).unwrap();
}
