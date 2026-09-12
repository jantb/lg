use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use tokio::process::Command;

use super::inputs::{BuildInput, NoInput, TestInput, parse_with_cwd};
use super::process::{CommandOutput, spawn_capture};
use super::report::{format_build_result, format_output_block};
use super::schema::schema_for;
use super::types::{ToolCallError, ToolCallResult, ToolDefinition, success_result};
use crate::mcp::output;

static RE_CARGO_TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"test result:\s+\w+\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;").unwrap()
});

#[derive(Clone)]
pub(crate) struct CargoTools {
    pub(crate) project_root: PathBuf,
}

impl CargoTools {
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        match name {
            "cargo_check" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_check(&cwd).await
            }
            "cargo_test" => {
                let (input, cwd) = parse_with_cwd::<TestInput>(&self.project_root, arguments)?;
                self.cargo_test(input, &cwd).await
            }
            "cargo_build" => {
                let (input, cwd) = parse_with_cwd::<BuildInput>(&self.project_root, arguments)?;
                self.cargo_build(input, &cwd).await
            }
            "cargo_clippy" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_clippy(&cwd).await
            }
            "cargo_fmt_check" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_fmt_check(&cwd).await
            }
            "cargo_fmt" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_fmt(&cwd).await
            }
            "cargo_update" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_update(&cwd).await
            }
            "cargo_upgrade_incompatible" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.cargo_upgrade_incompatible(&cwd).await
            }
            _ => Err(ToolCallError::UnknownTool(format!("unknown tool: {name}"))),
        }
    }

    async fn run(&self, cwd: &Path, args: &[&str]) -> Result<CommandOutput, ToolCallError> {
        let mut command = Command::new("cargo");
        command.args(args).current_dir(cwd);
        spawn_capture(command).await
    }

    async fn cargo_check(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["check", "--message-format=json"]).await?;
        let diagnostics = output::parse_diagnostics(output.stdout.as_bytes());
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|diag| diag.level == "error")
            .collect();
        let warnings: Vec<_> = diagnostics
            .iter()
            .filter(|diag| diag.level == "warning")
            .collect();
        Ok(success_result(format_build_result(
            "cargo check",
            "cargo-check",
            cwd,
            &output,
            &errors,
            &warnings,
        )))
    }

    async fn cargo_test(
        &self,
        input: TestInput,
        cwd: &Path,
    ) -> Result<ToolCallResult, ToolCallError> {
        let mut args = vec!["test"];
        let filter_holder;
        if let Some(filter) = input.filter {
            filter_holder = filter;
            args.push(filter_holder.as_str());
        }

        let output = self.run(cwd, &args).await?;
        let (passed, failed) = parse_cargo_test_summary(&output.combined);
        let failures = parse_cargo_test_failures(&output.combined);
        let fail_count = failed.max(failures.len() as u32);
        let md = if output.success {
            format!("cargo test passed ({passed} passed)")
        } else {
            let mut lines = vec![format!(
                "cargo test **failed** — {fail_count} failed, {passed} passed"
            )];
            for name in &failures {
                lines.push(format!("- `{name}`"));
            }
            lines.push(format_output_block(cwd, "cargo-test", &output.combined));
            lines.join("\n")
        };
        Ok(success_result(md))
    }

    async fn cargo_build(
        &self,
        input: BuildInput,
        cwd: &Path,
    ) -> Result<ToolCallResult, ToolCallError> {
        let mut args = vec!["build", "--message-format=json"];
        if input.release.unwrap_or(false) {
            args.push("--release");
        }
        let output = self.run(cwd, &args).await?;
        let diagnostics = output::parse_diagnostics(output.stdout.as_bytes());
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|diag| diag.level == "error")
            .collect();
        let warnings: Vec<_> = diagnostics
            .iter()
            .filter(|diag| diag.level == "warning")
            .collect();
        Ok(success_result(format_build_result(
            "cargo build",
            "cargo-build",
            cwd,
            &output,
            &errors,
            &warnings,
        )))
    }

    async fn cargo_clippy(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["clippy", "--message-format=json"]).await?;
        let warnings: Vec<_> = output::parse_diagnostics(output.stdout.as_bytes())
            .into_iter()
            .filter(|diag| diag.level == "warning")
            .collect();
        let warning_refs: Vec<_> = warnings.iter().collect();
        // Clippy lints arrive as warnings, so `errors` is always empty here —
        // which is exactly the case that used to mask a hard clippy failure.
        Ok(success_result(format_build_result(
            "cargo clippy",
            "cargo-clippy",
            cwd,
            &output,
            &[],
            &warning_refs,
        )))
    }

    async fn cargo_fmt_check(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["fmt", "--", "--check"]).await?;
        let mut files = output::parse_fmt_files(output.stdout.as_bytes());
        files.extend(output::parse_fmt_files(output.stderr.as_bytes()));
        files.sort();
        files.dedup();

        Ok(success_result(if output.success {
            "cargo fmt check passed".to_string()
        } else {
            let mut lines = vec!["cargo fmt check **needs formatting**:".to_string()];
            for f in &files {
                lines.push(format!("- `{f}`"));
            }
            lines.join("\n")
        }))
    }

    async fn cargo_fmt(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["fmt"]).await?;
        Ok(success_result(if output.success {
            "cargo fmt completed".to_string()
        } else {
            format!(
                "cargo fmt **failed**{}",
                format_output_block(cwd, "cargo-fmt", &output.combined)
            )
        }))
    }

    async fn cargo_update(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["update"]).await?;
        Ok(success_result(if output.success {
            "cargo update completed".to_string()
        } else {
            format!(
                "cargo update **failed**{}",
                format_output_block(cwd, "cargo-update", &output.combined)
            )
        }))
    }

    async fn cargo_upgrade_incompatible(
        &self,
        cwd: &Path,
    ) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(cwd, &["upgrade", "--incompatible"]).await?;
        Ok(success_result(if output.success {
            "cargo upgrade --incompatible completed".to_string()
        } else {
            format!(
                "cargo upgrade --incompatible **failed**{}",
                format_output_block(cwd, "cargo-upgrade", &output.combined)
            )
        }))
    }
}

const CARGO_TOOLS: &[&str] = &[
    "cargo_check",
    "cargo_test",
    "cargo_build",
    "cargo_clippy",
    "cargo_fmt_check",
    "cargo_fmt",
    "cargo_update",
    "cargo_upgrade_incompatible",
];

#[async_trait::async_trait]
impl super::Toolchain for CargoTools {
    fn tool_names(&self) -> &'static [&'static str] {
        CARGO_TOOLS
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        cargo_definitions()
    }

    fn server_name(&self) -> &'static str {
        "terrarium-cargo-tools"
    }

    fn instructions(&self) -> &'static str {
        super::cargo_instructions()
    }

    async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        CargoTools::call(self, name, arguments).await
    }
}

pub(crate) fn cargo_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "cargo_check",
            description: "Run `cargo check` and return compiler errors and warning count",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "cargo_test",
            description: "Run `cargo test` for the full suite, a single test, or a filtered subset and return a compact summary",
            input_schema: schema_for::<TestInput>(),
        },
        ToolDefinition {
            name: "cargo_build",
            description: "Run `cargo build` and return compiler errors",
            input_schema: schema_for::<BuildInput>(),
        },
        ToolDefinition {
            name: "cargo_clippy",
            description: "Run `cargo clippy` and return lint warnings",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "cargo_fmt_check",
            description: "Run `cargo fmt --check` and return files needing formatting",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "cargo_fmt",
            description: "Run `cargo fmt` to auto-format all Rust source files in the project",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "cargo_update",
            description: "Run `cargo update` and return lockfile update output",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "cargo_upgrade_incompatible",
            description: "Run `cargo upgrade --incompatible` and return dependency upgrade output",
            input_schema: schema_for::<NoInput>(),
        },
    ]
}

fn parse_cargo_test_summary(output: &str) -> (u32, u32) {
    RE_CARGO_TEST
        .captures_iter(output)
        .fold((0, 0), |(passed_total, failed_total), caps| {
            let passed = caps
                .get(1)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            let failed = caps
                .get(2)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            (
                passed_total.saturating_add(passed),
                failed_total.saturating_add(failed),
            )
        })
}

fn parse_cargo_test_failures(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            line.strip_prefix("---- ")
                .and_then(|rest| rest.strip_suffix(" stdout ----"))
                .map(|name| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_test_summary_sums_multiple_test_targets() {
        let output = "\
running 107 tests
test result: ok. 107 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s

running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s";

        assert_eq!(parse_cargo_test_summary(output), (107, 0));
    }

    #[test]
    fn parse_cargo_test_summary_sums_failures_across_targets() {
        let output = "\
test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s";

        assert_eq!(parse_cargo_test_summary(output), (5, 1));
    }
}
