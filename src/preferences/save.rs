//! Writing preferences back: a category or some fields of one, into the file
//! that decides them, and resetting or importing whole documents.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::Preferences;
use super::cache::{invalidate, load};
use super::layers::{Scope, checkout, folder_path, layers, merge, read_patch, scope_path};

pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("missing configuration directory")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(content)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn save_category(scope: Scope, category: &str, value: serde_json::Value) -> Result<()> {
    save_at(scope_path(scope)?, None, category, value)
}

pub fn save_folder(folder: &Path, category: &str, value: serde_json::Value) -> Result<()> {
    let folder = folder.canonicalize().context("folder must exist")?;
    save_at(folder_path(&folder), Some(&folder), category, value)
}

fn save_at(
    path: PathBuf,
    folder: Option<&Path>,
    category: &str,
    value: serde_json::Value,
) -> Result<()> {
    let mut patch = if path.exists() {
        read_patch(&path)?
    } else {
        serde_json::json!({"version":1})
    };
    patch[category] = value;
    let saved_folder = patch
        .as_object_mut()
        .with_context(|| format!("{}: configuration is not a table", path.display()))?
        .remove("folder");
    let mut effective = serde_json::to_value(load().config)?;
    merge(&mut effective, patch.clone());
    serde_json::from_value::<Preferences>(effective)?.validate()?;
    if let Some(folder) = folder {
        patch["folder"] = folder.display().to_string().into();
    } else if let Some(folder) = saved_folder {
        patch["folder"] = folder;
    }
    if path.exists() {
        std::fs::copy(&path, path.with_extension("toml.bak"))?;
    }
    invalidate();
    atomic_write(&path, toml::to_string_pretty(&patch)?.as_bytes())
}

/// Set some fields of `category` where the change will actually show: in the
/// most specific file that already sets the category, or at `fallback` when
/// that is more specific still. The rest of what that file says is kept.
///
/// For the screens that edit a handful of fields and know nothing of scopes.
/// Saving at a fixed scope from one of those reads back as nothing having
/// happened whenever a more specific file has a say in the same category.
pub fn save_fields(
    fallback: Scope,
    category: &str,
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let path = deciding_file(fallback, category)?;
    let mut value = existing_category(&path, category)?;
    for (key, field) in fields {
        value[key] = field;
    }
    save_at(path, None, category, value)
}

/// Take `keys` of `category` out of the file [`save_fields`] would write them
/// to, so whatever a broader file or the default says shows through again.
pub fn clear_fields(fallback: Scope, category: &str, keys: &[&str]) -> Result<()> {
    let path = deciding_file(fallback, category)?;
    let mut value = existing_category(&path, category)?;
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    if !keys.iter().any(|key| object.contains_key(*key)) {
        return Ok(());
    }
    for key in keys {
        object.remove(*key);
    }
    if object.is_empty() {
        reset_at(path, category)
    } else {
        save_at(path, None, category, value)
    }
}

/// What `path` says about `category`, as a table to add to; empty when it
/// says nothing.
fn existing_category(path: &Path, category: &str) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    Ok(read_patch(path)?
        .get(category)
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({})))
}

/// The file whose say on `category` wins: the most specific one that sets it,
/// unless `fallback` is more specific than that.
fn deciding_file(fallback: Scope, category: &str) -> Result<PathBuf> {
    let fallback_path = scope_path(fallback)?;
    let layers = layers(checkout().ok().as_ref());
    let fallback_rank = layers
        .iter()
        .position(|(_, path)| *path == fallback_path)
        .unwrap_or(0);
    let deciding = layers.into_iter().enumerate().rev().find(|(_, (_, path))| {
        path.exists() && read_patch(path).is_ok_and(|patch| patch.get(category).is_some())
    });
    Ok(match deciding {
        Some((rank, (_, path))) if rank > fallback_rank => path,
        _ => fallback_path,
    })
}

pub fn reset_category(scope: Scope, category: &str) -> Result<()> {
    reset_at(scope_path(scope)?, category)
}

fn reset_at(path: PathBuf, category: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut patch = read_patch(&path)?;
    patch
        .as_object_mut()
        .context("expected table")?
        .remove(category);
    std::fs::copy(&path, path.with_extension("toml.bak"))?;
    invalidate();
    atomic_write(&path, toml::to_string_pretty(&patch)?.as_bytes())
}

pub fn reset_folder(folder: &Path, category: &str) -> Result<()> {
    reset_at(folder_path(&folder.canonicalize()?), category)
}

pub fn import_document(scope: Scope, patch: serde_json::Value) -> Result<()> {
    let path = scope_path(scope)?;
    let mut document = if path.exists() {
        read_patch(&path)?
    } else {
        serde_json::json!({"version":1})
    };
    merge(&mut document, patch);
    let mut effective = serde_json::to_value(load().config)?;
    merge(&mut effective, document.clone());
    serde_json::from_value::<Preferences>(effective)?.validate()?;
    if path.exists() {
        std::fs::copy(&path, path.with_extension("toml.bak"))?;
    }
    atomic_write(&path, toml::to_string_pretty(&document)?.as_bytes())?;
    invalidate();
    Ok(())
}
