//! Turning raw command output and compiler diagnostics into the compact
//! Markdown a tool answers with.
//!
//! Output longer than [`MAX_OUTPUT_CHARS`] is spilled to a log file under
//! `~/.terrarium/projects/<project>/output/` and only its tail is inlined, so a
//! large build log never floods a tool result.

use std::path::{Path, PathBuf};

use crate::mcp::output;

use super::process::CommandOutput;

pub(crate) const MAX_OUTPUT_CHARS: usize = 4_000;

pub(crate) fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut truncated: String = text.chars().take(max_chars).collect();
    if truncated.len() < text.len() {
        truncated.push_str("\n...[truncated]");
    }
    truncated
}

fn write_output_file(project_root: &Path, label: &str, text: &str) -> Option<PathBuf> {
    let dir = crate::config::project::global_profile_dir(project_root).join("output");
    crate::config::project::create_private_dir(&dir).ok()?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{label}-{ts}.log"));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

pub(crate) fn format_output_block(project_root: &Path, label: &str, text: &str) -> String {
    if text.len() <= MAX_OUTPUT_CHARS {
        return format!("\n```\n{text}\n```");
    }
    let skip = text.chars().count().saturating_sub(MAX_OUTPUT_CHARS);
    let tail: String = text.chars().skip(skip).collect();
    if let Some(path) = write_output_file(project_root, label, text) {
        format!("\nFull output: `{}`\n```\n{tail}\n```", path.display())
    } else {
        format!("\n```\n{}\n```", truncate_text(text, MAX_OUTPUT_CHARS))
    }
}

/// Renders the result of a diagnostic-producing build command.
///
/// Consults `output.success`, not just the diagnostic list: cargo can fail
/// without emitting a single JSON diagnostic — an unparseable `Cargo.toml`, an
/// unresolvable dependency version, a missing toolchain — and reporting those as
/// a pass tells the caller the project builds when it cannot. The raw output is
/// attached in that case, since it holds the only explanation of the failure.
pub(crate) fn format_build_result(
    command: &str,
    label: &str,
    cwd: &Path,
    output: &CommandOutput,
    errors: &[&output::Diagnostic],
    warnings: &[&output::Diagnostic],
) -> String {
    if output.success && warnings.is_empty() {
        return format!("{command} passed");
    }
    if !output.success && errors.is_empty() {
        return format!(
            "{command} **failed** — cargo reported no diagnostics, so this is a manifest, dependency, or toolchain error rather than a code error{}",
            format_output_block(cwd, label, &output.combined)
        );
    }
    format_diagnostics_md(command, errors, warnings)
}

fn format_diagnostics_md(
    command: &str,
    errors: &[&output::Diagnostic],
    warnings: &[&output::Diagnostic],
) -> String {
    let mut lines = Vec::new();
    if errors.is_empty() {
        lines.push(format!("{command} passed — {} warnings", warnings.len()));
    } else {
        lines.push(format!(
            "{command} **failed** — {} errors, {} warnings",
            errors.len(),
            warnings.len()
        ));
    }
    for diag in errors {
        format_diag_line(&mut lines, diag);
    }
    for diag in warnings {
        format_diag_line(&mut lines, diag);
    }
    lines.join("\n")
}

fn format_diag_line(lines: &mut Vec<String>, diag: &output::Diagnostic) {
    let loc = match (&diag.file, diag.line) {
        (Some(f), Some(l)) => format!("{f}:{l}"),
        (Some(f), None) => f.clone(),
        _ => String::new(),
    };
    let code = diag.code.as_deref().unwrap_or("");
    if loc.is_empty() && code.is_empty() {
        lines.push(format!("- {}: {}", diag.level, diag.message));
    } else if code.is_empty() {
        lines.push(format!("- `{loc}` {}: {}", diag.level, diag.message));
    } else {
        lines.push(format!(
            "- `{loc}` {}: {} [{}]",
            diag.level, diag.message, code
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global::override_home_for_tests;
    use crate::mcp::tools::process::command_output;

    /// A cargo failure that produces no JSON diagnostics — unparseable
    /// `Cargo.toml`, unresolvable dependency, missing toolchain — used to be
    /// reported as "passed — 0 warnings", because only the diagnostic list was
    /// consulted. It must report a failure and surface cargo's own output.
    #[test]
    fn build_failure_without_diagnostics_is_not_reported_as_passing() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let failed = command_output(
            Vec::new(),
            b"error: failed to parse manifest at `Cargo.toml`".to_vec(),
            false,
        );

        let md = format_build_result("cargo check", "cargo-check", &cwd, &failed, &[], &[]);

        assert!(
            md.contains("**failed**"),
            "a failed cargo run must report failure, got: {md}"
        );
        assert!(!md.contains("passed"), "must not claim success, got: {md}");
        assert!(
            md.contains("failed to parse manifest"),
            "cargo's own error is the only explanation available, got: {md}"
        );
    }

    /// The ordinary success path must stay quiet.
    #[test]
    fn build_success_without_warnings_reports_passing() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ok = command_output(Vec::new(), Vec::new(), true);
        assert_eq!(
            format_build_result("cargo check", "cargo-check", &cwd, &ok, &[], &[]),
            "cargo check passed"
        );
    }

    #[test]
    fn long_output_is_written_to_global_terrarium_state() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("global_output_state_{}", uuid::Uuid::new_v4()));
        let home = base.join("home");
        let project = base.join("project");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&project).unwrap();
        let _guard = override_home_for_tests(home.clone());
        let text = "x".repeat(MAX_OUTPUT_CHARS + 1);

        let block = format_output_block(&project, "cargo-test", &text);

        assert!(block.contains(&home.join(".terrarium").display().to_string()));
        assert!(!project.join(".terrarium").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
