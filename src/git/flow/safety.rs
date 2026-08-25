//! The refs lg leaves behind so a flow can be undone.

use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::{head_branch, run};

pub(super) const SAFETY_REF_PREFIX: &str = "lg/backup/";

pub(super) const SAFETY_REF_KEEP: usize = 20;

pub(super) fn create_safety_ref(label: &str) -> Result<String> {
    let branch = head_branch().unwrap_or_else(|_| "detached".to_string());
    let clean_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let clean_branch: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{SAFETY_REF_PREFIX}{clean_label}-{clean_branch}-{ts}");
    run(&["branch", &name, "HEAD"])?;
    prune_safety_refs(SAFETY_REF_KEEP)?;
    Ok(name)
}

pub(super) fn delete_safety_ref(name: &str) -> Result<()> {
    if !name.starts_with(SAFETY_REF_PREFIX) {
        anyhow::bail!("refusing to delete non-safety branch {name}");
    }
    run(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
    Ok(())
}

pub(super) fn delete_latest_safety_ref(label: &str, branch: &str) -> Result<Option<String>> {
    let prefix = safety_ref_name_prefix(label, branch);
    let out = run(&[
        "for-each-ref",
        "--format=%(refname:short)",
        &format!("refs/heads/{SAFETY_REF_PREFIX}"),
    ])?;
    let latest = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(&prefix))
        .filter_map(|name| safety_ref_timestamp(name).map(|ts| (name.to_string(), ts)))
        .max_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(name, _)| name);

    if let Some(name) = latest {
        delete_safety_ref(&name)?;
        Ok(Some(name))
    } else {
        Ok(None)
    }
}

fn safety_ref_name_prefix(label: &str, branch: &str) -> String {
    let clean_label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let clean_branch: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{SAFETY_REF_PREFIX}{clean_label}-{clean_branch}-")
}

pub(super) fn prune_safety_refs(keep: usize) -> Result<usize> {
    let out = run(&[
        "for-each-ref",
        "--format=%(refname:short)",
        &format!("refs/heads/{SAFETY_REF_PREFIX}"),
    ])?;
    let mut refs: Vec<(String, u128)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(SAFETY_REF_PREFIX))
        .filter_map(|name| safety_ref_timestamp(name).map(|ts| (name.to_string(), ts)))
        .collect();
    refs.sort_by_key(|(_, ts)| std::cmp::Reverse(*ts));

    let mut deleted = 0usize;
    for (name, _) in refs.into_iter().skip(keep) {
        run(&["update-ref", "-d", &format!("refs/heads/{name}")])?;
        deleted += 1;
    }
    Ok(deleted)
}

fn safety_ref_timestamp(name: &str) -> Option<u128> {
    name.strip_prefix(SAFETY_REF_PREFIX)?
        .rsplit_once('-')?
        .1
        .parse()
        .ok()
}
