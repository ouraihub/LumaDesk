//! Real loopback protocol tests; no vendor credentials or external traffic.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

fn server(body: String, content_type: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&chunk[..n]);
            if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let size = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + size {
                    break;
                }
            }
        }
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        String::from_utf8(request).unwrap()
    });
    (address, worker)
}
fn config(provider: Provider, endpoint: String, key: bool) -> Config {
    Config {
        provider,
        endpoint,
        username: if provider == Provider::Searxng {
            "alice".into()
        } else {
            String::new()
        },
        keys: if key {
            vec![SavedKey::new("test-key").unwrap()]
        } else {
            Vec::new()
        },
    }
}
fn run<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
fn mcp_value() -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"Title: Found\nURL: https://example.org/source\nHighlights: intro\n\nbody\nTitle: Second\nURL: https://example.org/second\nText: second"}]}})
}
#[test]
fn every_search_provider_sends_its_protocol_and_returns_sources() {
    use serde_json::json;
    for provider in catalog::SEARCH_PROVIDERS {
        let row = json!({"title":"Found","name":"Found","url":"https://example.org/source","content":"body","text":"body","summary":"body","snippet":"body","description":"body","excerpts":["body"],"markdown":"body"});
        let value = match provider {
            Provider::Zhipu => json!({"search_result":[row]}),
            Provider::Bocha => json!({"code":200,"data":{"webPages":{"value":[row]}}}),
            Provider::Querit => json!({"error_code":200,"results":{"result":[row]}}),
            Provider::Brave => json!({"web":{"results":[row]}}),
            Provider::JinaSearch => json!({"data":[row]}),
            Provider::Firecrawl => json!({"success":true,"data":{"web":[row]}}),
            Provider::ExaMcp => mcp_value(),
            _ => json!({"results":[row]}),
        };
        let (endpoint, worker) = server(value.to_string(), "application/json");
        let mut prefs = SearchPreferences::default();
        prefs.max_results = 50;
        let result = run(search_query(
            config(provider, endpoint, true),
            "rust async".into(),
            prefs,
            CancellationToken::new(),
        ))
        .unwrap();
        let request = worker.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        let first = headers.lines().next().unwrap();
        let lower = headers.to_ascii_lowercase();
        let path = match provider {
            Provider::Zhipu | Provider::ExaMcp => "/",
            Provider::Tavily | Provider::Exa | Provider::Searxng => "/search",
            Provider::Bocha => "/v1/web-search",
            Provider::Querit | Provider::Parallel => "/v1/search",
            Provider::Brave => "/res/v1/web/search",
            Provider::JinaSearch => "/rust%20async",
            Provider::Firecrawl => "/v2/search",
            _ => unreachable!(),
        };
        assert!(first.contains(path), "{provider:?}: {first}");
        if provider == Provider::Searxng {
            assert!(first.starts_with("GET /search?q=rust+async&format=json&language=auto "));
            assert!(
                lower.contains(
                    "authorization: basic ywxpY2U6dGVzdC1rZXk="
                        .to_ascii_lowercase()
                        .as_str()
                )
            );
        } else if provider == Provider::Brave {
            assert!(first.starts_with("GET "));
            assert!(first.contains("count=20"));
            assert!(lower.contains("x-subscription-token: test-key"));
        } else {
            let auth = if matches!(
                provider,
                Provider::Exa | Provider::ExaMcp | Provider::Parallel
            ) {
                "x-api-key: test-key"
            } else {
                "authorization: bearer test-key"
            };
            assert!(lower.contains(auth), "{provider:?}");
            if provider == Provider::JinaSearch {
                assert!(first.starts_with("GET "));
            } else {
                assert!(first.starts_with("POST "));
                let payload: serde_json::Value = serde_json::from_str(body).unwrap();
                let query = match provider {
                    Provider::Zhipu => &payload["search_query"],
                    Provider::Parallel => &payload["objective"],
                    Provider::ExaMcp => &payload["params"]["arguments"]["query"],
                    _ => &payload["query"],
                };
                assert_eq!(query, "rust async", "{provider:?}");
                if provider == Provider::ExaMcp {
                    assert_eq!(payload["method"], "tools/call");
                    assert_eq!(payload["params"]["name"], "web_search_exa");
                    assert!(lower.contains("text/event-stream"));
                }
            }
        }
        assert_eq!(result.hits[0].title, "Found", "{provider:?}");
        assert_eq!(result.hits[0].url, "https://example.org/source");
        assert_eq!(
            result.hits[0].snippet,
            if provider == Provider::ExaMcp {
                "intro\n\nbody"
            } else {
                "body"
            }
        );
        if provider == Provider::ExaMcp {
            assert_eq!(result.hits[1].title, "Second");
        }
    }
}
#[test]
fn every_fetch_provider_sends_target_and_decodes_content() {
    for provider in catalog::FETCH_PROVIDERS {
        let (body, kind) = match provider {
            Provider::Direct => (
                "<title>Found</title><p>body</p><script>hidden()</script>".into(),
                "text/html",
            ),
            Provider::Jina => ("Title: Found\n\nbody".into(), "text/plain"),
            Provider::QueritFetch => (
                r#"{"results":[{"title":"Found","content":"body"}]}"#.into(),
                "application/json",
            ),
            Provider::FirecrawlFetch => (
                r#"{"success":true,"data":{"metadata":{"title":"Found"},"markdown":"body"}}"#
                    .into(),
                "application/json",
            ),
            _ => unreachable!(),
        };
        let (endpoint, worker) = server(body, kind);
        let target = if provider == Provider::Direct {
            format!("{endpoint}/page")
        } else {
            "https://example.org/page".into()
        };
        let page = run(fetch_page(
            config(provider, endpoint, provider.requires_key()),
            target.clone(),
            SearchPreferences::default(),
            CancellationToken::new(),
        ))
        .unwrap();
        let request = worker.join().unwrap();
        match provider {
            Provider::Direct => {
                assert!(request.starts_with("GET /page "));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
            }
            Provider::Jina => assert!(request.starts_with("GET /https://example.org/page ")),
            Provider::QueritFetch => {
                assert!(request.starts_with("POST /v1/contents "));
                let body: serde_json::Value =
                    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body["urls"][0], target);
            }
            Provider::FirecrawlFetch => {
                assert!(request.starts_with("POST /v2/scrape "));
                let body: serde_json::Value =
                    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body["url"], target);
            }
            _ => unreachable!(),
        }
        assert_eq!(page.title, "Found");
        assert_eq!(page.url, target);
        if provider == Provider::Direct {
            assert!(page.body.contains("body"));
            assert!(!page.body.contains("hidden()"));
            assert!(!page.body.contains('<'));
        } else {
            assert_eq!(page.body, if provider == Provider::Jina { "Title: Found\n\nbody" } else { "body" });
        }
    }
}
#[test]
fn mcp_sse_multiline_json_and_errors_remain_bounded_contracts() {
    let body = format!("event: message\ndata: {}\n\n", mcp_value());
    let (endpoint, worker) = server(body, "text/event-stream");
    let result = run(search_query(
        config(Provider::ExaMcp, endpoint, false),
        "rust".into(),
        SearchPreferences::default(),
        CancellationToken::new(),
    ))
    .unwrap();
    worker.join().unwrap();
    assert_eq!(result.hits[1].title, "Second");
    assert_eq!(result.hits[0].snippet, "intro\n\nbody");
    let pretty = serde_json::to_string_pretty(&mcp_value()).unwrap();
    assert_eq!(
        parse_mcp(&pretty).unwrap()[1].url,
        "https://example.org/second"
    );
    for body in [
        r#"{"error":{"message":"private-key"}}"#,
        r#"{"result":{"isError":true,"content":[]}}"#,
    ] {
        let error = parse_mcp(body).unwrap_err();
        assert!(!error.contains("private-key"));
    }
}
