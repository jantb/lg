//! The argument structs shared across toolchains, and the two ways of reading
//! them out of a `tools/call` request.
//!
//! Each tool's `inputSchema` is derived from its input struct via
//! [`super::schema::schema_for`], so the schema the client sees and the struct
//! the arguments deserialize into cannot drift apart. Field doc comments become
//! the JSON Schema `description` for that property — they are the tool's
//! user-facing documentation, not internal notes.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::paths::resolve_cwd;
use super::types::ToolCallError;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct NoInput {
    /// Optional working directory relative to project root
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    pub(crate) project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TestInput {
    /// Optional single test name or subset filter
    pub(crate) filter: Option<String>,
    /// Optional working directory relative to project root
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    pub(crate) project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BuildInput {
    /// Build in release mode
    pub(crate) release: Option<bool>,
    /// Optional working directory relative to project root
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    pub(crate) project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct GitLogInput {
    /// Maximum number of commits to return
    #[schemars(range(min = 1, max = 50))]
    pub(crate) limit: Option<u32>,
    /// Optional revision, branch, or tag to inspect
    pub(crate) revision: Option<String>,
    /// Optional working directory relative to project root
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    pub(crate) project: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct GitDiffInput {
    /// Inspect staged changes instead of unstaged changes
    pub(crate) staged: Option<bool>,
    /// Revision to diff against, supports range syntax (e.g. A..B, A...B)
    pub(crate) revision: Option<String>,
    /// Restrict the diff to a pathspec inside the cwd (e.g. a file or
    /// directory). For a symlinked repository, use 'path' as the cwd instead of
    /// passing the symlink as pathspec.
    pub(crate) pathspec: Option<String>,
    /// Return only file change statistics instead of full patch content
    /// (default: false)
    pub(crate) stat: Option<bool>,
    /// Optional working directory relative to project root
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    #[allow(dead_code)]
    pub(crate) project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitShowInput {
    /// Revision, branch, tag, or commit to show
    pub(crate) revision: String,
    /// Include full patch content (default: true). Set to false for stats only.
    pub(crate) patch: Option<bool>,
    /// Optional working directory relative to project root
    #[serde(default)]
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitCommitInput {
    /// Commit message
    pub(crate) message: String,
    /// Stage all tracked changes before committing (git add -u). Default: false
    /// (commit staged changes only)
    #[serde(default)]
    pub(crate) all: bool,
    /// Optional working directory relative to project root
    #[serde(default)]
    pub(crate) path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) project: Option<String>,
}

/// An input struct carrying the optional `path` that selects the working
/// directory, so [`parse_with_cwd`] can resolve it generically.
pub(crate) trait HasPath {
    fn path(&self) -> Option<&str>;
}

impl HasPath for NoInput {
    fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl HasPath for TestInput {
    fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl HasPath for BuildInput {
    fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

/// Parses tool arguments and resolves the working directory they select — the
/// pair every build tool performs before doing anything else.
pub(crate) fn parse_with_cwd<T>(
    project_root: &Path,
    arguments: Option<Value>,
) -> Result<(T, PathBuf), ToolCallError>
where
    T: Default + HasPath + for<'de> Deserialize<'de>,
{
    let input = parse_tool_args::<T>(arguments)?;
    let cwd = resolve_cwd(project_root, input.path())?;
    Ok((input, cwd))
}

/// Parses arguments for a tool whose every field is optional, so a missing
/// `arguments` object is the same as an empty one.
pub(crate) fn parse_tool_args<T>(arguments: Option<Value>) -> Result<T, ToolCallError>
where
    T: Default + for<'de> Deserialize<'de>,
{
    match arguments {
        None | Some(Value::Null) => Ok(T::default()),
        Some(value) => serde_json::from_value(value)
            .map_err(|err| ToolCallError::InvalidParams(format!("invalid tool arguments: {err}"))),
    }
}

/// Parses arguments for a tool with at least one required field, where a missing
/// `arguments` object is an error rather than a default.
pub(crate) fn require_tool_args<T>(arguments: Option<Value>) -> Result<T, ToolCallError>
where
    T: for<'de> Deserialize<'de>,
{
    match arguments {
        None | Some(Value::Null) => Err(ToolCallError::InvalidParams(
            "this tool requires arguments".to_string(),
        )),
        Some(value) => serde_json::from_value(value)
            .map_err(|err| ToolCallError::InvalidParams(format!("invalid tool arguments: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_args_reject_unknown_keys() {
        let err = parse_tool_args::<NoInput>(Some(json!({ "unexpected": true }))).unwrap_err();
        match err {
            ToolCallError::InvalidParams(message) => {
                assert!(message.contains("unknown field"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn no_input_accepts_path_and_project() {
        // `project` is accepted but inert in single-project mode.
        let args = Some(json!({"project": "foo", "path": "bar"}));
        assert!(parse_tool_args::<NoInput>(args).is_ok());
    }

    #[test]
    fn no_input_rejects_unknown_fields() {
        assert!(
            parse_tool_args::<NoInput>(Some(json!({"unexpected": true}))).is_err(),
            "NoInput should reject genuinely unknown fields"
        );
    }

    #[test]
    fn no_input_accepts_path_only() {
        assert!(parse_tool_args::<NoInput>(Some(json!({"path": "subdir"}))).is_ok());
    }

    #[test]
    fn cargo_build_schema_accepts_release_only() {
        let parsed: BuildInput =
            parse_tool_args(Some(json!({ "release": true }))).expect("valid args");
        assert_eq!(parsed.release, Some(true));
    }

    #[test]
    fn test_input_accepts_project() {
        let input: TestInput = parse_tool_args(Some(json!({
            "filter": "my_test",
            "project": "svc-a"
        })))
        .expect("TestInput should accept project");
        assert_eq!(input.filter.as_deref(), Some("my_test"));
        assert_eq!(input.project.as_deref(), Some("svc-a"));
    }

    #[test]
    fn build_input_accepts_project() {
        let input: BuildInput = parse_tool_args(Some(json!({
            "release": true,
            "project": "svc-a"
        })))
        .expect("BuildInput should accept project");
        assert_eq!(input.release, Some(true));
        assert_eq!(input.project.as_deref(), Some("svc-a"));
    }

    #[test]
    fn git_log_accepts_path_and_project() {
        let input: GitLogInput = parse_tool_args(Some(json!({
            "limit": 3,
            "path": "sub",
            "project": "svc-a"
        })))
        .expect("should parse");
        assert_eq!(input.limit, Some(3));
        assert_eq!(input.path.as_deref(), Some("sub"));
        assert_eq!(input.project.as_deref(), Some("svc-a"));
    }

    #[test]
    fn git_show_accepts_path_and_project() {
        let input: GitShowInput = require_tool_args(Some(json!({
            "revision": "HEAD",
            "path": "sub",
            "project": "svc-a"
        })))
        .expect("should parse");
        assert_eq!(input.revision, "HEAD");
        assert_eq!(input.path.as_deref(), Some("sub"));
        assert_eq!(input.project.as_deref(), Some("svc-a"));
    }

    #[test]
    fn git_commit_accepts_path_and_project() {
        let input: GitCommitInput = require_tool_args(Some(json!({
            "message": "m",
            "path": "sub",
            "project": "svc-a"
        })))
        .expect("should parse");
        assert_eq!(input.message, "m");
        assert_eq!(input.path.as_deref(), Some("sub"));
        assert_eq!(input.project.as_deref(), Some("svc-a"));
    }

    #[test]
    fn git_diff_pathspec_and_path_coexist() {
        let input: GitDiffInput = parse_tool_args(Some(json!({
            "path": "svc-a",
            "pathspec": "src",
            "revision": "HEAD"
        })))
        .expect("should parse");
        assert_eq!(input.path.as_deref(), Some("svc-a"));
        assert_eq!(input.pathspec.as_deref(), Some("src"));
        assert_eq!(input.revision.as_deref(), Some("HEAD"));
    }
}
