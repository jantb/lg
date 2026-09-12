//! Named bundles of sandbox rules and network allowlists, one per toolchain.
//!
//! A preset *spec* is a comma-separated list (`"rust,kotlin,node"`), because one
//! project can legitimately mix languages. Everything here takes a spec and
//! unions the individual presets' contributions: [`rules`] for the filesystem,
//! [`domains`] for the network.

mod domains;
mod rules;

pub use domains::ensure_defaults;

use crate::config::project::Rule;

pub fn known_presets() -> &'static [&'static str] {
    &["rust", "kotlin", "swift", "node", "python", "none"]
}

/// Splits a preset spec into its individual preset names.
///
/// Names are trimmed, lowercased and deduplicated; an empty spec resolves to
/// `["none"]`.
pub fn parse_presets(spec: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for part in spec.split(',') {
        let name = part.trim().to_ascii_lowercase();
        if name.is_empty() || names.contains(&name) {
            continue;
        }
        names.push(name);
    }
    if names.is_empty() {
        names.push("none".to_string());
    }
    names
}

/// Returns true when every preset in the spec is known.
pub fn is_known_preset(spec: &str) -> bool {
    parse_presets(spec)
        .iter()
        .all(|name| known_presets().contains(&name.as_str()))
}

/// Normalizes a preset spec to its canonical stored form.
pub fn normalize_preset(spec: &str) -> String {
    parse_presets(spec).join(",")
}

/// Returns rules for the given preset spec, or None if any preset is unknown.
///
/// For a multi-preset spec the rules are the union of the individual presets,
/// deduplicated so the rendered profile stays compact.
pub fn preset_rules(spec: &str) -> Option<Vec<Rule>> {
    let mut merged: Vec<Rule> = Vec::new();
    for name in parse_presets(spec) {
        for rule in single_preset_rules(&name)? {
            if !merged.contains(&rule) {
                merged.push(rule);
            }
        }
    }
    Some(merged)
}

fn single_preset_rules(name: &str) -> Option<Vec<Rule>> {
    match name {
        // Cargo runs outside the sandbox, so `rust` needs nothing beyond the base.
        "rust" | "none" => Some(rules::default_preset()),
        "kotlin" => Some(rules::kotlin_preset()),
        "swift" => Some(rules::swift_preset()),
        "node" => Some(rules::node_preset()),
        "python" => Some(rules::python_preset()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_presets_splits_trims_and_dedups() {
        assert_eq!(
            parse_presets(" Rust , kotlin,node ,rust"),
            vec!["rust", "kotlin", "node"]
        );
    }

    #[test]
    fn parse_presets_empty_spec_is_none_preset() {
        assert_eq!(parse_presets("  "), vec!["none"]);
    }

    #[test]
    fn normalize_preset_canonicalizes_spec() {
        assert_eq!(normalize_preset(" Rust ,KOTLIN , rust"), "rust,kotlin");
    }

    #[test]
    fn is_known_preset_accepts_multi_and_rejects_unknown_member() {
        assert!(is_known_preset("rust,kotlin,node"));
        assert!(!is_known_preset("rust,bogus"));
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(preset_rules("nonexistent").is_none());
    }

    #[test]
    fn multi_preset_with_unknown_member_returns_none() {
        assert!(preset_rules("rust,bogus").is_none());
    }

    #[test]
    fn known_presets_matches_preset_rules() {
        for name in known_presets() {
            assert!(
                preset_rules(name).is_some(),
                "known preset '{name}' should resolve via preset_rules()"
            );
        }
    }

    #[test]
    fn multi_preset_rules_are_union_without_duplicates() {
        let rules = preset_rules("rust,kotlin,node").unwrap();
        // Kotlin-only and node-only rules are both present.
        assert!(
            rules
                .iter()
                .any(|r| r.path_value.as_deref() == Some("MAVEN_HOME"))
        );
        assert!(
            rules
                .iter()
                .any(|r| r.path_value.as_deref() == Some("HOME/.yarn"))
        );
        // Shared base rules appear exactly once.
        let count_of = |path: &str| {
            rules
                .iter()
                .filter(|r| r.path_value.as_deref() == Some(path))
                .count()
        };
        assert_eq!(count_of("PROJECT_ROOT"), 1);
        assert_eq!(count_of("HOME/.claude"), 1);
    }
}
