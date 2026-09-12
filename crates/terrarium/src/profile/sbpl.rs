use std::collections::HashMap;

use crate::config::project::{PathType, Rule, RuleAction};

/// Renders a complete SBPL sandbox profile string.
pub fn render(rules: &[Rule], params: &HashMap<String, String>, proxy_enabled: bool) -> String {
    let mut out = String::new();

    out.push_str("(version 1)\n");
    out.push_str("(deny default)\n\n");
    out.push_str("(import \"system.sb\")\n\n");

    // -- System runtime: binaries, libraries, frameworks -----------------------
    out.push_str(";; === System runtime ===\n");
    out.push_str("(allow file-read*\n");
    out.push_str("  (subpath \"/usr\")\n");
    out.push_str("  (subpath \"/bin\")\n");
    out.push_str("  (subpath \"/sbin\")\n");
    out.push_str("  (subpath \"/opt\")\n");
    out.push_str("  (subpath \"/System\")\n");
    out.push_str("  (subpath \"/Library/Apple\")\n");
    out.push_str("  (subpath \"/Library/Frameworks\")\n");
    out.push_str("  (subpath \"/Library/Java\")\n");
    out.push_str("  (subpath \"/private/etc\")\n");
    out.push_str("  (subpath \"/private/var/db/timezone\")\n");
    out.push_str("  (literal \"/private/var/select/sh\")\n");
    out.push_str("  (literal \"/dev/urandom\")\n");
    out.push_str("  (literal \"/dev/random\")\n");
    out.push_str("  (literal \"/dev/null\")\n");
    out.push_str("  (literal \"/dev/zero\"))\n\n");

    // -- Process execution & control ------------------------------------------
    out.push_str(";; === Process control ===\n");
    out.push_str("(allow process-exec)\n");
    out.push_str("(allow process-fork)\n");
    out.push_str("(allow sysctl-read)\n");
    out.push_str("(allow pseudo-tty)\n");
    out.push_str("(allow signal (target self))\n");
    out.push_str("(allow process-info* (target same-sandbox))\n\n");

    // -- Temp directories (compilers, Node, MCP servers) ----------------------
    out.push_str(";; === Temp & cache ===\n");
    out.push_str("(allow file-read* file-write*\n");
    out.push_str("  (subpath \"/tmp\")\n");
    out.push_str("  (subpath \"/private/tmp\")\n");
    out.push_str("  (subpath \"/var/folders\")\n");
    out.push_str("  (subpath \"/private/var/folders\"))\n\n");

    // -- Device nodes for shell I/O & PTYs ------------------------------------
    out.push_str(";; === Devices ===\n");
    out.push_str("(allow file-read* file-write*\n");
    out.push_str("  (subpath \"/dev/fd\")\n");
    out.push_str("  (literal \"/dev/stdout\")\n");
    out.push_str("  (literal \"/dev/stderr\")\n");
    out.push_str("  (literal \"/dev/null\")\n");
    out.push_str("  (literal \"/dev/tty\")\n");
    out.push_str("  (literal \"/dev/ptmx\")\n");
    out.push_str("  (regex #\"^/dev/ttys\"))\n\n");
    out.push_str("(allow file-ioctl\n");
    out.push_str("  (literal \"/dev/tty\")\n");
    out.push_str("  (literal \"/dev/ptmx\")\n");
    out.push_str("  (regex #\"^/dev/ttys\"))\n\n");

    // -- Network (required for API calls, MCP, package downloads) -------------
    out.push_str(";; === Network ===\n");
    if proxy_enabled {
        out.push_str("(allow network* (remote ip \"localhost:*\"))\n");
        out.push_str("(allow network-outbound (remote tcp \"localhost:*\"))\n");
        out.push_str("(allow network-inbound (local tcp \"localhost:*\"))\n");
    } else {
        out.push_str("(allow network*)\n");
    }
    out.push_str("(allow system-socket)\n\n");

    // -- Mach/IPC services (DNS, keychain, logging, trust) --------------------
    out.push_str(";; === Mach services ===\n");
    out.push_str("(allow mach-lookup)\n\n");

    if !rules.is_empty() {
        out.push_str(";; === Configured rules ===\n");
        for rule in rules {
            out.push_str(&render_rule(rule, params));
            out.push('\n');
        }
    }

    out
}

/// Escapes characters that are special inside SBPL double-quoted strings.
/// For literal/subpath: escapes both backslashes and double-quotes.
/// For regex: only escapes double-quotes (backslashes are regex syntax).
fn escape_sbpl_string(s: &str, is_regex: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' if !is_regex => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}

fn render_rule(rule: &Rule, params: &HashMap<String, String>) -> String {
    let action = match rule.action {
        RuleAction::Allow => "allow",
        RuleAction::Deny => "deny",
    };

    if let (Some(path_type), Some(path_value)) = (&rule.path_type, &rule.path_value) {
        let resolved = substitute(path_value, params);
        let is_regex = matches!(path_type, PathType::Regex);
        let escaped = escape_sbpl_string(&resolved, is_regex);
        let path_clause = match path_type {
            PathType::Literal => format!("  (literal \"{escaped}\")"),
            PathType::Subpath => format!("  (subpath \"{escaped}\")"),
            PathType::Regex => format!("  (regex \"{escaped}\")"),
            PathType::Ancestors => format!("  (path-ancestors \"{escaped}\")"),
        };
        let ops: Vec<&str> = rule.operation.split_whitespace().collect();
        let ops_str = ops.join("\n  ");

        if let Some(comment) = &rule.comment {
            format!(";; {comment}\n({action} {ops_str}\n{path_clause})")
        } else {
            format!("({action} {ops_str}\n{path_clause})")
        }
    } else {
        // No path constraint — bare operation allow/deny
        let ops: Vec<&str> = rule.operation.split_whitespace().collect();
        let ops_str = ops.join("\n  ");
        if let Some(comment) = &rule.comment {
            format!(";; {comment}\n({action} {ops_str})")
        } else {
            format!("({action} {ops_str})")
        }
    }
}

pub(crate) fn substitute(value: &str, params: &HashMap<String, String>) -> String {
    if params.is_empty() {
        return value.to_string();
    }
    // Sort keys longest-first so CARGO_HOME is matched before HOME.
    // Single-pass replacement avoids double-substitution when a param value
    // contains another param's key as a substring.
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));

    let mut result = String::with_capacity(value.len());
    let mut i = 0;
    let bytes = value.as_bytes();
    while i < bytes.len() {
        if let Some(key) = keys.iter().find(|k| value[i..].starts_with(k.as_str())) {
            result.push_str(&params[key.as_str()]);
            i += key.len();
        } else {
            result.push(value[i..].chars().next().unwrap());
            i += value[i..].chars().next().unwrap().len_utf8();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::project::{PathType, Rule, RuleAction, RuleSource};

    fn make_params() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            "PROJECT_ROOT".to_string(),
            "/home/user/myproject".to_string(),
        );
        m.insert("CARGO_HOME".to_string(), "/home/user/.cargo".to_string());
        m
    }

    #[test]
    fn render_starts_with_version() {
        let output = render(&[], &HashMap::new(), false);
        assert!(output.starts_with("(version 1)"));
    }

    #[test]
    fn render_contains_deny_default() {
        let output = render(&[], &HashMap::new(), false);
        assert!(output.contains("(deny default)"));
    }

    #[test]
    fn render_substitutes_params() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read* file-write*".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some("PROJECT_ROOT".to_string()),
            comment: None,
            source: RuleSource::Preset,
        };
        let output = render(&[rule], &make_params(), false);
        assert!(output.contains("/home/user/myproject"));
        assert!(!output.contains("PROJECT_ROOT"));
    }

    #[test]
    fn render_literal_path_type() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some("/etc/hosts".to_string()),
            comment: Some("hosts file".to_string()),
            source: RuleSource::Manual,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(literal \"/etc/hosts\")"));
        assert!(output.contains(";; hosts file"));
    }

    #[test]
    fn render_bare_operation_no_path() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "network-outbound".to_string(),
            path_type: None,
            path_value: None,
            comment: None,
            source: RuleSource::Preset,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(allow network-outbound)"));
    }

    #[test]
    fn render_regex_path_type() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Regex),
            path_value: Some(r#"^/usr/lib/.*\.dylib$"#.to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        let output = render(&[rule], &HashMap::new(), false);
        // Backslashes in regex patterns are preserved (regex syntax)
        assert!(output.contains(r#"(regex "^/usr/lib/.*\.dylib$")"#));
    }

    #[test]
    fn render_ancestors_path_type() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-metadata file-test-existence".to_string(),
            path_type: Some(PathType::Ancestors),
            path_value: Some("/Users/me/dev/ref".to_string()),
            comment: Some("resolve symlink target".to_string()),
            source: RuleSource::Preset,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(path-ancestors \"/Users/me/dev/ref\")"));
    }

    #[test]
    fn render_deny_action() {
        let rule = Rule {
            action: RuleAction::Deny,
            operation: "network-outbound".to_string(),
            path_type: Some(PathType::Subpath),
            path_value: Some("/private/var".to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(deny network-outbound"));
        assert!(output.contains("(subpath \"/private/var\")"));
    }

    #[test]
    fn render_path_with_quotes_are_escaped() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some(r#"/tmp/file"name.txt"#.to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        // Quotes MUST be escaped to prevent SBPL injection
        let output = render(&[rule], &HashMap::new(), false);
        assert!(
            output.contains(r#"(literal "/tmp/file\"name.txt")"#),
            "quotes in paths must be backslash-escaped, got: {output}"
        );
    }

    #[test]
    fn render_path_with_backslash_is_escaped() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some(r#"/tmp/back\slash"#.to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(
            output.contains(r#"(literal "/tmp/back\\slash")"#),
            "backslashes in paths must be escaped, got: {output}"
        );
    }

    #[test]
    fn render_path_with_parens() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "file-read-data".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some("/tmp/foo(bar)".to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(literal \"/tmp/foo(bar)\")"));
    }

    #[test]
    fn render_empty_operation() {
        let rule = Rule {
            action: RuleAction::Allow,
            operation: "".to_string(),
            path_type: Some(PathType::Literal),
            path_value: Some("/etc/hosts".to_string()),
            comment: None,
            source: RuleSource::Manual,
        };
        // Should not panic; renders with empty op
        let output = render(&[rule], &HashMap::new(), false);
        assert!(output.contains("(allow"));
        assert!(output.contains("(literal \"/etc/hosts\")"));
    }

    #[test]
    fn substitute_no_matching_params() {
        let mut params = HashMap::new();
        params.insert("FOO".to_string(), "bar".to_string());
        let result = substitute("/some/other/path", &params);
        assert_eq!(result, "/some/other/path");
    }

    #[test]
    fn substitute_overlapping_keys_longest_first() {
        let mut params = HashMap::new();
        params.insert("HOME".to_string(), "/users/test".to_string());
        params.insert("CARGO_HOME".to_string(), "/users/test/.cargo".to_string());
        // CARGO_HOME must be substituted before HOME to avoid corrupting the key
        let result = substitute("CARGO_HOME/.registry", &params);
        assert_eq!(
            result, "/users/test/.cargo/.registry",
            "longest keys must be substituted first to avoid partial matches"
        );
    }

    #[test]
    fn substitute_value_containing_key_not_double_replaced() {
        let mut params = HashMap::new();
        params.insert("HOME".to_string(), "/users/test".to_string());
        // PROJECT_ROOT value contains "HOME" as a substring
        params.insert(
            "PROJECT_ROOT".to_string(),
            "/users/HOME_DIR/proj".to_string(),
        );
        let result = substitute("PROJECT_ROOT/src", &params);
        assert_eq!(
            result, "/users/HOME_DIR/proj/src",
            "param values must not be re-scanned for further substitutions"
        );
    }
}
