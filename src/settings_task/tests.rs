//! Exercise cancellation/deadline boundaries with pending real HTTP and typed outcomes.
use super::*;
use std::{io::Read, net::TcpListener, sync::mpsc, thread};

#[test]
fn pending_http_distinguishes_cancel_and_timeout() {
    for cancelled in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 2048];
            assert!(socket.read(&mut request).unwrap() > 0);
            accepted_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let worker = thread::spawn(move || {
            let (trace, root) = TraceContext::root("test.settings.request", "ui");
            let _entered = root.span().clone().entered();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = runtime.block_on(async {
                http(
                    &trace,
                    "test.settings.http",
                    &token,
                    deadline(1_000),
                    1_000,
                    async {
                        reqwest::Client::new()
                            .get(url)
                            .send()
                            .await
                            .map_err(|e| Failure::request(e, 1_000, 1_000))?;
                        Ok(())
                    },
                )
                .await
            });
            end(root, &result);
            result
        });
        accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if cancelled {
            cancel.cancel();
        }
        let error = worker.join().unwrap().unwrap_err();
        if cancelled {
            assert!(matches!(error, Failure::Cancelled));
        } else {
            assert!(matches!(error, Failure::Timeout(1_000)));
        }
        release_tx.send(()).unwrap();
        server.join().unwrap();
    }
    assert_eq!(status(429).code(), "SETTINGS_HTTP_RATE_LIMITED");
    assert_eq!(
        Failure::failed("SETTINGS_STORE_WRITE_FAILED", "private path").code(),
        "SETTINGS_STORE_WRITE_FAILED"
    );
}
