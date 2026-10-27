//! Telling test code from the code it tests, by path.

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
}
