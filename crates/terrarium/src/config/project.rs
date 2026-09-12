use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::global::global_terrarium_dir;
use crate::error::{Result, TerrariumError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PathType {
    Literal,
    Subpath,
    Regex,
    Ancestors,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleSource {
    Manual,
    Preset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub action: RuleAction,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_type: Option<PathType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub source: RuleSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_true")]
    pub proxy_enabled: bool,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            proxy_enabled: true,
            allowed_domains: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub params: HashMap<String, String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub allow_commit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl ProjectProfile {
    pub fn new(name: impl Into<String>, preset: Option<String>) -> Self {
        Self {
            name: name.into(),
            preset,
            params: HashMap::new(),
            rules: Vec::new(),
            network: NetworkConfig::default(),
            allow_commit: false,
            command: None,
        }
    }

    pub fn load(project_root: &Path) -> Result<Self> {
        let path = profile_path(project_root);
        if !path.exists() {
            return Err(TerrariumError::ProfileNotFound {
                path: path.display().to_string(),
            });
        }
        let content = std::fs::read_to_string(&path)?;
        let profile = toml::from_str(&content).map_err(TerrariumError::Toml)?;
        Ok(profile)
    }

    pub fn save(&self, project_root: &Path) -> Result<()> {
        let dir = global_profile_dir(project_root);
        create_private_dir(&dir)?;
        let path = profile_path(project_root);
        let content = toml::to_string_pretty(self).map_err(TerrariumError::TomlSer)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn remove_rule(&mut self, index: usize) {
        if index < self.rules.len() {
            self.rules.remove(index);
        }
    }
}

/// Returns the global directory for a project's terrarium data.
/// Profiles are stored at `~/.terrarium/projects/<sanitized-project-path>/`.
pub fn global_profile_dir(project_root: &Path) -> PathBuf {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let dir_name = canonical
        .to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-");
    global_terrarium_dir().join("projects").join(dir_name)
}

pub fn profile_path(project_root: &Path) -> PathBuf {
    global_profile_dir(project_root).join("profile.toml")
}

/// Creates a directory with owner-only permissions (0o700).
/// If the directory already exists, its permissions are left unchanged.
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

pub fn active_sb_path(project_root: &Path) -> PathBuf {
    global_profile_dir(project_root).join("active.sb")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;

    #[test]
    fn new_profile_has_empty_rules() {
        let p = ProjectProfile::new("test", Some("rust".to_string()));
        assert!(p.rules.is_empty());
        assert_eq!(p.preset, Some("rust".to_string()));
    }

    #[test]
    fn push_and_remove_rule() {
        let mut p = ProjectProfile::new("test", None);
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some("/etc/hosts".to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        p.rules.push(rule.clone());
        assert_eq!(p.rules.len(), 1);
        p.remove_rule(0);
        assert!(p.rules.is_empty());
    }

    #[test]
    fn remove_rule_out_of_bounds_is_noop() {
        let mut p = ProjectProfile::new("test", None);
        p.remove_rule(99);
        assert!(p.rules.is_empty());
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let mut p = ProjectProfile::new("myproject", Some("rust".to_string()));
        p.params
            .insert("PROJECT_ROOT".to_string(), "/home/user/proj".to_string());
        p.rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some("/usr".to_string()),
            comment: Some("test".to_string()),
            source: RuleSource::Preset,
        });
        let toml_str = toml::to_string_pretty(&p).unwrap();
        let p2: ProjectProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(p.name, p2.name);
        assert_eq!(p.preset, p2.preset);
        assert_eq!(p.rules.len(), p2.rules.len());
    }

    #[test]
    fn load_malformed_toml_returns_error() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("malformed_toml");
        let home = base.join("home");
        let root = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        let profile_dir = global_profile_dir(&root);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("profile.toml"), "{{{{not valid toml!").unwrap();
        let result = ProjectProfile::load(&root);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TerrariumError::Toml(_)));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_load_roundtrip_filesystem() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("roundtrip");
        let home = base.join("home");
        let root = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        std::fs::create_dir_all(&root).unwrap();
        let mut profile = ProjectProfile::new("roundtrip-test", Some("none".to_string()));
        profile
            .params
            .insert("PROJECT_ROOT".to_string(), "/my/proj".to_string());
        profile.rules.push(Rule {
            action: RuleAction::Deny,
            operation: "network-outbound".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some("/private/var".to_string()),
            comment: Some("block network".to_string()),
            source: RuleSource::Manual,
        });
        profile.save(&root).unwrap();
        let loaded = ProjectProfile::load(&root).unwrap();
        assert_eq!(loaded.name, "roundtrip-test");
        assert_eq!(loaded.preset, Some("none".to_string()));
        assert_eq!(loaded.rules.len(), 1);
        assert_eq!(loaded.rules[0].operation, "network-outbound");
        assert_eq!(loaded.params["PROJECT_ROOT"], "/my/proj");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn load_nonexistent_returns_profile_not_found() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("nonexistent");
        let home = base.join("home");
        let root = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        let result = ProjectProfile::load(&root);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TerrariumError::ProfileNotFound { .. }
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rule_with_path_type_but_no_path_value() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: None,
            comment: None,
            source: RuleSource::Manual,
        };
        // Serde roundtrip should work
        let toml_str = toml::to_string_pretty(&rule).unwrap();
        let _: Rule = toml::from_str(&toml_str).unwrap();
        // SBPL render should produce bare-op (not crash)
        use crate::profile::sbpl;
        let output = sbpl::render(&[rule], &std::collections::HashMap::new(), false);
        assert!(output.contains("(allow file-read-data)"));
    }

    #[test]
    fn new_profile_has_proxy_enabled_by_default() {
        let p = ProjectProfile::new("test", Some("rust".to_string()));
        assert!(p.network.proxy_enabled);
    }

    #[test]
    fn deserialize_missing_proxy_field_defaults_to_enabled() {
        let toml_str = r#"
name = "test"
preset = "rust"
"#;
        let p: ProjectProfile = toml::from_str(toml_str).unwrap();
        assert!(p.network.proxy_enabled);
    }

    #[test]
    fn save_creates_sx_dir_with_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("perms_check");
        let home = base.join("home");
        let root = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        let _guard = override_home_for_tests(home);
        std::fs::create_dir_all(&root).unwrap();
        let profile = ProjectProfile::new("perms-test", None);
        profile.save(&root).unwrap();
        let dir = global_profile_dir(&root);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o700,
            "profile directory should be owner-only (0o700), got {:#o}",
            mode
        );
        std::fs::remove_dir_all(&base).unwrap();
    }
}
