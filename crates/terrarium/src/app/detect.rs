//! Recognizing a project's toolchains from the files in its root.

use std::path::Path;

/// Files that mark a Node project. `package.json` is the canonical one; the
/// lockfiles and workspace manifests catch the roots that hold only tooling
/// config, and `deno.json`/`bun.lockb` the runtimes that skip npm entirely.
const NODE_MARKERS: &[&str] = &[
    "package.json",
    "pnpm-workspace.yaml",
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "deno.json",
    "deno.jsonc",
];

/// Files that mark a Python project, covering both PEP 621 layouts and the
/// older `requirements.txt`/`setup.py` shape.
const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "setup.cfg",
    "uv.lock",
    "Pipfile",
    "environment.yml",
];

const KOTLIN_MARKERS: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "gradlew",
];

fn has_any_marker(project_root: &Path, markers: &[&str]) -> bool {
    markers
        .iter()
        .any(|marker| project_root.join(marker).exists())
}

/// Detects every toolchain present in the project, in preset-priority order.
///
/// A project can legitimately mix languages (e.g. an AWS service with Node,
/// Kotlin and Rust parts), so all matching presets are returned.
pub fn detect_project_presets(project_root: &Path) -> Vec<&'static str> {
    let mut detected = Vec::new();

    if project_root.join("Package.swift").exists() {
        detected.push("swift");
    }

    if has_any_marker(project_root, KOTLIN_MARKERS) {
        detected.push("kotlin");
    }

    if project_root.join("Cargo.toml").exists() {
        detected.push("rust");
    }

    if has_any_marker(project_root, NODE_MARKERS) {
        detected.push("node");
    }

    if has_any_marker(project_root, PYTHON_MARKERS) {
        detected.push("python");
    }

    detected
}

/// Directories never worth descending into when looking for nested toolchains.
const SKIP_SCAN_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "DerivedData",
    "vendor",
    "venv",
];

/// How far below a project root a nested toolchain is still counted as part of
/// it. Three levels reach the `apps/<service>` and `infrastructure/<stack>`
/// layouts a polyglot monorepo uses, without walking whole source trees.
const NESTED_DETECT_DEPTH: usize = 3;

/// Detects every toolchain in `project_root` *and* in the module directories
/// below it, in preset-priority order.
///
/// A monorepo declares its languages in its sub-projects, not at its root: a
/// Rust workspace with a Gradle service under `apps/` and a CDK stack under
/// `infrastructure/` reads as Rust-only from the root alone, which would leave
/// the Gradle build without Maven Central and the CDK stack without npm.
pub fn detect_project_presets_deep(project_root: &Path) -> Vec<&'static str> {
    let mut detected = detect_project_presets(project_root);
    collect_nested_presets(project_root, NESTED_DETECT_DEPTH, &mut detected);
    // Re-sort into preset priority, which the nested walk does not preserve.
    let order = ["swift", "kotlin", "rust", "node", "python"];
    detected.sort_by_key(|name| order.iter().position(|o| o == name).unwrap_or(usize::MAX));
    detected
}

fn collect_nested_presets(dir: &Path, depth: usize, detected: &mut Vec<&'static str>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_SCAN_DIRS.contains(&name.as_ref()) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        for preset in detect_project_presets(&path) {
            if !detected.contains(&preset) {
                detected.push(preset);
            }
        }
        collect_nested_presets(&path, depth - 1, detected);
    }
}

/// Returns the deep-detected preset spec, or None when nothing is recognized.
pub fn detect_project_preset_spec_deep(project_root: &Path) -> Option<String> {
    let detected = detect_project_presets_deep(project_root);
    if detected.is_empty() {
        None
    } else {
        Some(detected.join(","))
    }
}

/// Returns the detected preset spec (comma-separated when several toolchains are present).
pub fn detect_project_preset_spec(project_root: &Path) -> Option<String> {
    let detected = detect_project_presets(project_root);
    if detected.is_empty() {
        None
    } else {
        Some(detected.join(","))
    }
}

/// Returns the highest-priority detected preset, or None when nothing is recognized.
pub fn detect_project_preset(project_root: &Path) -> Option<&'static str> {
    detect_project_presets(project_root).first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::temp_dir;
    use std::fs;

    #[test]
    fn detect_project_preset_from_project_markers() {
        let root = temp_dir();
        let rust = root.join("rust");
        let kotlin = root.join("kotlin");
        let swift = root.join("swift");
        let unknown = root.join("unknown");
        fs::create_dir_all(&rust).unwrap();
        fs::create_dir_all(&kotlin).unwrap();
        fs::create_dir_all(&swift).unwrap();
        fs::create_dir_all(&unknown).unwrap();
        fs::write(rust.join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(kotlin.join("build.gradle.kts"), "plugins {}\n").unwrap();
        fs::write(swift.join("Package.swift"), "// swift\n").unwrap();

        assert_eq!(detect_project_preset(&rust), Some("rust"));
        assert_eq!(detect_project_preset(&kotlin), Some("kotlin"));
        assert_eq!(detect_project_preset(&swift), Some("swift"));
        assert_eq!(detect_project_preset(&unknown), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detect_project_presets_returns_every_toolchain() {
        let root = temp_dir();
        let mixed = root.join("mixed");
        fs::create_dir_all(&mixed).unwrap();
        fs::write(mixed.join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(mixed.join("build.gradle.kts"), "plugins {}\n").unwrap();
        fs::write(mixed.join("package.json"), "{}\n").unwrap();

        assert_eq!(
            detect_project_presets(&mixed),
            vec!["kotlin", "rust", "node"]
        );
        assert_eq!(
            detect_project_preset_spec(&mixed),
            Some("kotlin,rust,node".to_string())
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// The alvminnelig shape: a Rust workspace whose Gradle service and CDK
    /// stack live in module directories below the root.
    #[test]
    fn detect_deep_finds_toolchains_in_module_directories() {
        let root = temp_dir();
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("apps/ai-worker")).unwrap();
        fs::create_dir_all(repo.join("infrastructure/cdk")).unwrap();
        fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
        fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(repo.join("apps/ai-worker/build.gradle.kts"), "plugins {}\n").unwrap();
        fs::write(repo.join("infrastructure/cdk/package.json"), "{}\n").unwrap();
        fs::write(repo.join("node_modules/pkg/Package.swift"), "// swift\n").unwrap();

        assert_eq!(
            detect_project_preset_spec_deep(&repo),
            Some("kotlin,rust,node".to_string()),
            "nested toolchains must be found, and node_modules must not be scanned"
        );
        // The shallow detector still reports only what the root itself declares.
        assert_eq!(detect_project_preset_spec(&repo), Some("rust".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detect_deep_on_an_empty_tree_finds_nothing() {
        let root = temp_dir();
        let empty = root.join("empty/nested");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(detect_project_preset_spec_deep(&root.join("empty")), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn detect_project_preset_from_python_markers() {
        let root = temp_dir();
        for marker in ["pyproject.toml", "requirements.txt", "setup.py", "uv.lock"] {
            let dir = root.join(marker);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(marker), "").unwrap();
            assert_eq!(
                detect_project_preset(&dir),
                Some("python"),
                "'{marker}' should mark a Python project"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// A Node root need not carry `package.json` — a pnpm workspace root or a
    /// Deno project is still Node as far as the sandbox is concerned.
    #[test]
    fn detect_project_preset_from_node_markers_beyond_package_json() {
        let root = temp_dir();
        for marker in ["pnpm-workspace.yaml", "yarn.lock", "deno.json", "bun.lock"] {
            let dir = root.join(marker);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(marker), "").unwrap();
            assert_eq!(
                detect_project_preset(&dir),
                Some("node"),
                "'{marker}' should mark a Node project"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// The Node dashboard and the Python scanner in one repo must both be seen.
    #[test]
    fn detect_project_presets_includes_python_alongside_node() {
        let root = temp_dir();
        let mixed = root.join("mixed");
        fs::create_dir_all(&mixed).unwrap();
        fs::write(mixed.join("package.json"), "{}\n").unwrap();
        fs::write(mixed.join("pyproject.toml"), "[project]\n").unwrap();

        assert_eq!(detect_project_presets(&mixed), vec!["node", "python"]);
        fs::remove_dir_all(&root).unwrap();
    }
}
