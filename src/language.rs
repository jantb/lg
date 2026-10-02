//! The programming languages lg recognises, and how a file path or a code
//! fence names one. What each part of lg does with a language — colouring it,
//! reviewing it — lives with that part.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    Rust,
    Kotlin,
    Java,
    JavaScript,
    TypeScript,
    CSharp,
    Markdown,
}

impl Language {
    pub const ALL: [Self; 7] = [
        Self::Rust,
        Self::Kotlin,
        Self::Java,
        Self::JavaScript,
        Self::TypeScript,
        Self::CSharp,
        Self::Markdown,
    ];

    /// The language of the file at `path`, judged by its extension.
    pub fn from_path(path: &str) -> Option<Self> {
        let extension = Path::new(path).extension()?.to_str()?;
        Some(match extension {
            "rs" => Self::Rust,
            "kt" | "kts" => Self::Kotlin,
            "java" => Self::Java,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "cs" | "csx" => Self::CSharp,
            "md" | "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    /// The language a Markdown code fence names after its backticks, such as
    /// the `rust` of a fence opened with three backticks and `rust`.
    pub fn from_fence_tag(tag: &str) -> Option<Self> {
        Some(match tag.trim().to_ascii_lowercase().as_str() {
            "rs" | "rust" => Self::Rust,
            "kt" | "kts" | "kotlin" => Self::Kotlin,
            "java" => Self::Java,
            "js" | "jsx" | "mjs" | "cjs" | "javascript" => Self::JavaScript,
            "ts" | "tsx" | "mts" | "cts" | "typescript" => Self::TypeScript,
            "cs" | "csx" | "csharp" | "c#" => Self::CSharp,
            "md" | "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Kotlin => "Kotlin",
            Self::Java => "Java",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::CSharp => "C#",
            Self::Markdown => "Markdown",
        }
    }
}

/// Whether `path` is a source file lg can open at a line: the files a
/// `path:line` reference or a diff header leads to the source view of.
pub fn is_source_path(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("kt" | "kts" | "java" | "md" | "rs" | "cs" | "csx")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_names_its_language_by_extension() {
        assert_eq!(Language::from_path("src/lib.rs"), Some(Language::Rust));
        assert_eq!(Language::from_path("App.kt"), Some(Language::Kotlin));
        assert_eq!(
            Language::from_path("build.gradle.kts"),
            Some(Language::Kotlin)
        );
        assert_eq!(Language::from_path("a/b/Main.java"), Some(Language::Java));
        assert_eq!(
            Language::from_path("web/app.jsx"),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::from_path("web/app.tsx"),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::from_path("Api/Program.cs"),
            Some(Language::CSharp)
        );
        assert_eq!(Language::from_path("README.md"), Some(Language::Markdown));
        assert_eq!(Language::from_path("Makefile"), None);
        assert_eq!(Language::from_path("data.json"), None);
    }

    #[test]
    fn a_fence_tag_names_its_language_in_any_case() {
        assert_eq!(Language::from_fence_tag("rust"), Some(Language::Rust));
        assert_eq!(Language::from_fence_tag("RS"), Some(Language::Rust));
        assert_eq!(Language::from_fence_tag("Kotlin"), Some(Language::Kotlin));
        assert_eq!(Language::from_fence_tag("c#"), Some(Language::CSharp));
        assert_eq!(Language::from_fence_tag(" csharp "), Some(Language::CSharp));
        assert_eq!(
            Language::from_fence_tag("typescript"),
            Some(Language::TypeScript)
        );
        assert_eq!(Language::from_fence_tag(""), None);
        assert_eq!(Language::from_fence_tag("text"), None);
    }

    #[test]
    fn source_paths_are_the_files_the_source_view_opens() {
        assert!(is_source_path("src/main.rs"));
        assert!(is_source_path("app/Main.java"));
        assert!(is_source_path("docs/guide.md"));
        assert!(!is_source_path("notes.txt"));
        assert!(!is_source_path("Makefile"));
    }
}
