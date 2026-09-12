//! The filesystem rules each preset contributes.
//!
//! Every preset starts from [`claude_code_base`] — what Claude Code itself needs
//! to run — and adds only what its toolchain requires beyond that. Build caches
//! like `CARGO_HOME` and `GRADLE_HOME` are deliberately absent: builds run in the
//! MCP server process outside the sandbox, so the sandboxed session has no reason
//! to reach them.

use crate::config::project::{PathType, Rule, RuleAction, RuleSource};

/// Read-write on a subpath, uncommented — used for `PROJECT_ROOT`, whose purpose
/// needs no explaining.
fn rw(path: &str) -> Rule {
    Rule {
        action: RuleAction::Allow,
        operation: "file-read* file-write*".to_string(),
        path_type: Some(PathType::Subpath),
        path_value: Some(path.to_string()),
        comment: None,
        source: RuleSource::Preset,
    }
}

fn rw_comment(path: &str, comment: &str) -> Rule {
    Rule {
        action: RuleAction::Allow,
        operation: "file-read* file-write*".to_string(),
        path_type: Some(PathType::Subpath),
        path_value: Some(path.to_string()),
        comment: Some(comment.to_string()),
        source: RuleSource::Preset,
    }
}

fn ro_comment(path: &str, comment: &str) -> Rule {
    Rule {
        action: RuleAction::Allow,
        operation: "file-read*".to_string(),
        path_type: Some(PathType::Subpath),
        path_value: Some(path.to_string()),
        comment: Some(comment.to_string()),
        source: RuleSource::Preset,
    }
}

/// Read-write on one exact path rather than a subtree.
fn rw_literal_comment(path: &str, comment: &str) -> Rule {
    Rule {
        action: RuleAction::Allow,
        operation: "file-read* file-write*".to_string(),
        path_type: Some(PathType::Literal),
        path_value: Some(path.to_string()),
        comment: Some(comment.to_string()),
        source: RuleSource::Preset,
    }
}

/// An operation allowed everywhere, with no path filter.
fn bare_allow(operation: &str, comment: &str) -> Rule {
    Rule {
        action: RuleAction::Allow,
        operation: operation.to_string(),
        path_type: None,
        path_value: None,
        comment: Some(comment.to_string()),
        source: RuleSource::Preset,
    }
}

/// Rules every preset shares: Claude Code config/state, SCM config, Node runtime.
fn claude_code_base() -> Vec<Rule> {
    vec![
        // -- File creation (mkdir + new files) --
        bare_allow("file-write-create", "Allow mkdir and new file creation"),
        // -- Claude Code config & state --
        rw_comment("HOME/.claude", "Claude Code config, sessions, skills"),
        rw_comment(
            "HOME/.terrarium",
            "terrarium config, sandbox profiles, instance registry",
        ),
        rw_comment(
            "HOME/.spenn-client-state-rs",
            "spenn-api-client global sled scenario store",
        ),
        rw_literal_comment(
            "HOME/.spenn-client.pid",
            "spenn-api-client singleton pid file",
        ),
        rw_comment("HOME/.claude.json", "Claude Code legacy config"),
        ro_comment("HOME/.mcp.json", "User MCP server config"),
        rw_comment(
            "HOME/Library/Application Support/Claude",
            "MCP desktop integration",
        ),
        ro_comment(
            "/Library/Application Support/ClaudeCode",
            "Managed enterprise policies",
        ),
        // -- Git --
        ro_comment("HOME/.gitconfig", "Git user config"),
        ro_comment("HOME/.config/git", "Git XDG config"),
        ro_comment("HOME/.ssh/config", "SSH host aliases for git"),
        ro_comment("HOME/.ssh/known_hosts", "SSH known hosts"),
        // -- SCM CLIs (gh, glab) --
        rw_comment("HOME/.config/gh", "GitHub CLI auth & config"),
        // -- Node.js runtime (Claude Code runs on Node) --
        rw_comment("HOME/.npm", "npm cache"),
        rw_comment("HOME/.nvm", "nvm Node version manager"),
        rw_comment("HOME/.fnm", "fnm Node version manager"),
        rw_comment("HOME/.local/share/pnpm", "pnpm global store"),
        rw_comment(
            "HOME/.local/share/claude",
            "Claude Code native install binaries",
        ),
        // -- Developer tools (git → xcode-select on macOS) --
        // /var is a symlink to /private/var; sandbox resolves to canonical path
        rw_literal_comment(
            "/private/var/select/developer_dir",
            "xcrun developer dir symlink",
        ),
        ro_comment(
            "/Library/Developer/CommandLineTools",
            "CLT SDK, toolchain, headers",
        ),
        ro_comment(
            "/Applications/Xcode.app/Contents/Developer",
            "Xcode SDK and toolchains",
        ),
    ]
}

/// The base plus the project root — also what `rust` and `none` resolve to, since
/// cargo runs outside the sandbox.
pub(super) fn default_preset() -> Vec<Rule> {
    let mut rules = claude_code_base();
    rules.push(rw("PROJECT_ROOT"));
    rules
}

pub(super) fn kotlin_preset() -> Vec<Rule> {
    let mut rules = default_preset();
    rules.push(ro_comment(
        "MAVEN_HOME",
        "Maven local repository (~/.m2) for mavenLocal dependencies",
    ));
    rules
}

pub(super) fn swift_preset() -> Vec<Rule> {
    let mut rules = default_preset();
    // Swift toolchains, DerivedData, device support
    rules.push(rw_comment(
        "HOME/Library/Developer",
        "Xcode toolchains, DerivedData, Swift toolchain",
    ));
    // Swift Package Manager cache
    rules.push(rw_comment(
        "HOME/Library/Caches/org.swift.swiftpm",
        "SPM package cache",
    ));
    rules
}

pub(super) fn node_preset() -> Vec<Rule> {
    let mut rules = default_preset();
    // -- Package manager config --
    // `.npmrc` holds the registry and its auth token: without it every install
    // resolves against the wrong registry, or fails on a private scope.
    rules.push(rw_literal_comment(
        "HOME/.npmrc",
        "npm registry and auth config",
    ));
    rules.push(rw_comment("HOME/.config/npm", "npm XDG config and cache"));
    rules.push(rw_literal_comment("HOME/.yarnrc.yml", "Yarn Berry config"));
    rules.push(rw_literal_comment("HOME/.yarnrc", "Yarn Classic config"));
    // -- Package manager caches and state beyond the Node runtime paths in the base --
    rules.push(rw_comment("HOME/.yarn", "Yarn global state"));
    rules.push(rw_comment("HOME/.cache/yarn", "Yarn cache"));
    rules.push(rw_comment("HOME/.config/yarn", "Yarn config"));
    rules.push(rw_comment("HOME/Library/pnpm", "pnpm home (macOS)"));
    rules.push(rw_comment("HOME/.bun", "Bun runtime and cache"));
    rules.push(rw_comment("HOME/.deno", "Deno module cache"));
    rules.push(rw_comment("HOME/Library/Caches/deno", "Deno cache (macOS)"));
    // Corepack shims the package manager named in `packageManager`, downloading
    // it on first use into the Node cache directory.
    rules.push(rw_comment(
        "HOME/.cache/node",
        "Corepack and Node compile cache",
    ));
    rules.push(rw_comment(
        "HOME/Library/Caches/node",
        "Corepack and Node compile cache (macOS)",
    ));
    // -- Version managers beyond the nvm/fnm pair in the base --
    rules.push(rw_comment("HOME/.volta", "Volta Node version manager"));
    rules.push(rw_comment("HOME/.asdf", "asdf version manager"));
    rules.push(rw_comment("HOME/.local/share/mise", "mise tool installs"));
    rules.push(rw_comment("HOME/.cache/mise", "mise cache"));
    rules.push(ro_comment("HOME/.config/mise", "mise config"));
    // -- Native build and type tooling --
    rules.push(rw_comment("HOME/.node-gyp", "node-gyp headers cache"));
    rules.push(rw_comment(
        "HOME/Library/Caches/node-gyp",
        "node-gyp headers cache (macOS)",
    ));
    rules.push(rw_comment(
        "HOME/Library/Caches/typescript",
        "TypeScript type acquisition cache",
    ));
    rules.push(rw_literal_comment(
        "HOME/.node_repl_history",
        "node REPL history",
    ));
    // -- Browser automation downloads its own browsers outside the project --
    rules.push(rw_comment(
        "HOME/Library/Caches/ms-playwright",
        "Playwright browser downloads (macOS)",
    ));
    rules.push(rw_comment(
        "HOME/.cache/ms-playwright",
        "Playwright browser downloads",
    ));
    rules.push(rw_comment(
        "HOME/.cache/puppeteer",
        "Puppeteer browser downloads",
    ));
    rules
}

pub(super) fn python_preset() -> Vec<Rule> {
    let mut rules = default_preset();
    // -- Index config: which package index, and the credentials for it --
    rules.push(rw_comment("HOME/.config/pip", "pip XDG config"));
    rules.push(rw_comment("HOME/.pip", "pip legacy config"));
    rules.push(rw_literal_comment(
        "HOME/.netrc",
        "index credentials for pip/uv",
    ));
    // -- Wheel and build caches --
    rules.push(rw_comment("HOME/.cache/pip", "pip wheel cache"));
    rules.push(rw_comment(
        "HOME/Library/Caches/pip",
        "pip wheel cache (macOS)",
    ));
    rules.push(rw_comment("HOME/.cache/uv", "uv cache"));
    rules.push(rw_comment("HOME/Library/Caches/uv", "uv cache (macOS)"));
    rules.push(rw_comment("HOME/.cache/pypoetry", "Poetry cache"));
    rules.push(rw_comment(
        "HOME/Library/Caches/pypoetry",
        "Poetry cache (macOS)",
    ));
    // -- Interpreters, virtualenvs, and the tools installed alongside them --
    rules.push(rw_comment(
        "HOME/.local/share/uv",
        "uv-managed interpreters",
    ));
    rules.push(rw_comment(
        "HOME/.local/share/virtualenvs",
        "Pipenv virtualenvs",
    ));
    rules.push(rw_comment(
        "HOME/.virtualenvs",
        "virtualenvwrapper environments",
    ));
    rules.push(rw_comment("HOME/.local/lib", "pip --user site-packages"));
    rules.push(rw_comment(
        "HOME/.local/bin",
        "console scripts from --user installs",
    ));
    rules.push(rw_comment("HOME/.pyenv", "pyenv interpreters and shims"));
    rules
}

#[cfg(test)]
mod tests {
    use super::super::{known_presets, preset_rules};
    use super::*;

    /// True when some rule's path mentions `needle`.
    fn grants_path(rules: &[Rule], needle: &str) -> bool {
        rules
            .iter()
            .any(|r| r.path_value.as_deref().is_some_and(|v| v.contains(needle)))
    }

    fn rules_for(preset: &str) -> Vec<Rule> {
        preset_rules(preset).unwrap_or_else(|| panic!("preset '{preset}' should resolve"))
    }

    /// The presets with a dedicated toolchain, as opposed to `node` and `none`.
    const TOOLCHAIN_PRESETS: &[&str] = &["rust", "kotlin", "swift"];

    #[test]
    fn rust_preset_has_project_root_rule() {
        assert!(grants_path(&rules_for("rust"), "PROJECT_ROOT"));
    }

    #[test]
    fn kotlin_preset_has_project_root_rule() {
        assert!(grants_path(&rules_for("kotlin"), "PROJECT_ROOT"));
    }

    #[test]
    fn swift_preset_has_project_root_rule() {
        assert!(grants_path(&rules_for("swift"), "PROJECT_ROOT"));
    }

    #[test]
    fn none_preset_includes_claude_base_and_project_root() {
        let rules = rules_for("none");
        assert!(!rules.is_empty());
        assert!(grants_path(&rules, ".claude"));
        assert!(grants_path(&rules, "PROJECT_ROOT"));
    }

    #[test]
    fn all_presets_include_native_claude_install() {
        for name in known_presets() {
            assert!(
                grants_path(&rules_for(name), ".local/share/claude"),
                "preset '{name}' must include native Claude install path"
            );
        }
    }

    #[test]
    fn all_presets_include_spenn_api_client_global_state() {
        for name in known_presets() {
            let rules = rules_for(name);
            assert!(
                grants_path(&rules, ".spenn-client-state-rs"),
                "preset {name} missing spenn-api-client sled state access"
            );
            assert!(
                grants_path(&rules, ".spenn-client.pid"),
                "preset {name} missing spenn-api-client pid file access"
            );
        }
    }

    #[test]
    fn all_presets_include_claude_code_config() {
        for name in TOOLCHAIN_PRESETS {
            assert!(
                grants_path(&rules_for(name), ".claude"),
                "preset '{name}' must include Claude Code config rules"
            );
        }
    }

    #[test]
    fn all_presets_include_git_config() {
        for name in TOOLCHAIN_PRESETS {
            assert!(
                grants_path(&rules_for(name), ".gitconfig"),
                "preset '{name}' must include git config rules"
            );
        }
    }

    #[test]
    fn all_preset_rules_have_preset_source() {
        for name in TOOLCHAIN_PRESETS {
            for rule in &rules_for(name) {
                assert_eq!(
                    rule.source,
                    RuleSource::Preset,
                    "preset '{name}' rule for {:?} should have Preset source",
                    rule.path_value
                );
            }
        }
    }

    #[test]
    fn all_preset_rules_are_allow() {
        for name in TOOLCHAIN_PRESETS {
            for rule in &rules_for(name) {
                assert_eq!(
                    rule.action,
                    RuleAction::Allow,
                    "preset '{name}' rule for {:?} should be Allow",
                    rule.path_value
                );
            }
        }
    }

    /// Cargo runs in the MCP server process, outside the sandbox, so the session
    /// never needs its caches.
    #[test]
    fn rust_preset_omits_direct_cargo_tooling_rules() {
        let rules = rules_for("rust");
        for absent in ["CARGO_HOME", "RUSTUP_HOME", ".config/cargo", "sccache"] {
            assert!(
                !grants_path(&rules, absent),
                "rust preset should not grant {absent}"
            );
        }
    }

    /// Likewise for Gradle: only the Maven local repo is read, for `mavenLocal()`.
    #[test]
    fn kotlin_preset_omits_direct_gradle_tooling_rules() {
        let rules = rules_for("kotlin");
        for absent in [
            "GRADLE_HOME",
            "JAVA_HOME",
            ".sdkman",
            ".jenv",
            "docker.sock",
            "Library/Java",
        ] {
            assert!(
                !grants_path(&rules, absent),
                "kotlin preset should not grant {absent}"
            );
        }
    }

    #[test]
    fn rust_preset_has_developer_tools_rules() {
        let rules = rules_for("rust");
        assert!(grants_path(&rules, "/private/var/select/developer_dir"));
        assert!(grants_path(&rules, "CommandLineTools"));
    }

    #[test]
    fn kotlin_preset_has_developer_tools_rules() {
        let rules = rules_for("kotlin");
        assert!(grants_path(&rules, "/private/var/select/developer_dir"));
        assert!(grants_path(&rules, "CommandLineTools"));
    }

    #[test]
    fn swift_preset_has_swift_toolchain_rules() {
        let rules = rules_for("swift");
        assert!(grants_path(&rules, "Library/Developer"));
        assert!(grants_path(&rules, "org.swift.swiftpm"));
    }

    #[test]
    fn kotlin_preset_has_maven_local_read_access() {
        assert!(rules_for("kotlin").iter().any(|r| {
            r.path_value.as_deref() == Some("MAVEN_HOME") && r.operation == "file-read*"
        }));
    }

    /// `.npmrc` names the registry and carries its auth token; without it a
    /// private-scope install fails in a way that reads as a registry outage.
    #[test]
    fn node_preset_grants_npmrc_and_corepack() {
        let rules = rules_for("node");
        assert!(grants_path(&rules, ".npmrc"));
        assert!(grants_path(&rules, "Library/Caches/node"));
    }

    /// Node is installed through a version manager as often as from Homebrew.
    #[test]
    fn node_preset_covers_the_common_version_managers() {
        let rules = rules_for("node");
        for manager in [".nvm", ".fnm", ".volta", ".asdf", "mise"] {
            assert!(
                grants_path(&rules, manager),
                "node preset should grant {manager}"
            );
        }
    }

    #[test]
    fn python_preset_has_project_root_and_index_caches() {
        let rules = rules_for("python");
        assert!(grants_path(&rules, "PROJECT_ROOT"));
        assert!(grants_path(&rules, ".cache/pip"));
        assert!(grants_path(&rules, ".cache/uv"));
        assert!(grants_path(&rules, ".pyenv"));
    }

    #[test]
    fn node_preset_has_project_root_and_npm_rules() {
        let rules = rules_for("node");
        assert!(
            rules
                .iter()
                .any(|r| r.path_value.as_deref() == Some("PROJECT_ROOT"))
        );
        assert!(
            rules
                .iter()
                .any(|r| r.path_value.as_deref() == Some("HOME/.npm"))
        );
    }
}
