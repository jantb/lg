//! Starting the domain-filtering proxy and the environment that points a
//! sandboxed process at it.
//!
//! The sandbox denies network access outright, so a session that needs the
//! network goes through this proxy, which allows only the configured domains.
//! `NO_PROXY` keeps loopback direct — that is how the session reaches terrarium's
//! own MCP server.

use std::path::Path;

use crate::config::workspace::WorkspaceConfig;
use crate::error::Result;

use super::policy::load_or_default_profile;

/// Starts the proxy when enabled and returns the environment variables that route
/// a child through it. An empty vec means no proxy.
pub(super) async fn start_if_enabled(
    project_root: &Path,
    ws_config: Option<&WorkspaceConfig>,
    proxy_enabled: bool,
) -> Result<Vec<(String, String)>> {
    if !proxy_enabled {
        return Ok(vec![]);
    }

    let allowed_domains = allowed_domains(project_root, ws_config)?;
    let proxy_port = crate::proxy::server::start(crate::proxy::server::ProxyConfig {
        allowed_domains: std::sync::Arc::new(tokio::sync::RwLock::new(allowed_domains)),
        project_root: project_root.to_path_buf(),
    })
    .await?;
    println!("terrarium: domain proxy started on port {proxy_port}");

    let proxy_url = format!("http://127.0.0.1:{proxy_port}");
    Ok(vec![
        ("HTTP_PROXY".to_string(), proxy_url.clone()),
        ("HTTPS_PROXY".to_string(), proxy_url.clone()),
        ("http_proxy".to_string(), proxy_url.clone()),
        ("https_proxy".to_string(), proxy_url),
        ("NO_PROXY".to_string(), "localhost,127.0.0.1".to_string()),
        ("no_proxy".to_string(), "localhost,127.0.0.1".to_string()),
    ])
}

/// The configured allowlist plus each preset's defaults, sorted and deduplicated.
fn allowed_domains(
    project_root: &Path,
    ws_config: Option<&WorkspaceConfig>,
) -> Result<Vec<String>> {
    let mut domains = match ws_config {
        Some(ws) => {
            let mut domains = ws.network.allowed_domains.clone();
            for project in &ws.projects {
                crate::profile::presets::ensure_defaults(&mut domains, &project.preset);
            }
            domains
        }
        None => {
            let profile = load_or_default_profile(project_root)?;
            let preset_name = profile.preset.as_deref().unwrap_or("none");
            let mut domains = profile.network.allowed_domains.clone();
            crate::profile::presets::ensure_defaults(&mut domains, preset_name);
            domains
        }
    };
    domains.sort();
    domains.dedup();
    Ok(domains)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::project::ProjectProfile;
    use crate::config::workspace::WorkspaceProject;

    #[test]
    fn no_proxy_means_no_environment() {
        let env = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(start_if_enabled(Path::new("/tmp"), None, false))
            .unwrap();
        assert!(env.is_empty());
    }

    /// A workspace unions its sub-projects' preset defaults on top of its own list.
    #[test]
    fn workspace_allowlist_unions_every_preset_default() {
        let config = WorkspaceConfig {
            projects: vec![
                WorkspaceProject {
                    path: "svc".to_string(),
                    preset: "rust".to_string(),
                },
                WorkspaceProject {
                    path: "web".to_string(),
                    preset: "node".to_string(),
                },
            ],
            network: crate::config::project::NetworkConfig {
                proxy_enabled: true,
                allowed_domains: vec!["internal.example".to_string()],
            },
            allow_commit: false,
            command: None,
        };

        let domains = allowed_domains(Path::new("/tmp"), Some(&config)).unwrap();

        assert!(domains.contains(&"internal.example".to_string()));
        assert!(domains.contains(&"crates.io".to_string()));
        assert!(domains.contains(&"registry.npmjs.org".to_string()));
        assert!(domains.contains(&"anthropic.com".to_string()));
        assert!(domains.windows(2).all(|w| w[0] <= w[1]), "must be sorted");
    }

    /// An uninitialized project still gets the base allowlist.
    #[test]
    fn missing_profile_falls_back_to_none_preset_defaults() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("proxy_env_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        let domains = allowed_domains(&root, None).unwrap();

        assert!(domains.contains(&"anthropic.com".to_string()));
        assert!(!domains.contains(&"crates.io".to_string()));
        let _ = ProjectProfile::load(&root); // no profile was written
        std::fs::remove_dir_all(&root).unwrap();
    }
}
