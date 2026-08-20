//! Per-checkout settings stored under the user's home directory.
//!
//! Settings live outside the worktree so a checkout never gains untracked files,
//! and each checkout gets its own directory keyed by its repository root path.
//! That keeps the commit prompt and PR language tuned per project instead of
//! globally, which is what a monorepo and a side project need at the same time.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::COMMIT_PROMPT_PREFIX;

const SETTINGS_DIR_ENV: &str = "LG_SETTINGS_DIR";
const SETTINGS_FILE: &str = "settings";
const COMMIT_PROMPT_FILE: &str = "commit-prompt.txt";
const KEY_PR_LANGUAGE: &str = "pr_language";
const KEY_COMMIT_SUBJECT_MAX_CHARS: &str = "commit_subject_max_chars";
const KEY_COMMIT_BODY_MAX_LINES: &str = "commit_body_max_lines";
const KEY_COMMENT_STYLE: &str = "comment_style";

pub const DEFAULT_PR_LANGUAGE: &str = "English";
/// A generous subject cap: long enough not to mangle a specific summary, short
/// enough that `git log --oneline` stays readable. Zero means unlimited.
pub const DEFAULT_COMMIT_SUBJECT_MAX_CHARS: usize = 72;
/// Body lines after the blank line. Zero means unlimited.
pub const DEFAULT_COMMIT_BODY_MAX_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSettings {
    pub pr_language: String,
    /// House style for prose the model writes — commit bodies, PR text, code
    /// comments. Empty means "no house style stated"; it is derived from the
    /// checkout on first use rather than guessed.
    pub comment_style: String,
    pub commit_subject_max_chars: usize,
    pub commit_body_max_lines: usize,
    /// Prompt prefix used to generate commit messages. Defaults to the built-in
    /// prefix; an edited `commit-prompt.txt` replaces it verbatim.
    pub commit_prompt: String,
}

impl Default for RepoSettings {
    fn default() -> Self {
        Self {
            pr_language: DEFAULT_PR_LANGUAGE.to_string(),
            comment_style: String::new(),
            commit_subject_max_chars: DEFAULT_COMMIT_SUBJECT_MAX_CHARS,
            commit_body_max_lines: DEFAULT_COMMIT_BODY_MAX_LINES,
            commit_prompt: COMMIT_PROMPT_PREFIX.to_string(),
        }
    }
}

impl RepoSettings {
    pub fn commit_prompt_is_custom(&self) -> bool {
        self.commit_prompt != COMMIT_PROMPT_PREFIX
    }
}

/// Loads the settings for the current checkout, falling back to defaults for
/// anything missing or unparsable. Settings are advisory, so a broken file must
/// never block committing.
pub fn load() -> RepoSettings {
    match repo_settings_dir() {
        Ok(dir) => load_from_dir(&dir),
        Err(_) => RepoSettings::default(),
    }
}

/// Whether this checkout has settings of its own yet. A fresh checkout gets
/// values derived from its own history instead of the bare defaults.
pub fn is_configured() -> bool {
    repo_settings_dir()
        .map(|dir| dir.join(SETTINGS_FILE).exists())
        .unwrap_or(false)
}

pub fn save(settings: &RepoSettings) -> Result<()> {
    let dir = repo_settings_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(SETTINGS_FILE);
    fs::write(&path, render_settings(settings))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Removes this checkout's saved settings, returning it to the defaults.
pub fn clear() -> Result<()> {
    let dir = repo_settings_dir()?;
    for file in [SETTINGS_FILE, COMMIT_PROMPT_FILE] {
        let path = dir.join(file);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn settings_dir_display() -> String {
    repo_settings_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|_| "$HOME/.config/lg/repos/<checkout>".to_string())
}

/// Writes the current prompt to `commit-prompt.txt` when it does not exist yet
/// and returns the path, so an editor always opens a file with real content to
/// edit rather than an empty buffer.
pub fn ensure_commit_prompt_file() -> Result<PathBuf> {
    let dir = repo_settings_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(COMMIT_PROMPT_FILE);
    if !path.exists() {
        fs::write(&path, COMMIT_PROMPT_PREFIX)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(path)
}

/// Appends the configured shape, language, and limits to a commit prompt prefix
/// so the model is told the same bounds that [`enforce_commit_limits`] enforces
/// afterwards, in the language this checkout's own history is written in.
pub fn commit_prompt_prefix(settings: &RepoSettings) -> String {
    let mut prompt = settings.commit_prompt.trim_end().to_string();
    let style = one_line(&settings.comment_style);
    if !style.is_empty() {
        prompt.push_str("\n\nMessage shape (follow the format this project already uses):\n- ");
        prompt.push_str(&style);
    }
    prompt.push_str("\n\nLanguage:\n");
    prompt.push_str(&language_instruction(settings));
    prompt = prompt.trim_end().to_string();
    let mut limits = Vec::new();
    if settings.commit_subject_max_chars > 0 {
        limits.push(format!(
            "- Keep the first line at or under {} characters.",
            settings.commit_subject_max_chars
        ));
    }
    if settings.commit_body_max_lines > 0 {
        limits.push(format!(
            "- Use at most {} body lines after the blank line.",
            settings.commit_body_max_lines
        ));
    }
    if !limits.is_empty() {
        prompt.push_str("\n\nLength limits:\n");
        prompt.push_str(&limits.join("\n"));
    }
    prompt.push_str("\n\nStaged changes:\n\n");
    prompt
}

/// Trims a generated commit message to the configured limits. Models overshoot
/// a stated cap often enough that the prompt alone is not a limit.
pub fn enforce_commit_limits(message: &str, settings: &RepoSettings) -> String {
    let mut lines = message.lines();
    let Some(subject) = lines.next() else {
        return String::new();
    };
    let mut out = truncate_subject(subject, settings.commit_subject_max_chars);

    let body: Vec<&str> = lines.collect();
    let body = limit_body_lines(trim_blank_edges(&body), settings.commit_body_max_lines);
    if body.is_empty() {
        return out;
    }
    out.push_str("\n\n");
    out.push_str(&body.join("\n"));
    out
}

/// Keeps at most `max_lines` non-blank body lines, preserving the blank lines
/// that separate the paragraphs that survive. Zero means unlimited.
fn limit_body_lines<'a>(body: &[&'a str], max_lines: usize) -> Vec<&'a str> {
    if max_lines == 0 {
        return trim_blank_edges(body).to_vec();
    }
    let mut kept = Vec::new();
    let mut taken = 0usize;
    for line in body {
        if line.trim().is_empty() {
            kept.push(*line);
            continue;
        }
        if taken == max_lines {
            break;
        }
        taken += 1;
        kept.push(*line);
    }
    trim_blank_edges(&kept).to_vec()
}

/// Instruction appended to prompts whose output the user reads, so generated PR
/// text comes back in the language configured for this checkout.
pub fn language_instruction(settings: &RepoSettings) -> String {
    let language = normalize_language(&settings.pr_language);
    format!(
        "Write all prose in {language}. Keep code identifiers, file paths, branch names,\n\
         and Markdown headings exactly as they appear in the context.\n"
    )
}

fn normalize_language(language: &str) -> String {
    let language = language.trim();
    if language.is_empty() {
        DEFAULT_PR_LANGUAGE.to_string()
    } else {
        language.to_string()
    }
}

fn truncate_subject(subject: &str, max_chars: usize) -> String {
    let subject = subject.trim();
    if max_chars == 0 || subject.chars().count() <= max_chars {
        return subject.to_string();
    }
    let clipped: String = subject.chars().take(max_chars).collect();
    if subject
        .chars()
        .nth(max_chars)
        .is_none_or(char::is_whitespace)
    {
        // The cut already landed on a word boundary; nothing is mid-token.
        return clipped.trim_end().to_string();
    }
    // Otherwise back up to the last space, but only when enough of the summary
    // survives to stay meaningful.
    match clipped.rfind(' ') {
        Some(idx) if idx * 2 >= clipped.len() => clipped[..idx].trim_end().to_string(),
        _ => clipped.trim_end().to_string(),
    }
}

fn trim_blank_edges<'a, 'b>(lines: &'b [&'a str]) -> &'b [&'a str] {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|idx| idx + 1)
        .unwrap_or(start);
    &lines[start..end]
}

fn load_from_dir(dir: &Path) -> RepoSettings {
    let text = fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap_or_default();
    let prompt = fs::read_to_string(dir.join(COMMIT_PROMPT_FILE))
        .ok()
        .map(|prompt| prompt.trim_end().to_string())
        .filter(|prompt| !prompt.trim().is_empty());
    parse_settings(&text, prompt)
}

fn parse_settings(text: &str, commit_prompt: Option<String>) -> RepoSettings {
    let mut settings = RepoSettings::default();
    if let Some(prompt) = commit_prompt {
        settings.commit_prompt = prompt;
    }
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            KEY_PR_LANGUAGE if !value.is_empty() => {
                settings.pr_language = normalize_language(value);
            }
            KEY_COMMIT_SUBJECT_MAX_CHARS => {
                if let Ok(parsed) = value.parse::<usize>() {
                    settings.commit_subject_max_chars = parsed;
                }
            }
            KEY_COMMIT_BODY_MAX_LINES => {
                if let Ok(parsed) = value.parse::<usize>() {
                    settings.commit_body_max_lines = parsed;
                }
            }
            KEY_COMMENT_STYLE => {
                settings.comment_style = value.to_string();
            }
            _ => {}
        }
    }
    settings
}

fn render_settings(settings: &RepoSettings) -> String {
    format!(
        "# lg settings for this checkout\n\
         # commit-prompt.txt in this directory overrides the built-in commit prompt.\n\
         {KEY_PR_LANGUAGE}={}\n\
         {KEY_COMMIT_SUBJECT_MAX_CHARS}={}\n\
         {KEY_COMMIT_BODY_MAX_LINES}={}\n\
         {KEY_COMMENT_STYLE}={}\n",
        normalize_language(&settings.pr_language),
        settings.commit_subject_max_chars,
        settings.commit_body_max_lines,
        one_line(&settings.comment_style),
    )
}

/// The settings file is one key per line, so a value may not carry newlines.
fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn repo_settings_dir() -> Result<PathBuf> {
    Ok(settings_base_dir()?.join(checkout_slug(&checkout_root()?)))
}

fn settings_base_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(SETTINGS_DIR_ENV)
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
        anyhow::bail!("HOME is not set");
    };
    Ok(PathBuf::from(home).join(".config/lg/repos"))
}

fn checkout_root() -> Result<String> {
    let root = crate::git::repo_root()?;
    if root.trim().is_empty() {
        anyhow::bail!("not inside a git checkout");
    }
    Ok(root)
}

/// Directory name for a checkout: a readable slug plus a hash of the full path,
/// so two checkouts of the same project never share settings.
fn checkout_slug(root: &str) -> String {
    let name: String = Path::new(root)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-').to_string();
    let name = if name.is_empty() {
        "checkout".to_string()
    } else {
        name
    };
    format!("{name}-{:016x}", path_hash(root))
}

fn path_hash(value: &str) -> u64 {
    // FNV-1a: stable across runs and platforms, unlike DefaultHasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_settings_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load_from_dir(dir.path());
        assert_eq!(settings, RepoSettings::default());
        assert!(!settings.commit_prompt_is_custom());
    }

    #[test]
    fn saved_values_round_trip_through_the_settings_file() {
        let settings = RepoSettings {
            pr_language: "Norwegian".to_string(),
            comment_style: String::new(),
            commit_subject_max_chars: 50,
            commit_body_max_lines: 3,
            commit_prompt: COMMIT_PROMPT_PREFIX.to_string(),
        };
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(SETTINGS_FILE), render_settings(&settings)).unwrap();
        assert_eq!(load_from_dir(dir.path()), settings);
    }

    #[test]
    fn commit_prompt_file_overrides_the_built_in_prompt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(COMMIT_PROMPT_FILE), "Custom prompt.\n").unwrap();
        let settings = load_from_dir(dir.path());
        assert_eq!(settings.commit_prompt, "Custom prompt.");
        assert!(settings.commit_prompt_is_custom());
    }

    #[test]
    fn blank_commit_prompt_file_falls_back_to_the_built_in_prompt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(COMMIT_PROMPT_FILE), "   \n\n").unwrap();
        assert_eq!(
            load_from_dir(dir.path()).commit_prompt,
            COMMIT_PROMPT_PREFIX
        );
    }

    #[test]
    fn unparsable_entries_keep_the_defaults() {
        let settings = parse_settings(
            "commit_subject_max_chars=abc\nnonsense\npr_language=\n",
            None,
        );
        assert_eq!(settings, RepoSettings::default());
    }

    #[test]
    fn zero_limits_are_honored_as_unlimited() {
        let settings = parse_settings(
            "commit_subject_max_chars=0\ncommit_body_max_lines=0\n",
            None,
        );
        assert_eq!(settings.commit_subject_max_chars, 0);
        assert_eq!(settings.commit_body_max_lines, 0);
        let long = format!("feat: {}", "x".repeat(200));
        assert_eq!(enforce_commit_limits(&long, &settings), long);
    }

    #[test]
    fn commit_prompt_prefix_states_the_configured_limits() {
        let settings = RepoSettings {
            commit_subject_max_chars: 50,
            commit_body_max_lines: 2,
            ..RepoSettings::default()
        };
        let prompt = commit_prompt_prefix(&settings);
        assert!(prompt.contains("at or under 50 characters"));
        assert!(prompt.contains("at most 2 body lines"));
        assert!(prompt.ends_with("Staged changes:\n\n"));
    }

    #[test]
    fn commit_prompt_prefix_omits_limits_when_unlimited() {
        let settings = RepoSettings {
            commit_subject_max_chars: 0,
            commit_body_max_lines: 0,
            ..RepoSettings::default()
        };
        let prompt = commit_prompt_prefix(&settings);
        assert!(!prompt.contains("Length limits"));
    }

    #[test]
    fn subject_is_truncated_on_a_word_boundary() {
        let settings = RepoSettings {
            commit_subject_max_chars: 24,
            ..RepoSettings::default()
        };
        let out = enforce_commit_limits("feat(llm): stream commit tokens to the panel", &settings);
        assert_eq!(out, "feat(llm): stream commit");
    }

    #[test]
    fn subject_without_a_late_space_is_hard_truncated() {
        let settings = RepoSettings {
            commit_subject_max_chars: 10,
            ..RepoSettings::default()
        };
        assert_eq!(
            enforce_commit_limits("fix aaaaaaaaaaaaaaaaaa", &settings),
            "fix aaaaaa"
        );
    }

    #[test]
    fn body_lines_beyond_the_limit_are_dropped() {
        let settings = RepoSettings {
            commit_body_max_lines: 2,
            ..RepoSettings::default()
        };
        let out = enforce_commit_limits("fix: thing\n\nfirst\nsecond\nthird\n", &settings);
        assert_eq!(out, "fix: thing\n\nfirst\nsecond");
    }

    #[test]
    fn subject_only_message_stays_single_line() {
        let out = enforce_commit_limits("fix: thing\n\n\n", &RepoSettings::default());
        assert_eq!(out, "fix: thing");
    }

    #[test]
    fn language_instruction_uses_the_configured_language() {
        let settings = RepoSettings {
            pr_language: "Norwegian".to_string(),
            comment_style: String::new(),
            ..RepoSettings::default()
        };
        assert!(language_instruction(&settings).starts_with("Write all prose in Norwegian."));
    }

    #[test]
    fn blank_language_falls_back_to_the_default() {
        let settings = RepoSettings {
            pr_language: "  ".to_string(),
            comment_style: String::new(),
            ..RepoSettings::default()
        };
        assert!(language_instruction(&settings).contains(DEFAULT_PR_LANGUAGE));
    }

    #[test]
    fn checkout_slug_is_stable_and_unique_per_path() {
        let a = checkout_slug("/Users/dev/priv/lg");
        assert_eq!(a, checkout_slug("/Users/dev/priv/lg"));
        assert!(a.starts_with("lg-"));
        assert_ne!(a, checkout_slug("/Users/dev/work/lg"));
    }

    #[test]
    fn the_message_shape_is_stated_in_the_commit_prompt() {
        let settings = RepoSettings {
            comment_style: "Conventional Commits subject, imperative mood, bullet body".to_string(),
            ..RepoSettings::default()
        };
        let prompt = commit_prompt_prefix(&settings);
        assert!(prompt.contains(
            "Message shape (follow the format this project already uses):\n- \
             Conventional Commits subject, imperative mood, bullet body"
        ));
        assert!(!commit_prompt_prefix(&RepoSettings::default()).contains("Message shape"));
    }

    #[test]
    fn the_commit_prompt_states_the_configured_language() {
        let settings = RepoSettings {
            pr_language: "Norwegian".to_string(),
            ..RepoSettings::default()
        };
        assert!(commit_prompt_prefix(&settings).contains("Write all prose in Norwegian"));
    }
}
