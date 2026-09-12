//! Read-only file inspection scoped to the project: listing, tree view, content
//! search, plus directory creation.
//!
//! These exist so an agent can inspect the project through the sandboxed server
//! process rather than through its own file access, which the Seatbelt profile
//! restricts.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::anyhow;
use ignore::WalkBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::process::Command;

use crate::mcp::tools::inputs::{parse_tool_args, require_tool_args};
use crate::mcp::tools::paths::{
    contains_hidden_terrarium_component, hidden_terrarium_path_error, resolve_existing_path,
};
use crate::mcp::tools::report::{MAX_OUTPUT_CHARS, truncate_text};
use crate::mcp::tools::types::{ToolCallError, ToolCallResult, success_result};

const LIST_FILES_LIMIT: usize = 500;
const DIR_STRUCT_MAX_DEPTH: u32 = 5;

/// Directories never worth walking: version control, build output, dependencies,
/// and terrarium's own state.
const SKIP_DIRS: &[&str] = &[".git", "target", "build", "node_modules", ".terrarium"];

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ListFilesInput {
    /// Glob pattern to filter files (e.g. "**/*.rs", "*.toml")
    pattern: Option<String>,
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Project name to target when multiple projects are available
    project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DirectoryStructureInput {
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Maximum depth to display (default 3, max 5)
    #[schemars(range(min = 1, max = 5))]
    depth: Option<u32>,
    /// Project name to target when multiple projects are available
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchFilesInput {
    /// Regex pattern to search for in file contents
    pattern: String,
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Glob pattern to filter files (e.g. "*.rs")
    glob: Option<String>,
    /// Perform case-insensitive search
    case_insensitive: Option<bool>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateDirectoryInput {
    /// Directory path to create, relative to the project root (e.g. "src/models")
    path: String,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

pub(crate) fn list_files(
    root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = parse_tool_args::<ListFilesInput>(arguments)?;
    let base = resolve_base(root, input.path.as_deref())?;
    let mut files: Vec<String> = Vec::new();
    let mut truncated = false;
    // Refused rather than ignored. `Option` here means "no pattern was asked for", and a
    // pattern that would not compile used to collapse into the same `None` — so a typo
    // answered with every file in the project, which reads as a filter that matched.
    let matcher = match input.pattern.as_deref() {
        Some(pattern) => Some(compile_glob(pattern).ok_or_else(|| {
            ToolCallError::InvalidParams(format!("pattern '{pattern}' is not a valid glob"))
        })?),
        None => None,
    };
    collect_files(root, &base, matcher.as_ref(), &mut files, &mut truncated);
    files.sort();
    let count = files.len();
    let mut lines = vec![format!(
        "{count} files{}",
        if truncated { " (truncated)" } else { "" }
    )];
    for f in &files {
        lines.push(f.clone());
    }
    Ok(success_result(lines.join("\n")))
}

pub(crate) fn directory_structure(
    root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = parse_tool_args::<DirectoryStructureInput>(arguments)?;
    let base = resolve_base(root, input.path.as_deref())?;
    let max_depth = input.depth.unwrap_or(3).min(DIR_STRUCT_MAX_DEPTH);
    let display_name = base.file_name().and_then(|n| n.to_str()).unwrap_or(".");
    let mut lines = vec![display_name.to_string()];
    build_tree(&base, "", max_depth, &mut lines);
    let tree = truncate_text(&lines.join("\n"), MAX_OUTPUT_CHARS);
    Ok(success_result(tree))
}

pub(crate) async fn search_files(
    root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<SearchFilesInput>(arguments)?;
    let search_path = match &input.path {
        Some(sub) => {
            let abs = resolve_existing_path(root, sub)?;
            abs.strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        }
        None => ".".to_string(),
    };
    let mut args: Vec<String> = vec![
        "--line-number".to_string(),
        "--no-heading".to_string(),
        "--color=never".to_string(),
    ];
    if input.case_insensitive.unwrap_or(false) {
        args.push("--ignore-case".to_string());
    }
    if let Some(glob) = &input.glob {
        args.push("--glob".to_string());
        args.push(glob.clone());
    }
    args.push(input.pattern.clone());
    args.push(search_path);

    let mut command = Command::new("rg");
    command
        .args(&args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = match command.spawn() {
        Ok(c) => c,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(success_result(
                "ripgrep (rg) is not installed or not in PATH".to_string(),
            ));
        }
        Err(err) => return Err(ToolCallError::Execution(anyhow!(err.to_string()))),
    };
    let output = child
        .wait_with_output()
        .await
        .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let matches = truncate_text(&stdout, MAX_OUTPUT_CHARS);
    Ok(success_result(if matches.is_empty() {
        format!("no matches for `{}`", input.pattern)
    } else {
        matches
    }))
}

pub(crate) fn create_directory(
    root: &Path,
    arguments: Option<serde_json::Value>,
) -> Result<ToolCallResult, ToolCallError> {
    let input = require_tool_args::<CreateDirectoryInput>(arguments)?;
    if Path::new(&input.path).is_absolute() {
        return Err(ToolCallError::InvalidParams(format!(
            "path '{}' must be relative to the project root",
            input.path
        )));
    }
    if contains_hidden_terrarium_component(Path::new(&input.path)) {
        return Err(hidden_terrarium_path_error(&input.path));
    }
    // The target does not exist yet, so `resolve_existing_path` cannot be used;
    // the containment check is done lexically against the canonical root.
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut resolved = canonical_root.clone();
    for component in Path::new(&input.path).components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::Normal(c) => resolved.push(c),
            _ => {}
        }
    }
    if !resolved.starts_with(&canonical_root) {
        return Err(ToolCallError::InvalidParams(format!(
            "path '{}' escapes the project root",
            input.path
        )));
    }
    std::fs::create_dir_all(&resolved).map_err(|e| {
        ToolCallError::Execution(anyhow!("failed to create directory '{}': {e}", input.path))
    })?;
    let rel = resolved.strip_prefix(&canonical_root).unwrap_or(&resolved);
    Ok(success_result(format!("Created {}", rel.display())))
}

fn resolve_base(root: &Path, sub: Option<&str>) -> Result<PathBuf, ToolCallError> {
    match sub {
        Some(sub) => resolve_existing_path(root, sub),
        None => Ok(root.to_path_buf()),
    }
}

/// Compiles a `list_files` pattern.
///
/// `literal_separator` keeps `*` from crossing a path separator, so `*.toml`
/// matches `Cargo.toml` but not `src/Cargo.toml`; `**/` still spans zero or more
/// directories. An unparseable pattern matches nothing rather than erroring, as
/// the hand-rolled matcher this replaced did.
fn compile_glob(pattern: &str) -> Option<globset::GlobMatcher> {
    globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .ok()
        .map(|glob| glob.compile_matcher())
}

fn collect_files(
    root: &Path,
    dir: &Path,
    pattern: Option<&globset::GlobMatcher>,
    files: &mut Vec<String>,
    truncated: &mut bool,
) {
    let walk = WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !SKIP_DIRS.contains(&name.as_ref())
        })
        .build();

    for result in walk {
        if *truncated {
            break;
        }
        let Ok(entry) = result else { continue };
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel.to_string_lossy().to_string();
        if pattern.is_none_or(|p| p.is_match(&rel_str)) {
            if files.len() >= LIST_FILES_LIMIT {
                *truncated = true;
                break;
            }
            files.push(rel_str);
        }
    }
}

fn build_tree(dir: &Path, prefix: &str, depth: u32, lines: &mut Vec<String>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            !SKIP_DIRS.contains(&s.as_ref()) && !s.starts_with('.')
        })
        .collect();
    entries.sort_by(|a, b| {
        let a_dir = a.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let b_dir = b.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        b_dir.cmp(&a_dir).then(a.file_name().cmp(&b.file_name()))
    });
    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i + 1 == count;
        let connector = if is_last { "└── " } else { "├── " };
        lines.push(format!(
            "{prefix}{connector}{}",
            entry.file_name().to_string_lossy()
        ));
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let ext = if is_last { "    " } else { "│   " };
            build_tree(&entry.path(), &format!("{prefix}{ext}"), depth - 1, lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(name);
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    /// Bridges the old direct-call tests onto the compiled matcher.
    fn glob_is_match(pattern: &str, path: &str) -> bool {
        compile_glob(pattern).is_some_and(|m| m.is_match(path))
    }

    /// A filter that cannot be compiled must not answer with everything. The whole project
    /// looks exactly like a pattern that matched, which is the one answer a filter must
    /// never give by accident.
    #[test]
    fn an_unparseable_pattern_is_refused_rather_than_ignored() {
        let base = temp_dir(&format!("list_files_bad_glob_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("lib.rs"), "").unwrap();

        let err = list_files(&base, Some(json!({"pattern": "[unclosed"}))).unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("not a valid glob")),
            "expected invalid params, got {err:?}"
        );
        // A pattern that does compile still filters.
        let result = list_files(&base, Some(json!({"pattern": "**/*.rs"}))).unwrap();
        assert!(result.content[0].text.contains("lib.rs"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn glob_matches_double_star() {
        assert!(glob_is_match("**/*.rs", "src/lib.rs"));
        assert!(glob_is_match("**/*.rs", "lib.rs"));
        assert!(glob_is_match("**/*.rs", "a/b/c/foo.rs"));
        assert!(!glob_is_match("**/*.rs", "src/lib.toml"));
    }

    #[test]
    fn glob_matches_single_star() {
        assert!(glob_is_match("*.toml", "Cargo.toml"));
        assert!(!glob_is_match("*.toml", "src/Cargo.toml"));
        assert!(!glob_is_match("*.toml", "hello.rs"));
    }

    #[test]
    fn list_files_returns_matching_files() {
        let base = temp_dir("list_files_matching");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("foo.rs"), "").unwrap();
        std::fs::write(base.join("bar.toml"), "").unwrap();

        let result = list_files(&base, Some(json!({ "pattern": "*.rs" }))).unwrap();
        let text = &result.content[0].text;
        assert!(text.contains("foo.rs"));
        assert!(!text.contains("bar.toml"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_files_skips_skip_dirs() {
        let base = temp_dir("list_files_skip_dirs");
        let target_dir = base.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("artifact.rs"), "").unwrap();
        std::fs::write(base.join("main.rs"), "").unwrap();

        let result = list_files(&base, None).unwrap();
        let text = &result.content[0].text;
        assert!(!text.contains("target"));
        assert!(text.contains("main.rs"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_files_rejects_hidden_terrarium_path() {
        let base = temp_dir("list_files_hidden_terrarium");
        std::fs::create_dir_all(base.join(".terrarium")).unwrap();
        std::fs::write(base.join(".terrarium/profile.toml"), "").unwrap();

        let err = list_files(&base, Some(json!({ "path": ".terrarium" }))).unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("hidden")),
            "expected hidden-path InvalidParams, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn list_files_allows_symlinked_repo_under_project() {
        let base = temp_dir(&format!("list_files_linked_repo_{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        let linked_repo = base.join("linked-repo");
        let link = project.join("linked");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(linked_repo.join("src")).unwrap();
        std::fs::write(linked_repo.join("src/lib.rs"), "").unwrap();
        std::os::unix::fs::symlink(&linked_repo, &link).unwrap();

        let result = list_files(&project, Some(json!({"path": "linked"}))).unwrap();
        let text = &result.content[0].text;

        assert!(
            text.contains("linked/src/lib.rs"),
            "expected linked repo file in: {text}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn directory_structure_respects_depth() {
        let base = temp_dir("dir_struct_depth");
        let sub = base.join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("deep.rs"), "").unwrap();
        std::fs::write(base.join("src").join("lib.rs"), "").unwrap();

        let result = directory_structure(&base, Some(json!({ "depth": 1 }))).unwrap();
        let tree = &result.content[0].text;
        assert!(tree.contains("src"));
        assert!(!tree.contains("deep.rs"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn directory_structure_skips_skip_dirs() {
        let base = temp_dir("dir_struct_skip");
        let node_modules = base.join("node_modules");
        std::fs::create_dir_all(&node_modules).unwrap();
        std::fs::write(node_modules.join("pkg.js"), "").unwrap();
        std::fs::write(base.join("index.js"), "").unwrap();

        let result = directory_structure(&base, None).unwrap();
        let tree = &result.content[0].text;
        assert!(!tree.contains("node_modules"));
        assert!(tree.contains("index.js"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn search_files_allows_symlinked_repo_under_project() {
        let base = temp_dir(&format!(
            "search_files_linked_repo_{}",
            uuid::Uuid::new_v4()
        ));
        let project = base.join("project");
        let linked_repo = base.join("linked-repo");
        let link = project.join("linked");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(linked_repo.join("src")).unwrap();
        std::fs::write(linked_repo.join("src/lib.rs"), "needle").unwrap();
        std::os::unix::fs::symlink(&linked_repo, &link).unwrap();

        let result = search_files(
            &project,
            Some(json!({"path": "linked", "pattern": "needle"})),
        )
        .await
        .unwrap();
        let text = &result.content[0].text;

        assert!(
            text.contains("linked/src/lib.rs"),
            "expected linked repo match in: {text}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_directory_creates_nested_dirs() {
        let base = temp_dir("create_dir_nested");
        std::fs::create_dir_all(&base).unwrap();

        let result = create_directory(&base, Some(json!({ "path": "a/b/c" }))).unwrap();
        assert!(base.join("a/b/c").is_dir());
        assert!(result.content[0].text.contains("a/b/c"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_directory_rejects_traversal() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join("create_dir_traversal");
        std::fs::create_dir_all(&base).unwrap();

        let err = create_directory(&base, Some(json!({ "path": "../../escape" }))).unwrap_err();
        assert!(
            matches!(err, ToolCallError::InvalidParams(_)),
            "expected InvalidParams, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_directory_rejects_absolute_path() {
        let base = temp_dir("create_dir_absolute");
        std::fs::create_dir_all(&base).unwrap();

        let err = create_directory(&base, Some(json!({ "path": "/tmp/terrarium-absolute" })))
            .unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("must be relative")),
            "expected relative-path InvalidParams, got {err:?}"
        );
        assert!(!base.join("tmp/terrarium-absolute").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_directory_rejects_hidden_terrarium_path() {
        let base = temp_dir("create_dir_hidden_terrarium");
        std::fs::create_dir_all(&base).unwrap();

        let err =
            create_directory(&base, Some(json!({ "path": ".terrarium/output" }))).unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("hidden")),
            "expected hidden-path InvalidParams, got {err:?}"
        );
        assert!(!base.join(".terrarium").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
