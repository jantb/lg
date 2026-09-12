//! Scriptable access to the same configuration used by the Settings screen.
use crate::preferences::{self, Scope};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub fn run(args: &[String]) -> Result<()> {
    let loaded = preferences::load();
    match args.first().map(String::as_str) {
        None | Some("show") => {
            println!("{}", toml::to_string_pretty(&loaded.config)?);
            for (key, source) in loaded.sources {
                eprintln!("{key}: {source}");
            }
            for error in loaded.errors {
                eprintln!("configuration error: {error}");
            }
        }
        Some("appearance") => println!(
            "decorative_animations={}",
            preferences::animations_enabled()
        ),
        Some("export") => {
            let path = args.get(1).context("usage: lg config export FILE")?;
            // Portable project policy excludes machine commands, endpoints,
            // identities and local filesystem grants.
            let value = serde_json::json!({"version":1, "writing":loaded.config.writing, "branches":loaded.config.branches});
            preferences::atomic_write(Path::new(path), toml::to_string_pretty(&value)?.as_bytes())?;
            println!("Exported project writing and branch policy to {path}");
        }
        Some("import") => {
            let path = args.get(1).context(
                "usage: lg config import FILE [--apply] [--scope user|repository|worktree]",
            )?;
            let text = std::fs::read_to_string(path)?;
            let document: toml::Value = toml::from_str(&text)?;
            let patch = serde_json::to_value(document)?;
            let mut candidate = serde_json::to_value(&loaded.config)?;
            preferences::merge(&mut candidate, patch.clone());
            serde_json::from_value::<preferences::Preferences>(candidate)?.validate()?;
            println!("Import into {}:\n{}", scope(args)?.label(), text);
            if !args.iter().any(|a| a == "--apply") {
                println!(
                    "Preview only. Add --apply to accept these settings, including any executable or permission changes."
                );
                return Ok(());
            }
            preferences::import_document(scope(args)?, patch)?;
        }
        Some("set") => {
            let field = args.get(1).context(
                "usage: lg config set CATEGORY.FIELD JSON_VALUE [--scope user|repository|worktree]",
            )?;
            let value: serde_json::Value =
                serde_json::from_str(args.get(2).context("missing JSON value")?)?;
            let (category, field) = field.split_once('.').context("use category.field")?;
            let mut config = serde_json::to_value(&loaded.config)?;
            let pointer = format!("/{category}/{}", field.replace('.', "/"));
            *config.pointer_mut(&pointer).context("unknown setting")? = value;
            if scope(args)? == Scope::Folder {
                let i = args
                    .iter()
                    .position(|a| a == "--folder")
                    .context("folder scope needs --folder PATH")?;
                preferences::save_folder(
                    Path::new(args.get(i + 1).context("missing folder path")?),
                    category,
                    config[category].clone(),
                )?;
            } else {
                preferences::save_category(scope(args)?, category, config[category].clone())?;
            }
            println!("Saved {category}.{field} at {} scope", scope(args)?.label());
        }
        Some("reset") => preferences::reset_category(
            scope(args)?,
            args.get(1).context("usage: lg config reset CATEGORY")?,
        )?,
        _ => bail!(
            "usage: lg config [show|set CATEGORY.FIELD JSON_VALUE|reset CATEGORY|export FILE|import FILE [--apply]] [--scope user|repository|worktree]"
        ),
    }
    Ok(())
}
fn scope(args: &[String]) -> Result<Scope> {
    match args
        .iter()
        .position(|a| a == "--scope")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("repository")
    {
        "folder" => Ok(Scope::Folder),
        "user" => Ok(Scope::User),
        "repository" => Ok(Scope::Repository),
        "worktree" => Ok(Scope::Worktree),
        _ => bail!("scope must be user, repository or worktree"),
    }
}
