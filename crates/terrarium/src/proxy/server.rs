use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

pub struct ProxyConfig {
    pub allowed_domains: Arc<RwLock<Vec<String>>>,
    pub project_root: PathBuf,
}

/// Starts the proxy on a random port and returns it. Runs as background tokio tasks.
/// A reload task periodically re-reads the project profile so TUI changes take effect live.
pub async fn start(config: ProxyConfig) -> crate::error::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(crate::error::TerrariumError::Io)?;
    let port = listener.local_addr()?.port();
    let cfg = Arc::new(config);

    // Spawn reload task that picks up profile changes (e.g. from TUI).
    let reload_cfg = Arc::clone(&cfg);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            if let Some(fresh) = load_allowed_domains(&reload_cfg.project_root) {
                *reload_cfg.allowed_domains.write().await = fresh;
            }
        }
    });

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let cfg = Arc::clone(&cfg);
                    tokio::spawn(async move {
                        let _ = handle(stream, cfg).await;
                    });
                }
                Err(e) => eprintln!("terrarium: proxy: accept error: {e}"),
            }
        }
    });
    Ok(port)
}

/// How long a client may take to send its request line and headers.
const HEADER_PHASE_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on a request line plus headers, so a client cannot grow the buffer forever.
const MAX_REQUEST_HEAD_BYTES: usize = 64 * 1024;

struct RequestHead {
    method: String,
    target: String,
    version: String,
    headers: Vec<String>,
}

async fn handle(client: TcpStream, cfg: Arc<ProxyConfig>) -> std::io::Result<()> {
    handle_with_header_timeout(client, cfg, HEADER_PHASE_TIMEOUT).await
}

async fn handle_with_header_timeout(
    mut client: TcpStream,
    cfg: Arc<ProxyConfig>,
    header_timeout: Duration,
) -> std::io::Result<()> {
    let Ok(head) = tokio::time::timeout(header_timeout, read_request_head(&mut client)).await
    else {
        return Ok(());
    };
    let Some(RequestHead {
        method,
        target,
        version,
        headers,
    }) = head?
    else {
        return Ok(());
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        // target is host:port
        let domain = host_without_port(&target).to_lowercase();
        {
            let allowed = cfg.allowed_domains.read().await;
            if !is_allowed(&domain, &allowed) {
                drop(allowed);
                block(&mut client, &domain, &method, &target, &cfg.project_root).await?;
                return Ok(());
            }
        }
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let mut remote = TcpStream::connect(&target).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote).await?;
    } else {
        let host_header = headers.iter().find_map(|line| header_value(line, "host"));
        // The allowlist check and the dial must use the same authority, or a
        // request can be checked against one host and sent to another.
        let authority = authority_from_url(&target)
            .or(host_header)
            .unwrap_or_default()
            .to_lowercase();
        let domain = host_without_port(&authority);

        {
            let allowed = cfg.allowed_domains.read().await;
            if !is_allowed(domain, &allowed) {
                drop(allowed);
                block(&mut client, domain, &method, &target, &cfg.project_root).await?;
                return Ok(());
            }
        }

        let connect_target = if authority.contains(':') {
            authority.clone()
        } else {
            format!("{authority}:80")
        };

        let mut remote = TcpStream::connect(&connect_target).await?;
        remote
            .write_all(single_request_head(&method, &target, &version, &headers).as_bytes())
            .await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote).await?;
    }
    Ok(())
}

/// Reads the request line and headers; `None` when the request line is malformed.
async fn read_request_head(stream: &mut TcpStream) -> std::io::Result<Option<RequestHead>> {
    let mut remaining = MAX_REQUEST_HEAD_BYTES;
    let first_line = read_line(stream, &mut remaining).await?;
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return Ok(None);
    }
    let mut headers = Vec::new();
    loop {
        let line = read_line(stream, &mut remaining).await?;
        if line.is_empty() {
            break;
        }
        headers.push(line);
    }
    Ok(Some(RequestHead {
        method: parts[0].to_string(),
        target: parts[1].to_string(),
        version: parts[2].to_string(),
        headers,
    }))
}

/// The request as sent upstream. Later requests on the client connection may
/// name other hosts, but the bytes after this one are piped to this upstream
/// unchecked, so the upstream is told to close after answering this one.
fn single_request_head(method: &str, target: &str, version: &str, headers: &[String]) -> String {
    let mut head = format!("{method} {target} {version}\r\n");
    for line in headers {
        let hop_by_hop = header_value(line, "connection").is_some()
            || header_value(line, "proxy-connection").is_some();
        if !hop_by_hop {
            head.push_str(line);
            head.push_str("\r\n");
        }
    }
    head.push_str("Connection: close\r\n\r\n");
    head
}

fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let (field, value) = line.split_once(':')?;
    field
        .trim()
        .eq_ignore_ascii_case(name)
        .then(|| value.trim())
}

/// Reads bytes until `\r\n`, returning the line without the terminator.
/// Fails once more than `remaining` bytes have been read across calls.
async fn read_line(stream: &mut TcpStream, remaining: &mut usize) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if *remaining == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
        stream.read_exact(&mut byte).await?;
        *remaining -= 1;
        if byte[0] == b'\n' {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        buf.push(byte[0]);
    }
}

#[cfg(test)]
fn domain_from_url(url: &str) -> Option<&str> {
    authority_from_url(url).map(host_without_port)
}

fn authority_from_url(url: &str) -> Option<&str> {
    let after = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    after.split('/').next()
}

fn host_without_port(authority: &str) -> &str {
    authority.split(':').next().unwrap_or(authority)
}

fn is_allowed(domain: &str, allowed: &[String]) -> bool {
    if domain.is_empty() {
        return false;
    }
    allowed.iter().any(|a| {
        let a = a.to_lowercase();
        domain == a || domain.ends_with(&format!(".{a}"))
    })
}

fn load_allowed_domains(project_root: &Path) -> Option<Vec<String>> {
    if let Ok(ws) = crate::config::workspace::WorkspaceConfig::load(project_root) {
        let mut fresh = ws.network.allowed_domains;
        for project in &ws.projects {
            crate::profile::presets::ensure_defaults(&mut fresh, &project.preset);
        }
        fresh.sort();
        fresh.dedup();
        return Some(fresh);
    }

    let profile = crate::config::project::ProjectProfile::load(project_root).ok()?;
    let mut fresh = profile.network.allowed_domains;
    let preset = profile.preset.as_deref().unwrap_or("none");
    crate::profile::presets::ensure_defaults(&mut fresh, preset);
    fresh.sort();
    fresh.dedup();
    Some(fresh)
}

async fn block(
    client: &mut TcpStream,
    domain: &str,
    method: &str,
    url: &str,
    project_root: &Path,
) -> std::io::Result<()> {
    eprintln!("terrarium: proxy blocked {domain}");
    let body = format!(
        "Blocked by terrarium: {domain} is not in the allow-list.\n\
         Run `terrarium tui` to manage allowed domains.\n"
    );
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {body}",
        body.len()
    );
    client.write_all(response.as_bytes()).await?;
    let root = project_root.to_path_buf();
    let domain = domain.to_string();
    let method = method.to_string();
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        let _ = crate::proxy::log::append(&root, &domain, &method, &url);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_allowed_exact_match() {
        let allowed = vec!["example.com".to_string()];
        assert!(is_allowed("example.com", &allowed));
    }

    #[test]
    fn is_allowed_subdomain_match() {
        let allowed = vec!["example.com".to_string()];
        assert!(is_allowed("sub.example.com", &allowed));
    }

    #[test]
    fn is_allowed_rejects_unmatched() {
        let allowed = vec!["example.com".to_string()];
        assert!(!is_allowed("evil.com", &allowed));
    }

    #[test]
    fn is_allowed_rejects_empty_domain() {
        let allowed = vec!["example.com".to_string()];
        assert!(!is_allowed("", &allowed));
    }

    #[test]
    fn is_allowed_case_insensitive() {
        let allowed = vec!["Example.COM".to_string()];
        assert!(is_allowed("example.com", &allowed));
    }

    #[test]
    fn is_allowed_does_not_match_suffix() {
        let allowed = vec!["example.com".to_string()];
        assert!(!is_allowed("notexample.com", &allowed));
    }

    #[test]
    fn domain_from_url_http() {
        assert_eq!(domain_from_url("http://foo.com/path"), Some("foo.com"));
    }

    #[test]
    fn domain_from_url_https_with_port() {
        assert_eq!(domain_from_url("https://bar.io:443/x"), Some("bar.io"));
    }

    #[test]
    fn domain_from_url_no_scheme() {
        assert_eq!(domain_from_url("foo.com/path"), None);
    }

    #[tokio::test]
    async fn proxy_allows_listed_domain() {
        let allowed = Arc::new(RwLock::new(vec!["httpbin.org".to_string()]));
        let cfg = Arc::new(ProxyConfig {
            allowed_domains: allowed,
            project_root: PathBuf::from("/tmp"),
        });
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let cfg = Arc::clone(&cfg);
            tokio::spawn(async move {
                if let Ok((stream, _)) = listener.accept().await {
                    let _ = handle(stream, cfg).await;
                }
            });
            port
        };
        // Send a CONNECT to an allowed domain — expect 200
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(b"CONNECT httpbin.org:443 HTTP/1.1\r\nHost: httpbin.org\r\n\r\n")
            .await
            .unwrap();
        let mut buf = vec![0u8; 256];
        let n = client.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(response.contains("200"), "expected 200, got: {response}");
    }

    #[tokio::test]
    async fn proxy_blocks_unlisted_domain() {
        let allowed = Arc::new(RwLock::new(vec!["safe.com".to_string()]));
        let cfg = Arc::new(ProxyConfig {
            allowed_domains: allowed,
            project_root: PathBuf::from("/tmp"),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cfg2 = Arc::clone(&cfg);
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let _ = handle(stream, cfg2).await;
            }
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(b"CONNECT evil.com:443 HTTP/1.1\r\nHost: evil.com\r\n\r\n")
            .await
            .unwrap();
        let mut buf = vec![0u8; 256];
        let n = client.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(response.contains("403"), "expected 403, got: {response}");
    }

    #[tokio::test]
    async fn proxy_plain_http_preserves_host_header_port() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = upstream.accept().await {
                let mut buf = vec![0u8; 512];
                let _ = stream.read(&mut buf).await;
                let body = "ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let allowed = Arc::new(RwLock::new(vec!["127.0.0.1".to_string()]));
        let cfg = Arc::new(ProxyConfig {
            allowed_domains: allowed,
            project_root: PathBuf::from("/tmp"),
        });
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let cfg2 = Arc::clone(&cfg);
        tokio::spawn(async move {
            if let Ok((stream, _)) = proxy_listener.accept().await {
                let _ = handle(stream, cfg2).await;
            }
        });

        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{upstream_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = vec![0u8; 512];
        let n = client.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert!(
            response.contains("200 OK"),
            "expected upstream response, got: {response}"
        );
    }

    /// Serves every request on a connection with `body` until a request asks
    /// for `Connection: close`, recording each request line it answered.
    async fn keep_alive_upstream(body: &'static str) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    let mut pending = Vec::new();
                    let mut buf = [0u8; 1024];
                    loop {
                        while let Some(end) = pending.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head: Vec<u8> = pending.drain(..end + 4).collect();
                            let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                            recorded
                                .lock()
                                .unwrap()
                                .push(head.lines().next().unwrap_or_default().to_string());
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                body.len()
                            );
                            if stream.write_all(response.as_bytes()).await.is_err()
                                || head.contains("connection: close")
                            {
                                return;
                            }
                        }
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => pending.extend_from_slice(&buf[..n]),
                        }
                    }
                });
            }
        });
        (port, seen)
    }

    async fn spawn_proxy(allowed: &[&str], header_timeout: Duration) -> u16 {
        let cfg = Arc::new(ProxyConfig {
            allowed_domains: Arc::new(RwLock::new(allowed.iter().map(|d| d.to_string()).collect())),
            project_root: PathBuf::from("/tmp"),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let cfg = Arc::clone(&cfg);
                tokio::spawn(async move {
                    let _ = handle_with_header_timeout(stream, cfg, header_timeout).await;
                });
            }
        });
        port
    }

    /// Reads until the peer closes or `wait` passes; `None` means it never closed.
    async fn read_until_closed(stream: &mut TcpStream, wait: Duration) -> Option<String> {
        let mut received = Vec::new();
        let mut buf = [0u8; 1024];
        let outcome = tokio::time::timeout(wait, async {
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                }
            }
        })
        .await;
        outcome
            .ok()
            .map(|_| String::from_utf8_lossy(&received).into_owned())
    }

    async fn send_through_proxy(proxy_port: u16, request: String) -> String {
        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        client.shutdown().await.unwrap();
        read_until_closed(&mut client, Duration::from_secs(5))
            .await
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn plain_http_reaches_the_url_host_not_a_disagreeing_host_header() {
        let (checked_port, _) = keep_alive_upstream("checked").await;
        let (other_port, _) = keep_alive_upstream("other").await;
        let proxy_port = spawn_proxy(&["127.0.0.1"], HEADER_PHASE_TIMEOUT).await;

        let response = send_through_proxy(
            proxy_port,
            format!(
                "GET http://127.0.0.1:{checked_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{other_port}\r\n\r\n"
            ),
        )
        .await;

        assert!(
            response.ends_with("checked"),
            "expected the upstream named in the URL, got: {response}"
        );
    }

    #[tokio::test]
    async fn plain_http_keeps_the_url_port_when_the_host_header_has_none() {
        let (port, _) = keep_alive_upstream("upstream").await;
        let proxy_port = spawn_proxy(&["127.0.0.1"], HEADER_PHASE_TIMEOUT).await;

        let response = send_through_proxy(
            proxy_port,
            format!("GET http://127.0.0.1:{port}/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
        )
        .await;

        assert!(
            response.ends_with("upstream"),
            "expected the upstream on the URL port, got: {response}"
        );
    }

    #[tokio::test]
    async fn origin_form_request_is_routed_by_its_host_header() {
        let (port, _) = keep_alive_upstream("upstream").await;
        let proxy_port = spawn_proxy(&["127.0.0.1"], HEADER_PHASE_TIMEOUT).await;

        let response = send_through_proxy(
            proxy_port,
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
        )
        .await;

        assert!(
            response.ends_with("upstream"),
            "expected the upstream named by Host, got: {response}"
        );
    }

    #[tokio::test]
    async fn a_reused_connection_never_carries_a_second_request_to_the_first_upstream() {
        let (first_port, first_seen) = keep_alive_upstream("first").await;
        let (second_port, _) = keep_alive_upstream("second").await;
        let proxy_port = spawn_proxy(&["127.0.0.1"], HEADER_PHASE_TIMEOUT).await;

        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{first_port}/one HTTP/1.1\r\nHost: 127.0.0.1:{first_port}\r\nProxy-Connection: keep-alive\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = [0u8; 512];
        let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).ends_with("first"));

        let _ = client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{second_port}/two HTTP/1.1\r\nHost: 127.0.0.1:{second_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await;
        let _ = read_until_closed(&mut client, Duration::from_secs(2)).await;

        let seen = first_seen.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.contains("/two")),
            "the request for the second host reached the first upstream: {seen:?}"
        );
    }

    #[tokio::test]
    async fn an_endless_header_line_is_cut_off() {
        let proxy_port = spawn_proxy(&["127.0.0.1"], HEADER_PHASE_TIMEOUT).await;
        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();

        let _ = client.write_all(&vec![b'a'; 1024 * 1024]).await;

        assert!(
            read_until_closed(&mut client, Duration::from_secs(5))
                .await
                .is_some(),
            "the proxy kept reading a header line with no end"
        );
    }

    #[tokio::test]
    async fn a_client_that_never_finishes_its_headers_is_disconnected() {
        let proxy_port = spawn_proxy(&["127.0.0.1"], Duration::from_millis(200)).await;
        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();

        client
            .write_all(b"GET http://127.0.0.1/ HTTP/1.1\r\n")
            .await
            .unwrap();

        assert!(
            read_until_closed(&mut client, Duration::from_secs(5))
                .await
                .is_some(),
            "the proxy held a stalled connection open"
        );
    }

    #[tokio::test]
    async fn proxy_hot_reload_picks_up_new_domains() {
        let allowed = Arc::new(RwLock::new(vec!["initial.com".to_string()]));
        assert!(!is_allowed("added.com", &allowed.read().await));
        // Simulate hot-reload by writing to the shared state
        allowed.write().await.push("added.com".to_string());
        assert!(is_allowed("added.com", &allowed.read().await));
    }

    #[tokio::test]
    async fn proxy_hot_reload_always_includes_preset_defaults() {
        // Simulate a hot-reload with only a custom domain — ensure_defaults must
        // add the preset's required domains so they are never lost after a reload.
        let allowed = Arc::new(RwLock::new(vec!["custom.example.com".to_string()]));
        // Verify preset defaults are not yet present
        assert!(!is_allowed("anthropic.com", &allowed.read().await));
        assert!(!is_allowed("crates.io", &allowed.read().await));
        // Simulate what the reload task does: apply ensure_defaults over fresh domains
        let mut fresh = vec!["custom.example.com".to_string()];
        crate::profile::presets::ensure_defaults(&mut fresh, "rust");
        *allowed.write().await = fresh;
        // After reload, preset defaults must be present alongside the custom domain
        assert!(is_allowed("anthropic.com", &allowed.read().await));
        assert!(is_allowed("crates.io", &allowed.read().await));
        assert!(is_allowed("custom.example.com", &allowed.read().await));
    }

    #[test]
    fn load_allowed_domains_reads_workspace_config() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("proxy_workspace_domains_{}", uuid::Uuid::new_v4()));
        let home = base.join("home");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let _home = crate::config::global::override_home_for_tests(home);
        let config = crate::config::workspace::WorkspaceConfig {
            projects: vec![
                crate::config::workspace::WorkspaceProject {
                    path: "api".to_string(),
                    preset: "rust".to_string(),
                },
                crate::config::workspace::WorkspaceProject {
                    path: "mobile".to_string(),
                    preset: "swift".to_string(),
                },
            ],
            network: crate::config::project::NetworkConfig {
                proxy_enabled: true,
                allowed_domains: vec!["custom.example.com".to_string()],
            },
            allow_commit: false,
            command: None,
        };
        config.save(&workspace).unwrap();

        let domains = load_allowed_domains(&workspace).unwrap();

        assert!(is_allowed("custom.example.com", &domains));
        assert!(is_allowed("crates.io", &domains));
        assert!(is_allowed("claude.ai", &domains));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
