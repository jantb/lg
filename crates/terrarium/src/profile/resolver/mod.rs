//! Assembling the rule set for a project or workspace and rendering it to SBPL.
//!
//! One request type covers both shapes. The rules come from three places: the
//! preset(s), the profile's own manual rules, and the derived rules this module's
//! submodules contribute for [`aliases`] and [`symlinks`].

mod aliases;
mod params;
mod symlinks;

pub use params::{build_params, build_workspace_params, detect_java_home};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::project::{PathType, ProjectProfile, Rule, RuleAction, RuleSource};
use crate::config::workspace::WorkspaceConfig;
use crate::error::{Result, TerrariumError};
use crate::profile::presets;
use crate::profile::sbpl;

use aliases::{hidden_project_state_alias_rules, project_root_alias_rules};
use symlinks::{claude_skills_symlink_rules, expand_rules_for_symlink_targets};

/// What to render an SBPL profile for.
enum Target<'a> {
    Project {
        profile: &'a ProjectProfile,
        proxy_enabled: bool,
    },
    Workspace {
        root: &'a Path,
        config: &'a WorkspaceConfig,
    },
}

/// One render request, replacing the former single/workspace ×
/// aliases/no-aliases function matrix. Aliases are the ordinary case with an
/// empty slice, so callers only mention them when they have some.
pub struct RenderRequest<'a> {
    target: Target<'a>,
    aliases: &'a [PathBuf],
}

impl<'a> RenderRequest<'a> {
    /// A single project root.
    pub fn project(profile: &'a ProjectProfile, proxy_enabled: bool) -> Self {
        Self {
            target: Target::Project {
                profile,
                proxy_enabled,
            },
            aliases: &[],
        }
    }

    /// A workspace of sub-projects, whose preset rules are merged.
    pub fn workspace(root: &'a Path, config: &'a WorkspaceConfig) -> Self {
        Self {
            target: Target::Workspace { root, config },
            aliases: &[],
        }
    }

    /// Additional lexical paths that point at the root (e.g. a symlink the
    /// caller was invoked through).
    pub fn with_aliases(mut self, aliases: &'a [PathBuf]) -> Self {
        self.aliases = aliases;
        self
    }
}

/// Resolves all rules for a request and renders the SBPL profile.
pub fn render(request: &RenderRequest<'_>) -> Result<String> {
    let (rules, params, proxy_enabled) = match &request.target {
        Target::Project {
            profile,
            proxy_enabled,
        } => {
            let mut rules: Vec<Rule> = Vec::new();
            if let Some(preset_name) = &profile.preset {
                rules.extend(presets::preset_rules(preset_name).ok_or_else(|| {
                    TerrariumError::InvalidPreset {
                        name: preset_name.clone(),
                    }
                })?);
            }
            rules.extend(profile.rules.iter().cloned());
            // The single-project path appends alias rules without deduplicating.
            rules.extend(project_root_alias_rules(request.aliases, &profile.params));
            (rules, profile.params.clone(), *proxy_enabled)
        }
        Target::Workspace { root, config } => {
            // Union of every sub-project's preset rules, deduplicated.
            let mut rules: Vec<Rule> = Vec::new();
            let mut seen = HashSet::new();
            for project in &config.projects {
                let preset = presets::preset_rules(&project.preset).ok_or_else(|| {
                    TerrariumError::InvalidPreset {
                        name: project.preset.clone(),
                    }
                })?;
                for rule in preset {
                    if seen.insert(rule_key(&rule)) {
                        rules.push(rule);
                    }
                }
            }
            // The workspace path deduplicates alias rules against the preset
            // rules already collected, since sub-projects commonly share them.
            let params = build_workspace_params(root);
            for rule in project_root_alias_rules(request.aliases, &params) {
                if seen.insert(rule_key(&rule)) {
                    rules.push(rule);
                }
            }
            (rules, params, config.network.proxy_enabled)
        }
    };

    Ok(finish(rules, &params, proxy_enabled, request.aliases))
}

/// Appends the symlink and hidden-state rules every target needs, then renders.
/// This tail was previously duplicated across the single-project and workspace
/// paths.
fn finish(
    rules: Vec<Rule>,
    params: &HashMap<String, String>,
    proxy_enabled: bool,
    aliases: &[PathBuf],
) -> String {
    let mut rules = expand_rules_for_symlink_targets(&rules, params);
    rules.extend(claude_skills_symlink_rules(params));
    rules.push(hidden_project_state_rule());
    rules.extend(hidden_project_state_alias_rules(aliases, params));
    sbpl::render(&rules, params, proxy_enabled)
}

/// Dedup identity for a rule: operation, path type, and path value.
fn rule_key(rule: &Rule) -> (String, Option<PathType>, String) {
    (
        rule.operation.clone(),
        rule.path_type.clone(),
        rule.path_value.clone().unwrap_or_default(),
    )
}

/// Terrarium's own state sits inside the project root, which the profile grants
/// wholesale; this denies it back so sandboxed tools cannot read or edit it.
fn hidden_project_state_rule() -> Rule {
    Rule {
        action: RuleAction::Deny,
        operation: "file-read* file-write* file-test-existence".to_string(),
        path_type: Some(PathType::Subpath),
        path_value: Some("PROJECT_ROOT/.terrarium".to_string()),
        comment: Some("Hide project-local terrarium state from sandboxed tools".to_string()),
        source: RuleSource::Preset,
    }
}

/// Renders a request and writes it to the project's global `active.sb` path.
pub fn write_active(request: &RenderRequest<'_>, project_root: &Path) -> Result<String> {
    let content = render(request)?;
    let sb_path = crate::config::project::active_sb_path(project_root);
    if let Some(parent) = sb_path.parent() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    std::fs::write(&sb_path, &content)?;
    Ok(sb_path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::project::NetworkConfig;
    use crate::config::workspace::WorkspaceProject;

    fn temp_dir(label: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{label}_{}", uuid::Uuid::new_v4()))
    }

    fn workspace_config(projects: &[(&str, &str)]) -> WorkspaceConfig {
        WorkspaceConfig {
            projects: projects
                .iter()
                .map(|(path, preset)| WorkspaceProject {
                    path: (*path).to_string(),
                    preset: (*preset).to_string(),
                })
                .collect(),
            network: NetworkConfig {
                proxy_enabled: false,
                allowed_domains: vec![],
            },
            allow_commit: false,
            command: None,
        }
    }

    #[test]
    fn resolve_none_preset_produces_minimal_profile() {
        let profile = ProjectProfile::new("test", Some("none".to_string()));
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        assert!(output.contains("(deny default)"));
    }

    #[test]
    fn resolve_rust_preset_contains_project_root_literal() {
        let mut profile = ProjectProfile::new("test", Some("rust".to_string()));
        profile
            .params
            .insert("PROJECT_ROOT".to_string(), "/my/project".to_string());
        profile
            .params
            .insert("CARGO_HOME".to_string(), "/my/.cargo".to_string());
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        assert!(output.contains("/my/project"));
    }

    #[test]
    fn resolve_hides_project_local_terrarium_state() {
        let mut profile = ProjectProfile::new("test", Some("rust".to_string()));
        profile
            .params
            .insert("PROJECT_ROOT".to_string(), "/my/project".to_string());

        let output = render(&RenderRequest::project(&profile, false)).unwrap();

        assert!(output.contains("Hide project-local terrarium state"));
        assert!(output.contains("(deny file-read*\n  file-write*\n  file-test-existence"));
        assert!(output.contains("(subpath \"/my/project/.terrarium\")"));
    }

    #[test]
    fn resolve_invalid_preset_returns_error() {
        let profile = ProjectProfile::new("test", Some("bogus".to_string()));
        assert!(render(&RenderRequest::project(&profile, false)).is_err());
    }

    #[test]
    fn resolve_no_preset_with_project_rules() {
        let mut profile = ProjectProfile::new("test", Some("none".to_string()));
        profile.rules.push(Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some("/etc/hosts".to_string()),
            comment: None,
            source: RuleSource::Manual,
        });
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        assert!(output.contains("(allow file-read-data"));
        assert!(output.contains("/etc/hosts"));
    }

    #[test]
    fn resolve_preset_with_no_project_rules() {
        let profile = ProjectProfile::new("test", Some("rust".to_string()));
        let output = render(&RenderRequest::project(&profile, false)).unwrap();
        // Rust preset should produce rules containing PROJECT_ROOT (unsubstituted since no params)
        assert!(output.contains("(allow"));
        assert!(output.contains("PROJECT_ROOT"));
    }

    #[test]
    fn write_active_profile_creates_file() {
        let base = temp_dir("resolver");
        let dir = base.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::config::global::override_home_for_tests(base.join("home"));
        let profile = ProjectProfile::new("test", Some("none".to_string()));
        write_active(&RenderRequest::project(&profile, false), &dir).unwrap();
        assert!(crate::config::project::active_sb_path(&dir).exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn write_active_profile_content_matches_render() {
        let base = temp_dir("resolver");
        let dir = base.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::config::global::override_home_for_tests(base.join("home"));
        let profile = ProjectProfile::new("test", Some("none".to_string()));
        let path = write_active(&RenderRequest::project(&profile, false), &dir).unwrap();
        let file_content = std::fs::read_to_string(&path).unwrap();
        let rendered = render(&RenderRequest::project(&profile, false)).unwrap();
        assert_eq!(file_content, rendered);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn resolve_workspace_with_rust_and_swift() {
        let config = workspace_config(&[("backend", "rust"), ("ios-app", "swift")]);
        let output = render(&RenderRequest::workspace(
            Path::new("/tmp/workspace"),
            &config,
        ))
        .unwrap();
        assert!(output.contains("(deny default)"));
        // Should contain PROJECT_ROOT reference
        assert!(output.contains("/tmp/workspace"));
    }

    #[test]
    fn resolve_workspace_invalid_preset_returns_error() {
        let config = workspace_config(&[("broken", "bogus")]);
        let err = render(&RenderRequest::workspace(
            Path::new("/tmp/workspace"),
            &config,
        ))
        .unwrap_err();
        assert!(matches!(err, TerrariumError::InvalidPreset { name } if name == "bogus"));
    }

    #[test]
    fn workspace_deduplicates_shared_rules() {
        // Two identical rust presets — shared rules should be deduplicated
        let config = workspace_config(&[("a", "rust"), ("b", "rust")]);
        let output = render(&RenderRequest::workspace(Path::new("/tmp/ws"), &config)).unwrap();
        // With two identical rust presets the broad PROJECT_ROOT allow rule should
        // be deduped. The project-local .terrarium deny rule is checked separately.
        let count = output.matches("(subpath \"/tmp/ws\")").count();
        assert_eq!(
            count, 1,
            "PROJECT_ROOT allow should appear exactly once, found {count}"
        );
        assert!(output.contains("(subpath \"/tmp/ws/.terrarium\")"));
    }
}
