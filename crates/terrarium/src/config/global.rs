use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{Result, TerrariumError};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    #[serde(default = "default_command")]
    pub command: String,
}

fn default_command() -> String {
    "claude".to_string()
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            command: default_command(),
        }
    }
}

impl GlobalConfig {
    pub fn load() -> Result<Self> {
        let path = global_config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config = toml::from_str(&content).map_err(TerrariumError::Toml)?;
        Ok(config)
    }
}

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static TEST_HOME_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestHomeGuard {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        TEST_HOME_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
    }
}

#[cfg(test)]
pub(crate) fn override_home_for_tests(path: PathBuf) -> TestHomeGuard {
    let previous = TEST_HOME_OVERRIDE.with(|cell| cell.borrow_mut().replace(path));
    TestHomeGuard { previous }
}

pub fn home_dir() -> PathBuf {
    #[cfg(test)]
    {
        let result = TEST_HOME_OVERRIDE.with(|cell| cell.borrow().clone());
        if let Some(path) = result {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
}

pub fn claude_dir() -> PathBuf {
    home_dir().join(".claude")
}

pub fn global_terrarium_dir() -> PathBuf {
    home_dir().join(".terrarium")
}

fn global_config_path() -> PathBuf {
    global_terrarium_dir().join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = GlobalConfig::default();
        assert_eq!(config.defaults.command, "claude");
    }

    #[test]
    fn deserialize_partial_config() {
        let toml = r#"
[defaults]
command = "myclaude"
"#;
        let config: GlobalConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.defaults.command, "myclaude");
    }

    #[test]
    fn deserialize_empty_toml_gives_defaults() {
        let config: GlobalConfig = toml::from_str("").unwrap();
        assert_eq!(config.defaults.command, "claude");
    }

    #[test]
    fn deserialize_unknown_keys_ignored() {
        let toml = r#"
[defaults]
command = "test"
unknown_field = "should be ignored"

[some_unknown_section]
foo = "bar"
"#;
        // serde with deny_unknown_fields would fail — verify it doesn't
        let config: GlobalConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.defaults.command, "test");
    }
}
