use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use sysinfo::System;

use crate::config::global::global_terrarium_dir;
use crate::error::{Result, TerrariumError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub id: String,
    pub pid: u32,
    pub project_root: String,
    pub profile_name: String,
    pub started_at: DateTime<Utc>,
    pub sb_profile_path: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct InstancesFile {
    #[serde(default)]
    instances: Vec<InstanceRecord>,
}

fn instances_path() -> PathBuf {
    global_terrarium_dir().join("instances.toml")
}

fn lock_path() -> PathBuf {
    global_terrarium_dir().join("instances.lock")
}

/// Acquires an exclusive flock on the lock file, returns the locked handle.
/// The lock is released when the handle is dropped.
fn acquire_lock() -> Result<nix::fcntl::Flock<File>> {
    let dir = global_terrarium_dir();
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
        .open(lock_path())?;
    nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, _)| TerrariumError::RegistryLockFailed)
}

fn load_raw() -> Result<InstancesFile> {
    let path = instances_path();
    if !path.exists() {
        return Ok(InstancesFile::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let file = toml::from_str(&content).map_err(TerrariumError::Toml)?;
    Ok(file)
}

fn load_raw_for_read() -> Result<InstancesFile> {
    let path = instances_path();
    if !path.exists() {
        return Ok(InstancesFile::default());
    }

    match acquire_lock() {
        Ok(_lock) => load_raw(),
        Err(TerrariumError::RegistryLockFailed) => load_raw(),
        Err(TerrariumError::Io(err)) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            load_raw()
        }
        Err(err) => Err(err),
    }
}

fn save_raw(file: &InstancesFile) -> Result<()> {
    let dir = global_terrarium_dir();
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
    }
    let content = toml::to_string_pretty(file).map_err(TerrariumError::TomlSer)?;
    std::fs::write(instances_path(), content)?;
    Ok(())
}

fn is_pid_alive(sys: &System, pid: u32) -> bool {
    sys.process(sysinfo::Pid::from_u32(pid)).is_some()
}

fn prune_dead(instances: Vec<InstanceRecord>) -> Vec<InstanceRecord> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    instances
        .into_iter()
        .filter(|i| is_pid_alive(&sys, i.pid))
        .collect()
}

/// Registers a new instance, pruning dead ones first.
pub fn register(record: InstanceRecord) -> Result<()> {
    let _lock = acquire_lock()?;
    let mut data = load_raw()?;
    data.instances = prune_dead(data.instances);
    data.instances.push(record);
    save_raw(&data)
}

/// Removes an instance by ID.
pub fn unregister(id: &str) -> Result<()> {
    let _lock = acquire_lock()?;
    let mut data = load_raw()?;
    data.instances = prune_dead(data.instances);
    data.instances.retain(|i| i.id != id);
    save_raw(&data)
}

/// Returns all live instances (dead PIDs are pruned in-memory only).
pub fn list_live() -> Result<Vec<InstanceRecord>> {
    let data = load_raw_for_read()?;
    Ok(prune_dead(data.instances))
}

/// Finds a live instance by ID.
pub fn find(id: &str) -> Result<Option<InstanceRecord>> {
    let data = load_raw_for_read()?;
    Ok(prune_dead(data.instances).into_iter().find(|i| i.id == id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_dead_removes_nonexistent_pids() {
        let record = InstanceRecord {
            id: "test-id".to_string(),
            pid: 999_999_999,
            project_root: "/tmp/test".to_string(),
            profile_name: "none".to_string(),
            started_at: Utc::now(),
            sb_profile_path: "/tmp/test/.terrarium/active.sb".to_string(),
        };
        let pruned = prune_dead(vec![record]);
        assert!(pruned.is_empty());
    }

    #[test]
    fn prune_dead_keeps_live_pids() {
        let my_pid = std::process::id();
        let record = InstanceRecord {
            id: "live-id".to_string(),
            pid: my_pid,
            project_root: "/tmp/test".to_string(),
            profile_name: "none".to_string(),
            started_at: Utc::now(),
            sb_profile_path: "/tmp/test/.terrarium/active.sb".to_string(),
        };
        let pruned = prune_dead(vec![record]);
        assert_eq!(pruned.len(), 1);
    }

    use crate::config::global::override_home_for_tests;
    use crate::test_support;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("temp")
            .join(format!("{}_{}", label, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn live_record(id: &str) -> InstanceRecord {
        InstanceRecord {
            id: id.to_string(),
            pid: std::process::id(), // current process is alive
            project_root: "/tmp/test".to_string(),
            profile_name: "none".to_string(),
            started_at: Utc::now(),
            sb_profile_path: "/tmp/test/.terrarium/active.sb".to_string(),
        }
    }

    #[test]
    fn register_and_find_instance() {
        let _lock = test_support::lock();
        let home = temp_dir("reg_find");
        let _home = override_home_for_tests(home.clone());
        register(live_record("test-abc")).unwrap();
        let found = find("test-abc").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "test-abc");
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn unregister_removes_instance() {
        let _lock = test_support::lock();
        let home = temp_dir("reg_unreg");
        let _home = override_home_for_tests(home.clone());
        register(live_record("remove-me")).unwrap();
        unregister("remove-me").unwrap();
        let found = find("remove-me").unwrap();
        assert!(found.is_none());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn find_nonexistent_returns_none() {
        let _lock = test_support::lock();
        let home = temp_dir("reg_missing");
        let _home = override_home_for_tests(home.clone());
        let found = find("does-not-exist").unwrap();
        assert!(found.is_none());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn list_live_returns_registered_instances() {
        let _lock = test_support::lock();
        let home = temp_dir("reg_list");
        let _home = override_home_for_tests(home.clone());
        register(live_record("list-a")).unwrap();
        register(live_record("list-b")).unwrap();
        let live = list_live().unwrap();
        assert!(live.iter().any(|i| i.id == "list-a"));
        assert!(live.iter().any(|i| i.id == "list-b"));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
