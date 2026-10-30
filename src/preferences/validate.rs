//! Checking loaded preferences before they are used.

use anyhow::{Result, bail};
use std::collections::HashSet;

use super::{Confinement, Preferences, Promotion};

impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported configuration version {}", self.version);
        }
        if self.writing.language.trim().is_empty() {
            bail!("writing.language must not be empty");
        }
        if !self.models.endpoint.starts_with("http://")
            && !self.models.endpoint.starts_with("https://")
        {
            bail!("models.endpoint must be an HTTP(S) URL");
        }
        validate_ref(&self.branches.base)?;
        validate_remote(&self.branches.remote)?;
        let mut ids = HashSet::new();
        let mut refs = HashSet::new();
        for e in &self.branches.environments {
            if e.id.is_empty() || e.id == "feature" || !ids.insert(e.id.as_str()) {
                bail!("environment IDs must be unique, nonempty, and not 'feature'");
            }
            validate_remote(&e.remote)?;
            if !e.branch.is_empty() {
                validate_ref(&e.branch)?;
                if !refs.insert((&e.remote, &e.branch)) {
                    bail!("multiple environments map to {}/{}", e.remote, e.branch);
                }
            }
        }
        for b in &self.branches.protected {
            validate_ref(b)?;
        }
        for p in &self.branches.promotions {
            if !ids.contains(p.to.as_str())
                || (p.from != "feature" && !ids.contains(p.from.as_str()))
            {
                bail!("promotion references an unknown environment");
            }
            let mut seen = HashSet::new();
            if cycle(
                &p.from,
                &self.branches.promotions,
                &mut seen,
                &mut HashSet::new(),
            ) {
                bail!("promotion paths contain a cycle");
            }
        }
        let mut names = HashSet::new();
        if self.agents.iter().filter(|a| a.default).count() > 1 {
            bail!("choose only one default agent");
        }
        for a in &self.agents {
            if a.name.trim().is_empty() || !names.insert(&a.name) {
                bail!("agent names must be unique and nonempty");
            }
            if a.executable.is_empty() || a.executable.contains('\0') {
                bail!("agent executable must not be empty");
            }
            if a.confinement == Confinement::Agent && !a.adapter.has_own_permissions() {
                bail!(
                    "{} does not provide an agent-managed permission adapter",
                    a.name
                );
            }
        }
        Ok(())
    }
}

fn cycle<'a>(
    node: &'a str,
    edges: &'a [Promotion],
    stack: &mut HashSet<&'a str>,
    done: &mut HashSet<&'a str>,
) -> bool {
    if done.contains(node) {
        return false;
    }
    if !stack.insert(node) {
        return true;
    }
    for e in edges.iter().filter(|e| e.from == node) {
        if cycle(&e.to, edges, stack, done) {
            return true;
        }
    }
    stack.remove(node);
    done.insert(node);
    false
}

fn validate_ref(s: &str) -> Result<()> {
    if s.is_empty()
        || s.starts_with('-')
        || s.contains([' ', '\n', '\r', '\0', ':', '~', '^', '?', '*', '[', '\\'])
        || s.contains("..")
        || s.contains("@{")
        || s.ends_with('/')
        || s.ends_with('.')
        || s.ends_with(".lock")
    {
        bail!("invalid branch name: {s}");
    }
    Ok(())
}

fn validate_remote(s: &str) -> Result<()> {
    if s.is_empty() || s.starts_with('-') || s.contains(['/', ' ', '\n', '\0']) {
        bail!("invalid remote name: {s}");
    }
    Ok(())
}
