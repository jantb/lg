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

pub(super) fn is_test_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("test/")
        || path.contains("/test/")
        || path.starts_with("src/test/")
        || path.contains("/src/test/")
}

fn is_migration_path(path: &str) -> bool {
    path.contains("/db/migration/") || path.starts_with("db/migration/") || path.ends_with(".sql")
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
    (path.starts_with("src/") || path.starts_with("app/") || path.starts_with("lib/"))
        && !is_test_path(path)
        && !is_migration_path(path)
        && !is_doc_path(path)
}
