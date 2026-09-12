//! The Claude Code skill files describing terrarium's MCP tools.
//!
//! Skills are installed per project, under `.claude/skills/`, so they only load
//! for the terrarium-managed project whose MCP server actually provides the tools
//! they describe. Earlier releases installed them globally into `~/.claude/skills`,
//! where they loaded everywhere; those are cleaned up on the way past.

use std::path::{Path, PathBuf};

use crate::config::global::claude_dir;
use crate::config::project::ProjectProfile;
use crate::error::Result;
use crate::profile::presets;

use super::gitignore::ensure_project_gitignore_entries;
use super::{TERRARIUM_SKILL_MARKER, TERRARIUM_VERSION_TAG};

/// Placeholder the skill templates carry in place of the version tag.
const VERSION_TAG_PLACEHOLDER: &str = "{version_tag}";

/// A toolchain that has both a skill file and a `CLAUDE.local.md` section.
///
/// Python is deliberately absent: it has no dedicated MCP build tools, so it
/// contributes guidance only (see [`super::claude_md`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SkillKind {
    Rust,
    Kotlin,
    Swift,
    Node,
}

impl SkillKind {
    pub(super) fn dir_name(self) -> &'static str {
        match self {
            SkillKind::Rust => "terrarium-rust-tools",
            SkillKind::Kotlin => "terrarium-kotlin-tools",
            SkillKind::Swift => "terrarium-swift-tools",
            SkillKind::Node => "terrarium-node-tools",
        }
    }

    fn content(self) -> String {
        let template = match self {
            SkillKind::Rust => include_str!("assets/skill-rust.md"),
            SkillKind::Kotlin => include_str!("assets/skill-kotlin.md"),
            SkillKind::Swift => include_str!("assets/skill-swift.md"),
            SkillKind::Node => include_str!("assets/skill-node.md"),
        };
        template.replace(VERSION_TAG_PLACEHOLDER, TERRARIUM_VERSION_TAG)
    }
}

/// Resolves the skill kinds for a project from its saved (possibly
/// multi-language) preset.
pub(super) fn skill_kinds_for_project(project_root: &Path) -> Vec<SkillKind> {
    let Some(preset) = ProjectProfile::load(project_root)
        .ok()
        .and_then(|profile| profile.preset)
    else {
        return Vec::new();
    };
    skill_kinds_for_preset(&preset)
}

pub(super) fn skill_kinds_for_preset(preset: &str) -> Vec<SkillKind> {
    presets::parse_presets(preset)
        .iter()
        .filter_map(|name| match name.as_str() {
            "rust" => Some(SkillKind::Rust),
            "kotlin" => Some(SkillKind::Kotlin),
            "swift" => Some(SkillKind::Swift),
            "node" => Some(SkillKind::Node),
            _ => None,
        })
        .collect()
}

/// Returns the skill directory names to offer for installation (one per toolchain).
pub fn skill_prompt_names(project_root: &Path) -> Vec<&'static str> {
    skill_kinds_for_project(project_root)
        .into_iter()
        .map(SkillKind::dir_name)
        .collect()
}

/// Returns true if every project skill file already contains the current version tag.
pub fn skill_is_current(project_root: &Path) -> bool {
    skill_kinds_for_project(project_root)
        .into_iter()
        .all(|kind| {
            let skill_path = skill_path(project_root, kind);
            skill_path.exists()
                && std::fs::read_to_string(&skill_path)
                    .map(|c| c.contains(TERRARIUM_VERSION_TAG))
                    .unwrap_or(false)
        })
}

pub(super) fn ensure_skill_file(project_root: &Path) -> Result<()> {
    let kinds = skill_kinds_for_project(project_root);
    if kinds.is_empty() {
        return Ok(());
    }
    let ignore_entries: Vec<String> = kinds
        .iter()
        .map(|kind| format!("/.claude/skills/{}/", kind.dir_name()))
        .collect();
    let ignore_refs: Vec<&str> = ignore_entries.iter().map(String::as_str).collect();
    ensure_project_gitignore_entries(project_root, &ignore_refs)?;

    for kind in kinds {
        remove_legacy_global_skill(kind);

        let skill_path = skill_path(project_root, kind);
        if let Some(parent) = skill_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // A file already carrying this version may have been edited by the user;
        // only a version change justifies overwriting it.
        if skill_path.exists() {
            let existing = std::fs::read_to_string(&skill_path).unwrap_or_default();
            if existing.contains(TERRARIUM_VERSION_TAG) {
                continue;
            }
        }

        std::fs::write(&skill_path, kind.content())?;
    }
    Ok(())
}

fn skill_path(project_root: &Path, kind: SkillKind) -> PathBuf {
    project_root
        .join(".claude")
        .join("skills")
        .join(kind.dir_name())
        .join("SKILL.md")
}

/// Path of the pre-project-local install location, kept only for cleanup.
fn legacy_global_skill_dir(kind: SkillKind) -> PathBuf {
    claude_dir().join("skills").join(kind.dir_name())
}

/// Removes a terrarium-authored skill from the global skills directory.
/// User-authored or symlinked entries are left untouched.
fn remove_legacy_global_skill(kind: SkillKind) {
    let dir = legacy_global_skill_dir(kind);
    let Ok(metadata) = std::fs::symlink_metadata(&dir) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }
    let is_terrarium_authored = std::fs::read_to_string(dir.join("SKILL.md"))
        .map(|content| content.contains(TERRARIUM_SKILL_MARKER))
        .unwrap_or(false);
    if !is_terrarium_authored {
        return;
    }
    if std::fs::remove_dir_all(&dir).is_ok() {
        println!(
            "terrarium: removed globally installed skill {}",
            dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::temp_project;
    use crate::app::{configure_claude, init_project};
    use std::fs;

    fn installed_skill(project_root: &Path, dir_name: &str) -> String {
        let path = project_root
            .join(".claude")
            .join("skills")
            .join(dir_name)
            .join("SKILL.md");
        assert!(path.exists(), "SKILL.md should be created at {path:?}");
        fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn multi_language_project_offers_every_skill() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "kotlin,rust", true, false).unwrap();
        assert_eq!(
            skill_prompt_names(&dir),
            vec!["terrarium-kotlin-tools", "terrarium-rust-tools"]
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn skill_is_current_returns_false_when_missing() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        assert!(!skill_is_current(&dir));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn skill_is_current_returns_true_after_install() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        configure_claude(&dir, true).unwrap();
        assert!(skill_is_current(&dir));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn ensure_skill_file_creates_skill_md() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        configure_claude(&dir, true).unwrap();

        let content = installed_skill(&dir, "terrarium-rust-tools");
        assert!(content.contains("terrarium-rust-tools"));
        assert!(content.contains("cargo_check"));
        assert!(content.contains("cargo_update"));
        assert!(content.contains("git_status"));
        assert!(content.contains("git_unmerged"));
        assert!(content.contains("git_log"));
        assert!(content.contains("Cargo.toml"));
        assert!(content.contains("run the tests with `cargo_test`"));
        assert!(!content.contains("gradle_check"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn ensure_skill_file_not_overwritten_on_same_version() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        configure_claude(&dir, true).unwrap();
        let skill_path = dir
            .join(".claude")
            .join("skills")
            .join("terrarium-rust-tools")
            .join("SKILL.md");
        // Append something to detect overwrite
        let original = fs::read_to_string(&skill_path).unwrap();
        fs::write(&skill_path, format!("{original}\n<!-- marker -->")).unwrap();

        configure_claude(&dir, true).unwrap();

        let after = fs::read_to_string(&skill_path).unwrap();
        assert!(
            after.contains("<!-- marker -->"),
            "file should not be overwritten when version matches"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn ensure_kotlin_skill_file_creates_kotlin_skill_md() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "kotlin", true, false).unwrap();
        configure_claude(&dir, true).unwrap();

        let content = installed_skill(&dir, "terrarium-kotlin-tools");
        assert!(content.contains("terrarium-kotlin-tools"));
        assert!(content.contains("gradle_check"));
        assert!(content.contains("gradle_format_check"));
        assert!(content.contains("git_status"));
        assert!(content.contains("git_unmerged"));
        assert!(content.contains("git_show"));
        assert!(content.contains("Gradle build files"));
        assert!(content.contains("settings.gradle.kts"));
        assert!(content.contains("gradle/libs.versions.toml"));
        assert!(content.contains("Do not assume Android-specific layouts"));
        assert!(content.contains("prefer the smallest useful Gradle MCP command first"));
        assert!(content.contains("Prefer small real test implementations over mocks"));
        assert!(content.contains("do not mock returned results with Mockito"));
        assert!(content.contains("Test observable effects and outcomes"));
        assert!(content.contains("prefer pure functions where practical"));
        assert!(content.contains("run the tests with `gradle_test`"));
        assert!(!content.contains("cargo_update"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn ensure_swift_skill_file_creates_swift_skill_md() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "swift", true, false).unwrap();
        configure_claude(&dir, true).unwrap();

        let content = installed_skill(&dir, "terrarium-swift-tools");
        assert!(content.contains("terrarium-swift-tools"));
        assert!(content.contains("swift_build"));
        assert!(content.contains("swift_test"));
        assert!(content.contains("swift_format_check"));
        assert!(content.contains("git_status"));
        assert!(content.contains("Package.swift"));
        assert!(content.contains("run the tests with `swift_test`"));
        assert!(!content.contains("cargo_check"));
        assert!(!content.contains("gradle_check"));
        fs::remove_dir_all(&base).unwrap();
    }

    /// The templates carry `{version_tag}`; leaving it unsubstituted would make
    /// every install look stale to [`skill_is_current`].
    #[test]
    fn skill_content_substitutes_the_version_tag() {
        for kind in [SkillKind::Rust, SkillKind::Kotlin, SkillKind::Swift] {
            let content = kind.content();
            assert!(
                content.contains(TERRARIUM_VERSION_TAG),
                "{} is missing the version tag",
                kind.dir_name()
            );
            assert!(
                !content.contains(VERSION_TAG_PLACEHOLDER),
                "{} left the placeholder unsubstituted",
                kind.dir_name()
            );
        }
    }
}
