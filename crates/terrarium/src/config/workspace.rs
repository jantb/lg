use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::project::{NetworkConfig, create_private_dir, global_profile_dir};
use crate::error::{Result, TerrariumError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProject {
    pub path: String,
    pub preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub projects: Vec<WorkspaceProject>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub allow_commit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl WorkspaceConfig {
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = workspace_path(workspace_root);
        if !path.exists() {
            return Err(TerrariumError::WorkspaceNotFound {
                path: path.display().to_string(),
            });
        }
        let content = std::fs::read_to_string(&path)?;
        let config = toml::from_str(&content).map_err(TerrariumError::Toml)?;
        Ok(config)
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let dir = global_profile_dir(workspace_root);
        create_private_dir(&dir)?;
        let path = workspace_path(workspace_root);
        let content = toml::to_string_pretty(self).map_err(TerrariumError::TomlSer)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

pub fn workspace_path(workspace_root: &Path) -> PathBuf {
    global_profile_dir(workspace_root).join("workspace.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;

    #[test]
    fn serialize_deserialize_roundtrip() {
        let config = WorkspaceConfig {
            projects: vec![
                WorkspaceProject {
                    path: "backend".to_string(),
                    preset: "rust".to_string(),
                },
                WorkspaceProject {
                    path: "mobile".to_string(),
                    preset: "swift".to_string(),
                },
            ],
            network: NetworkConfig::default(),
            allow_commit: false,
            command: None,
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let config2: WorkspaceConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(config2.projects.len(), 2);
        assert_eq!(config2.projects[0].path, "backend");
        assert_eq!(config2.projects[0].preset, "rust");
        assert_eq!(config2.projects[1].path, "mobile");
        assert_eq!(config2.projects[1].preset, "swift");
        assert!(config2.network.proxy_enabled);
    }

    #[test]
    fn load_nonexistent_returns_workspace_not_found() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("workspace_nonexistent");
        let home = base.join("home");
        let root = base.join("workspace");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        let result = WorkspaceConfig::load(&root);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TerrariumError::WorkspaceNotFound { .. }
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn save_load_roundtrip_filesystem() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("workspace_roundtrip");
        let home = base.join("home");
        let root = base.join("workspace");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        std::fs::create_dir_all(&root).unwrap();
        let config = WorkspaceConfig {
            projects: vec![
                WorkspaceProject {
                    path: "services/api".to_string(),
                    preset: "kotlin".to_string(),
                },
                WorkspaceProject {
                    path: "cli".to_string(),
                    preset: "rust".to_string(),
                },
            ],
            network: NetworkConfig {
                proxy_enabled: false,
                allowed_domains: vec!["crates.io".to_string()],
            },
            allow_commit: true,
            command: Some("myclaude".to_string()),
        };
        config.save(&root).unwrap();
        let loaded = WorkspaceConfig::load(&root).unwrap();
        assert_eq!(loaded.projects.len(), 2);
        assert_eq!(loaded.projects[0].path, "services/api");
        assert_eq!(loaded.projects[0].preset, "kotlin");
        assert_eq!(loaded.projects[1].path, "cli");
        assert_eq!(loaded.projects[1].preset, "rust");
        assert!(!loaded.network.proxy_enabled);
        assert_eq!(loaded.network.allowed_domains, vec!["crates.io"]);
        assert!(loaded.allow_commit);
        assert_eq!(loaded.command.as_deref(), Some("myclaude"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn deserialize_missing_workspace_settings_get_defaults() {
        let toml = r#"
[[projects]]
path = "svc"
preset = "rust"
"#;
        let config: WorkspaceConfig = toml::from_str(toml).unwrap();
        assert!(!config.allow_commit);
        assert!(config.command.is_none());
        assert!(config.network.proxy_enabled);
    }
}
