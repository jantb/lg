use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::config::project::global_profile_dir;
use crate::error::{Result, TerrariumError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedEntry {
    pub domain: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub count: u64,
    #[serde(default)]
    pub last_method: String,
    #[serde(default)]
    pub last_url: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct BlockedFile {
    #[serde(default)]
    entries: Vec<BlockedEntry>,
}

pub fn log_path(project_root: &Path) -> PathBuf {
    global_profile_dir(project_root).join("blocked_domains.toml")
}

fn lock_path(project_root: &Path) -> PathBuf {
    global_profile_dir(project_root).join("blocked_domains.lock")
}

fn acquire_lock(project_root: &Path) -> Result<nix::fcntl::Flock<File>> {
    let dir = global_profile_dir(project_root);
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path(project_root))?;
    nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, _)| TerrariumError::RegistryLockFailed)
}

fn load_raw(project_root: &Path) -> Result<BlockedFile> {
    let path = log_path(project_root);
    if !path.exists() {
        return Ok(BlockedFile::default());
    }
    let content = std::fs::read_to_string(&path)?;
    toml::from_str(&content).map_err(TerrariumError::Toml)
}

fn save_raw(project_root: &Path, file: &BlockedFile) -> Result<()> {
    let content = toml::to_string_pretty(file).map_err(TerrariumError::TomlSer)?;
    std::fs::write(log_path(project_root), content)?;
    Ok(())
}

pub fn load(project_root: &Path) -> Result<Vec<BlockedEntry>> {
    Ok(load_raw(project_root)?.entries)
}

pub fn append(project_root: &Path, domain: &str, method: &str, url: &str) -> Result<()> {
    let _lock = acquire_lock(project_root)?;
    let mut data = load_raw(project_root)?;
    let now = Utc::now();
    if let Some(entry) = data.entries.iter_mut().find(|e| e.domain == domain) {
        entry.count += 1;
        entry.last_seen = now;
        entry.last_method = method.to_string();
        entry.last_url = url.to_string();
    } else {
        data.entries.push(BlockedEntry {
            domain: domain.to_string(),
            first_seen: now,
            last_seen: now,
            count: 1,
            last_method: method.to_string(),
            last_url: url.to_string(),
        });
    }
    save_raw(project_root, &data)
}

pub fn remove(project_root: &Path, domain: &str) -> Result<()> {
    let _lock = acquire_lock(project_root)?;
    let mut data = load_raw(project_root)?;
    data.entries.retain(|e| e.domain != domain);
    save_raw(project_root, &data)
}
