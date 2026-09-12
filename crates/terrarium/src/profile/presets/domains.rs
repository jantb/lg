//! The outbound hosts each preset's proxy allowlist starts from.
//!
//! Only consulted when the proxy is enabled; the sandbox itself denies network
//! access outright either way.

use super::parse_presets;

/// Hosts every preset allows, regardless of toolchain: Claude's own API, the
/// telemetry endpoints Claude Code reports to, and GitHub for source fetches.
///
/// The GitHub set is deliberately wider than `github.com`: git-based
/// dependencies clone through `codeload`, and the prebuilt binaries that npm,
/// cargo and pip packages download during install (esbuild, sharp, node-gyp
/// headers, ...) are release assets served from `objects.githubusercontent.com`.
const BASE_DOMAINS: &[&str] = &[
    "anthropic.com",
    "claude.ai",
    "platform.claude.com",
    "sentry.io",
    "datadoghq.com",
    "otel-agents.alv.no",
    "alvminnelig.test.alv.no",
    "alvminnelig.alv.no",
    "storage.googleapis.com",
    "github.com",
    "api.github.com",
    "codeload.github.com",
    "raw.githubusercontent.com",
    "objects.githubusercontent.com",
    "githubusercontent.com",
];

/// Returns the default allowed domains for a preset spec (union for multi-preset specs).
pub fn default_allowed_domains(spec: &str) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for name in parse_presets(spec) {
        for domain in single_preset_domains(&name) {
            push_unique(&mut domains, domain);
        }
    }
    domains
}

fn single_preset_domains(preset: &str) -> Vec<String> {
    let mut domains: Vec<&str> = BASE_DOMAINS.to_vec();
    match preset {
        // `docs.rs` is for reading a dependency's API before writing against it.
        // Without it the only way to learn an unfamiliar crate's surface is to
        // compile against guesses and follow the errors.
        "rust" => domains.extend([
            "crates.io",
            "static.crates.io",
            "index.crates.io",
            "docs.rs",
            "static.rust-lang.org",
            "rust-lang.org",
        ]),
        // `repo.maven.apache.org` redirects to `repo1.maven.org`, and the Gradle
        // wrapper's download from `services.gradle.org` lands on
        // `downloads.gradle.org` — both hops need to be allowed, not just the
        // hostname the build file names.
        "kotlin" => domains.extend([
            "repo.maven.apache.org",
            "repo1.maven.org",
            "plugins.gradle.org",
            "services.gradle.org",
            "downloads.gradle.org",
            "repo.gradle.org",
            "maven.google.com",
            "dl.google.com",
            "jitpack.io",
            "oss.sonatype.org",
            "s01.oss.sonatype.org",
            "central.sonatype.com",
        ]),
        "node" => domains.extend([
            "registry.npmjs.org",
            "registry.npmjs.com",
            "npmjs.org",
            "npmjs.com",
            "registry.yarnpkg.com",
            "yarnpkg.com",
            "nodejs.org",
            "unpkg.com",
            "cdn.jsdelivr.net",
            "esm.sh",
            "jsr.io",
            "deno.land",
            "playwright.azureedge.net",
            "playwright-akamai.azureedge.net",
            "playwright-verizon.azureedge.net",
            "cdn.playwright.dev",
        ]),
        "python" => domains.extend([
            "pypi.org",
            "pypi.python.org",
            "files.pythonhosted.org",
            "astral.sh",
            "python.org",
        ]),
        // Swift resolves packages from git hosts, already covered by the base.
        "swift" => domains.extend(["swift.org", "download.swift.org"]),
        _ => {}
    }
    domains.into_iter().map(String::from).collect()
}

/// Extends `domains` with preset defaults, deduplicating case-insensitively.
pub fn ensure_defaults(domains: &mut Vec<String>, preset: &str) {
    for d in default_allowed_domains(preset) {
        push_unique(domains, d);
    }
}

/// Hostnames are case-insensitive, so `Anthropic.COM` must not be added twice.
fn push_unique(domains: &mut Vec<String>, domain: String) {
    if !domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
        domains.push(domain);
    }
}

#[cfg(test)]
mod tests {
    use super::super::known_presets;
    use super::*;

    #[test]
    fn base_domains_include_anthropic() {
        let domains = default_allowed_domains("none");
        assert!(domains.contains(&"anthropic.com".to_string()));
    }

    #[test]
    fn base_domains_include_datadog() {
        let domains = default_allowed_domains("none");
        assert!(domains.contains(&"datadoghq.com".to_string()));
    }

    #[test]
    fn base_domains_include_alv_otel_agents() {
        let domains = default_allowed_domains("none");
        assert!(domains.contains(&"otel-agents.alv.no".to_string()));
    }

    #[test]
    fn all_presets_include_base_domains() {
        for preset in known_presets() {
            let domains = default_allowed_domains(preset);
            for expected in ["anthropic.com", "claude.ai", "datadoghq.com"] {
                assert!(
                    domains.contains(&expected.to_string()),
                    "{preset} missing {expected}"
                );
            }
        }
    }

    #[test]
    fn multi_preset_domains_are_union() {
        let domains = default_allowed_domains("rust,kotlin,node");
        assert!(domains.contains(&"crates.io".to_string()));
        assert!(domains.contains(&"repo.maven.apache.org".to_string()));
        assert!(domains.contains(&"registry.npmjs.org".to_string()));
        let anthropic = domains
            .iter()
            .filter(|d| d.eq_ignore_ascii_case("anthropic.com"))
            .count();
        assert_eq!(anthropic, 1, "base domains must not be duplicated");
    }

    /// Every toolchain must be able to reach the index it resolves packages
    /// from — this is the allowlist entry whose absence looks like a network
    /// outage rather than a policy decision.
    #[test]
    fn every_toolchain_reaches_its_package_index() {
        for (preset, index) in [
            ("rust", "crates.io"),
            ("kotlin", "repo1.maven.org"),
            ("node", "registry.npmjs.org"),
            ("python", "files.pythonhosted.org"),
            ("swift", "download.swift.org"),
        ] {
            assert!(
                default_allowed_domains(preset).contains(&index.to_string()),
                "preset '{preset}' cannot reach '{index}'"
            );
        }
    }

    /// Install scripts fetch prebuilt binaries from GitHub release assets, which
    /// are served from a different host than `github.com`.
    #[test]
    fn base_domains_include_github_release_assets() {
        let domains = default_allowed_domains("none");
        assert!(domains.contains(&"objects.githubusercontent.com".to_string()));
        assert!(domains.contains(&"codeload.github.com".to_string()));
    }

    #[test]
    fn ensure_defaults_adds_missing() {
        let mut domains: Vec<String> = vec![];
        ensure_defaults(&mut domains, "rust");
        assert!(
            domains.contains(&"anthropic.com".to_string()),
            "anthropic.com should be added"
        );
        assert!(
            domains.contains(&"crates.io".to_string()),
            "crates.io should be added"
        );
    }

    #[test]
    fn ensure_defaults_no_duplicates() {
        let mut domains = vec!["anthropic.com".to_string()];
        ensure_defaults(&mut domains, "rust");
        let count = domains
            .iter()
            .filter(|d| d.eq_ignore_ascii_case("anthropic.com"))
            .count();
        assert_eq!(count, 1, "anthropic.com should appear exactly once");
    }

    #[test]
    fn ensure_defaults_case_insensitive_dedup() {
        let mut domains = vec!["Anthropic.COM".to_string()];
        ensure_defaults(&mut domains, "rust");
        let count = domains
            .iter()
            .filter(|d| d.eq_ignore_ascii_case("anthropic.com"))
            .count();
        assert_eq!(
            count, 1,
            "anthropic.com should not be duplicated despite different casing"
        );
    }
}
