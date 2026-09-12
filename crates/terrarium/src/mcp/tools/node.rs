//! The npm/pnpm/yarn/bun tools a Node project gets.
//!
//! Unlike the other toolchains there is no single program to invoke: which
//! package manager runs a script is a property of the project, recorded in its
//! lockfile or its `packageManager` field. [`PackageManager::detect`] reads that
//! once per call, so the caller names a script rather than a command line.
//!
//! Everything except `npm_test` goes through [`run_external_command`], the same
//! live-log path make and just use: Node work is dominated by watchers and dev
//! servers, which need `background=true` and a log to tail rather than a
//! captured result.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::Deserialize;

use super::common::command::{command_timeout, run_external_command, validate_command_args};
use super::inputs::{TestInput, parse_tool_args, parse_with_cwd, require_tool_args};
use super::paths::resolve_cwd;
use super::process::{CommandOutput, spawn_capture};
use super::report::format_output_block;
use super::schema::schema_for;
use super::types::{ToolCallError, ToolCallResult, ToolDefinition, success_result};

/// `Tests  1 failed | 12 passed (13)` — vitest.
static RE_VITEST_SUMMARY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Tests\s+(?:(\d+) failed\s*\|\s*)?(\d+) passed").unwrap());
/// `Tests:       1 failed, 12 passed, 13 total` — jest.
static RE_JEST_SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Tests:\s+(?:(\d+) failed,\s*)?(?:\d+ skipped,\s*)?(\d+) passed").unwrap()
});
/// `node --test` counts, from either of its reporters: the spec reporter (the
/// default when output is not a TTY-backed TAP stream) prefixes them with `\u{2139}`,
/// the TAP reporter with `#`.
static RE_NODE_TEST_PASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:#|\u{2139})\s*pass (\d+)").unwrap());
static RE_NODE_TEST_FAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:#|\u{2139})\s*fail (\d+)").unwrap());

/// The package manager a project's scripts run under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    /// The lockfile that names each manager, most specific first. `npm` is last
    /// because `package-lock.json` is also what npm writes alongside another
    /// manager's lockfile when someone runs it by accident.
    const BY_LOCKFILE: &'static [(&'static str, PackageManager)] = &[
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("bun.lockb", PackageManager::Bun),
        ("bun.lock", PackageManager::Bun),
        ("yarn.lock", PackageManager::Yarn),
        ("package-lock.json", PackageManager::Npm),
    ];

    fn program(self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
        }
    }

    /// Detects the manager for `dir`, falling back to `npm`.
    ///
    /// The lockfile wins over `packageManager`: a checked-in lockfile is what
    /// the install will actually be resolved against, whereas the field is a
    /// declaration corepack may not have acted on yet.
    pub(crate) fn detect(dir: &Path) -> Self {
        for (lockfile, manager) in Self::BY_LOCKFILE {
            if dir.join(lockfile).exists() {
                return *manager;
            }
        }
        Self::from_package_manager_field(dir).unwrap_or(PackageManager::Npm)
    }

    /// Reads the `packageManager` field corepack keys off, e.g. `"pnpm@9.1.0"`.
    fn from_package_manager_field(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
        let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
        let field = manifest.get("packageManager")?.as_str()?;
        let name = field.split('@').next()?;
        match name {
            "npm" => Some(PackageManager::Npm),
            "pnpm" => Some(PackageManager::Pnpm),
            "yarn" => Some(PackageManager::Yarn),
            "bun" => Some(PackageManager::Bun),
            _ => None,
        }
    }

    /// The argv that installs exactly what the lockfile pins.
    ///
    /// Without a lockfile there is nothing to be faithful to, so the plain
    /// install runs instead of the frozen one, which would just fail.
    fn install_args(self, dir: &Path, frozen: bool) -> Vec<String> {
        let owned = |parts: &[&str]| parts.iter().map(|s| (*s).to_string()).collect();
        if !frozen || !self.has_lockfile(dir) {
            return match self {
                PackageManager::Npm => owned(&["install"]),
                PackageManager::Pnpm => owned(&["install"]),
                PackageManager::Yarn => owned(&["install"]),
                PackageManager::Bun => owned(&["install"]),
            };
        }
        match self {
            PackageManager::Npm => owned(&["ci"]),
            PackageManager::Pnpm => owned(&["install", "--frozen-lockfile"]),
            // Berry spells it `--immutable`; Classic only knows
            // `--frozen-lockfile`. The config file is what tells them apart.
            PackageManager::Yarn if dir.join(".yarnrc.yml").exists() => {
                owned(&["install", "--immutable"])
            }
            PackageManager::Yarn => owned(&["install", "--frozen-lockfile"]),
            PackageManager::Bun => owned(&["install", "--frozen-lockfile"]),
        }
    }

    fn has_lockfile(self, dir: &Path) -> bool {
        Self::BY_LOCKFILE
            .iter()
            .any(|(lockfile, manager)| *manager == self && dir.join(lockfile).exists())
    }

    /// The argv that adds dependencies. `yarn`/`bun` use `add`; npm and pnpm
    /// accept package names after `install`.
    fn add_args(self, packages: &[String], dev: bool) -> Vec<String> {
        let mut args: Vec<String> = match self {
            PackageManager::Npm | PackageManager::Pnpm => vec!["install".to_string()],
            PackageManager::Yarn | PackageManager::Bun => vec!["add".to_string()],
        };
        if dev {
            args.push(match self {
                PackageManager::Npm => "--save-dev".to_string(),
                _ => "--dev".to_string(),
            });
        }
        args.extend(packages.iter().cloned());
        args
    }

    /// The argv that runs a package.json script.
    ///
    /// Every manager needs `--` before the script's own arguments, or they are
    /// consumed as flags to the manager itself.
    fn run_args(self, script: &str, script_args: &[String]) -> Vec<String> {
        let mut args = vec!["run".to_string(), script.to_string()];
        if !script_args.is_empty() {
            args.push("--".to_string());
            args.extend(script_args.iter().cloned());
        }
        args
    }

    /// The argv that executes a binary from `node_modules/.bin`.
    fn exec_args(self, package: &str, package_args: &[String]) -> Vec<String> {
        let mut args: Vec<String> = match self {
            PackageManager::Npm => vec!["--yes".to_string()],
            PackageManager::Pnpm | PackageManager::Yarn => vec!["exec".to_string()],
            PackageManager::Bun => vec!["x".to_string()],
        };
        args.push(package.to_string());
        args.extend(package_args.iter().cloned());
        args
    }

    /// `npx` rather than `npm` is what executes a package's binary.
    fn exec_program(self) -> &'static str {
        match self {
            PackageManager::Npm => "npx",
            other => other.program(),
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct NpmInstallInput {
    /// Packages to add, e.g. ["vitest", "@types/node"]. Omit to install what the
    /// lockfile already pins.
    packages: Vec<String>,
    /// Add the packages as devDependencies. Ignored when `packages` is empty.
    dev: Option<bool>,
    /// Install exactly what the lockfile pins (npm ci / --frozen-lockfile).
    /// Default true. Ignored when `packages` is given. Set false to let the
    /// install update the lockfile.
    frozen: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Optional working directory relative to project root
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NpmRunInput {
    /// The package.json script to run, e.g. "build" or "dev".
    script: String,
    /// Arguments appended to the script after `--`.
    #[serde(default)]
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    #[serde(default)]
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    #[serde(default)]
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[serde(default)]
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the script and return immediately with a live log path and PID.
    /// Use this for dev servers and watch modes, which never exit.
    #[serde(default)]
    background: Option<bool>,
    /// Optional working directory relative to project root
    #[serde(default)]
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeRunInput {
    /// Arguments passed directly after `node`, e.g. ["--test", "src/app.test.js"]
    /// or ["scripts/bundle-assets.mjs"].
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    #[serde(default)]
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    #[serde(default)]
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[serde(default)]
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the command and return immediately with a live log path and PID.
    #[serde(default)]
    background: Option<bool>,
    /// Optional working directory relative to project root
    #[serde(default)]
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NpxRunInput {
    /// The package whose binary to execute, e.g. "tsc" or "playwright".
    package: String,
    /// Arguments passed to that binary, e.g. ["--noEmit"].
    #[serde(default)]
    args: Vec<String>,
    /// Optional Rust regex. When provided, only matching output lines are
    /// returned.
    #[serde(default)]
    output_regex: Option<String>,
    /// Apply output_regex case-insensitively.
    #[serde(default)]
    case_insensitive: Option<bool>,
    /// Synchronous wait timeout in seconds. Default 3600, max 21600.
    #[serde(default)]
    #[schemars(range(min = 1, max = 21600))]
    timeout_secs: Option<u64>,
    /// Start the command and return immediately with a live log path and PID.
    #[serde(default)]
    background: Option<bool>,
    /// Optional working directory relative to project root
    #[serde(default)]
    path: Option<String>,
    /// Project name to target when multiple projects are available
    #[serde(default)]
    #[allow(dead_code)]
    project: Option<String>,
}

#[derive(Clone)]
pub(crate) struct NodeTools {
    pub(crate) project_root: PathBuf,
}

impl NodeTools {
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        match name {
            "npm_install" => self.npm_install(arguments).await,
            "npm_run" => self.npm_run(arguments).await,
            "npm_test" => {
                let (input, cwd) = parse_with_cwd::<TestInput>(&self.project_root, arguments)?;
                self.npm_test(&cwd, input).await
            }
            "node_run" => self.node_run(arguments).await,
            "npx_run" => self.npx_run(arguments).await,
            _ => Err(ToolCallError::UnknownTool(format!("unknown tool: {name}"))),
        }
    }

    async fn npm_install(
        &self,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let input = parse_tool_args::<NpmInstallInput>(arguments)?;
        let cwd = resolve_cwd(&self.project_root, input.path.as_deref())?;
        let manager = PackageManager::detect(&cwd);
        let packages = validate_command_args(input.packages, "packages")?;
        let args = if packages.is_empty() {
            manager.install_args(&cwd, input.frozen.unwrap_or(true))
        } else {
            manager.add_args(&packages, input.dev.unwrap_or(false))
        };
        run_external_command(
            &cwd,
            manager.program(),
            &args,
            None,
            false,
            command_timeout(input.timeout_secs)?,
            false,
        )
        .await
    }

    async fn npm_run(
        &self,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let input = require_tool_args::<NpmRunInput>(arguments)?;
        let cwd = resolve_cwd(&self.project_root, input.path.as_deref())?;
        let script = non_empty(&input.script, "script")?;
        let manager = PackageManager::detect(&cwd);
        let script_args = validate_command_args(input.args, "script args")?;
        run_external_command(
            &cwd,
            manager.program(),
            &manager.run_args(&script, &script_args),
            input.output_regex.as_deref(),
            input.case_insensitive.unwrap_or(false),
            command_timeout(input.timeout_secs)?,
            input.background.unwrap_or(false),
        )
        .await
    }

    async fn npm_test(
        &self,
        cwd: &Path,
        input: TestInput,
    ) -> Result<ToolCallResult, ToolCallError> {
        let manager = PackageManager::detect(cwd);
        let mut args = manager.run_args("test", &[]);
        if let Some(filter) = input.filter {
            let filter = non_empty(&filter, "filter")?;
            // Every runner in common use (vitest, jest, mocha) treats a bare
            // trailing argument as a name pattern.
            if args.last().map(String::as_str) != Some("--") {
                args.push("--".to_string());
            }
            args.push(filter);
        }
        let output = self.capture(cwd, manager.program(), &args).await?;
        let (passed, failed) = parse_test_summary(&output.combined);
        Ok(success_result(if output.success {
            match passed {
                Some(passed) => format!("{} test passed ({passed} passed)", manager.program()),
                None => format!("{} test passed", manager.program()),
            }
        } else {
            let counts = match (passed, failed) {
                (Some(passed), Some(failed)) => format!(" — {failed} failed, {passed} passed"),
                _ => String::new(),
            };
            format!(
                "{} test **failed**{counts}{}",
                manager.program(),
                format_output_block(cwd, "npm-test", &output.combined)
            )
        }))
    }

    async fn node_run(
        &self,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let input = require_tool_args::<NodeRunInput>(arguments)?;
        let cwd = resolve_cwd(&self.project_root, input.path.as_deref())?;
        let args = validate_command_args(input.args, "node args")?;
        if args.is_empty() {
            return Err(ToolCallError::InvalidParams(
                "node args must contain at least one argument".to_string(),
            ));
        }
        run_external_command(
            &cwd,
            "node",
            &args,
            input.output_regex.as_deref(),
            input.case_insensitive.unwrap_or(false),
            command_timeout(input.timeout_secs)?,
            input.background.unwrap_or(false),
        )
        .await
    }

    async fn npx_run(
        &self,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        let input = require_tool_args::<NpxRunInput>(arguments)?;
        let cwd = resolve_cwd(&self.project_root, input.path.as_deref())?;
        let package = non_empty(&input.package, "package")?;
        let manager = PackageManager::detect(&cwd);
        let package_args = validate_command_args(input.args, "package args")?;
        run_external_command(
            &cwd,
            manager.exec_program(),
            &manager.exec_args(&package, &package_args),
            input.output_regex.as_deref(),
            input.case_insensitive.unwrap_or(false),
            command_timeout(input.timeout_secs)?,
            input.background.unwrap_or(false),
        )
        .await
    }

    async fn capture(
        &self,
        cwd: &Path,
        program: &str,
        args: &[String],
    ) -> Result<CommandOutput, ToolCallError> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args).current_dir(cwd);
        spawn_capture(cmd).await
    }
}

/// A blank script or package name would run the manager's default instead of
/// what the caller asked for, so it is rejected rather than trimmed away.
fn non_empty(value: &str, label: &str) -> Result<String, ToolCallError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolCallError::InvalidParams(format!(
            "{label} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

/// Reads `(passed, failed)` out of whichever runner produced the output.
///
/// Node has no common test protocol, so each runner is matched separately and an
/// unrecognized one yields `None` — the raw output is reported instead of a
/// fabricated count.
fn parse_test_summary(output: &str) -> (Option<u32>, Option<u32>) {
    let number = |m: Option<regex::Match<'_>>| m.and_then(|m| m.as_str().parse::<u32>().ok());

    for re in [&*RE_VITEST_SUMMARY, &*RE_JEST_SUMMARY] {
        if let Some(caps) = re.captures_iter(output).last() {
            return (number(caps.get(2)), Some(number(caps.get(1)).unwrap_or(0)));
        }
    }
    if let Some(caps) = RE_NODE_TEST_PASS.captures_iter(output).last() {
        let failed = RE_NODE_TEST_FAIL
            .captures_iter(output)
            .last()
            .and_then(|c| number(c.get(1)));
        return (number(caps.get(1)), Some(failed.unwrap_or(0)));
    }
    (None, None)
}

const NODE_TOOLS: &[&str] = &["npm_install", "npm_run", "npm_test", "node_run", "npx_run"];

#[async_trait::async_trait]
impl super::Toolchain for NodeTools {
    fn tool_names(&self) -> &'static [&'static str] {
        NODE_TOOLS
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        node_definitions()
    }

    fn server_name(&self) -> &'static str {
        "terrarium-node-tools"
    }

    fn instructions(&self) -> &'static str {
        super::node_instructions()
    }

    async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        NodeTools::call(self, name, arguments).await
    }
}

pub(crate) fn node_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "npm_install",
            description: "Install dependencies with the project's package manager (npm, pnpm, yarn, or bun, detected from the lockfile). Omit `packages` for a lockfile-faithful install; pass `packages` to add dependencies.",
            input_schema: schema_for::<NpmInstallInput>(),
        },
        ToolDefinition {
            name: "npm_run",
            description: "Run a package.json script with the project's package manager. Output is always written live to a log. Use `args` for arguments passed to the script, optional `background` for dev servers and watch modes, and optional `output_regex` to return only matching output lines.",
            input_schema: schema_for::<NpmRunInput>(),
        },
        ToolDefinition {
            name: "npm_test",
            description: "Run the project's `test` script and return a compact pass/fail summary. Optional `filter` is passed through as a test name pattern.",
            input_schema: schema_for::<TestInput>(),
        },
        ToolDefinition {
            name: "node_run",
            description: "Run `node` directly with structured arguments, for example [\"--test\", \"src/app.test.js\"] or [\"scripts/build.mjs\"]. Output is always written live to a log.",
            input_schema: schema_for::<NodeRunInput>(),
        },
        ToolDefinition {
            name: "npx_run",
            description: "Execute a package binary from node_modules/.bin (npx, pnpm exec, yarn exec, or bun x, matching the project's package manager). Use for one-off tools such as `tsc --noEmit` or `playwright test`.",
            input_schema: schema_for::<NpxRunInput>(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A scratch project directory holding just the marker files a test needs.
    fn dir_with(files: &[(&str, &str)]) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("node_tools_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        for (name, contents) in files {
            fs::write(dir.join(name), contents).unwrap();
        }
        dir
    }

    #[test]
    fn detects_package_manager_from_lockfile() {
        for (lockfile, expected) in [
            ("pnpm-lock.yaml", PackageManager::Pnpm),
            ("yarn.lock", PackageManager::Yarn),
            ("bun.lockb", PackageManager::Bun),
            ("package-lock.json", PackageManager::Npm),
        ] {
            let dir = dir_with(&[(lockfile, "")]);
            assert_eq!(PackageManager::detect(&dir), expected, "for {lockfile}");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// A pnpm lockfile beside a stray `package-lock.json` still means pnpm.
    #[test]
    fn lockfile_precedence_prefers_the_specific_manager() {
        let dir = dir_with(&[("pnpm-lock.yaml", ""), ("package-lock.json", "{}")]);
        assert_eq!(PackageManager::detect(&dir), PackageManager::Pnpm);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_package_manager_from_corepack_field() {
        let dir = dir_with(&[("package.json", r#"{"packageManager":"pnpm@9.1.0"}"#)]);
        assert_eq!(PackageManager::detect(&dir), PackageManager::Pnpm);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn defaults_to_npm_without_any_marker() {
        let dir = dir_with(&[("package.json", "{}")]);
        assert_eq!(PackageManager::detect(&dir), PackageManager::Npm);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frozen_install_uses_the_lockfile_faithful_command() {
        let dir = dir_with(&[("package-lock.json", "{}")]);
        assert_eq!(
            PackageManager::Npm.install_args(&dir, true),
            vec!["ci".to_string()]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `npm ci` fails outright without a lockfile, so the plain install runs.
    #[test]
    fn frozen_install_without_lockfile_falls_back_to_plain_install() {
        let dir = dir_with(&[("package.json", "{}")]);
        assert_eq!(
            PackageManager::Npm.install_args(&dir, true),
            vec!["install".to_string()]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Berry and Classic spell the same intent differently.
    #[test]
    fn yarn_frozen_install_matches_the_configured_major_version() {
        let berry = dir_with(&[
            ("yarn.lock", ""),
            (".yarnrc.yml", "nodeLinker: node-modules"),
        ]);
        assert_eq!(
            PackageManager::Yarn.install_args(&berry, true),
            vec!["install".to_string(), "--immutable".to_string()]
        );
        fs::remove_dir_all(&berry).unwrap();

        let classic = dir_with(&[("yarn.lock", "")]);
        assert_eq!(
            PackageManager::Yarn.install_args(&classic, true),
            vec!["install".to_string(), "--frozen-lockfile".to_string()]
        );
        fs::remove_dir_all(&classic).unwrap();
    }

    /// Without the separator, `--port` would be read as a flag to npm itself.
    #[test]
    fn script_arguments_are_separated_from_the_manager() {
        assert_eq!(
            PackageManager::Npm.run_args("dev", &["--port".to_string(), "3000".to_string()]),
            vec!["run", "dev", "--", "--port", "3000"]
        );
        assert_eq!(
            PackageManager::Npm.run_args("build", &[]),
            vec!["run", "build"]
        );
    }

    #[test]
    fn exec_uses_each_managers_own_runner() {
        assert_eq!(PackageManager::Npm.exec_program(), "npx");
        assert_eq!(
            PackageManager::Npm.exec_args("tsc", &["--noEmit".to_string()]),
            vec!["--yes", "tsc", "--noEmit"]
        );
        assert_eq!(PackageManager::Pnpm.exec_program(), "pnpm");
        assert_eq!(
            PackageManager::Pnpm.exec_args("tsc", &["--noEmit".to_string()]),
            vec!["exec", "tsc", "--noEmit"]
        );
        assert_eq!(PackageManager::Bun.exec_args("tsc", &[]), vec!["x", "tsc"]);
    }

    #[test]
    fn add_args_place_the_dev_flag_each_manager_understands() {
        assert_eq!(
            PackageManager::Npm.add_args(&["vitest".to_string()], true),
            vec!["install", "--save-dev", "vitest"]
        );
        assert_eq!(
            PackageManager::Yarn.add_args(&["vitest".to_string()], true),
            vec!["add", "--dev", "vitest"]
        );
        assert_eq!(
            PackageManager::Pnpm.add_args(&["vitest".to_string()], false),
            vec!["install", "vitest"]
        );
    }

    #[test]
    fn parses_vitest_summary() {
        assert_eq!(
            parse_test_summary("Tests  1 failed | 12 passed (13)"),
            (Some(12), Some(1))
        );
        assert_eq!(
            parse_test_summary("Tests  13 passed (13)"),
            (Some(13), Some(0))
        );
    }

    #[test]
    fn parses_jest_summary() {
        assert_eq!(
            parse_test_summary("Tests:       1 failed, 12 passed, 13 total"),
            (Some(12), Some(1))
        );
        assert_eq!(
            parse_test_summary("Tests:       2 skipped, 11 passed, 13 total"),
            (Some(11), Some(0))
        );
    }

    #[test]
    fn parses_node_test_runner_tap_summary() {
        let output = "# tests 13\n# suites 0\n# pass 12\n# fail 1\n# cancelled 0\n";
        assert_eq!(parse_test_summary(output), (Some(12), Some(1)));
    }

    /// What `node --test` actually prints today: the spec reporter, not TAP.
    #[test]
    fn parses_node_test_runner_spec_summary() {
        let output = "\u{2714} adds (0.28ms)\n\u{2139} tests 2\n\u{2139} suites 0\n\u{2139} pass 2\n\u{2139} fail 0\n";
        assert_eq!(parse_test_summary(output), (Some(2), Some(0)));
    }

    /// An unrecognized runner must not produce a made-up count.
    #[test]
    fn unrecognized_runner_reports_no_counts() {
        assert_eq!(parse_test_summary("everything is fine"), (None, None));
    }
}
