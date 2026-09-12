//! The MCP tool surface: which tools a project advertises, and where a call to
//! one of them is routed.
//!
//! Tools come from three places. Each build toolchain ([`cargo`], [`gradle`],
//! [`swift`]) implements [`Toolchain`] and owns its own tool names. The [`git`]
//! and [`common`] tools are unconditional — every project gets them. [`ProjectTools`]
//! composes whichever of those apply and answers `tools/list` and `tools/call`.

mod cargo;
pub(crate) mod common;
mod git;
mod gradle;
mod inputs;
mod modes;
mod node;
mod paths;
mod process;
mod report;
mod schema;
mod swift;
pub mod types;

pub use common::MCP_TOOL_TIMEOUT_MS;
pub use types::{TextContent, ToolCallError, ToolCallResult, ToolDefinition};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::config::project::ProjectProfile;

use inputs::{NoInput, parse_tool_args};
pub(crate) use modes::ToolMode;
use modes::preset_to_modes;
use paths::{resolve_cwd, strip_project};

macro_rules! sandbox_notice {
    () => {
        concat!(
            "\n\nIMPORTANT: You are running inside a macOS sandbox with a deny-default policy. ",
            "If you encounter a permission denied error or an operation that is blocked by the sandbox, ",
            "call `report_violation` with a description of what was attempted and the error. ",
            "This logs the violation to violations.md so the sandbox profile can be reviewed.",
        )
    };
}

/// A build toolchain (cargo, Gradle, Swift, ...) exposed as MCP tools.
///
/// One implementation per toolchain. Composition — several toolchains on one
/// root, or one per workspace sub-project — is expressed by holding several of
/// these in [`ProjectTools::chains`], not by nesting.
#[async_trait::async_trait]
pub(crate) trait Toolchain: Send + Sync {
    /// Tool names this toolchain owns. Drives both dispatch and `handles`.
    fn tool_names(&self) -> &'static [&'static str];

    fn definitions(&self) -> Vec<ToolDefinition>;

    /// Server name reported when this is the only toolchain in play.
    fn server_name(&self) -> &'static str;

    /// Instructions reported when this is the only toolchain in play.
    fn instructions(&self) -> &'static str;

    async fn call(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<ToolCallResult, ToolCallError>;

    fn handles(&self, name: &str) -> bool {
        self.tool_names().contains(&name)
    }
}

/// The MCP tool surface for one terrarium project or workspace.
///
/// A single struct covers every shape that used to need its own enum variant:
/// no toolchain (empty `chains`), one toolchain, several toolchains on the same
/// root (each entry's path is empty), and a workspace (each entry's path names a
/// sub-project directory).
#[derive(Clone)]
pub struct ProjectTools {
    /// Project root, or workspace root when `is_workspace`.
    root: PathBuf,
    /// `(project-relative path, toolchain)`. The path is empty for a single root.
    chains: Vec<(String, Arc<dyn Toolchain>)>,
    is_workspace: bool,
    allow_commit: bool,
}

impl ProjectTools {
    pub fn new(project_root: PathBuf) -> Self {
        let profile = ProjectProfile::load(&project_root).ok();
        let allow_commit = profile.as_ref().map(|p| p.allow_commit).unwrap_or(false);
        let preset = profile.and_then(|p| p.preset);
        let modes = match preset.as_deref() {
            Some(spec) => preset_to_modes(spec),
            None => vec![ToolMode::Cargo],
        };
        Self {
            chains: chains_for_modes(&modes, &project_root),
            root: project_root,
            is_workspace: false,
            allow_commit,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_mode(mode: ToolMode, project_root: PathBuf) -> Self {
        Self {
            chains: chains_for_modes(&[mode], &project_root),
            root: project_root,
            is_workspace: false,
            allow_commit: false,
        }
    }

    pub fn for_workspace(
        workspace_root: PathBuf,
        config: &crate::config::workspace::WorkspaceConfig,
    ) -> Self {
        let chains = config
            .projects
            .iter()
            .flat_map(|project| {
                let sub_root = workspace_root.join(&project.path);
                // A multi-language sub-project contributes one entry per toolchain,
                // all pointing at the same directory.
                chains_for_modes(&preset_to_modes(&project.preset), &sub_root)
                    .into_iter()
                    .map(move |(_, chain)| (project.path.clone(), chain))
            })
            .collect();

        Self {
            root: workspace_root,
            chains,
            is_workspace: true,
            allow_commit: config.allow_commit,
        }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        // Union of every toolchain's tools, keeping the first of each name: a
        // workspace of three Rust projects still advertises `cargo_test` once.
        let mut seen = std::collections::HashSet::new();
        let mut definitions: Vec<ToolDefinition> = self
            .chains
            .iter()
            .flat_map(|(_, chain)| chain.definitions())
            .filter(|def| seen.insert(def.name))
            .collect();
        definitions.extend(git::git_definitions(self.allow_commit));
        definitions.extend(common::common_definitions(&self.root));
        definitions
    }

    pub fn server_name(&self) -> &'static str {
        match self.shape() {
            Shape::Workspace => "terrarium-workspace-tools",
            Shape::Multi => "terrarium-multi-tools",
            Shape::Single(chain) => chain.server_name(),
            Shape::Empty => "terrarium-tools",
        }
    }

    pub fn instructions(&self) -> &'static str {
        match self.shape() {
            Shape::Workspace => concat!(
                "Run build commands for multiple sub-projects from the terrarium workspace MCP server. ",
                "Each build tool targets its configured sub-project directory. ",
                "Use the built-in git inspection tools for read-only repository queries.",
                sandbox_notice!(),
            ),
            Shape::Multi => concat!(
                "This project uses several toolchains at once; the cargo, Gradle, Swift, and Node ",
                "tools listed here all target the same project root. Pick the tool family that matches ",
                "the part of the project you are changing, and keep the corresponding build files ",
                "(Cargo.toml, Gradle build files, Package.swift, package.json) aligned with your work. ",
                "Use the built-in git inspection tools for read-only repository queries.",
                sandbox_notice!(),
            ),
            Shape::Single(chain) => chain.instructions(),
            Shape::Empty => concat!(
                "Use the built-in git inspection tools for read-only repository queries.",
                sandbox_notice!(),
            ),
        }
    }

    fn shape(&self) -> Shape<'_> {
        if self.is_workspace {
            Shape::Workspace
        } else {
            match self.chains.as_slice() {
                [] => Shape::Empty,
                [(_, chain)] => Shape::Single(chain.as_ref()),
                _ => Shape::Multi,
            }
        }
    }

    pub async fn call(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        if is_common_or_git(name) {
            // In a workspace the common and git tools are project-aware: they
            // resolve against a named sub-project rather than the shared root.
            let (effective_root, arguments) = if self.is_workspace {
                self.workspace_common_root(name, &arguments)?
            } else {
                (self.root.clone(), arguments.clone())
            };
            if let Some(result) =
                dispatch_common(&effective_root, self.allow_commit, name, arguments).await
            {
                return result;
            }
        }

        self.dispatch_build(name, arguments).await
    }

    /// Resolves which root a workspace common/git tool applies to, and strips the
    /// `project` argument the toolchains below do not understand.
    fn workspace_common_root(
        &self,
        name: &str,
        arguments: &Option<Value>,
    ) -> Result<(PathBuf, Option<Value>), ToolCallError> {
        let arg = |key: &str| {
            arguments
                .as_ref()
                .and_then(|a| a.get(key))
                .and_then(|v| v.as_str())
        };
        let project = arg("project");
        if name.starts_with("git_") && project.is_none() && arg("path").is_none() {
            return Err(ToolCallError::InvalidParams(format!(
                "workspace git tool '{}' requires a 'project' or 'path' parameter to avoid diffing the workspace root. Available projects: {}",
                name,
                self.project_paths().join(", ")
            )));
        }
        let root = match project {
            Some(project) => self.resolve_project_dir(project, name)?,
            None => self.root.clone(),
        };
        Ok((root, strip_project(arguments)))
    }

    /// Routes a build tool to the toolchain that owns it.
    async fn dispatch_build(
        &self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let matches: Vec<&(String, Arc<dyn Toolchain>)> = self
            .chains
            .iter()
            .filter(|(_, chain)| chain.handles(name))
            .collect();

        match matches.as_slice() {
            [] => Err(ToolCallError::UnknownTool(name.to_string())),
            [(_, chain)] => chain.call(name, arguments).await,
            // Several sub-projects share this tool, so the caller must say which.
            _ => {
                let project = arguments
                    .as_ref()
                    .and_then(|args| args.get("project"))
                    .and_then(|v| v.as_str());
                let available = || {
                    matches
                        .iter()
                        .map(|(path, _)| path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let Some(project) = project else {
                    return Err(ToolCallError::InvalidParams(format!(
                        "multiple projects can handle '{}'. Specify a 'project' parameter. Available: {}",
                        name,
                        available()
                    )));
                };
                match matches.iter().find(|(path, _)| path == project) {
                    Some((_, chain)) => chain.call(name, strip_project(&arguments)).await,
                    None => Err(ToolCallError::InvalidParams(format!(
                        "project '{}' not found. Available projects for {}: {}",
                        project,
                        name,
                        available()
                    ))),
                }
            }
        }
    }

    fn project_paths(&self) -> Vec<&str> {
        self.chains.iter().map(|(path, _)| path.as_str()).collect()
    }

    fn resolve_project_dir(&self, project: &str, tool: &str) -> Result<PathBuf, ToolCallError> {
        if self.chains.iter().any(|(path, _)| path == project) {
            return Ok(self.root.join(project));
        }
        Err(ToolCallError::InvalidParams(format!(
            "project '{}' not found. Available projects for {}: {}",
            project,
            tool,
            self.project_paths().join(", ")
        )))
    }
}

/// Which of the four arrangements a [`ProjectTools`] is in, for the two methods
/// whose answer depends on the whole set rather than one toolchain.
enum Shape<'a> {
    Empty,
    Single(&'a dyn Toolchain),
    Multi,
    Workspace,
}

/// A project with no build toolchain. It owns no tools, but must still occupy a
/// slot in [`ProjectTools::chains`]: a workspace sub-project on the `none` preset
/// is a valid target for the project-aware git and common tools.
struct NoTools;

#[async_trait::async_trait]
impl Toolchain for NoTools {
    fn tool_names(&self) -> &'static [&'static str] {
        &[]
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![]
    }

    fn server_name(&self) -> &'static str {
        "terrarium-tools"
    }

    fn instructions(&self) -> &'static str {
        concat!(
            "Use the built-in git inspection tools for read-only repository queries.",
            sandbox_notice!(),
        )
    }

    async fn call(
        &self,
        name: &str,
        _arguments: Option<Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        Err(ToolCallError::UnknownTool(name.to_string()))
    }
}

// --- Per-toolchain instructions ---
//
// These live here because `sandbox_notice!` is a local macro_rules macro and is
// not in scope in the toolchain modules.

pub(crate) fn cargo_instructions() -> &'static str {
    concat!(
        "Run cargo commands for the project from the terrarium MCP server process. ",
        "Use the built-in git inspection tools for read-only repository queries, ",
        "keep Cargo.toml aligned with the implemented feature set, ",
        "and use the dependency update tools when refreshing Rust dependencies.",
        sandbox_notice!(),
    )
}

pub(crate) fn gradle_instructions() -> &'static str {
    concat!(
        "Run Gradle commands for the project from the terrarium MCP server process. ",
        "Use the built-in git inspection tools for read-only repository queries.",
        sandbox_notice!(),
    )
}

pub(crate) fn swift_instructions() -> &'static str {
    concat!(
        "Run Swift build commands for the project from the terrarium MCP server process. ",
        "Use the built-in git inspection tools for read-only repository queries, ",
        "and keep Package.swift aligned with the implemented feature set.",
        sandbox_notice!(),
    )
}

pub(crate) fn node_instructions() -> &'static str {
    concat!(
        "Run npm/pnpm/yarn/bun commands for the project from the terrarium MCP server process, ",
        "which resolves the package manager from the project's lockfile. ",
        "Prefer npm_run over a shell invocation, and pass background=true for dev servers ",
        "and watch modes so the call returns a live log path instead of hanging. ",
        "Use the built-in git inspection tools for read-only repository queries, ",
        "and keep package.json and the lockfile aligned with the implemented feature set.",
        sandbox_notice!(),
    )
}

/// Builds one toolchain per mode against `root`. Paths are empty: a single root.
fn chains_for_modes(modes: &[ToolMode], root: &Path) -> Vec<(String, Arc<dyn Toolchain>)> {
    modes
        .iter()
        .map(|mode| {
            let chain: Arc<dyn Toolchain> = match mode {
                ToolMode::Cargo => Arc::new(cargo::CargoTools {
                    project_root: root.to_path_buf(),
                }),
                ToolMode::Gradle => Arc::new(gradle::GradleTools {
                    project_root: root.to_path_buf(),
                }),
                ToolMode::Swift => Arc::new(swift::SwiftTools {
                    project_root: root.to_path_buf(),
                }),
                ToolMode::Node => Arc::new(node::NodeTools {
                    project_root: root.to_path_buf(),
                }),
                ToolMode::None => Arc::new(NoTools),
            };
            (String::new(), chain)
        })
        .collect()
}

/// Dispatches common and git tools. Returns `Some(result)` if handled, `None` if unrecognized.
async fn dispatch_common(
    project_root: &Path,
    allow_commit: bool,
    name: &str,
    arguments: Option<Value>,
) -> Option<Result<ToolCallResult, ToolCallError>> {
    use common::{command, files, runners, spenn, violations};

    match name {
        "make_install" => Some(match parse_tool_args::<NoInput>(arguments) {
            Ok(input) => match resolve_cwd(project_root, input.path.as_deref()) {
                Ok(effective_root) => runners::make_install(&effective_root).await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }),
        "make_run" => Some(runners::make_run(project_root, arguments).await),
        "just_run" => Some(runners::just_run(project_root, arguments).await),
        "docker_run" => Some(runners::docker_run(project_root, arguments).await),
        "command_log_read" => Some(command::command_log_read(project_root, arguments)),
        "report_violation" => Some(violations::report_violation(project_root, arguments)),
        "list_files" => Some(files::list_files(project_root, arguments)),
        "directory_structure" => Some(files::directory_structure(project_root, arguments)),
        "search_files" => Some(files::search_files(project_root, arguments).await),
        "create_directory" => Some(files::create_directory(project_root, arguments)),
        "spenn_list_scenarios" => Some(spenn::spenn_list_scenarios(project_root, arguments).await),
        "spenn_run_scenario" => Some(spenn::spenn_run_scenario(project_root, arguments).await),
        _ if name.starts_with("git_") => Some(
            git::GitTools {
                project_root: project_root.to_path_buf(),
                allow_commit,
            }
            .call(name, arguments)
            .await,
        ),
        _ => None,
    }
}

/// Returns true if `name` is handled by [`dispatch_common`] (common + git tools).
fn is_common_or_git(name: &str) -> bool {
    matches!(
        name,
        "report_violation"
            | "make_install"
            | "make_run"
            | "just_run"
            | "docker_run"
            | "command_log_read"
            | "list_files"
            | "directory_structure"
            | "search_files"
            | "create_directory"
            | "spenn_list_scenarios"
            | "spenn_run_scenario"
    ) || name.starts_with("git_")
}

#[cfg(test)]
mod tests;
