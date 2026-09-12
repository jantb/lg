//! Which build toolchains a preset spec asks for.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolMode {
    Cargo,
    Gradle,
    Swift,
    Node,
    None,
}

fn preset_to_mode(preset: &str) -> ToolMode {
    match preset {
        "kotlin" => ToolMode::Gradle,
        "swift" => ToolMode::Swift,
        "node" => ToolMode::Node,
        // Python has no MCP build tools yet; its preset is a sandbox and
        // allowlist grant, so it carries the same empty tool set as `none`.
        "none" | "python" => ToolMode::None,
        _ => ToolMode::Cargo,
    }
}

/// Maps a (possibly multi-language) preset spec to its tool modes.
///
/// `ToolMode::None` carries no build tools, so it is dropped when another
/// toolchain is present (e.g. `node,rust` exposes the cargo tools).
pub(crate) fn preset_to_modes(spec: &str) -> Vec<ToolMode> {
    let mut modes: Vec<ToolMode> = Vec::new();
    for preset in crate::profile::presets::parse_presets(spec) {
        let mode = preset_to_mode(&preset);
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    if modes.len() > 1 {
        modes.retain(|mode| *mode != ToolMode::None);
    }
    modes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::{TestHomeGuard, override_home_for_tests};
    use crate::config::project::ProjectProfile;
    use std::path::{Path, PathBuf};

    /// The mode a project's saved profile selects, defaulting to cargo when the
    /// profile is missing. Only the tests need this — production code goes
    /// through [`preset_to_modes`], which handles multi-language specs.
    fn detect_tool_mode(project_root: &Path) -> ToolMode {
        ProjectProfile::load(project_root)
            .ok()
            .and_then(|profile| profile.preset)
            .as_deref()
            .map(preset_to_mode)
            .unwrap_or(ToolMode::Cargo)
    }

    /// Writes a profile with `preset` into a scratch project and returns its root.
    fn project_with_preset(name: &str, preset: &str) -> (PathBuf, PathBuf, TestHomeGuard) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(name);
        let project = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&project).unwrap();
        let guard = override_home_for_tests(base.join("home"));
        ProjectProfile::new("project", Some(preset.to_string()))
            .save(&project)
            .unwrap();
        (base, project, guard)
    }

    #[test]
    fn preset_to_modes_covers_every_toolchain() {
        assert_eq!(
            preset_to_modes("node,kotlin,rust"),
            vec![ToolMode::Node, ToolMode::Gradle, ToolMode::Cargo]
        );
        assert_eq!(preset_to_modes("node"), vec![ToolMode::Node]);
        assert_eq!(preset_to_modes("swift"), vec![ToolMode::Swift]);
        assert_eq!(preset_to_modes("python"), vec![ToolMode::None]);
    }

    /// `none` carries no tools, so it drops out beside a real toolchain — but a
    /// Python sub-project must not drop the Node tools of its sibling either.
    #[test]
    fn preset_to_modes_drops_the_empty_mode_beside_a_toolchain() {
        assert_eq!(preset_to_modes("python,node"), vec![ToolMode::Node]);
        assert_eq!(preset_to_modes("none"), vec![ToolMode::None]);
    }

    #[test]
    fn detects_gradle_mode_from_kotlin_profile() {
        let (base, project, _guard) = project_with_preset("mcp_tools_kotlin_profile", "kotlin");
        assert_eq!(detect_tool_mode(&project), ToolMode::Gradle);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn detects_swift_mode_from_swift_profile() {
        let (base, project, _guard) = project_with_preset("mcp_tools_swift_profile", "swift");
        assert_eq!(detect_tool_mode(&project), ToolMode::Swift);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn detects_none_mode_from_none_profile() {
        let (base, project, _guard) = project_with_preset("mcp_tools_none_profile", "none");
        assert_eq!(detect_tool_mode(&project), ToolMode::None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn detects_node_mode_from_node_profile() {
        let (base, project, _guard) = project_with_preset("mcp_tools_node_profile", "node");
        assert_eq!(detect_tool_mode(&project), ToolMode::Node);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn defaults_to_cargo_mode_when_profile_missing() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("mcp_tools_missing_profile");
        let project = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&project).unwrap();

        assert_eq!(detect_tool_mode(&project), ToolMode::Cargo);
        let _ = std::fs::remove_dir_all(&base);
    }
}
