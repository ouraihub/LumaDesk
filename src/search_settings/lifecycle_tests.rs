//! Fault-injected persistence and completed-worker outcomes, including real file logs.
use super::*;
use crate::settings_task::{self as task, Failure};
use lumakit_observability::{LogHandle, LogLevel, TraceContext, parse_line};

fn finished_job(result: Result<Output, Failure>, discard: bool) -> Job {
    let (trace, root) = TraceContext::root("settings.search.query", "ui");
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        assert!(sender.send(result).is_ok());
    });
    Job {
        cancel: CancellationToken::new(),
        thread,
        receiver,
        discard,
        check: None,
        trace,
        root,
    }
}

#[test]
fn completed_failure_beats_cancel_discard_and_close() {
    for (discard, closed) in [(false, None), (true, None), (true, Some("settings_closed"))] {
        let job = finished_job(Err(Failure::Http(503)), discard);
        job.cancel.cancel();
        let (_, result) = worker::complete_job(job, closed);
        assert!(matches!(result, Err(Failure::Http(503))));
    }
    let job = finished_job(
        Ok(Output::Fetch(Page {
            title: "discarded".into(),
            url: String::new(),
            body: "never apply".into(),
        })),
        true,
    );
    assert!(matches!(
        worker::complete_job(job, None).1,
        Err(Failure::Cancelled)
    ));
}

#[test]
fn discarded_failure_is_still_visible_after_page_reopens() {
    let root = std::env::temp_dir().join(format!(
        "search-discard-{}",
        lumakit_support::new_id().unwrap()
    ));
    let mut settings = SearchSettings::load(&root).unwrap();
    settings.job = Some(finished_job(Err(Failure::Http(503)), true));
    settings.job.as_ref().unwrap().cancel.cancel();
    while !settings.job.as_ref().unwrap().thread.is_finished() {
        thread::yield_now();
    }
    settings.drain();
    assert!(settings.status.contains("503"));
    assert!(
        settings
            .status
            .contains(settings.diagnostic_trace.as_deref().unwrap())
    );
    assert!(settings.result.is_none());
    drop(settings);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_faults_record_one_final_error_without_private_storage_details() {
    const CHILD: &str = "LUMADESK_SEARCH_LOG_CHILD";
    let root = if let Some(root) = std::env::var_os(CHILD) {
        PathBuf::from(root)
    } else {
        let root =
            std::env::temp_dir().join(format!("search-log-{}", lumakit_support::new_id().unwrap()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "search_settings::lifecycle_tests::search_faults_record_one_final_error_without_private_storage_details", "--nocapture"])
            .env(CHILD, &root).output().unwrap();
        assert!(
            output.status.success(),
            "child status: {}\nstdout:\n{}\nstderr:\n{}\napp.log:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            std::fs::read_to_string(root.join("logs/app.log"))
                .unwrap_or_else(|error| format!("unable to read child log: {:?}", error.kind()))
        );
        std::fs::remove_dir_all(root).unwrap();
        return;
    };
    let logs = LogHandle::init(&root.join("logs"), LogLevel::Debug, 4 * 1024 * 1024).unwrap();
    let data = root.join("private-root-marker");
    let mut settings = SearchSettings::load(&data).unwrap();
    let original = settings.stored.configs[0].endpoint.clone();
    let db = rusqlite::Connection::open(data.join("settings.db")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_search_write BEFORE INSERT ON search_configs BEGIN SELECT RAISE(ABORT,'secret-sql-payload-marker'); END;").unwrap();
    settings.drafts[0].endpoint =
        "https://example.com/private-path-marker/private-query-marker".into();
    let notice = settings.save(0, None).unwrap_err();
    let failed_trace = settings.card_diagnostic_trace[0].clone().unwrap();
    assert!(notice.contains("SETTINGS_STORE_WRITE_FAILED"), "{notice}");
    assert!(notice.contains(&failed_trace));
    assert!(!notice.contains("secret-sql-payload-marker"));
    assert_eq!(settings.stored.configs[0].endpoint, original);
    assert_eq!(
        SearchSettings::load(&data).unwrap().stored.configs[0].endpoint,
        original
    );
    settings.drafts[0].endpoint = "invalid-private-endpoint-marker".into();
    let (trace, guard) = TraceContext::root("settings.search.check", "ui");
    let invalid_trace = trace.trace_id().to_owned();
    let result = {
        let _entered = guard.span().clone().entered();
        settings.save_with_trace(0, None, &trace)
    };
    assert_eq!(
        result.as_ref().unwrap_err().code(),
        "SETTINGS_INVALID_INPUT"
    );
    task::end(guard, &result);
    let job = finished_job(Err(Failure::Http(503)), true);
    let cancelled_trace = job.trace.trace_id().to_owned();
    job.cancel.cancel();
    assert!(matches!(
        worker::complete_job(job, Some("settings_closed")).1,
        Err(Failure::Http(503))
    ));
    let (panic_trace, panic_root) = TraceContext::root("settings.search.query", "ui");
    let panic_id = panic_trace.trace_id().to_owned();
    let span = panic_root.span().clone();
    let (sender, receiver) = mpsc::channel();
    let panicked = thread::spawn(move || {
        let _entered = span.entered();
        drop(sender);
        panic!("private-panic-payload-marker");
    });
    let job = Job {
        cancel: CancellationToken::new(),
        thread: panicked,
        receiver,
        discard: true,
        check: None,
        trace: panic_trace,
        root: panic_root,
    };
    job.cancel.cancel();
    assert!(matches!(
        worker::complete_job(job, Some("settings_closed")).1,
        Err(Failure::Failed {
            code: "SETTINGS_WORKER_PANICKED",
            ..
        })
    ));
    logs.flush().unwrap();
    let text = std::fs::read_to_string(logs.status().path).unwrap();
    let lines: Vec<_> = text.lines().map(parse_line).collect();
    for id in [&failed_trace, &invalid_trace, &cancelled_trace, &panic_id] {
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.trace_id.as_ref() == Some(id) && line.level == "ERROR")
                .count(),
            1,
            "{text}"
        );
    }
    assert!(
        lines
            .iter()
            .any(|line| line.trace_id.as_ref() == Some(&failed_trace)
                && line.operation.as_deref() == Some("settings.search.store.save")
                && line.level == "WARN")
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.trace_id.as_ref() == Some(&invalid_trace)
                && line.operation.as_deref() == Some("settings.search.store.save"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.trace_id.as_ref() == Some(&cancelled_trace)
                && line.event.as_deref() == Some("CANCEL"))
    );
    assert!(text.contains("RUST_PANIC"));
    for private in [
        "secret-sql-payload-marker",
        "private-root-marker",
        "private-path-marker",
        "private-query-marker",
        "invalid-private-endpoint-marker",
        "private-panic-payload-marker",
    ] {
        assert!(!text.contains(private), "leaked {private}");
    }
    drop(db);
    drop(settings);
}

#[test]
fn disconnected_worker_is_failure_even_after_cancel() {
    let (trace, root) = TraceContext::root("settings.search.query", "ui");
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || drop(sender));
    let job = Job {
        cancel: CancellationToken::new(),
        thread,
        receiver,
        discard: true,
        check: None,
        trace,
        root,
    };
    job.cancel.cancel();
    assert!(matches!(
        worker::complete_job(job, Some("settings_closed")).1,
        Err(Failure::Failed {
            code: "SETTINGS_WORKER_DISCONNECTED",
            ..
        })
    ));
}
