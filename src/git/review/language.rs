//! The languages the review reads item boundaries and imports for.
//!
//! Every language here gets three things: a way to name the item a change
//! falls in (so an entry point reads `fn render` rather than `@@ -12,3`), a
//! test for import lines (so an import-only hunk is not an entry point), and
//! a block of review notes the model applies on top of the checkout's own
//! style guide. Anything else is reviewed by file, with no item names.

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

    pub fn of_path(path: &str) -> Option<Self> {
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

    /// Every language a piece of review text mentions a file of, in a fixed
    /// order, so the notes appended to a prompt do not reshuffle between runs.
    pub fn mentioned_in(text: &str) -> Vec<Self> {
        let mut found: Vec<Self> = text
            .split(|c: char| c.is_whitespace() || matches!(c, ':' | '(' | ')' | '`' | '"' | ','))
            .filter(|token| token.contains('.'))
            .filter_map(Self::of_path)
            .collect();
        found.sort();
        found.dedup();
        found
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

    /// Whether the language is written in layers (controllers, services,
    /// repositories) that a file's path usually names, so a placement verdict
    /// about it means something.
    pub fn is_layered(self) -> bool {
        matches!(
            self,
            Self::Kotlin | Self::Java | Self::CSharp | Self::TypeScript | Self::JavaScript
        )
    }

    /// The item declared on this line, as the review names it, if any.
    pub fn item_label(self, line: &str) -> Option<String> {
        match self {
            Self::Rust => rust_item_label(line),
            Self::Kotlin => kotlin_item_label(line),
            Self::Java => java_item_label(line),
            Self::JavaScript | Self::TypeScript => script_item_label(line),
            Self::CSharp => csharp_item_label(line),
            Self::Markdown => markdown_item_label(line),
        }
    }

    /// Whether a changed line only brings a name into scope.
    pub fn is_import_line(self, line: &str) -> bool {
        let line = line
            .strip_prefix("pub ")
            .or_else(|| line.strip_prefix("public "))
            .unwrap_or(line);
        match self {
            Self::Rust => line.starts_with("use ") || line.starts_with("extern crate "),
            Self::Kotlin | Self::Java => {
                line.starts_with("import ") || line.starts_with("package ")
            }
            Self::CSharp => line.starts_with("using ") || line.starts_with("namespace "),
            Self::JavaScript | Self::TypeScript => {
                line.starts_with("import ")
                    || (line.starts_with("export ") && line.contains(" from "))
                    || (line.contains("require(") && line.starts_with("const "))
            }
            Self::Markdown => false,
        }
    }

    /// What a reviewer of this language checks that a language-neutral guide
    /// cannot say. Appended to the built-in guide, never to a checkout's own.
    pub fn review_notes(self) -> &'static str {
        match self {
            Self::Rust => "\
Rust:
- No unwrap/expect/panic on paths reachable in production; propagate with ? and give errors context.
- Prefer borrowing over cloning; a clone that exists only to satisfy the borrow checker deserves a comment or a restructure.
- unsafe needs a comment stating the invariant it relies on.
- Match exhaustively over enums the crate owns; a wildcard arm hides the variant added next.
- New dependencies and features belong in Cargo.toml with the lockfile refreshed.",
            Self::Kotlin => "\
Kotlin:
- Immutable by default: val, read-only collections, data-class copy() over mutation.
- No !! outside tests; model absence with nullable types or sealed results.
- Use sealed interfaces/classes for variants with different data; enums only for plain tags.
- Coroutines: no blocking calls inside suspend functions; scope and cancellation are explicit.
- Constructor injection; no field injection or service locators.",
            Self::Java => "\
Java:
- Prefer records, final fields and immutable collections; expose no mutable internals.
- No null returns for collections or optionals; Optional is a return type, not a field or parameter.
- Checked exceptions are either handled or wrapped with context, never swallowed.
- Streams stay short and side-effect free; a loop is fine when it reads better.
- Constructor injection; no static mutable state.",
            Self::JavaScript => "\
JavaScript:
- const by default, let when reassigned, never var.
- Every promise is awaited or returned; no unhandled rejections, no async callbacks in forEach.
- Strict equality; explicit null/undefined handling rather than truthiness on values that can be 0 or empty.
- No mutation of function arguments or shared module state.
- Errors carry context and are not swallowed by an empty catch.",
            Self::TypeScript => "\
TypeScript:
- No any or unchecked casts (as, !) outside tests; narrow with type guards or discriminated unions.
- Prefer readonly, const and immutable updates over mutation.
- Every promise is awaited or returned; no floating promises.
- Public functions and exported values have explicit types; inference is for locals.
- Errors carry context and are not swallowed by an empty catch.",
            Self::CSharp => "\
C# / .NET:
- Async all the way: no .Result or .Wait(); async methods take a CancellationToken and pass it on.
- Prefer records and init-only or readonly members; expose IReadOnlyList/IReadOnlyDictionary, not List.
- Nullable reference types are honoured; no ! (null-forgiving) outside tests.
- Dispose IDisposable/IAsyncDisposable with using; do not hold them in fields without ownership.
- Constructor injection; controllers stay thin and services hold the rules.",
            Self::Markdown => "\
Markdown:
- Headings form one outline; a change under a heading should still belong there.
- Commands and paths are in code spans and match what the code actually does.",
        }
    }
}

pub(super) fn rust_item_label(line: &str) -> Option<String> {
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("pub "))
        .unwrap_or(line);
    for prefix in [
        "async fn ",
        "fn ",
        "impl ",
        "trait ",
        "struct ",
        "enum ",
        "mod ",
        "const ",
        "static ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = declared_name(rest);
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    None
}

pub(super) fn kotlin_item_label(line: &str) -> Option<String> {
    let line = strip_modifiers(
        line,
        &[
            "private ",
            "internal ",
            "protected ",
            "public ",
            "override ",
            "open ",
            "abstract ",
            "suspend ",
            "inline ",
        ],
    );
    for prefix in [
        "fun ",
        "class ",
        "data class ",
        "sealed class ",
        "sealed interface ",
        "enum class ",
        "value class ",
        "object ",
        "interface ",
        "companion object",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let label = prefix.trim_end();
            if prefix == "companion object" {
                return Some(label.to_string());
            }
            // An extension or generic function names the receiver first:
            // `fun <T> List<T>.second()` is reported as `fun second`.
            let name = if prefix == "fun " {
                rest.split('(')
                    .next()
                    .and_then(|head| head.rsplit(['.', '>']).next())
                    .map(str::trim)
                    .unwrap_or(rest)
            } else {
                declared_name(rest)
            };
            if !name.is_empty() {
                return Some(format!("{label} {name}"));
            }
        }
    }
    None
}

pub(super) fn java_item_label(line: &str) -> Option<String> {
    if line.starts_with('@') || line.starts_with("//") || line.starts_with('*') {
        return None;
    }
    let line = strip_modifiers(
        line,
        &[
            "public ",
            "private ",
            "protected ",
            "static ",
            "final ",
            "abstract ",
            "synchronized ",
            "native ",
            "default ",
            "sealed ",
            "non-sealed ",
            "strictfp ",
        ],
    );
    for prefix in ["class ", "interface ", "enum ", "record ", "@interface "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = declared_name(rest);
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    typed_method_name(line).map(|name| format!("method {name}"))
}

/// JavaScript and TypeScript share a shape: declarations by keyword, arrow
/// functions bound to a name, and class members recognised by `name(...) {`.
pub(super) fn script_item_label(line: &str) -> Option<String> {
    if line.starts_with("//") || line.starts_with('*') || line.starts_with('@') {
        return None;
    }
    let line = strip_modifiers(
        line,
        &[
            "export default ",
            "export ",
            "declare ",
            "abstract ",
            "async ",
            "public ",
            "private ",
            "protected ",
            "static ",
            "readonly ",
            "override ",
        ],
    );
    for prefix in [
        "function* ",
        "function ",
        "class ",
        "interface ",
        "type ",
        "enum ",
        "namespace ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = declared_name(rest.trim_start_matches('*').trim_start());
            if !name.is_empty() {
                return Some(format!(
                    "{} {name}",
                    prefix.trim_end().trim_end_matches('*')
                ));
            }
        }
    }
    for prefix in ["const ", "let ", "var "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = declared_name(rest);
            let value = rest.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
            let is_function = value.contains("=>")
                || value.starts_with("function")
                || value.starts_with("async ");
            return (is_function && !name.is_empty()).then(|| format!("function {name}"));
        }
    }
    if line.starts_with("get ") || line.starts_with("set ") {
        let name = declared_name(&line[4..]);
        return (!name.is_empty()).then(|| format!("{} {name}", &line[..3]));
    }
    class_member_name(line).map(|name| format!("method {name}"))
}

pub(super) fn csharp_item_label(line: &str) -> Option<String> {
    if line.starts_with('[') || line.starts_with("//") {
        return None;
    }
    let line = strip_modifiers(
        line,
        &[
            "public ",
            "private ",
            "protected ",
            "internal ",
            "static ",
            "sealed ",
            "abstract ",
            "virtual ",
            "override ",
            "partial ",
            "async ",
            "readonly ",
            "unsafe ",
            "new ",
        ],
    );
    for prefix in [
        "namespace ",
        "class ",
        "record struct ",
        "record ",
        "struct ",
        "interface ",
        "enum ",
        "delegate ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = declared_name(rest);
            if !name.is_empty() {
                return Some(format!("{} {name}", prefix.trim_end()));
            }
        }
    }
    typed_method_name(line).map(|name| format!("method {name}"))
}

/// Markdown has no items, but its headings are the sections a reviewer thinks
/// in, so a change is reported against the heading it falls under.
pub(super) fn markdown_item_label(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let title = line[hashes..].trim();
    (!title.is_empty()).then(|| format!("section {title}"))
}

fn strip_modifiers<'a>(mut line: &'a str, modifiers: &[&str]) -> &'a str {
    loop {
        match modifiers.iter().find_map(|m| line.strip_prefix(m)) {
            Some(rest) => line = rest,
            None => return line,
        }
    }
}

fn declared_name(rest: &str) -> &str {
    rest.split(|c: char| matches!(c, '(' | '<' | ':' | '{' | '=' | ';' | ')') || c.is_whitespace())
        .next()
        .unwrap_or(rest)
        .trim()
}

/// A Java or C# method has no keyword to key off, so it is recognised by
/// shape: a return type, a name, then a parameter list on the same line.
fn typed_method_name(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let head = line[..open].trim_end();
    if head.contains('=') || head.ends_with(',') || head.is_empty() {
        return None;
    }
    let name = head.rsplit(char::is_whitespace).next()?;
    let (name, generics) = name
        .split_once('<')
        .map_or((name, false), |(n, _)| (n, true));
    if name.is_empty() || (!head.contains(char::is_whitespace) && !generics) {
        return None;
    }
    if is_statement_keyword(head.split(char::is_whitespace).next().unwrap_or("")) {
        return None;
    }
    let valid = name
        .chars()
        .all(|c| c == '_' || c == '.' || c.is_alphanumeric());
    valid.then(|| name.to_string())
}

/// A class member in JavaScript or TypeScript is bare: `name(args) {` or
/// `name<T>(args): R {`, with nothing before the name once modifiers are gone.
fn class_member_name(line: &str) -> Option<String> {
    let open = line.find('(')?;
    let name = line[..open].trim_end();
    let name = name.split_once('<').map_or(name, |(n, _)| n);
    if name.is_empty()
        || is_statement_keyword(name)
        || !name
            .chars()
            .all(|c| c == '_' || c == '$' || c == '#' || c.is_alphanumeric())
    {
        return None;
    }
    let after = line[open..].split_once(')').map(|(_, rest)| rest)?;
    after.contains('{').then(|| name.to_string())
}

fn is_statement_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "return"
            | "throw"
            | "new"
            | "await"
            | "yield"
            | "catch"
            | "try"
            | "using"
            | "lock"
            | "foreach"
            | "typeof"
            | "super"
            | "this"
            | "constructor"
    ) || word.starts_with("this.")
        || word.starts_with("super.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_names_its_language_by_extension() {
        assert_eq!(Language::of_path("src/lib.rs"), Some(Language::Rust));
        assert_eq!(Language::of_path("App.kt"), Some(Language::Kotlin));
        assert_eq!(
            Language::of_path("build.gradle.kts"),
            Some(Language::Kotlin)
        );
        assert_eq!(Language::of_path("a/b/Main.java"), Some(Language::Java));
        assert_eq!(Language::of_path("web/app.jsx"), Some(Language::JavaScript));
        assert_eq!(Language::of_path("web/app.tsx"), Some(Language::TypeScript));
        assert_eq!(Language::of_path("Api/Program.cs"), Some(Language::CSharp));
        assert_eq!(Language::of_path("README.md"), Some(Language::Markdown));
        assert_eq!(Language::of_path("Makefile"), None);
        assert_eq!(Language::of_path("data.json"), None);
    }

    #[test]
    fn the_languages_of_a_review_are_read_from_the_paths_it_mentions() {
        let text = "Files changed:\n- src/main.rs\n- web/src/App.tsx:12 in render\n- `docs/x.md`";
        assert_eq!(
            Language::mentioned_in(text),
            [Language::Rust, Language::TypeScript, Language::Markdown]
        );
        assert!(Language::mentioned_in("nothing here").is_empty());
    }

    #[test]
    fn java_items_are_named_by_keyword_or_method_shape() {
        assert_eq!(
            java_item_label("public final class OrderService {"),
            Some("class OrderService".into())
        );
        assert_eq!(
            java_item_label("public record Money(long cents) {}"),
            Some("record Money".into())
        );
        assert_eq!(
            java_item_label("private static <T> List<T> twice(T value) {"),
            Some("method twice".into())
        );
        assert_eq!(
            java_item_label("public Optional<Order> find(OrderId id) throws IOException {"),
            Some("method find".into())
        );
        assert_eq!(java_item_label("@Override"), None);
        assert_eq!(java_item_label("return service.find(id);"), None);
        assert_eq!(java_item_label("if (order == null) {"), None);
    }

    #[test]
    fn script_items_cover_functions_classes_types_and_members() {
        assert_eq!(
            script_item_label("export async function loadUser(id) {"),
            Some("function loadUser".into())
        );
        assert_eq!(
            script_item_label("export default class Store extends Base {"),
            Some("class Store".into())
        );
        assert_eq!(
            script_item_label("export interface Props {"),
            Some("interface Props".into())
        );
        assert_eq!(
            script_item_label("type Result<T> = Ok<T> | Err;"),
            Some("type Result".into())
        );
        assert_eq!(
            script_item_label("const render = async (state) => {"),
            Some("function render".into())
        );
        assert_eq!(
            script_item_label("export const total = items.length;"),
            None,
            "a plain value is not an entry point"
        );
        assert_eq!(
            script_item_label("private async fetchAll<T>(url: string): Promise<T[]> {"),
            Some("method fetchAll".into())
        );
        assert_eq!(script_item_label("get size() {"), Some("get size".into()));
        assert_eq!(script_item_label("if (ready) {"), None);
        assert_eq!(script_item_label("this.load(id);"), None);
        assert_eq!(script_item_label("await fetch(url);"), None);
    }

    #[test]
    fn kotlin_extension_and_generic_functions_are_named_by_their_own_name() {
        assert_eq!(
            kotlin_item_label("fun <T> List<T>.second(): T = this[1]"),
            Some("fun second".into())
        );
        assert_eq!(
            kotlin_item_label("override suspend fun handle(event: Event) {"),
            Some("fun handle".into())
        );
        assert_eq!(
            kotlin_item_label("sealed interface Outcome"),
            Some("sealed interface Outcome".into())
        );
    }

    #[test]
    fn import_lines_are_recognised_per_language() {
        assert!(Language::Rust.is_import_line("use std::fs;"));
        assert!(Language::Java.is_import_line("import java.util.List;"));
        assert!(Language::Kotlin.is_import_line("public import a.b.C"));
        assert!(Language::CSharp.is_import_line("using System.Text;"));
        assert!(Language::TypeScript.is_import_line("import { x } from './x';"));
        assert!(Language::TypeScript.is_import_line("export { y } from './y';"));
        assert!(Language::JavaScript.is_import_line("const fs = require('fs');"));
        assert!(!Language::TypeScript.is_import_line("export function x() {}"));
        assert!(!Language::Rust.is_import_line("let x = 1;"));
    }

    #[test]
    fn every_language_has_review_notes_that_name_it() {
        for language in Language::ALL {
            let notes = language.review_notes();
            assert!(notes.starts_with(language.name()), "{notes}");
            assert!(notes.lines().count() >= 3, "{notes}");
        }
    }
}
