use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use tokio::process::Command;

use super::inputs::{BuildInput, NoInput, TestInput, parse_with_cwd};
use super::process::{CommandOutput, spawn_capture};
use super::report::format_output_block;
use super::schema::schema_for;
use super::types::{ToolCallError, ToolCallResult, ToolDefinition, success_result};

static RE_SWIFT_TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"Test Suite .+ (passed|failed) at .+\.\s+Executed (\d+) tests?, with (\d+) failures?",
    )
    .unwrap()
});
static RE_SWIFT_TESTING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Test run with (\d+) tests? (?:passed|completed .+?with (\d+) tests? failing)")
        .unwrap()
});
static RE_XCTEST_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Test Case '-\[.+? (.+?)\]' (passed|failed)").unwrap());
static RE_SWIFT_TESTING_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[✔✘◆] Test "(.+?)" (passed|failed)"#).unwrap());

#[derive(Clone)]
pub(crate) struct SwiftTools {
    pub(crate) project_root: PathBuf,
}

impl SwiftTools {
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        match name {
            "swift_build" => {
                let (input, cwd) = parse_with_cwd::<BuildInput>(&self.project_root, arguments)?;
                self.swift_build(&cwd, input).await
            }
            "swift_test" => {
                let (input, cwd) = parse_with_cwd::<TestInput>(&self.project_root, arguments)?;
                self.swift_test(&cwd, input).await
            }
            "swift_format_check" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.swift_format_check(&cwd).await
            }
            "swift_format" => {
                let (_, cwd) = parse_with_cwd::<NoInput>(&self.project_root, arguments)?;
                self.swift_format(&cwd).await
            }
            _ => Err(ToolCallError::UnknownTool(format!("unknown tool: {name}"))),
        }
    }

    async fn run(
        &self,
        effective_root: &Path,
        command: &str,
        args: &[&str],
    ) -> Result<CommandOutput, ToolCallError> {
        let mut cmd = Command::new(command);
        cmd.args(args).current_dir(effective_root);
        spawn_capture(cmd).await
    }

    async fn swift_build(
        &self,
        cwd: &Path,
        input: BuildInput,
    ) -> Result<ToolCallResult, ToolCallError> {
        let mut args = vec!["build"];
        if input.release.unwrap_or(false) {
            args.push("-c");
            args.push("release");
        }
        let output = self.run(cwd, "swift", &args).await?;
        Ok(success_result(if output.success {
            "swift build passed".to_string()
        } else {
            format!(
                "swift build **failed**{}",
                format_output_block(cwd, "swift-build", &output.combined)
            )
        }))
    }

    async fn swift_test(
        &self,
        cwd: &Path,
        input: TestInput,
    ) -> Result<ToolCallResult, ToolCallError> {
        let mut args = vec!["test"];
        let filter_holder;
        if let Some(filter) = input.filter {
            filter_holder = filter;
            args.push("--filter");
            args.push(filter_holder.as_str());
        }
        let output = self.run(cwd, "swift", &args).await?;
        let (passed, failed) = parse_swift_test_summary(&output.combined);
        let (passed_names, failed_names) = parse_swift_test_names(&output.combined);
        let md = if output.success {
            let mut lines = vec![format!("swift test passed ({passed} passed)")];
            for name in &passed_names {
                lines.push(format!("- `{name}`"));
            }
            lines.join("\n")
        } else {
            let mut lines = vec![format!(
                "swift test **failed** — {failed} failed, {passed} passed"
            )];
            for name in &failed_names {
                lines.push(format!("- `{name}`"));
            }
            lines.push(format_output_block(cwd, "swift-test", &output.combined));
            lines.join("\n")
        };
        Ok(success_result(md))
    }

    async fn swift_format_check(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self
            .run(cwd, "swift-format", &["lint", "--recursive", "."])
            .await?;
        Ok(success_result(if output.success {
            "swift-format check passed".to_string()
        } else {
            format!(
                "swift-format check **found issues**{}",
                format_output_block(cwd, "swift-format", &output.combined)
            )
        }))
    }

    async fn swift_format(&self, cwd: &Path) -> Result<ToolCallResult, ToolCallError> {
        let output = self
            .run(
                cwd,
                "swift-format",
                &["format", "--in-place", "--recursive", "."],
            )
            .await?;
        Ok(success_result(if output.success {
            "swift-format completed".to_string()
        } else {
            format!(
                "swift-format **failed**{}",
                format_output_block(cwd, "swift-format", &output.combined)
            )
        }))
    }
}

const SWIFT_TOOLS: &[&str] = &[
    "swift_build",
    "swift_test",
    "swift_format_check",
    "swift_format",
];

#[async_trait::async_trait]
impl super::Toolchain for SwiftTools {
    fn tool_names(&self) -> &'static [&'static str] {
        SWIFT_TOOLS
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        swift_definitions()
    }

    fn server_name(&self) -> &'static str {
        "terrarium-swift-tools"
    }

    fn instructions(&self) -> &'static str {
        super::swift_instructions()
    }

    async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        SwiftTools::call(self, name, arguments).await
    }
}

pub(crate) fn swift_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "swift_build",
            description: "Run `swift build` and return compiler errors",
            input_schema: schema_for::<BuildInput>(),
        },
        ToolDefinition {
            name: "swift_test",
            description: "Run `swift test` for the full suite or a filtered subset and return a compact summary",
            input_schema: schema_for::<TestInput>(),
        },
        ToolDefinition {
            name: "swift_format_check",
            description: "Run `swift-format lint --recursive .` and return formatting issues",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "swift_format",
            description: "Run `swift-format format --in-place --recursive .` to auto-format all Swift sources",
            input_schema: schema_for::<NoInput>(),
        },
    ]
}

/// Reads the run summary from either XCTest's or swift-testing's output.
fn parse_swift_test_summary(output: &str) -> (u32, u32) {
    if let Some(caps) = RE_SWIFT_TEST.captures_iter(output).last() {
        let total = caps
            .get(2)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        let failed = caps
            .get(3)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        return (total.saturating_sub(failed), failed);
    }
    if let Some(caps) = RE_SWIFT_TESTING.captures_iter(output).last() {
        let total = caps
            .get(1)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        let failed = caps
            .get(2)
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0);
        return (total.saturating_sub(failed), failed);
    }
    (0, 0)
}

/// Extracts individual case names from either runner's per-test lines.
fn parse_swift_test_names(output: &str) -> (Vec<String>, Vec<String>) {
    let mut passed = Vec::new();
    let mut failed = Vec::new();
    for line in output.lines() {
        if let Some(caps) = RE_XCTEST_CASE.captures(line) {
            let name = caps[1].to_string();
            match &caps[2] {
                "passed" => passed.push(name),
                _ => failed.push(name),
            }
        } else if let Some(caps) = RE_SWIFT_TESTING_CASE.captures(line) {
            let name = caps[1].to_string();
            match &caps[2] {
                "passed" => passed.push(name),
                _ => failed.push(name),
            }
        }
    }
    (passed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_swift_test_summary_from_xctest() {
        let output = "Test Suite 'All tests' passed at 2026-03-13 12:00:00. Executed 15 tests, with 2 failures";
        assert_eq!(parse_swift_test_summary(output), (13, 2));
    }

    #[test]
    fn parses_swift_testing_summary_all_passed() {
        let output = "Test run with 5 tests passed after 0.123 seconds.";
        assert_eq!(parse_swift_test_summary(output), (5, 0));
    }

    #[test]
    fn parses_swift_testing_summary_with_failures() {
        let output = "Test run with 10 tests completed after 0.5 seconds, with 3 tests failing.";
        assert_eq!(parse_swift_test_summary(output), (7, 3));
    }

    #[test]
    fn parses_xctest_case_names() {
        let output = "\
Test Case '-[ModuleTests.SomeTest testExample]' passed (0.001 seconds).
Test Case '-[ModuleTests.SomeTest testAnother]' passed (0.002 seconds).
Test Case '-[ModuleTests.SomeTest testFailing]' failed (0.003 seconds).
Test Suite 'All tests' passed at 2026-03-13 12:00:00. Executed 3 tests, with 1 failures";
        let (passed, failed) = parse_swift_test_names(output);
        assert_eq!(passed, vec!["testExample", "testAnother"]);
        assert_eq!(failed, vec!["testFailing"]);
    }

    #[test]
    fn parses_swift_testing_case_names() {
        let output = "\
◇ Test \"testExample\" started.
✔ Test \"testExample\" passed after 0.001 seconds.
◇ Test \"testFailing\" started.
✘ Test \"testFailing\" failed after 0.002 seconds.
Test run with 2 tests completed after 0.5 seconds, with 1 tests failing.";
        let (passed, failed) = parse_swift_test_names(output);
        assert_eq!(passed, vec!["testExample"]);
        assert_eq!(failed, vec!["testFailing"]);
    }
}
