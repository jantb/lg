//! Which release branches exist and whether a branch has reached them.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crate::config::{BRANCH_TEST, DEV_BRANCH_NAMES, is_protected_branch_name};

use super::commits::{commit_oid, preferred_commit_ref, rev_list};
use super::run;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ReleaseStatusCacheKey {
    branch: String,
    branch_oid: String,
    base_oid: String,
    environment_refs: Vec<(String, Option<String>)>,
    develop_oid: Option<String>,
    test_oid: Option<String>,
}

static RELEASE_STATUS_CACHE: OnceLock<Mutex<HashMap<ReleaseStatusCacheKey, BranchReleaseStatus>>> =
    OnceLock::new();

fn release_status_cache() -> &'static Mutex<HashMap<ReleaseStatusCacheKey, BranchReleaseStatus>> {
    RELEASE_STATUS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The two environments lg releases into. Which branch feeds each one is
/// per checkout, so the environment and the branch name are kept apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseEnv {
    Dev,
    Test,
}

/// Deploy branches found in a checkout. Neither is required: alv.no deploys
/// `test` and has no develop branch, and a repository with only a develop
/// branch works the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseBranches {
    pub configured: bool,
    dev: Option<String>,
    test: Option<String>,
}

impl ReleaseBranches {
    pub fn new(dev: Option<String>, test: Option<String>) -> Self {
        Self {
            dev,
            test,
            configured: false,
        }
    }

    pub fn branch(&self, env: ReleaseEnv) -> Option<&str> {
        match env {
            ReleaseEnv::Dev => self.dev.as_deref(),
            ReleaseEnv::Test => self.test.as_deref(),
        }
    }

    pub fn any(&self) -> bool {
        self.dev.is_some() || self.test.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchReleaseStatus {
    pub environments: std::collections::BTreeMap<String, ReleaseTargetStatus>,
    pub main: Option<ReleaseTargetStatus>,
    pub develop: Option<ReleaseTargetStatus>,
    pub test: Option<ReleaseTargetStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTargetStatus {
    pub released_at: String,
    pub missing_commits: usize,
}

pub fn branch_release_status(branch: &str) -> Result<BranchReleaseStatus> {
    let configured_base = crate::preferences::base_branch();
    let configured_remote = crate::preferences::remote();
    if branch.is_empty() || is_protected_branch_name(branch) {
        return Ok(BranchReleaseStatus::default());
    }

    let base_ref = preferred_commit_ref(
        &format!("{configured_remote}/{configured_base}"),
        configured_base.as_str(),
    )
    .unwrap_or_else(|| configured_base.as_str().to_string());
    let Some(base_oid) = commit_oid(&base_ref) else {
        return Ok(BranchReleaseStatus::default());
    };
    let Some(branch_oid) = commit_oid(branch) else {
        return Ok(BranchReleaseStatus::default());
    };
    let targets = release_branches();
    let develop_ref = release_branch_ref(targets.branch(ReleaseEnv::Dev));
    let test_ref = release_branch_ref(targets.branch(ReleaseEnv::Test));
    let configured_environments = crate::preferences::load().config.branches.environments;
    let environment_refs: Vec<_> = configured_environments
        .iter()
        .map(|e| {
            let reference = format!("{}/{}", e.remote, e.branch);
            (e.id.clone(), commit_oid(&reference))
        })
        .collect();
    let key = ReleaseStatusCacheKey {
        environment_refs,
        branch: branch.to_string(),
        branch_oid,
        base_oid,
        develop_oid: develop_ref.as_deref().and_then(commit_oid),
        test_oid: test_ref.as_deref().and_then(commit_oid),
    };
    if let Ok(cache) = release_status_cache().lock()
        && let Some(status) = cache.get(&key)
    {
        return Ok(status.clone());
    }

    let unique_commits = rev_list(&["--reverse", branch, &format!("^{base_ref}")])?;
    let mut environments = std::collections::BTreeMap::new();
    for env in &configured_environments {
        if env.branch.is_empty() {
            continue;
        }
        let reference = format!("{}/{}", env.remote, env.branch);
        if let Some(status) =
            release_target_status(branch, &unique_commits, &base_ref, Some(&reference))
                .ok()
                .flatten()
        {
            environments.insert(env.id.clone(), status);
        }
    }
    if unique_commits.is_empty() {
        // Branch tip is reachable from main (regular or rebase merge); record
        // the merge date so the deployment panel can show it as merged.
        let released_at = first_containing_commit_date(&base_ref, branch)
            .or_else(|| commit_date(branch).ok())
            .unwrap_or_else(|| "unknown".to_string());
        let status = BranchReleaseStatus {
            environments,
            main: Some(ReleaseTargetStatus {
                released_at,
                missing_commits: 0,
            }),
            develop: None,
            test: None,
        };
        if let Ok(mut cache) = release_status_cache().lock() {
            cache.insert(key, status.clone());
        }
        return Ok(status);
    }

    let status = BranchReleaseStatus {
        environments,
        main: Some(ReleaseTargetStatus {
            released_at: String::new(),
            missing_commits: unique_commits.len(),
        }),
        develop: release_target_status(branch, &unique_commits, &base_ref, develop_ref.as_deref())?,
        test: release_target_status(branch, &unique_commits, &base_ref, test_ref.as_deref())?,
    };
    if let Ok(mut cache) = release_status_cache().lock() {
        cache.insert(key, status.clone());
    }
    Ok(status)
}

/// The deploy branches this checkout actually has. Each environment is looked
/// up on its own so a repository with only one of them still gets its release
/// actions and deployment status.
pub fn release_branches() -> ReleaseBranches {
    let loaded = crate::preferences::load();
    if loaded.sources.contains_key("branches") {
        return configured_release_branches(&loaded.config.branches.environments);
    }
    ReleaseBranches::new(
        DEV_BRANCH_NAMES
            .into_iter()
            .find(|name| release_branch_ref(Some(name)).is_some())
            .map(str::to_string),
        release_branch_ref(Some(BRANCH_TEST)).map(|_| BRANCH_TEST.to_string()),
    )
}

/// The release slots of a configured environment list. An environment named
/// `dev` or `test` takes its slot; otherwise the slots fill in the order the
/// environments are listed, so a repository whose staging environment is
/// called something else still gets its release actions.
pub fn configured_release_branches(
    environments: &[crate::preferences::Environment],
) -> ReleaseBranches {
    let with_branch: Vec<_> = environments
        .iter()
        .filter(|e| !e.branch.is_empty())
        .collect();
    let by_id = |id: &str| with_branch.iter().find(|e| e.id == id).copied();
    let named_dev = by_id("dev");
    let named_test = by_id("test");
    let mut rest = with_branch
        .iter()
        .copied()
        .filter(|e| Some(*e) != named_dev && Some(*e) != named_test);
    let dev = named_dev.or_else(|| rest.next());
    let test = named_test.or_else(|| rest.next());
    let mut branches = ReleaseBranches::new(
        dev.map(|e| e.branch.clone()),
        test.map(|e| e.branch.clone()),
    );
    branches.configured = true;
    branches
}

/// The ref to compare against for a deploy branch, preferring the remote so an
/// out-of-date local checkout does not report a release that never landed.
fn release_branch_ref(branch: Option<&str>) -> Option<String> {
    let configured_remote = crate::preferences::remote();
    let branch = branch?;
    preferred_commit_ref(&format!("{configured_remote}/{branch}"), branch)
}

fn release_target_status(
    branch: &str,
    unique_commits: &[String],
    base_ref: &str,
    target_ref: Option<&str>,
) -> Result<Option<ReleaseTargetStatus>> {
    let Some(target_ref) = target_ref else {
        return Ok(None);
    };

    let missing = rev_list(&[branch, &format!("^{base_ref}"), "--not", target_ref])?;
    let missing_set: HashSet<&str> = missing.iter().map(String::as_str).collect();
    let latest_released = unique_commits
        .iter()
        .rev()
        .find(|sha| !missing_set.contains(sha.as_str()));

    let Some(latest_released) = latest_released else {
        return Ok(Some(ReleaseTargetStatus {
            released_at: String::new(),
            missing_commits: missing.len(),
        }));
    };

    let released_at = first_containing_commit_date(target_ref, latest_released)
        .or_else(|| commit_date(latest_released).ok())
        .unwrap_or_else(|| "unknown".to_string());
    Ok(Some(ReleaseTargetStatus {
        released_at,
        missing_commits: missing.len(),
    }))
}

fn first_containing_commit_date(target_ref: &str, commit: &str) -> Option<String> {
    let first_parent = rev_list(&["--first-parent", "--reverse", target_ref]).ok()?;
    if first_parent
        .iter()
        .any(|target_commit| target_commit == commit)
    {
        return commit_date(commit).ok();
    }

    let range = format!("{commit}..{target_ref}");
    let containing_path =
        rev_list(&["--first-parent", "--reverse", "--ancestry-path", &range]).ok()?;
    containing_path
        .first()
        .and_then(|target_commit| commit_date(target_commit).ok())
}

fn commit_date(commit: &str) -> Result<String> {
    let out = run(&[
        "show",
        "-s",
        "--format=%cd",
        "--date=format:%Y-%m-%d %H:%M",
        commit,
    ])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Environment;

    fn env(id: &str, branch: &str) -> Environment {
        Environment {
            id: id.into(),
            branch: branch.into(),
            ..Environment::default()
        }
    }

    #[test]
    fn environments_named_dev_and_test_take_their_slots() {
        let branches = configured_release_branches(&[env("test", "qa"), env("dev", "develop")]);
        assert_eq!(branches.branch(ReleaseEnv::Dev), Some("develop"));
        assert_eq!(branches.branch(ReleaseEnv::Test), Some("qa"));
    }

    #[test]
    fn otherwise_environments_fill_the_slots_in_listed_order() {
        let branches =
            configured_release_branches(&[env("staging", "staging"), env("preprod", "release")]);
        assert_eq!(branches.branch(ReleaseEnv::Dev), Some("staging"));
        assert_eq!(branches.branch(ReleaseEnv::Test), Some("release"));
        assert!(branches.configured);
    }

    #[test]
    fn a_named_slot_is_kept_while_the_other_falls_back() {
        let branches =
            configured_release_branches(&[env("staging", "staging"), env("test", "test")]);
        assert_eq!(branches.branch(ReleaseEnv::Dev), Some("staging"));
        assert_eq!(branches.branch(ReleaseEnv::Test), Some("test"));
        let empty = configured_release_branches(&[env("staging", "")]);
        assert!(!empty.any());
    }
}
