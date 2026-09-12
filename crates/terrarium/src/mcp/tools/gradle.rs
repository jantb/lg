use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::anyhow;
use regex::Regex;
use tokio::process::Command;

use super::inputs::{NoInput, TestInput, parse_tool_args};
use super::paths::resolve_cwd;
use super::process::{CommandOutput, spawn_capture};
use super::report::format_output_block;
use super::schema::schema_for;
use super::types::{ToolCallError, ToolCallResult, ToolDefinition, success_result};

static RE_GRADLE_FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^> Task (.+?) FAILED$").unwrap());
static RE_GRADLE_TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(\d+)\s+tests?\s+completed(?:,\s+(\d+)\s+failed)?(?:,\s+(\d+)\s+skipped)?")
        .unwrap()
});
static RE_XML_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([A-Za-z_:][A-Za-z0-9_.:-]*)="([^"]*)""#).unwrap());

#[derive(Clone)]
pub(crate) struct GradleTools {
    pub(crate) project_root: PathBuf,
}

impl GradleTools {
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        match name {
            "gradle_check" => {
                self.gradle_check(parse_tool_args::<NoInput>(arguments)?)
                    .await
            }
            "gradle_test" => {
                self.gradle_test(parse_tool_args::<TestInput>(arguments)?)
                    .await
            }
            "gradle_build" => {
                self.gradle_build(parse_tool_args::<NoInput>(arguments)?)
                    .await
            }
            "gradle_format_check" => {
                self.gradle_format_check(parse_tool_args::<NoInput>(arguments)?)
                    .await
            }
            "gradle_format" => {
                self.gradle_format(parse_tool_args::<NoInput>(arguments)?)
                    .await
            }
            _ => Err(ToolCallError::UnknownTool(format!("unknown tool: {name}"))),
        }
    }

    async fn run(
        &self,
        task_args: &[String],
        effective_root: &Path,
    ) -> Result<CommandOutput, ToolCallError> {
        let (command, args) = if gradle_wrapper_is_usable(effective_root) {
            let mut args = Vec::with_capacity(task_args.len() + 3);
            args.push("./gradlew".to_string());
            args.push("--console=plain".to_string());
            args.push("--no-daemon".to_string());
            args.extend(task_args.iter().cloned());
            ("/bin/sh".to_string(), args)
        } else {
            let mut args = Vec::with_capacity(task_args.len() + 2);
            args.push("--console=plain".to_string());
            args.push("--no-daemon".to_string());
            args.extend(task_args.iter().cloned());
            ("gradle".to_string(), args)
        };

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = Command::new(&command);
        command.args(&arg_refs).current_dir(effective_root);
        if let Some(java_home) = detect_java_home().as_deref() {
            command.env("JAVA_HOME", java_home);
        }
        spawn_capture(command).await
    }

    fn clear_project_cache(&self) -> Result<(), ToolCallError> {
        let project_cache = self.project_root.join(".gradle");
        if !project_cache.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&project_cache).map_err(|err| {
            ToolCallError::Execution(anyhow!(
                "failed to clear project Gradle cache at {}: {err}",
                project_cache.display()
            ))
        })
    }

    async fn run_gradle_task(
        &self,
        task: &str,
        label: &str,
        effective_root: &Path,
    ) -> Result<ToolCallResult, ToolCallError> {
        let output = self.run(&[String::from(task)], effective_root).await?;
        let failed_tasks = parse_gradle_failed_tasks(&output.combined);
        Ok(success_result(if output.success {
            format!("gradle {label} passed")
        } else {
            let mut lines = vec![format!(
                "gradle {label} **failed** in {} tasks:",
                failed_tasks.len()
            )];
            for t in &failed_tasks {
                lines.push(format!("- `{t}`"));
            }
            lines.push(format_output_block(
                effective_root,
                &format!("gradle-{label}"),
                &output.combined,
            ));
            lines.join("\n")
        }))
    }

    async fn gradle_check(&self, input: NoInput) -> Result<ToolCallResult, ToolCallError> {
        let root = resolve_cwd(&self.project_root, input.path.as_deref())?;
        self.run_gradle_task("check", "check", &root).await
    }

    async fn gradle_test(&self, input: TestInput) -> Result<ToolCallResult, ToolCallError> {
        let root = resolve_cwd(&self.project_root, input.path.as_deref())?;
        self.clear_project_cache()?;
        let args = gradle_test_args(input.filter.as_deref());
        let output = self.run(&args, &root).await?;
        let (total, failed, skipped, _count_source, _result_files_found) =
            summarize_gradle_test_results(&root, &output.combined);
        let passed = total.saturating_sub(failed + skipped);
        let md = if output.success {
            format!("gradle test passed ({passed} passed)")
        } else {
            let failed_tasks = parse_gradle_failed_tasks(&output.combined);
            let mut lines = vec![format!(
                "gradle test **failed** — {failed} failed, {passed} passed, {skipped} skipped"
            )];
            for t in &failed_tasks {
                lines.push(format!("- `{t}`"));
            }
            lines.push(format_output_block(&root, "gradle-test", &output.combined));
            lines.join("\n")
        };
        Ok(success_result(md))
    }

    async fn gradle_build(&self, input: NoInput) -> Result<ToolCallResult, ToolCallError> {
        let root = resolve_cwd(&self.project_root, input.path.as_deref())?;
        self.run_gradle_task("build", "build", &root).await
    }

    async fn gradle_format_check(&self, input: NoInput) -> Result<ToolCallResult, ToolCallError> {
        let root = resolve_cwd(&self.project_root, input.path.as_deref())?;
        self.run_gradle_task("spotlessCheck", "format check", &root)
            .await
    }

    async fn gradle_format(&self, input: NoInput) -> Result<ToolCallResult, ToolCallError> {
        let root = resolve_cwd(&self.project_root, input.path.as_deref())?;
        self.run_gradle_task("spotlessApply", "format", &root).await
    }
}

const GRADLE_TOOLS: &[&str] = &[
    "gradle_check",
    "gradle_test",
    "gradle_build",
    "gradle_format_check",
    "gradle_format",
];

#[async_trait::async_trait]
impl super::Toolchain for GradleTools {
    fn tool_names(&self) -> &'static [&'static str] {
        GRADLE_TOOLS
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        gradle_definitions()
    }

    fn server_name(&self) -> &'static str {
        "terrarium-gradle-tools"
    }

    fn instructions(&self) -> &'static str {
        super::gradle_instructions()
    }

    async fn call(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<ToolCallResult, ToolCallError> {
        GradleTools::call(self, name, arguments).await
    }
}

pub(crate) fn gradle_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "gradle_check",
            description: "Run `gradle check` and return failed tasks",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "gradle_test",
            description: "Clear the local Gradle cache, then run `gradle cleanTest test --rerun-tasks --no-build-cache` for the full suite, a single test, or a filtered subset and return a compact summary",
            input_schema: schema_for::<TestInput>(),
        },
        ToolDefinition {
            name: "gradle_build",
            description: "Run `gradle build` and return failed tasks",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "gradle_format_check",
            description: "Run `gradle spotlessCheck` and return formatting failures",
            input_schema: schema_for::<NoInput>(),
        },
        ToolDefinition {
            name: "gradle_format",
            description: "Run `gradle spotlessApply` to auto-format all sources in the project",
            input_schema: schema_for::<NoInput>(),
        },
    ]
}

fn detect_java_home() -> Option<String> {
    crate::profile::resolver::detect_java_home()
}

pub(crate) fn gradle_wrapper_is_usable(project_root: &Path) -> bool {
    project_root.join("gradlew").exists()
        && project_root
            .join("gradle")
            .join("wrapper")
            .join("gradle-wrapper.jar")
            .exists()
}

pub(crate) fn parse_gradle_failed_tasks(output: &str) -> Vec<String> {
    let mut tasks: Vec<String> = RE_GRADLE_FAILED
        .captures_iter(output)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .collect();
    tasks.sort();
    tasks.dedup();
    tasks
}

fn parse_gradle_test_summary(output: &str) -> (u32, u32, u32) {
    RE_GRADLE_TEST
        .captures_iter(output)
        .last()
        .map(|caps| {
            let total = caps
                .get(1)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            let failed = caps
                .get(2)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            let skipped = caps
                .get(3)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            (total, failed, skipped)
        })
        .unwrap_or((0, 0, 0))
}

pub(crate) fn summarize_gradle_test_results(
    project_root: &Path,
    output: &str,
) -> (u32, u32, u32, &'static str, usize) {
    if let Some((total, failed, skipped, file_count)) =
        parse_gradle_test_summary_from_xml_files(project_root)
    {
        return (total, failed, skipped, "xml_results", file_count);
    }
    let (total, failed, skipped) = parse_gradle_test_summary(output);
    (total, failed, skipped, "console_output", 0)
}

fn parse_gradle_test_summary_from_xml_files(project_root: &Path) -> Option<(u32, u32, u32, usize)> {
    let files = collect_gradle_test_result_files(project_root);
    if files.is_empty() {
        return None;
    }

    let mut total = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut parsed_files = 0;

    for path in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some((file_total, file_failed, file_skipped)) =
            parse_gradle_test_summary_from_xml(&content)
        else {
            continue;
        };
        total += file_total;
        failed += file_failed;
        skipped += file_skipped;
        parsed_files += 1;
    }

    if parsed_files == 0 {
        None
    } else {
        Some((total, failed, skipped, parsed_files))
    }
}

fn collect_gradle_test_result_files(project_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_gradle_test_result_files_from_dir(project_root, &mut files);
    files.sort();
    files.dedup();
    files
}

fn collect_gradle_test_result_files_from_dir(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            if is_gradle_build_dir_name(name) {
                collect_gradle_test_result_files_in_build_dir(&path, files);
                continue;
            }

            if should_skip_gradle_results_walk_dir(name) {
                continue;
            }

            collect_gradle_test_result_files_from_dir(&path, files);
        }
    }
}

fn collect_gradle_test_result_files_in_build_dir(build_dir: &Path, files: &mut Vec<PathBuf>) {
    collect_xml_files_recursively(&build_dir.join("test-results"), files);
}

fn is_gradle_build_dir_name(name: &str) -> bool {
    matches!(name, "build" | "target")
}

fn collect_xml_files_recursively(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            collect_xml_files_recursively(&path, files);
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("TEST-") && name.ends_with(".xml") {
            files.push(path);
        }
    }
}

fn should_skip_gradle_results_walk_dir(name: &str) -> bool {
    matches!(name, ".git" | ".gradle" | "node_modules" | ".idea" | ".svn")
}

pub(crate) fn parse_gradle_test_summary_from_xml(xml: &str) -> Option<(u32, u32, u32)> {
    let start = xml.find("<testsuite")?;
    let end = xml[start..].find('>')? + start;
    let tag = &xml[start..=end];

    let mut total = None;
    let mut failures = 0;
    let mut errors = 0;
    let mut skipped = 0;

    for caps in RE_XML_ATTR.captures_iter(tag) {
        let name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let value = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let parsed = value.parse::<u32>().ok();
        match name {
            "tests" => total = parsed,
            "failures" => failures = parsed.unwrap_or(0),
            "errors" => errors = parsed.unwrap_or(0),
            "skipped" => skipped = parsed.unwrap_or(0),
            _ => {}
        }
    }

    total.map(|total| (total, failures + errors, skipped))
}

/// `cleanTest` plus cache-defeating flags: Gradle would otherwise report a
/// cached result and never re-run the tests the caller asked for.
fn gradle_test_args(filter: Option<&str>) -> Vec<String> {
    let mut args = vec![
        String::from("cleanTest"),
        String::from("test"),
        String::from("--rerun-tasks"),
        String::from("--no-build-cache"),
    ];
    if let Some(filter) = filter {
        args.push("--tests".to_string());
        args.push(filter.to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(name);
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn parses_gradle_failures() {
        let output = "> Task :compileKotlin FAILED\n> Task :test FAILED\n";
        assert_eq!(
            parse_gradle_failed_tasks(output),
            vec![":compileKotlin".to_string(), ":test".to_string()]
        );
    }

    #[test]
    fn parses_gradle_test_summary_from_xml_tag() {
        let xml = r#"<testsuite name="example" tests="42" skipped="3" failures="2" errors="1">"#;
        assert_eq!(parse_gradle_test_summary_from_xml(xml), Some((42, 3, 3)));
    }

    /// Writes one JUnit XML result file under `<project>/<build_dir>/test-results/test/`.
    fn write_test_results(base: &Path, build_dir: &str, xml: &str) {
        let dir = base
            .join("service-a")
            .join(build_dir)
            .join("test-results")
            .join("test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("TEST-example.xml"), xml).unwrap();
    }

    #[test]
    fn summarize_gradle_test_results_prefers_xml_files() {
        let base = temp_dir("mcp_tools_gradle_xml_summary");
        write_test_results(
            &base,
            "build",
            r#"<testsuite tests="5" failures="1" errors="0" skipped="1"></testsuite>"#,
        );

        assert_eq!(
            summarize_gradle_test_results(&base, "0 tests completed"),
            (5, 1, 1, "xml_results", 1)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn summarize_gradle_test_results_supports_target_build_directory() {
        let base = temp_dir("mcp_tools_gradle_target_summary");
        write_test_results(
            &base,
            "target",
            r#"<testsuite tests="7" failures="2" errors="0" skipped="1"></testsuite>"#,
        );

        assert_eq!(
            summarize_gradle_test_results(&base, "0 tests completed"),
            (7, 2, 1, "xml_results", 1)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn summarize_gradle_test_results_falls_back_to_console_output() {
        let base = temp_dir("mcp_tools_gradle_console_summary");
        std::fs::create_dir_all(&base).unwrap();

        assert_eq!(
            summarize_gradle_test_results(&base, "42 tests completed, 2 failed, 3 skipped"),
            (42, 2, 3, "console_output", 0)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gradle_test_args_force_fresh_execution() {
        assert_eq!(
            gradle_test_args(Some("example.Test")),
            vec![
                "cleanTest",
                "test",
                "--rerun-tasks",
                "--no-build-cache",
                "--tests",
                "example.Test",
            ]
        );
    }

    #[test]
    fn gradle_wrapper_is_usable_requires_script_and_jar() {
        let base = temp_dir("gradle_wrapper_is_usable");
        std::fs::create_dir_all(base.join("gradle").join("wrapper")).unwrap();
        std::fs::write(base.join("gradlew"), "#!/bin/sh\n").unwrap();

        assert!(!gradle_wrapper_is_usable(&base));

        std::fs::write(
            base.join("gradle")
                .join("wrapper")
                .join("gradle-wrapper.jar"),
            "",
        )
        .unwrap();
        assert!(gradle_wrapper_is_usable(&base));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Gradle runs with an explicit `JAVA_HOME`, so the resolver's env lookup is
    /// what decides which JDK a build uses.
    #[test]
    fn detect_java_home_prefers_env_var() {
        let original_java_home = std::env::var("JAVA_HOME").ok();
        unsafe { std::env::set_var("JAVA_HOME", "/custom/java/home") };
        assert_eq!(detect_java_home(), Some("/custom/java/home".to_string()));
        match original_java_home {
            Some(value) => unsafe { std::env::set_var("JAVA_HOME", value) },
            None => unsafe { std::env::remove_var("JAVA_HOME") },
        }
    }
}
