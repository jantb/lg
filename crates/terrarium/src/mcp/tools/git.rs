use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::anyhow;
use tokio::process::Command;

use super::inputs::{
    GitCommitInput, GitDiffInput, GitLogInput, GitShowInput, NoInput, parse_tool_args,
    require_tool_args,
};
use super::paths::resolve_cwd;
use super::process::{CommandOutput, command_output};
use super::report::format_output_block;
use super::schema::schema_for;
use super::types::{TextContent, ToolCallError, ToolCallResult, ToolDefinition};

#[derive(Clone)]
pub(crate) struct GitTools {
    pub(crate) project_root: PathBuf,
    pub(crate) allow_commit: bool,
}

impl GitTools {
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        match name {
            "git_status" => {
                let input = parse_tool_args::<NoInput>(arguments)?;
                self.git_status(input.path.as_deref()).await
            }
            "git_unmerged" => {
                let input = parse_tool_args::<NoInput>(arguments)?;
                self.git_unmerged(input.path.as_deref()).await
            }
            "git_log" => {
                let input = parse_tool_args::<GitLogInput>(arguments)?;
                self.git_log(input).await
            }
            "git_diff" => {
                let input = parse_tool_args::<GitDiffInput>(arguments)?;
                self.git_diff(input).await
            }
            "git_show" => {
                let input = require_tool_args::<GitShowInput>(arguments)?;
                self.git_show(input).await
            }
            "git_branches_unmerged" => {
                let input = parse_tool_args::<NoInput>(arguments)?;
                self.git_branches_unmerged(input.path.as_deref()).await
            }
            "git_commit" => {
                if !self.allow_commit {
                    return Err(ToolCallError::UnknownTool(
                        "unknown tool: git_commit".to_string(),
                    ));
                }
                let input = require_tool_args::<GitCommitInput>(arguments)?;
                self.git_commit(input).await
            }
            _ => Err(ToolCallError::UnknownTool(format!("unknown tool: {name}"))),
        }
    }

    async fn run(
        &self,
        args: &[String],
        path: Option<&str>,
    ) -> Result<CommandOutput, ToolCallError> {
        let cwd = resolve_cwd(&self.project_root, path)?;
        let cwd = cwd.canonicalize().unwrap_or(cwd);
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|err| ToolCallError::Execution(anyhow!(err.to_string())))?;
        Ok(command_output(
            output.stdout,
            output.stderr,
            output.status.success(),
        ))
    }

    pub(super) async fn git_status(
        &self,
        path: Option<&str>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let output = self
            .run(
                &[
                    "--no-pager".to_string(),
                    "status".to_string(),
                    "--short".to_string(),
                    "--branch".to_string(),
                ],
                path,
            )
            .await?;
        git_result_with_file(&self.project_root, "git status", output)
    }

    async fn git_log(&self, input: GitLogInput) -> Result<ToolCallResult, ToolCallError> {
        let limit = input.limit.unwrap_or(10).clamp(1, 50);
        let mut args = vec![
            "--no-pager".to_string(),
            "log".to_string(),
            "--oneline".to_string(),
            "--decorate".to_string(),
            format!("-n{limit}"),
        ];
        if let Some(revision) = input.revision {
            args.push(validate_git_revision(&revision)?);
        }
        let output = self.run(&args, input.path.as_deref()).await?;
        git_result_with_file(&self.project_root, "git log", output)
    }

    async fn git_unmerged(&self, path: Option<&str>) -> Result<ToolCallResult, ToolCallError> {
        let output = self
            .run(
                &[
                    "--no-pager".to_string(),
                    "diff".to_string(),
                    "--name-only".to_string(),
                    "--diff-filter=U".to_string(),
                ],
                path,
            )
            .await?;
        git_result_with_file(
            &self.project_root,
            "git diff --name-only --diff-filter=U",
            output,
        )
    }

    async fn git_branches_unmerged(
        &self,
        path: Option<&str>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let output = self
            .run(
                &[
                    "branch".to_string(),
                    "--no-merged".to_string(),
                    "main".to_string(),
                ],
                path,
            )
            .await?;
        let filtered = output
            .combined
            .lines()
            .filter(|line| {
                let name = line.trim().trim_start_matches("* ");
                !matches!(name, "develop" | "release/next")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let label = "git-branch--no-merged-main";
        let text = if output.success {
            format!(
                "git branch --no-merged main completed{}",
                format_output_block(&self.project_root, label, &filtered)
            )
        } else {
            format!(
                "git branch --no-merged main **failed**{}",
                format_output_block(&self.project_root, label, &output.combined)
            )
        };
        Ok(ToolCallResult {
            content: vec![TextContent { kind: "text", text }],
            is_error: !output.success,
        })
    }

    async fn git_diff(&self, input: GitDiffInput) -> Result<ToolCallResult, ToolCallError> {
        let path = input.path.as_deref();
        let mut args = vec!["--no-pager".to_string(), "diff".to_string()];
        if input.stat.unwrap_or(false) {
            args.push("--stat".to_string());
        }
        if input.staged.unwrap_or(false) {
            args.push("--cached".to_string());
        }
        if let Some(revision) = input.revision {
            args.push(validate_git_revision(&revision)?);
        }
        if let Some(pathspec) = input.pathspec {
            let validated = validate_git_pathspec(&pathspec)?;
            reject_symlinked_repo_pathspec(&self.project_root, path, &validated)?;
            args.push("--".to_string());
            args.push(validated);
        }
        let output = self.run(&args, path).await?;
        git_result_with_file(&self.project_root, "git diff", output)
    }

    async fn git_show(&self, input: GitShowInput) -> Result<ToolCallResult, ToolCallError> {
        let mut args = vec!["--no-pager".to_string(), "show".to_string()];
        if input.patch.unwrap_or(true) {
            args.push("-p".to_string());
        } else {
            args.push("--stat".to_string());
            args.push("--summary".to_string());
        }
        args.push("--format=fuller".to_string());
        args.push(validate_git_revision(&input.revision)?);
        let output = self.run(&args, input.path.as_deref()).await?;
        git_result_with_file(&self.project_root, "git show", output)
    }

    async fn git_commit(&self, input: GitCommitInput) -> Result<ToolCallResult, ToolCallError> {
        let message = input.message.trim();
        if message.is_empty() {
            return Err(ToolCallError::InvalidParams(
                "commit message must not be empty".to_string(),
            ));
        }
        if input.all {
            let stage_output = self
                .run(
                    &["add".to_string(), "-u".to_string()],
                    input.path.as_deref(),
                )
                .await?;
            if !stage_output.success {
                return git_result_with_file(&self.project_root, "git add -u", stage_output);
            }
        }
        let output = self
            .run(
                &["commit".to_string(), "-m".to_string(), message.to_string()],
                input.path.as_deref(),
            )
            .await?;
        git_result_with_file(&self.project_root, "git commit", output)
    }
}

pub(crate) fn git_definitions(allow_commit: bool) -> Vec<ToolDefinition> {
    let mut defs = vec![
        ToolDefinition {
            name: "git_status",
            description: "Run `git status --short --branch` and return the output",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "git_unmerged",
            description: "Run `git diff --name-only --diff-filter=U` and return unresolved merge files",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "git_branches_unmerged",
            description: "List branches not yet merged into main, filtering out develop and release/next",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "git_log",
            description: "Run `git log --oneline --decorate` for a limited number of commits on an optional revision and return the output",
            input_schema: schema_for::<GitLogInput>(),
        },
        ToolDefinition {
            name: "git_diff",
            description: "Run git diff for the working tree or staged changes, returning full patch content by default. Supports revision ranges and path filtering.",
            input_schema: schema_for::<GitDiffInput>(),
        },
        ToolDefinition {
            name: "git_show",
            description: "Run git show for a single revision, returning full patch content by default",
            input_schema: schema_for::<GitShowInput>(),
        },
    ];
    if allow_commit {
        defs.push(ToolDefinition {
            name: "git_commit",
            description: "Stage tracked changes (optional) and create a git commit with the given message",
            input_schema: schema_for::<GitCommitInput>(),
        });
    }
    defs
}

fn git_result_with_file(
    project_root: &Path,
    command: &str,
    output: CommandOutput,
) -> Result<ToolCallResult, ToolCallError> {
    let label = command.replace(' ', "-");
    let text = if output.success {
        format!(
            "{command} completed{}",
            format_output_block(project_root, &label, &output.combined)
        )
    } else {
        format!(
            "{command} **failed**{}",
            format_output_block(project_root, &label, &output.combined)
        )
    };
    Ok(ToolCallResult {
        content: vec![TextContent { kind: "text", text }],
        is_error: !output.success,
    })
}

fn validate_git_arg(value: &str, label: &str) -> Result<String, ToolCallError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    if trimmed.starts_with('-') {
        return Err(ToolCallError::InvalidParams(format!(
            "{label} must not start with '-'"
        )));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_git_revision(revision: &str) -> Result<String, ToolCallError> {
    validate_git_arg(revision, "git revision")
}

fn validate_git_pathspec(path: &str) -> Result<String, ToolCallError> {
    validate_git_arg(path, "pathspec")
}

fn reject_symlinked_repo_pathspec(
    project_root: &Path,
    cwd_path: Option<&str>,
    pathspec: &str,
) -> Result<(), ToolCallError> {
    if cwd_path.is_some() {
        return Ok(());
    }

    let path = Path::new(pathspec);
    let Some(std::path::Component::Normal(first)) = path.components().next() else {
        return Ok(());
    };
    let link = project_root.join(first);
    let Ok(metadata) = std::fs::symlink_metadata(&link) else {
        return Ok(());
    };
    if !metadata.file_type().is_symlink() {
        return Ok(());
    }

    let Ok(target) = link.canonicalize() else {
        return Ok(());
    };
    let Ok(root) = project_root.canonicalize() else {
        return Ok(());
    };
    if target.starts_with(&root) || !target.join(".git").exists() {
        return Ok(());
    }

    let path = first.to_string_lossy();
    Err(ToolCallError::InvalidParams(format!(
        "pathspec '{}' points at a symlinked git repository. Use 'path': '{}' to run git in that repository instead of diffing the outer project.",
        pathspec, path
    )))
}

/// Git fixtures shared with the workspace-routing tests in [`super::tests`].
#[cfg(test)]
pub(super) mod test_support {
    use std::path::Path;

    pub(crate) fn init_git_repo(repo: &Path) {
        run_git(repo, &["init"]);
        run_git(repo, &["checkout", "-B", "main"]);
    }

    /// Stages everything and commits it with an identity, so the fixture does not
    /// depend on the machine's git config.
    pub(crate) fn commit_all(repo: &Path, message: &str) {
        run_git(repo, &["add", "-A"]);
        run_git(
            repo,
            &[
                "-c",
                "user.name=Terrarium Test",
                "-c",
                "user.email=terrarium@example.invalid",
                "commit",
                "-m",
                message,
            ],
        );
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git should be available");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            repo.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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

    fn tools_for(project_root: PathBuf) -> GitTools {
        GitTools {
            project_root,
            allow_commit: false,
        }
    }

    #[test]
    fn git_definitions_are_available() {
        let definitions = git_definitions(false);
        for name in [
            "git_status",
            "git_unmerged",
            "git_log",
            "git_diff",
            "git_show",
        ] {
            assert!(
                definitions.iter().any(|tool| tool.name == name),
                "{name} missing"
            );
        }
    }

    #[test]
    fn git_revision_rejects_option_injection() {
        let err = validate_git_revision("--cached").unwrap_err();
        match err {
            ToolCallError::InvalidParams(message) => {
                assert!(message.contains("must not start with '-'"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn git_schemas_expose_path_and_project() {
        let defs = git_definitions(true);
        let check = |name: &str| {
            let def = defs.iter().find(|d| d.name == name).expect(name);
            let props = def.input_schema.get("properties").expect("has properties");
            assert!(props.get("path").is_some(), "{name}: missing 'path'");
            assert!(props.get("project").is_some(), "{name}: missing 'project'");
        };
        check("git_log");
        check("git_show");
        check("git_commit");
        check("git_diff");
    }

    /// `path` selects the cwd and `pathspec` filters inside it — two different
    /// things that a single `path` parameter used to conflate.
    #[test]
    fn git_diff_schema_has_pathspec_not_path_alias() {
        let defs = git_definitions(false);
        let def = defs.iter().find(|d| d.name == "git_diff").unwrap();
        let props = def.input_schema.get("properties").unwrap();
        assert!(props.get("pathspec").is_some(), "missing 'pathspec'");
        assert!(props.get("path").is_some(), "missing 'path' (cwd)");
        let desc = props["pathspec"]["description"].as_str().unwrap();
        assert!(
            desc.to_lowercase().contains("pathspec"),
            "pathspec description should say 'pathspec', got: {desc}"
        );
    }

    #[tokio::test]
    async fn git_run_rejects_path_traversal() {
        let base = temp_dir("git_run_traversal");
        std::fs::create_dir_all(&base).unwrap();
        // create a sibling so the traversal target exists
        let sibling = temp_dir("git_run_traversal_outside");
        std::fs::create_dir_all(&sibling).unwrap();

        let err = tools_for(base.clone())
            .git_status(Some("../git_run_traversal_outside"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg) if msg.contains("escapes the project root")),
            "expected escapes error, got: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    #[tokio::test]
    async fn single_project_git_tool_accepts_path() {
        let base = temp_dir("single_project_git_path");
        std::fs::create_dir_all(base.join("sub")).unwrap();

        // `sub` exists, so resolving the cwd must succeed even though git itself
        // then fails for want of a repository.
        let result = tools_for(base.clone()).git_status(Some("sub")).await;

        if let Err(ToolCallError::InvalidParams(msg)) = &result {
            panic!("should not get InvalidParams for valid sub path, got: {msg}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_diff_rejects_symlinked_repo_as_pathspec() {
        let base = temp_dir(&format!(
            "git_diff_pathspec_symlink_{}",
            uuid::Uuid::new_v4()
        ));
        let project = base.join("project");
        let linked_repo = base.join("linked-repo");
        let link = project.join("linked");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&linked_repo).unwrap();
        std::os::unix::fs::symlink(&linked_repo, &link).unwrap();
        test_support::init_git_repo(&linked_repo);

        let err = tools_for(project.clone())
            .call(
                "git_diff",
                Some(json!({
                    "revision": "main",
                    "pathspec": "linked"
                })),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(err, ToolCallError::InvalidParams(ref msg)
                if msg.contains("symlinked git repository")
                    && msg.contains("Use 'path': 'linked'")),
            "expected symlinked repo pathspec guidance, got: {err:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
