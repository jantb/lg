//! The block terrarium manages inside a project's `CLAUDE.local.md`.
//!
//! The file belongs to the user; terrarium owns only the region between
//! [`TERRARIUM_CLAUDE_BLOCK_START`] and [`TERRARIUM_CLAUDE_BLOCK_END`], which is
//! rewritten in place on every `configure_claude`. Content outside the markers
//! survives untouched, and a file that held nothing else is removed rather than
//! left empty.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::project::ProjectProfile;
use crate::config::workspace::WorkspaceProject;
use crate::error::Result;
use crate::profile::presets;

use super::gitignore::ensure_project_gitignore_entries;
use super::skills::{SkillKind, skill_kinds_for_preset};
use super::{TERRARIUM_CLAUDE_BLOCK_END, TERRARIUM_CLAUDE_BLOCK_START, TERRARIUM_VERSION_TAG};

/// Python has no dedicated MCP build tools, so it contributes guidance only.
const PYTHON_CLAUDE_MD_SECTION: &str = include_str!("assets/claude-md-python.md");

/// This toolchain's section of the managed block (no markers).
fn section_for(kind: SkillKind) -> &'static str {
    match kind {
        SkillKind::Rust => include_str!("assets/claude-md-rust.md"),
        SkillKind::Kotlin => include_str!("assets/claude-md-kotlin.md"),
        SkillKind::Swift => include_str!("assets/claude-md-swift.md"),
        SkillKind::Node => include_str!("assets/claude-md-node.md"),
    }
}

/// Builds the managed block covering every toolchain in the preset.
fn managed_claude_block_with_python(kinds: &[SkillKind], include_python: bool) -> String {
    let version_tag = TERRARIUM_VERSION_TAG;
    let mut sections: Vec<&str> = kinds.iter().copied().map(section_for).collect();
    if include_python {
        sections.push(PYTHON_CLAUDE_MD_SECTION);
    }
    format!(
        "{TERRARIUM_CLAUDE_BLOCK_START}\n{version_tag}\n{}{TERRARIUM_CLAUDE_BLOCK_END}\n",
        sections.join("\n")
    )
}

pub(super) fn ensure_project_claude_md(project_root: &Path) -> Result<()> {
    cleanup_legacy_startup_rules(project_root);
    let preset = ProjectProfile::load(project_root)
        .ok()
        .and_then(|profile| profile.preset)
        .unwrap_or_default();
    let kinds = skill_kinds_for_preset(&preset);
    let include_python = presets::parse_presets(&preset)
        .iter()
        .any(|name| name == "python");
    if kinds.is_empty() && !include_python {
        return remove_managed_claude_block(project_root);
    }
    ensure_project_gitignore_entries(project_root, &["/CLAUDE.local.md"])?;
    remove_managed_claude_block_at_path(&legacy_claude_md_path(project_root))?;

    write_managed_block(
        project_root,
        &managed_claude_block_with_python(&kinds, include_python),
    )
}

/// Generates a managed block for a workspace listing all tool families per sub-project.
pub fn workspace_claude_md_block(projects: &[WorkspaceProject]) -> String {
    let version_tag = TERRARIUM_VERSION_TAG;
    let mut block = format!(
        "{TERRARIUM_CLAUDE_BLOCK_START}\n{version_tag}\n# Terrarium Workspace Tools\n\n\
         This workspace contains multiple sub-projects managed by terrarium.\n\n"
    );

    for project in projects {
        let tools = preset_tool_names(&project.preset);
        if !tools.is_empty() {
            let _ = writeln!(block, "## {}/ ({})\n", project.path, project.preset);
            for tool in &tools {
                let _ = writeln!(block, "- `{tool}`");
            }
            let _ = writeln!(block);
        }
    }

    let _ = write!(
        block,
        "{}\n{TERRARIUM_CLAUDE_BLOCK_END}\n",
        include_str!("assets/claude-md-workspace-shared.md")
    );

    block
}

/// Configures `CLAUDE.local.md` for a workspace.
pub fn configure_workspace(workspace_root: &Path, projects: &[WorkspaceProject]) -> Result<()> {
    ensure_project_gitignore_entries(workspace_root, &["/CLAUDE.local.md"])?;
    remove_managed_claude_block_at_path(&legacy_claude_md_path(workspace_root))?;
    write_managed_block(workspace_root, &workspace_claude_md_block(projects))
}

/// Returns the build tool names offered for a preset spec (union for multi-preset specs).
fn preset_tool_names(spec: &str) -> Vec<&'static str> {
    let mut tools = Vec::new();
    for preset in presets::parse_presets(spec) {
        let names: &[&str] = match preset.as_str() {
            "rust" => &[
                "cargo_check",
                "cargo_test",
                "cargo_build",
                "cargo_clippy",
                "cargo_fmt_check",
                "cargo_fmt",
                "cargo_update",
                "cargo_upgrade_incompatible",
            ],
            "kotlin" => &[
                "gradle_check",
                "gradle_test",
                "gradle_build",
                "gradle_format_check",
            ],
            "swift" => &["swift_build", "swift_test", "swift_format_check"],
            "node" => &["npm_install", "npm_run", "npm_test", "node_run", "npx_run"],
            _ => &[],
        };
        for name in names {
            if !tools.contains(name) {
                tools.push(*name);
            }
        }
    }
    tools
}

/// Replaces the managed block in `CLAUDE.local.md`, leaving the rest of the file
/// alone and skipping the write when nothing changed.
fn write_managed_block(root: &Path, block: &str) -> Result<()> {
    let path = claude_md_path(root);
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let updated = upsert_managed_claude_block(&existing, block);
    if updated != existing {
        std::fs::write(path, updated)?;
    }
    Ok(())
}

pub(super) fn remove_managed_claude_block(project_root: &Path) -> Result<()> {
    remove_managed_claude_block_at_path(&claude_md_path(project_root))?;
    remove_managed_claude_block_at_path(&legacy_claude_md_path(project_root))?;
    Ok(())
}

fn remove_managed_claude_block_at_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(path)?;
    let updated = strip_managed_claude_block(&existing);
    if updated.trim().is_empty() {
        std::fs::remove_file(path)?;
    } else if updated != existing {
        std::fs::write(path, updated)?;
    }
    Ok(())
}

/// Removes the per-toolchain rule files a pre-skill release wrote.
fn cleanup_legacy_startup_rules(project_root: &Path) {
    let rules_dir = project_root.join(".claude").join("rules");
    let _ = std::fs::remove_file(rules_dir.join("terrarium-rust-tools.md"));
    let _ = std::fs::remove_file(rules_dir.join("terrarium-kotlin-tools.md"));
    let _ = std::fs::remove_file(rules_dir.join("terrarium-swift-tools.md"));
}

fn upsert_managed_claude_block(existing: &str, block: &str) -> String {
    let stripped = strip_managed_claude_block(existing);
    let trimmed = stripped.trim_end();
    if trimmed.is_empty() {
        format!("{block}\n")
    } else {
        format!("{trimmed}\n\n{block}\n")
    }
}

fn strip_managed_claude_block(existing: &str) -> String {
    let Some(start) = existing.find(TERRARIUM_CLAUDE_BLOCK_START) else {
        return existing.to_string();
    };
    let Some(relative_end) = existing[start..].find(TERRARIUM_CLAUDE_BLOCK_END) else {
        return existing.to_string();
    };
    let end = start + relative_end + TERRARIUM_CLAUDE_BLOCK_END.len();
    let before = existing[..start].trim_end();
    let after = existing[end..].trim_start();

    match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("{before}\n"),
        (true, false) => format!("{after}\n"),
        (false, false) => format!("{before}\n\n{after}\n"),
    }
}

pub(super) fn claude_md_path(project_root: &Path) -> PathBuf {
    project_root.join("CLAUDE.local.md")
}

/// Releases before the split wrote into the shared `CLAUDE.md`; the block is
/// still stripped from there so a migrating project does not carry two copies.
pub(super) fn legacy_claude_md_path(project_root: &Path) -> PathBuf {
    project_root.join("CLAUDE.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::temp_project;
    use crate::app::{configure_claude, init_project};
    use std::fs;

    /// The managed block for a single toolchain, as an older release would have
    /// written it into `CLAUDE.md`.
    fn single_toolchain_block(kind: SkillKind) -> String {
        managed_claude_block_with_python(&[kind], false)
    }

    #[test]
    fn multi_language_project_claude_md_lists_every_toolchain() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "node,kotlin,rust", true, false).unwrap();

        configure_claude(&dir, false).unwrap();

        let local_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(local_md.contains("Terrarium Kotlin Tools"));
        assert!(local_md.contains("Terrarium Rust Tools"));
        assert!(local_md.contains("Terrarium Node Tools"));
        assert_eq!(local_md.matches("terrarium-managed:start").count(), 1);
        assert_eq!(local_md.matches("terrarium-managed:end").count(), 1);
        fs::remove_dir_all(&base).unwrap();
    }

    /// Python contributes guidance without a skill file, so its section must
    /// still reach the managed block.
    #[test]
    fn python_project_claude_md_gets_a_section() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "python", true, false).unwrap();

        configure_claude(&dir, false).unwrap();

        let local_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(local_md.contains("Terrarium Python Tools"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn workspace_block_lists_node_tools() {
        let projects = vec![WorkspaceProject {
            path: "web".to_string(),
            preset: "node".to_string(),
        }];
        let block = workspace_claude_md_block(&projects);
        assert!(block.contains("npm_run"));
        assert!(block.contains("npm_test"));
    }

    #[test]
    fn workspace_block_lists_tools_for_multi_language_project() {
        let projects = vec![WorkspaceProject {
            path: "svc-a".to_string(),
            preset: "kotlin,rust".to_string(),
        }];
        let block = workspace_claude_md_block(&projects);
        assert!(block.contains("gradle_check"));
        assert!(block.contains("cargo_check"));
    }

    /// The workspace block is assembled from a template; the shared-tools tail and
    /// the closing marker have to survive that.
    #[test]
    fn workspace_block_is_delimited_and_documents_shared_tools() {
        let block = workspace_claude_md_block(&[WorkspaceProject {
            path: "svc-a".to_string(),
            preset: "rust".to_string(),
        }]);
        assert!(block.starts_with(TERRARIUM_CLAUDE_BLOCK_START));
        assert!(block.trim_end().ends_with(TERRARIUM_CLAUDE_BLOCK_END));
        assert!(block.contains("## Shared tools"));
        assert!(block.contains("`git_status`"));
        assert!(block.contains("optional `project` parameter"));
    }

    #[test]
    fn configure_claude_both_disabled_is_noop() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        configure_claude(&dir, false).unwrap();
        // CLAUDE.local.md should be updated, but no MCP config or installed skill is created.
        assert!(!dir.join(".mcp.json").exists());
        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(
            gitignore
                .lines()
                .any(|line| line.trim() == "/CLAUDE.local.md")
        );
        let claude_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(claude_md.contains("Terrarium Rust Tools"));
        assert!(claude_md.contains("cargo_test"));
        assert!(claude_md.contains("git_status"));
        assert!(claude_md.contains("git_unmerged"));
        assert!(claude_md.contains("run the tests with `cargo_test`"));
        assert!(
            !dir.join(".claude")
                .join("skills")
                .join("terrarium-rust-tools")
                .exists()
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_writes_kotlin_guidance_to_claude_local_md() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "kotlin", true, false).unwrap();
        configure_claude(&dir, false).unwrap();
        let claude_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(claude_md.contains("Terrarium Kotlin Tools"));
        assert!(claude_md.contains("gradle_test"));
        assert!(claude_md.contains("git_status"));
        assert!(claude_md.contains("git_unmerged"));
        assert!(claude_md.contains("settings.gradle.kts"));
        assert!(claude_md.contains("gradle/libs.versions.toml"));
        assert!(claude_md.contains("Do not assume Android-specific layouts"));
        assert!(claude_md.contains("prefer the smallest useful Gradle MCP command first"));
        assert!(claude_md.contains("Prefer small real test implementations over mocks"));
        assert!(claude_md.contains("do not mock returned results with Mockito"));
        assert!(claude_md.contains("Test observable effects and outcomes"));
        assert!(claude_md.contains("prefer pure functions where practical"));
        assert!(claude_md.contains("run the tests with `gradle_test`"));
        assert!(!claude_md.contains("cargo_update"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_writes_swift_guidance_to_claude_local_md() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "swift", true, false).unwrap();
        configure_claude(&dir, false).unwrap();
        let claude_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(claude_md.contains("Terrarium Swift Tools"));
        assert!(claude_md.contains("swift_test"));
        assert!(claude_md.contains("git_status"));
        assert!(claude_md.contains("git_unmerged"));
        assert!(claude_md.contains("run the tests with `swift_test`"));
        assert!(!claude_md.contains("cargo_update"));
        assert!(!claude_md.contains("gradle_test"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_skips_skill_install_for_none_preset() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "none", true, false).unwrap();
        configure_claude(&dir, true).unwrap();
        assert!(
            !dir.join(".claude")
                .join("skills")
                .join("terrarium-rust-tools")
                .exists()
        );
        assert!(
            !dir.join(".claude")
                .join("skills")
                .join("terrarium-kotlin-tools")
                .exists()
        );
        assert!(!dir.join("CLAUDE.local.md").exists());
        assert!(!dir.join("CLAUDE.md").exists());
        assert!(
            !dir.join(".gitignore").exists(),
            "none preset should not create Claude ignore entries"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_preserves_existing_gitignore_content() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        fs::write(dir.join(".gitignore"), "target/\n/CLAUDE.local.md\n").unwrap();

        configure_claude(&dir, false).unwrap();

        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert!(gitignore.contains("target/"));
        assert_eq!(gitignore.matches("/CLAUDE.local.md").count(), 1);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_preserves_existing_claude_md_content_and_writes_local_file() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        fs::write(dir.join("CLAUDE.md"), "# Existing Notes\n\nKeep this.\n").unwrap();

        configure_claude(&dir, false).unwrap();

        let claude_md = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains("# Existing Notes"));
        assert!(claude_md.contains("Keep this."));
        assert!(!claude_md.contains("Terrarium Rust Tools"));
        assert_eq!(claude_md.matches("terrarium-managed:start").count(), 0);
        let local_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(local_md.contains("Terrarium Rust Tools"));
        assert_eq!(local_md.matches("terrarium-managed:start").count(), 1);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn configure_claude_migrates_legacy_managed_block_to_local_file() {
        let (base, dir, _guard) = temp_project();
        init_project(&dir, "rust", true, false).unwrap();
        fs::write(
            dir.join("CLAUDE.md"),
            format!(
                "# Existing Notes\n\n{}\n",
                single_toolchain_block(SkillKind::Rust)
            ),
        )
        .unwrap();

        configure_claude(&dir, false).unwrap();

        let claude_md = fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains("# Existing Notes"));
        assert!(!claude_md.contains("Terrarium Rust Tools"));
        assert_eq!(claude_md.matches("terrarium-managed:start").count(), 0);
        let local_md = fs::read_to_string(dir.join("CLAUDE.local.md")).unwrap();
        assert!(local_md.contains("Terrarium Rust Tools"));
        assert_eq!(local_md.matches("terrarium-managed:start").count(), 1);
        fs::remove_dir_all(&base).unwrap();
    }
}
