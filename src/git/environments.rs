//! Explicit promotion previews and optimistic ref checks before mutation.
use super::{git_command, run};
use crate::preferences::{Environment, Promotion};
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionPreview {
    pub source: String,
    pub source_oid: String,
    pub target_oid: String,
    pub environment: Environment,
    pub rule: Promotion,
    pub commits: Vec<String>,
}
pub fn preview_promotion(source: &str, environment_id: &str) -> Result<PromotionPreview> {
    let loaded = crate::preferences::load();
    if !loaded.errors.is_empty() {
        bail!(
            "fix configuration errors before promoting: {}",
            loaded.errors.join("; ")
        );
    }
    let config = loaded.config.branches;
    let env = config
        .environments
        .iter()
        .find(|e| e.id == environment_id)
        .context("environment not configured")?
        .clone();
    if env.branch.is_empty() {
        bail!("this environment has no branch deployment source");
    }
    let source_role = config
        .environments
        .iter()
        .find(|e| e.branch == source)
        .map_or("feature", |e| e.id.as_str());
    let rule = config
        .promotions
        .iter()
        .find(|p| p.from == source_role && p.to == env.id)
        .context("no allowed promotion path from this source to that environment")?
        .clone();
    let source_oid = oid(&format!("refs/heads/{source}"))?;
    let remote_ref = format!("refs/remotes/{}/{}", env.remote, env.branch);
    let target_oid = oid(&remote_ref)
        .with_context(|| format!("fetch {}/{} before promoting", env.remote, env.branch))?;
    if let Ok(local) = oid(&format!("refs/heads/{}", env.branch))
        && local != target_oid
    {
        bail!(
            "local {} differs from {}/{}; reconcile it before promotion",
            env.branch,
            env.remote,
            env.branch
        );
    }
    let out = run(&[
        "log",
        "--format=%h %s",
        &format!("{target_oid}..{source_oid}"),
    ])?;
    let commits = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    Ok(PromotionPreview {
        source: source.into(),
        source_oid,
        target_oid,
        environment: env,
        rule,
        commits,
    })
}
fn oid(reference: &str) -> Result<String> {
    let out = run(&["rev-parse", "--verify", &format!("{reference}^{{commit}}")])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().into())
}
/// Merge in a detached worktree. Leave the checkout intact, and retain conflict
/// work with a named safety ref if Git cannot finish the promotion.
pub fn promote(preview: &PromotionPreview) -> Result<String> {
    let refreshed = preview_promotion(&preview.source, &preview.environment.id)?;
    if &refreshed != preview {
        bail!("promotion inputs changed; reopen the preview");
    }
    let env = &preview.environment;
    let remote_ref = format!("refs/heads/{}", env.branch);
    let remote = run(&["ls-remote", "--exit-code", &env.remote, &remote_ref])?;
    if String::from_utf8_lossy(&remote.stdout)
        .split_whitespace()
        .next()
        != Some(preview.target_oid.as_str())
    {
        bail!("remote target moved; fetch and reopen the preview");
    }
    let parent = super::common_git_dir()?.join("lg-promotions");
    std::fs::create_dir_all(&parent)?;
    let scratch = tempfile::Builder::new()
        .prefix("promotion-")
        .tempdir_in(&parent)?;
    let dir = scratch.keep();
    let dir_string = dir.to_string_lossy().into_owned();
    run(&[
        "worktree",
        "add",
        "--detach",
        &dir_string,
        &preview.target_oid,
    ])?;
    let safety = format!(
        "refs/lg/promotions/{}",
        dir.file_name().unwrap().to_string_lossy()
    );
    run(&["update-ref", &safety, &preview.target_oid])?;
    let merged = super::with_repo(&dir, || -> Result<String> {
        match preview.rule.strategy.as_str() {
            "ff-only" => {
                run(&["merge", "--ff-only", &preview.source_oid])?;
            }
            "squash" => {
                run(&["merge", "--squash", &preview.source_oid])?;
                run(&[
                    "commit",
                    "-m",
                    &format!("Promote {} to {}", preview.source, env.name),
                ])?;
            }
            "merge" => {
                run(&["merge", "--no-edit", "--no-ff", &preview.source_oid])?;
            }
            _ => bail!("unsupported merge strategy"),
        }
        oid("HEAD")
    });
    let merged = match merged {
        Ok(oid) => oid,
        Err(e) => bail!(
            "{e}\nPromotion worktree retained at {}. Resolve or abort there; original checkout is unchanged. Safety ref: {safety}",
            dir.display()
        ),
    };
    run(&["update-ref", &safety, &merged])?;
    if preview.rule.push {
        // Normal push rejects non-fast-forward races; lease also rejects forward
        // movement since the preview, rather than integrating unreviewed refs.
        let result = run(&[
            "push",
            &format!("--force-with-lease={remote_ref}:{}", preview.target_oid),
            &env.remote,
            &format!("{merged}:{remote_ref}"),
        ]);
        if let Err(e) = result {
            bail!("{e}\nResult retained at {safety} and {}", dir.display());
        }
    }
    run(&["worktree", "remove", &dir_string])?;
    if preview.rule.push {
        let tracking = format!("refs/remotes/{}/{}", env.remote, env.branch);
        // Compare-and-swap only the tracking ref we previewed. Another fetch may
        // already have updated it; its newer observation must win.
        let _ = git_command(&["update-ref", &tracking, &merged, &preview.target_oid]).output();
        run(&["update-ref", "-d", &safety])?;
        Ok(format!(
            "Promoted {} to {}/{}; deployment status unknown",
            preview.source, env.remote, env.branch
        ))
    } else {
        Ok(format!(
            "Promotion prepared at {safety} ({merged}); push was disabled"
        ))
    }
}
