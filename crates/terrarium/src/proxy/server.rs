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

async fn handle(mut client: TcpStream, cfg: Arc<ProxyConfig>) -> std::io::Result<()> {
    let first_line = read_line(&mut client).await?;
    let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return Ok(());
    }
    let (method, target, version) = (parts[0], parts[1], parts[2]);

    if method.eq_ignore_ascii_case("CONNECT") {
        // target is host:port
        let domain = target.split(':').next().unwrap_or(target).to_lowercase();
        {
            let allowed = cfg.allowed_domains.read().await;
            if !is_allowed(&domain, &allowed) {
                drop(allowed);
                block(&mut client, &domain, method, target, &cfg.project_root).await?;
                return Ok(());
            }
        }
        drain_headers(&mut client).await?;
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let mut remote = TcpStream::connect(target).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote).await?;
    } else {
        // Plain HTTP — buffer headers and detect Host.
        let mut header_buf: Vec<u8> = Vec::new();
        let mut host_header: Option<String> = None;
        loop {
            let line = read_line(&mut client).await?;
            if line.is_empty() {
                header_buf.extend_from_slice(b"\r\n");
                break;
            }
            if host_header.is_none() && line.to_ascii_lowercase().starts_with("host:") {
                host_header = Some(line["host:".len()..].trim().to_lowercase());
            }
            header_buf.extend_from_slice(line.as_bytes());
            header_buf.extend_from_slice(b"\r\n");
        }

        let domain = domain_from_url(target)
            .map(|d| d.to_lowercase())
            .or_else(|| {
                host_header
                    .as_deref()
                    .map(host_without_port)
                    .map(str::to_string)
            })
            .unwrap_or_default();

        {
            let allowed = cfg.allowed_domains.read().await;
            if !is_allowed(&domain, &allowed) {
                drop(allowed);
                block(&mut client, &domain, method, target, &cfg.project_root).await?;
                return Ok(());
            }
        }

        let connect_target = match &host_header {
            Some(h) if h.contains(':') => h.clone(),
            Some(h) => format!("{h}:80"),
            None => authority_from_url(target)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{domain}:80")),
        };

        let mut remote = TcpStream::connect(&connect_target).await?;
        remote
            .write_all(format!("{method} {target} {version}\r\n").as_bytes())
            .await?;
        remote.write_all(&header_buf).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote).await?;
    }
    Ok(())
}

/// Reads bytes until `\r\n`, returning the line without the terminator.
async fn read_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        buf.push(byte[0]);
    }
}

async fn drain_headers(stream: &mut TcpStream) -> std::io::Result<()> {
    loop {
        if read_line(stream).await?.is_empty() {
            return Ok(());
        }
    }
}

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
