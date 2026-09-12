//! Tests for tool composition and dispatch. Per-toolchain parsing and the shared
//! helpers are tested in their own modules.

use super::*;
use crate::config::global::override_home_for_tests;
use crate::config::workspace::{WorkspaceConfig, WorkspaceProject};
use serde_json::json;

fn temp_dir(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("temp")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    base
}

/// Builds workspace tools from `(path, preset)` pairs.
fn workspace_tools(root: &Path, projects: &[(&str, &str)]) -> ProjectTools {
    let config = WorkspaceConfig {
        projects: projects
            .iter()
            .map(|(path, preset)| WorkspaceProject {
                path: (*path).to_string(),
                preset: (*preset).to_string(),
            })
            .collect(),
        network: Default::default(),
        allow_commit: false,
        command: None,
    };
    ProjectTools::for_workspace(root.to_path_buf(), &config)
}

/// A workspace of `none`-preset sub-projects, each an existing directory.
fn workspace_of(name: &str, projects: &[&str]) -> (PathBuf, ProjectTools) {
    let base = temp_dir(name);
    for project in projects {
        std::fs::create_dir_all(base.join(project)).unwrap();
    }
    let pairs: Vec<(&str, &str)> = projects.iter().map(|p| (*p, "none")).collect();
    let tools = workspace_tools(&base, &pairs);
    (base, tools)
}

// --- Tool composition ---

#[test]
fn multi_language_project_exposes_all_build_tools() {
    let base = temp_dir("mcp_tools_multi_profile");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let _guard = override_home_for_tests(base.join("home"));

    let profile = ProjectProfile::new("project", Some("node,kotlin,rust".to_string()));
    profile.save(&project).unwrap();

    let tools = ProjectTools::new(project.clone());
    let names: Vec<&str> = tools.definitions().iter().map(|d| d.name).collect();
    assert!(
        names.contains(&"cargo_check"),
        "missing cargo tools: {names:?}"
    );
    assert!(
        names.contains(&"gradle_check"),
        "missing gradle tools: {names:?}"
    );
    assert!(
        names.contains(&"git_status"),
        "missing git tools: {names:?}"
    );
    assert_eq!(tools.server_name(), "terrarium-multi-tools");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn multi_language_workspace_project_exposes_all_build_tools() {
    let base = temp_dir("mcp_tools_multi_workspace");
    std::fs::create_dir_all(base.join("svc-a")).unwrap();

    let tools = workspace_tools(&base, &[("svc-a", "kotlin,rust")]);
    let names: Vec<&str> = tools.definitions().iter().map(|d| d.name).collect();
    assert!(
        names.contains(&"cargo_check"),
        "missing cargo tools: {names:?}"
    );
    assert!(
        names.contains(&"gradle_check"),
        "missing gradle tools: {names:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn project_tools_include_git_definitions_for_both_modes() {
    for mode in [ToolMode::Cargo, ToolMode::Gradle] {
        let defs = ProjectTools::for_mode(mode, PathBuf::from(".")).definitions();
        assert!(
            defs.iter().any(|tool| tool.name == "git_status"),
            "git_status missing for {mode:?}"
        );
        assert!(
            defs.iter().any(|tool| tool.name == "git_unmerged"),
            "git_unmerged missing for {mode:?}"
        );
    }
}

#[test]
fn project_tools_include_swift_definitions_for_swift_mode() {
    let tools = ProjectTools::for_mode(ToolMode::Swift, PathBuf::from("."));
    let defs = tools.definitions();
    assert!(defs.iter().any(|t| t.name == "swift_build"));
    assert!(defs.iter().any(|t| t.name == "swift_test"));
    assert!(defs.iter().any(|t| t.name == "swift_format_check"));
    assert!(defs.iter().any(|t| t.name == "swift_format"));
    assert!(defs.iter().any(|t| t.name == "git_status"));
    assert!(defs.iter().any(|t| t.name == "report_violation"));
    assert!(!defs.iter().any(|t| t.name == "cargo_check"));
    assert!(!defs.iter().any(|t| t.name == "gradle_check"));
}

#[test]
fn none_mode_definitions_have_no_build_tools_but_have_git() {
    let tools = ProjectTools::for_mode(ToolMode::None, PathBuf::from("."));
    let defs = tools.definitions();
    assert!(defs.iter().any(|t| t.name == "git_status"));
    assert!(defs.iter().any(|t| t.name == "report_violation"));
    assert!(!defs.iter().any(|t| t.name == "cargo_check"));
    assert!(!defs.iter().any(|t| t.name == "gradle_check"));
    assert!(!defs.iter().any(|t| t.name == "swift_build"));
}

#[test]
fn common_file_tools_in_all_modes() {
    for mode in [
        ToolMode::Cargo,
        ToolMode::Gradle,
        ToolMode::Swift,
        ToolMode::None,
    ] {
        let tools = ProjectTools::for_mode(mode, PathBuf::from("."));
        let defs = tools.definitions();
        for name in [
            "list_files",
            "directory_structure",
            "search_files",
            "create_directory",
        ] {
            assert!(
                defs.iter().any(|t| t.name == name),
                "{name} missing for {mode:?}"
            );
        }
    }
}

// --- Workspace routing ---

#[tokio::test]
async fn workspace_routes_common_tool_to_sub_project() {
    let (base, tools) = workspace_of("ws_routes_common", &["svc-a", "svc-b"]);
    std::fs::write(base.join("svc-a/sentinel-a.txt"), "a").unwrap();
    std::fs::write(base.join("svc-b/sentinel-b.txt"), "b").unwrap();

    let result = tools
        .call("list_files", Some(json!({"project": "svc-a"})))
        .await
        .expect("should succeed");
    let text = &result.content[0].text;
    assert!(
        text.contains("sentinel-a.txt"),
        "expected sentinel-a.txt in: {text}"
    );
    assert!(
        !text.contains("sentinel-b.txt"),
        "should not see sentinel-b.txt in: {text}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn workspace_rejects_unknown_project() {
    let (base, tools) = workspace_of("ws_rejects_unknown", &["svc-a", "svc-b"]);

    let err = tools
        .call("list_files", Some(json!({"project": "nope"})))
        .await
        .unwrap_err();
    match &err {
        ToolCallError::InvalidParams(msg) => {
            assert!(msg.contains("nope"), "expected 'nope' in: {msg}");
            assert!(msg.contains("svc-a"), "expected 'svc-a' in: {msg}");
            assert!(msg.contains("svc-b"), "expected 'svc-b' in: {msg}");
        }
        other => panic!("expected InvalidParams, got: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn workspace_common_tool_without_project_uses_root() {
    let (base, tools) = workspace_of("ws_no_project_root", &["svc-a", "svc-b"]);
    std::fs::write(base.join("svc-a/sentinel-a.txt"), "a").unwrap();
    std::fs::write(base.join("svc-b/sentinel-b.txt"), "b").unwrap();

    let result = tools
        .call("list_files", None)
        .await
        .expect("should succeed");
    let text = &result.content[0].text;
    // Both sentinels should appear when listing from workspace root
    assert!(
        text.contains("sentinel-a.txt"),
        "expected sentinel-a.txt in: {text}"
    );
    assert!(
        text.contains("sentinel-b.txt"),
        "expected sentinel-b.txt in: {text}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn workspace_project_param_stripped_before_common() {
    // Any leakage of `project` into common tool impls would trip deny_unknown_fields.
    let (base, tools) = workspace_of("ws_project_stripped", &["svc-a"]);
    std::fs::write(base.join("svc-a/file.txt"), "x").unwrap();

    let result = tools
        .call("list_files", Some(json!({"project": "svc-a"})))
        .await;
    assert!(
        result.is_ok(),
        "project should be stripped before impl: {result:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn workspace_git_commit_honors_allow_commit_config_setting() {
    let base = temp_dir("ws_git_commit_allowed");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(workspace.join("svc-a")).unwrap();

    let config = WorkspaceConfig {
        projects: vec![WorkspaceProject {
            path: "svc-a".to_string(),
            preset: "none".to_string(),
        }],
        network: Default::default(),
        allow_commit: true,
        command: None,
    };

    let tools = ProjectTools::for_workspace(workspace.clone(), &config);
    assert!(
        tools
            .definitions()
            .iter()
            .any(|def| def.name == "git_commit")
    );

    let err = tools
        .call(
            "git_commit",
            Some(json!({"project": "svc-a", "message": "   "})),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("commit message must not be empty")),
        "git_commit should be dispatched when allowed, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn workspace_git_tool_requires_project_or_path() {
    let (base, tools) = workspace_of("ws_git_requires_target", &["svc-a", "svc-b"]);

    let err = tools
        .call("git_diff", Some(json!({"revision": "main"})))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ToolCallError::InvalidParams(ref msg)
            if msg.contains("requires a 'project' or 'path' parameter")
                && msg.contains("svc-a")
                && msg.contains("svc-b")),
        "expected explicit target error, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_git_diff_path_targets_symlinked_repo() {
    use crate::mcp::tools::git::test_support::{commit_all, init_git_repo};

    let base = temp_dir(&format!("ws_git_symlink_diff_{}", uuid::Uuid::new_v4()));
    let workspace = base.join("workspace");
    let linked_repo = base.join("linked-repo");
    let link = workspace.join("linked");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&linked_repo).unwrap();
    std::os::unix::fs::symlink(&linked_repo, &link).unwrap();

    init_git_repo(&linked_repo);
    std::fs::write(linked_repo.join("inner.txt"), "base\n").unwrap();
    commit_all(&linked_repo, "initial");
    std::fs::write(linked_repo.join("inner.txt"), "changed\n").unwrap();

    init_git_repo(&workspace);
    std::fs::write(workspace.join("outer.txt"), "outer base\n").unwrap();
    commit_all(&workspace, "outer initial");
    std::fs::write(workspace.join("outer.txt"), "outer changed\n").unwrap();

    let tools = workspace_tools(&workspace, &[("linked", "none")]);
    let result = tools
        .call(
            "git_diff",
            Some(json!({"path": "linked", "revision": "main"})),
        )
        .await
        .expect("git diff through symlink should succeed");
    let text = &result.content[0].text;

    assert!(
        text.contains("inner.txt"),
        "expected linked repo diff: {text}"
    );
    assert!(
        !text.contains("outer.txt"),
        "should not include workspace-root diff: {text}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn single_project_common_tool_ignores_project_param() {
    let base = temp_dir("single_project_ignores_project");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("file.rs"), "").unwrap();

    let tools = ProjectTools::for_mode(ToolMode::None, base.clone());
    // In single-project mode, `project` is accepted but inert
    let result = tools
        .call("list_files", Some(json!({"project": "anything"})))
        .await;
    assert!(
        result.is_ok(),
        "single-project should accept and ignore project: {result:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// The `project` parameter is resolved before any command runs, so these test the
// resolution directly rather than shelling out to git and mutating PATH.
#[test]
fn workspace_git_status_project_resolves_correct_dir() {
    let (base, tools) = workspace_of("ws_git_resolve", &["svc-a"]);

    let resolved = tools.resolve_project_dir("svc-a", "git_status").unwrap();

    assert_eq!(resolved, base.join("svc-a"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn workspace_project_param_resolves_nested_dir() {
    let (base, tools) = workspace_of("ws_nested_resolve", &["apps/api"]);

    let resolved = tools.resolve_project_dir("apps/api", "git_status").unwrap();

    assert_eq!(resolved, base.join("apps").join("api"));
    let _ = std::fs::remove_dir_all(&base);
}

// --- dispatch_common ---

#[tokio::test]
async fn dispatch_common_make_install_accepts_project_field() {
    let base = temp_dir("dispatch_common_project");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("Makefile"), "install:\n\t@echo ok\n").unwrap();

    // `project` is inert in single-project mode, but must still be accepted.
    let result = dispatch_common(
        &base,
        false,
        "make_install",
        Some(json!({"project": "foo"})),
    )
    .await;
    let result = result.expect("make_install should be handled by dispatch_common");
    if let Err(ToolCallError::InvalidParams(msg)) = &result {
        panic!("should not fail with InvalidParams, got: {msg}");
    }

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn dispatch_common_make_install_works_without_project() {
    let base = temp_dir("dispatch_common_no_project");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("Makefile"), "install:\n\t@echo ok\n").unwrap();

    let result = dispatch_common(&base, false, "make_install", Some(json!({}))).await;
    assert!(
        result.expect("should be handled").is_ok(),
        "make_install should succeed without project field"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn dispatch_common_just_run_runs_recipe() {
    let base = temp_dir("dispatch_common_just_run");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("justfile"), "ok:\n\t@echo ok\n").unwrap();

    let result = dispatch_common(&base, false, "just_run", Some(json!({"recipe": "ok"}))).await;
    let result = result.expect("just_run should be handled by dispatch_common");
    if let Err(ToolCallError::InvalidParams(msg)) = &result {
        panic!("should not fail with InvalidParams, got: {msg}");
    }

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn dispatch_common_just_run_rejects_empty_recipe() {
    let base = temp_dir("dispatch_common_just_run_empty");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("justfile"), "ok:\n\t@echo ok\n").unwrap();

    let result = dispatch_common(&base, false, "just_run", Some(json!({"recipe": "  "}))).await;
    assert!(
        matches!(
            result.expect("should be handled"),
            Err(ToolCallError::InvalidParams(_))
        ),
        "empty recipe should be rejected with InvalidParams"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn spenn_run_scenario_requires_name() {
    let base = temp_dir("spenn_run_scenario_requires_name");
    std::fs::create_dir_all(&base).unwrap();

    let result = dispatch_common(&base, false, "spenn_run_scenario", Some(json!({}))).await;
    let result = result.expect("spenn_run_scenario should be handled by dispatch_common");
    assert!(matches!(result, Err(ToolCallError::InvalidParams(_))));

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn spenn_list_scenarios_rejects_unknown_fields_before_spawn() {
    let base = temp_dir("spenn_list_scenarios_rejects_unknown_fields");
    std::fs::create_dir_all(&base).unwrap();

    let result = dispatch_common(
        &base,
        false,
        "spenn_list_scenarios",
        Some(json!({ "name": "not-used" })),
    )
    .await;
    let result = result.expect("spenn_list_scenarios should be handled by dispatch_common");
    assert!(matches!(result, Err(ToolCallError::InvalidParams(_))));

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn report_violation_rejects_path_traversal() {
    let base = temp_dir("report_violation_traversal");
    let outside = temp_dir("report_violation_traversal_outside");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let args = Some(json!({
        "description": "test",
        "path": "../report_violation_traversal_outside"
    }));
    let result = dispatch_common(&base, false, "report_violation", args).await;
    let err = result.expect("should be handled").unwrap_err();
    assert!(
        matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("escapes the project root")),
        "expected escapes error, got: {err:?}"
    );
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn every_toolchain_can_apply_formatting_not_only_check_it() {
    for (mode, check, apply) in [
        (ToolMode::Cargo, "cargo_fmt_check", "cargo_fmt"),
        (ToolMode::Gradle, "gradle_format_check", "gradle_format"),
        (ToolMode::Swift, "swift_format_check", "swift_format"),
    ] {
        let defs = ProjectTools::for_mode(mode, PathBuf::from(".")).definitions();
        assert!(
            defs.iter().any(|tool| tool.name == check),
            "{check} missing for {mode:?}"
        );
        assert!(
            defs.iter().any(|tool| tool.name == apply),
            "{apply} missing for {mode:?}"
        );
    }
}

#[test]
fn advertised_toolchain_definitions_are_all_dispatchable() {
    for mode in [ToolMode::Cargo, ToolMode::Gradle, ToolMode::Swift] {
        let tools = ProjectTools::for_mode(mode, PathBuf::from("."));
        let advertised: Vec<&str> = tools
            .definitions()
            .iter()
            .map(|def| def.name)
            .filter(|name| {
                name.starts_with("cargo_")
                    || name.starts_with("gradle_")
                    || name.starts_with("swift_")
            })
            .collect();
        for name in advertised {
            assert!(
                tools.chains.iter().any(|(_, chain)| chain.handles(name)),
                "{name} is advertised for {mode:?} but not routed"
            );
        }
    }
}
