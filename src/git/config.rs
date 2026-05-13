use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{file_diff, repo_root, run};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorConfig {
    pub name: Option<String>,
    pub email: Option<String>,
    pub local_name: Option<String>,
    pub local_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeOpenCommand {
    pub program: String,
    pub args: Vec<String>,
    pub line: usize,
}

pub fn author_config() -> Result<AuthorConfig> {
    Ok(AuthorConfig {
        name: config_value(&["config", "--get", "user.name"])?,
        email: config_value(&["config", "--get", "user.email"])?,
        local_name: config_value(&["config", "--local", "--get", "user.name"])?,
        local_email: config_value(&["config", "--local", "--get", "user.email"])?,
    })
}

pub fn add_to_gitignore(path: &str, is_dir: bool) -> Result<String> {
    let root = repo_root()?;
    let entry = gitignore_entry(path, is_dir);
    let ignore_path = Path::new(&root).join(".gitignore");
    let existing = fs::read_to_string(&ignore_path).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == entry) {
        return Ok(format!("{entry} already ignored"));
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ignore_path)
        .with_context(|| format!("failed to open {}", ignore_path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "{entry}")?;
    Ok(format!("ignored {entry}"))
}

fn gitignore_entry(path: &str, is_dir: bool) -> String {
    let mut entry = path
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    if is_dir && !entry.ends_with('/') {
        entry.push('/');
    }
    entry
}

pub fn ide_open_command(path: &str) -> Result<IdeOpenCommand> {
    let root = repo_root()?;
    let line = first_changed_line(path).unwrap_or(1);
    build_ide_open_command(&root, path, line)
        .ok_or_else(|| anyhow::anyhow!("no JetBrains IDE mapping for {path}"))
}

pub fn open_file_in_ide(path: &str) -> Result<String> {
    let command = ide_open_command(path)?;
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to launch {}", command.program))?;
    Ok(format!(
        "opened {path} in {} at line {}",
        command.program, command.line
    ))
}

pub fn project_open_command() -> Result<IdeOpenCommand> {
    let root = repo_root()?;
    Ok(build_project_open_command(&root))
}

pub fn open_project_in_ide() -> Result<String> {
    let command = project_open_command()?;
    open_project_command(command)
}

pub fn open_project_path_in_ide(path: &Path) -> Result<String> {
    let command = build_project_open_command(&path.to_string_lossy());
    open_project_command(command)
}

fn open_project_command(command: IdeOpenCommand) -> Result<String> {
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to launch {}", command.program))?;
    Ok(format!("opened project in {}", command.program))
}

fn build_project_open_command(root: &str) -> IdeOpenCommand {
    let program = ide_program_for_project(Path::new(root));
    IdeOpenCommand {
        program: program.to_string(),
        args: vec![root.to_string()],
        line: 1,
    }
}

pub(super) fn build_ide_open_command(
    root: &str,
    path: &str,
    line: usize,
) -> Option<IdeOpenCommand> {
    let program = ide_program_for_path(path)?;
    let file = {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            Path::new(root).join(path)
        }
    };
    let line = line.max(1);
    Some(IdeOpenCommand {
        program: program.to_string(),
        args: vec![
            root.to_string(),
            "--line".to_string(),
            line.to_string(),
            file.to_string_lossy().into_owned(),
        ],
        line,
    })
}

fn ide_program_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("kt" | "kts" | "java" | "md") => Some("idea"),
        Some("rs") => Some("rustrover"),
        _ => None,
    }
}

fn ide_program_for_project(root: &Path) -> &'static str {
    if root.join("Cargo.toml").exists() || project_contains_extension(root, "rs") {
        return "rustrover";
    }
    "idea"
}

fn project_contains_extension(root: &Path, extension: &str) -> bool {
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|name| name.to_str());
            if matches!(file_name, Some(".git" | "target" | "build" | ".gradle")) {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                return true;
            }
        }
    }
    false
}

fn first_changed_line(path: &str) -> Option<usize> {
    let diff = file_diff(path).ok()?;
    diff.lines().find_map(diff_hunk_start_line)
}

pub(super) fn diff_hunk_start_line(line: &str) -> Option<usize> {
    if !line.starts_with("@@ ") {
        return None;
    }
    parse_hunk_side(line, '+').or_else(|| parse_hunk_side(line, '-'))
}

fn parse_hunk_side(line: &str, marker: char) -> Option<usize> {
    let start = line.find(marker)? + marker.len_utf8();
    let digits: String = line[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let parsed = digits.parse::<usize>().ok()?;
    (parsed > 0).then_some(parsed)
}

pub fn set_local_author(name: &str, email: &str) -> Result<()> {
    set_optional_local_config("user.name", name)?;
    set_optional_local_config("user.email", email)?;
    Ok(())
}

pub fn set_subtree_author(path: &str, name: &str, email: &str) -> Result<()> {
    let path = normalize_author_path(path)?;
    let config_path = subtree_author_config_path(&path)?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = format!(
        "[user]\n\tname = {}\n\temail = {}\n",
        escape_config_value(name.trim()),
        escape_config_value(email.trim())
    );
    fs::write(&config_path, text)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    let key = subtree_include_key(&path);
    let config_path_str = config_path.to_string_lossy().to_string();
    run(&["config", "--global", &key, &config_path_str]).map(|_| ())
}

pub fn clear_subtree_author(path: &str) -> Result<()> {
    let path = normalize_author_path(path)?;
    let key = subtree_include_key(&path);
    Command::new("git")
        .args(["config", "--global", "--unset-all", &key])
        .output()
        .with_context(|| format!("failed to spawn git config --global --unset-all {key}"))?;
    if let Ok(config_path) = subtree_author_config_path(&path) {
        let _ = fs::remove_file(config_path);
    }
    Ok(())
}

pub fn subtree_author_rule_exists(path: &str) -> bool {
    normalize_author_path(path)
        .map(|path| subtree_include_key(&path))
        .ok()
        .and_then(|key| config_value(&["config", "--global", "--get", &key]).ok())
        .flatten()
        .is_some()
}

pub fn clear_local_author() -> Result<()> {
    unset_optional_local_config("user.name")?;
    unset_optional_local_config("user.email")?;
    Ok(())
}

fn normalize_author_path(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("author folder is empty");
    }
    let expanded = if trimmed == "~" {
        home_dir()?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(std::env::current_dir()?.join(expanded))
    }
}

fn subtree_include_key(path: &Path) -> String {
    let mut path = path.to_string_lossy().trim_end_matches('/').to_string();
    path.push_str("/**");
    format!("includeIf.gitdir:{path}.path")
}

fn subtree_author_config_path(path: &Path) -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config/lg/git-author")
        .join(format!("{}.gitconfig", author_path_slug(path))))
}

fn author_path_slug(path: &Path) -> String {
    let slug: String = path
        .to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-').to_string()
}

fn escape_config_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', " ")
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

fn set_optional_local_config(key: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        unset_optional_local_config(key)
    } else {
        run(&["config", "--local", key, value]).map(|_| ())
    }
}

fn unset_optional_local_config(key: &str) -> Result<()> {
    Command::new("git")
        .args(["config", "--local", "--unset-all", key])
        .output()
        .with_context(|| format!("failed to spawn git config --local --unset-all {key}"))?;
    Ok(())
}

fn config_value(args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {}", args.join(" ")))?;
    if !out.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}
