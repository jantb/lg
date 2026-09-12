//! Which part of the tree a changed path belongs to.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ReviewEntryCategory {
    Production,
    Tests,
    Migrations,
    Docs,
    Other,
}

impl ReviewEntryCategory {
    pub(super) const ALL: [Self; 5] = [
        Self::Production,
        Self::Tests,
        Self::Migrations,
        Self::Docs,
        Self::Other,
    ];

    pub(super) fn for_path(path: &str) -> Self {
        if is_test_path(path) {
            Self::Tests
        } else if is_migration_path(path) {
            Self::Migrations
        } else if is_doc_path(path) {
            Self::Docs
        } else if is_production_path(path) {
            Self::Production
        } else {
            Self::Other
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Tests => "tests",
            Self::Migrations => "migrations",
            Self::Docs => "docs",
            Self::Other => "other",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Production => "Production",
            Self::Tests => "Tests",
            Self::Migrations => "Migrations",
            Self::Docs => "Docs",
            Self::Other => "Other",
        }
    }
}

/// Whether a path is test code, by the conventions of the ecosystems lg
/// reviews: Rust and Go `tests/` trees, Maven and Gradle `src/test`, Jest and
/// Vitest `__tests__` and `.test.`/`.spec.` files, JUnit and xUnit `*Test`,
/// `*Tests` and `*Spec` classes, and .NET `Something.Tests` projects.
pub fn is_test_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let Some((file, dirs)) = segments.split_last() else {
        return false;
    };
    if dirs.iter().any(|dir| {
        matches!(
            *dir,
            "tests"
                | "test"
                | "__tests__"
                | "spec"
                | "Tests"
                | "Test"
                | "UnitTests"
                | "IntegrationTests"
        ) || dir.ends_with(".Tests")
            || dir.ends_with(".Test")
            || dir.ends_with(".UnitTests")
            || dir.ends_with(".IntegrationTests")
    }) {
        return true;
    }
    let stem = file.split_once('.').map_or(*file, |(stem, _)| stem);
    let infix = file.get(stem.len()..).unwrap_or("");
    infix.contains(".test.")
        || infix.contains(".spec.")
        || infix.starts_with(".test.")
        || infix.starts_with(".spec.")
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || stem.ends_with("Spec")
        || stem.starts_with("test_")
        || stem.ends_with("_test")
}

fn is_migration_path(path: &str) -> bool {
    path.contains("/db/migration/")
        || path.starts_with("db/migration/")
        || path.contains("/migrations/")
        || path.starts_with("migrations/")
        || path.contains("/Migrations/")
        || path.ends_with(".sql")
}

fn is_doc_path(path: &str) -> bool {
    path.starts_with("docs/")
        || path.starts_with(".agent/")
        || path.ends_with(".md")
        || path.ends_with(".adoc")
        || path.ends_with(".rst")
        || path.ends_with(".txt")
}

fn is_production_path(path: &str) -> bool {
    let in_source_tree = path.starts_with("src/")
        || path.starts_with("app/")
        || path.starts_with("lib/")
        || path.contains("/src/")
        || path.contains("/app/")
        || path.contains("/lib/");
    in_source_tree && !is_test_path(path) && !is_migration_path(path) && !is_doc_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_follow_each_ecosystems_convention() {
        for path in [
            "tests/e2e.rs",
            "crates/x/tests/it.rs",
            "src/test/kotlin/AppTest.kt",
            "src/main/kotlin/OrderServiceTest.kt",
            "src/test/java/com/x/OrderServiceTest.java",
            "web/src/__tests__/App.tsx",
            "web/src/App.test.tsx",
            "web/src/store.spec.ts",
            "Api.Tests/OrdersControllerTests.cs",
            "tests/Api.IntegrationTests/Smoke.cs",
            "pkg/thing_test.go",
            "app/test_models.py",
        ] {
            assert!(is_test_path(path), "{path} is a test");
        }
        for path in [
            "src/main.rs",
            "src/main/kotlin/OrderService.kt",
            "web/src/App.tsx",
            "Api/Controllers/OrdersController.cs",
            "src/testing_helpers.rs",
            "src/contest.rs",
        ] {
            assert!(!is_test_path(path), "{path} is production");
        }
    }

    #[test]
    fn categories_read_each_ecosystems_layout() {
        use ReviewEntryCategory as C;
        assert_eq!(C::for_path("src/main.rs"), C::Production);
        assert_eq!(C::for_path("web/src/App.tsx"), C::Production);
        assert_eq!(C::for_path("Api/Controllers/OrdersController.cs"), C::Other);
        assert_eq!(C::for_path("web/src/App.test.tsx"), C::Tests);
        assert_eq!(C::for_path("db/migration/V1__init.sql"), C::Migrations);
        assert_eq!(
            C::for_path("Api/Migrations/20240101_Init.cs"),
            C::Migrations
        );
        assert_eq!(C::for_path("web/migrations/0001_users.ts"), C::Migrations);
        assert_eq!(C::for_path("docs/guide.md"), C::Docs);
        assert_eq!(C::for_path("Cargo.toml"), C::Other);
    }
}
