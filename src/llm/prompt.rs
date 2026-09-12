//! The prompt each task sends, and the repo style guide they share.

use std::path::Path;

use crate::git::Language;
use crate::settings::RepoSettings;

use super::diff::{diff_excerpt, summarize_diff};

pub fn build_commit_prompt(diff: &str, settings: &RepoSettings) -> String {
    format!(
        "{}{}\n\nDiff excerpt:\n{}\n",
        crate::settings::commit_prompt_prefix(settings),
        summarize_diff(diff),
        diff_excerpt(diff)
    )
}

pub fn build_review_assist_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "Assess this selected subtree from a full diff against main as a patch review.\n\
         Use the branch overview first: commit subjects/bodies and changed file lists are evidence\n\
         of intended behavior and test coverage. Be factual and specific.\n\
         Before calling something a regression, compare it with the stated patch intent and the\n\
         changed tests. Treat an intentional behavior change as a bug only when it contradicts an\n\
         invariant, API contract, data contract, or caller expectation shown in the context.\n\
         When tests are changed, say what they cover and name any precise uncovered case; do not\n\
         claim tests are missing merely because the selected production subtree is in focus.\n\
         If the evidence is incomplete, say what is missing instead of inventing consumers or behavior.\n\
         Focus on behavior, call flow, tests, regression risks, and maintainability concerns.\n\
         Say whether the patch appears minimal; flag unnecessary scope, simpler alternatives,\n\
         or refactors that would reduce complexity without changing intended behavior.\n\
         Review the change against the established repo style below and call out concrete violations.\n\
         Output 6-12 substantive bullets or short sections. Avoid padding. Do not invent files\n\
         or behavior not shown. Do not use code fences.\n\n\
         {}\n\n\
         {}\n\
         Selected review subtree:\n{context}",
        review_style(settings, context),
        crate::settings::language_instruction(settings)
    )
}

/// What a Claude Code session is asked when it reviews the branch. Unlike the
/// local model it has the checkout in front of it, so the assisted review is
/// its starting point rather than its whole evidence, and it is told where to
/// write so lg can read the findings back under the review tree.
pub fn build_review_agent_prompt(
    context: &str,
    base_ref: &str,
    findings_path: &Path,
    settings: &RepoSettings,
) -> String {
    format!(
        "Review this branch against {base_ref} as a senior engineer would before approving a pull request.\n\
         Start from the assisted review below, then read the actual code: run `git diff {base_ref}...HEAD`\n\
         and open the changed files and their callers and tests. Do not change any file in the checkout.\n\n\
         Look for: behavior that contradicts the stated intent or an existing invariant, error paths that\n\
         are swallowed or unreachable, concurrency and lifecycle problems, missing or misleading tests,\n\
         compatibility risks for callers and stored data, and concrete violations of the repo style below.\n\
         Prefer a few findings you have verified in the code over many you suspect. For each finding give\n\
         the file and line, what is wrong, why it matters, and what to do instead.\n\n\
         When you are done, write your findings as Markdown to this file and nothing else:\n\
         {findings}\n\
         Use this structure:\n\
         ## Verdict\n\
         - One line: approve, approve with nits, or request changes, and why.\n\
         ## Findings\n\
         - One bullet per finding, most severe first, starting with `path:line`.\n\
         ## Tests\n\
         - What the changed tests cover and the precise cases they miss, or that none changed.\n\
         ## Questions\n\
         - Only what you could not settle from the code; omit the section if there is nothing.\n\n\
         Write the file even when you find nothing wrong, saying so. Then stop.\n\n\
         {style}\n\n\
         {language}\
         Assisted review:\n{context}",
        findings = findings_path.display(),
        style = review_style(settings, context),
        language = crate::settings::language_instruction(settings),
    )
}

/// The style guide a review prompt carries: the checkout's own, verbatim, or
/// the built-in one followed by notes for every language the reviewed text
/// names a file of. A checkout that wrote its own guide said what it wants
/// measured, so nothing is added to that.
pub fn review_style(settings: &RepoSettings, reviewed: &str) -> String {
    let guide = settings.review_style.trim_end();
    if settings.review_style_is_custom() {
        return guide.to_string();
    }
    let mut out = guide.to_string();
    for language in Language::mentioned_in(reviewed) {
        out.push_str("\n\n");
        out.push_str(language.review_notes());
    }
    out
}

pub fn build_review_chat_system_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "You are a senior code reviewer helping inspect a full branch review against main.\n\
         Use only the supplied review context and the conversation. Treat commit subjects/bodies,\n\
         changed file lists, and changed tests as evidence of patch intent and coverage. Be concrete\n\
         about weaknesses, missed tests, risky flows, compatibility concerns, and follow-up checks.\n\
         Before calling an intentional behavior change a regression, point to a shown invariant,\n\
         contract, caller, or test expectation it violates. When useful, cite file paths, function\n\
         names, and line numbers from the context. If the context is insufficient, say what is\n\
         missing instead of guessing. Review answers against the\n\
         established repo style below and call out concrete violations.\n\n\
         {}\n\n\
         {}\n\
         Review context:\n{context}",
        review_style(settings, context),
        crate::settings::language_instruction(settings)
    )
}

pub fn build_review_pr_text_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "Write a copy-ready pull request description for this branch review against main.\n\
         Use only the supplied review context. Treat commit subjects/bodies, changed files,\n\
         entry points, diffstat, and changed tests as evidence of the patch intent.\n\
         Do not invent tickets, reviewers, deployment steps, commands, tests, behavior, or files.\n\
         If testing evidence is not shown, say that explicitly instead of making up a command.\n\
         Mention user-visible behavior and compatibility risks when the context supports them.\n\
         Keep the writing concise and useful for a reviewer who has not read the diff yet.\n\n\
         Output Markdown only, with this exact structure:\n\
         ## Summary\n\
         - 2-4 bullets describing what changed and why.\n\n\
         ## Testing\n\
         - Bullets naming changed tests or saying what testing is not shown.\n\n\
         ## Review Notes\n\
         - 2-5 bullets with the highest-risk review areas, invariants, compatibility points,\n\
           or operational checks. Omit generic advice.\n\n\
         ## Follow-up\n\
         - Include only if the context shows a real follow-up or uncertainty; otherwise omit.\n\n\
         Do not include code fences, preamble, sign-off, or placeholder text.\n\n\
         {}\n\
         Review context:\n{context}",
        crate::settings::language_instruction(settings)
    )
}

pub fn build_review_style_flag_prompt(
    path: &str,
    context: &str,
    settings: &RepoSettings,
) -> String {
    let file_role = review_style_file_role(path);
    format!(
        "Review this single changed source file for concrete violations of the established repo style.\n\
         Apply only the style rules that are relevant to this file's language, framework, and role.\n\
         Return exactly three lines. Keep the three line keys and the severity words in English:\n\
         severity: OK|WARN|FAIL\n\
         line: <new-file line number, or unknown>\n\
         reason: <one concise reason, or \"No style issue found.\">\n\n\
         Use OK for files that look consistent or where there is insufficient evidence.\n\
         Use WARN for likely style issues that deserve manual attention, including vague or misleading names.\n\
         Use FAIL for clear violations of a rule stated in the style guide below, such as business rules\n\
         in a controller or handler, swallowed errors, shared mutable state, or edits to generated code.\n\n\
         Treat the File role below as authoritative. A service-layer or persistence-layer file may call\n\
         repositories and hold business rules by its placement; do not flag that. Return OK for that\n\
         concern unless another concrete style rule is violated.\n\
         A role of unclassified means lg could not read a layer from the path: never flag layer\n\
         placement for such a file, and apply only rules that are about the code itself.\n\n\
         For naming issues, include a concrete rename suggestion in the reason.\n\n\
         {}\n\n\
         {}\n\
         File: {path}\n\
         File role: {file_role}\n\
         Review context:\n{context}",
        review_style(settings, &format!("{path}\n{context}")),
        crate::settings::language_instruction(settings)
    )
}

/// Where a file sits in a layered codebase, as far as its path gives that away.
///
/// Only languages that are written in layers get one, and only when the path
/// names the layer. Anything else is unclassified: reporting a Rust or Go file
/// as being in the wrong layer told the model that business logic in it was a
/// violation by placement, and that is a verdict about a convention the
/// checkout does not use.
pub fn review_style_file_role(path: &str) -> &'static str {
    if !Language::of_path(path).is_some_and(Language::is_layered) {
        return "unclassified";
    }
    let lower = path.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&[
        "controller",
        "endpoint",
        "handler",
        "resource",
        "router",
        "routes",
    ]) {
        "controller"
    } else if has(&["service", "usecase", "use_case", "flow", "domain"]) {
        "service-layer"
    } else if has(&["repository", "repositories", "dao", "persistence", "store"]) {
        "persistence-layer"
    } else {
        "unclassified"
    }
}

/// What the local model is asked to do about one conflict: write the merged
/// lines for that region and nothing else.
///
/// The hunk goes in as labelled sides rather than as the markers git wrote,
/// because the answer has to come back without markers in it and showing the
/// model a set is the surest way to get one echoed. `GIVE_UP_PHRASE` is offered
/// on purpose: a model that says it cannot do this is worth far more than one
/// that guesses, because saying so is what hands the conflict to claude.
pub fn build_conflict_hunk_prompt(
    path: &str,
    hunk: &crate::git::ConflictHunk,
    sides: &crate::git::ConflictSides,
    before: &str,
    after: &str,
) -> String {
    let mut prompt = format!(
        "Resolve one git merge conflict in {path}.\n\n\
         Output only the merged lines that replace the conflicted region.\n\
         Rules:\n\
         - No explanation, no commentary, no code fences, no conflict markers.\n\
         - Keep the file's language, indentation, and surrounding style.\n\
         - When the two sides change different things, keep both.\n\
         - When they change the same thing, keep the version that subsumes the other; \
         do not merge them into something neither side wrote.\n\
         - Do not invent code, imports, or text that appears on neither side.\n\
         - Do not touch anything outside the conflicted region.\n\
         - When both sides replaced the same single value — a version, an image tag, \
         a hash, a timestamp, a generated number — keep the value from the side whose \
         commit is newer. The commits are listed below.\n\
         - If the two sides make incompatible decisions, or resolving this needs \
         knowledge that is not shown, reply with exactly: {GIVE_UP_PHRASE}\n"
    );
    if sides.ours.is_some() || sides.theirs.is_some() {
        prompt.push_str("\nWhere each side comes from (last commit touching this file):\n");
        if let Some(commit) = &sides.ours {
            prompt.push_str(&format!("- our side: {}\n", commit.describe()));
        }
        if let Some(commit) = &sides.theirs {
            prompt.push_str(&format!("- their side: {}\n", commit.describe()));
        }
    }
    if !before.trim().is_empty() {
        prompt.push_str(&format!("\nLines before the conflict:\n{before}"));
        if !before.ends_with('\n') {
            prompt.push('\n');
        }
    }
    prompt.push_str(&format!(
        "\nOur side ({}):\n{}",
        label_or(&hunk.ours_label, "ours"),
        hunk.ours
    ));
    if let Some(base) = &hunk.base {
        prompt.push_str(&format!("\nCommon ancestor:\n{base}"));
    }
    prompt.push_str(&format!(
        "\nTheir side ({}):\n{}",
        label_or(&hunk.theirs_label, "theirs"),
        hunk.theirs
    ));
    if !after.trim().is_empty() {
        prompt.push_str(&format!("\nLines after the conflict:\n{after}"));
        if !after.ends_with('\n') {
            prompt.push('\n');
        }
    }
    prompt.push_str("\nMerged lines:\n");
    prompt
}

/// What the model says when it will not answer, and what the resolver reads as
/// a request to hand the conflict on.
pub const GIVE_UP_PHRASE: &str = "CANNOT RESOLVE";

fn label_or<'a>(label: &'a str, fallback: &'a str) -> &'a str {
    let label = label.trim();
    if label.is_empty() { fallback } else { label }
}

pub fn build_conventions_prompt(history: &str) -> String {
    let history: String = history.chars().take(8_000).collect();
    format!(
        "Read these recent commit messages from one repository and report the \
         conventions most of them follow.\n\n\
         Judge by what the majority of the messages do, not by one outlier.\n\
         For the shape, describe the format only \u{2014} subject prefix or tag convention \
         (for example Conventional Commits or a ticket key) or the absence of one, \
         capitalisation and trailing punctuation of the subject, grammatical mood \
         (imperative or past tense), and whether a body appears and whether it is \
         bullets or prose. Do not describe the topics the commits are about.\n\n\
         Answer with exactly four lines and nothing else:\n\
         language: <the English name of the natural language the prose is written in>\n\
         shape: <one sentence, at most 25 words, describing the format most messages use>\n\
         shape: <the same for the next most common variant, or a stricter reading of the first>\n\
         shape: <one more plausible reading of the format>\n\n\
         Order the shape lines from most to least representative. Make them differ from \
         each other; do not repeat one wording three times.\n\n\
         Commit messages:\n\n{history}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_assist_prompt_includes_repo_style() {
        let prompt = build_review_assist_prompt("src/main/kotlin/App.kt", &RepoSettings::default());

        assert!(prompt.contains("Assess this selected subtree"));
        assert!(prompt.contains("commit subjects/bodies"));
        assert!(prompt.contains("stated patch intent"));
        assert!(prompt.contains("changed tests"));
        assert!(prompt.contains("claim tests are missing"));
        assert!(prompt.contains("instead of inventing consumers or behavior"));
        assert!(prompt.contains("6-12 substantive bullets"));
        assert!(prompt.contains("whether the patch appears minimal"));
        assert!(prompt.contains("simpler alternatives"));
        assert!(prompt.contains("refactors that would reduce complexity"));
        assert!(prompt.contains("Established repo style:"));
        assert!(prompt.contains("Layers stay honest"));
        assert!(prompt.contains("Selected review subtree:\nsrc/main/kotlin/App.kt"));
    }

    /// The built-in guide is about code in general; what a Kotlin reviewer
    /// checks that a TypeScript reviewer does not is appended per language,
    /// and only for the languages the reviewed text names files of.
    #[test]
    fn the_built_in_guide_gains_notes_for_each_language_under_review() {
        let prompt = build_review_assist_prompt(
            "Files changed:\n- src/main/kotlin/App.kt\n- web/src/App.tsx",
            &RepoSettings::default(),
        );

        assert!(prompt.contains("\nKotlin:\n"), "{prompt}");
        assert!(prompt.contains("\nTypeScript:\n"), "{prompt}");
        assert!(!prompt.contains("\nRust:\n"), "{prompt}");
        assert!(!prompt.contains("\nC# / .NET:\n"), "{prompt}");
    }

    #[test]
    fn review_chat_system_prompt_includes_repo_style() {
        let prompt =
            build_review_chat_system_prompt("full review context", &RepoSettings::default());

        assert!(prompt.contains("commit subjects/bodies"));
        assert!(prompt.contains("patch intent"));
        assert!(prompt.contains("intentional behavior change"));
        assert!(prompt.contains("Established repo style:"));
        assert!(prompt.contains("Review context:\nfull review context"));
    }

    #[test]
    fn review_pr_text_prompt_is_copy_ready_and_grounded() {
        let prompt = build_review_pr_text_prompt("full review context", &RepoSettings::default());

        assert!(prompt.contains("copy-ready pull request description"));
        assert!(prompt.contains("commit subjects/bodies"));
        assert!(prompt.contains("changed tests"));
        assert!(prompt.contains("Do not invent"));
        assert!(prompt.contains("## Summary"));
        assert!(prompt.contains("## Testing"));
        assert!(prompt.contains("## Review Notes"));
        assert!(prompt.contains("Review context:\nfull review context"));
    }

    #[test]
    fn review_style_flag_prompt_is_single_file() {
        let prompt = build_review_style_flag_prompt(
            "src/main/kotlin/App.kt",
            "updates controller logic",
            &RepoSettings::default(),
        );

        assert!(prompt.contains("single changed source file"));
        assert!(prompt.contains("severity: OK|WARN|FAIL"));
        assert!(prompt.contains("line: <new-file line number, or unknown>"));
        assert!(prompt.contains("relevant to this file's language"));
        assert!(prompt.contains("concrete rename suggestion"));
        assert!(prompt.contains("never flag layer\nplacement"));
        assert!(prompt.contains("File: src/main/kotlin/App.kt"));
        assert!(prompt.contains("File role: unclassified"));
        assert!(
            prompt.contains("\nKotlin:\n"),
            "notes for the file's language"
        );
        assert!(prompt.contains("updates controller logic"));
    }

    /// The style guide travels with the checkout, so a repo that wrote its own
    /// is measured against that and never against the built-in one.
    #[test]
    fn the_review_prompts_use_this_checkouts_own_style_guide() {
        let settings = RepoSettings {
            review_style: "- Rust 2024, no unwrap outside tests.".to_string(),
            ..RepoSettings::default()
        };

        for prompt in [
            build_review_assist_prompt("ctx", &settings),
            build_review_chat_system_prompt("ctx", &settings),
            build_review_style_flag_prompt("src/lib.rs", "ctx", &settings),
        ] {
            assert!(prompt.contains("no unwrap outside tests"), "{prompt}");
            assert!(
                !prompt.contains("Layers stay honest"),
                "the built-in guide leaked into a checkout that replaced it"
            );
            assert!(
                !prompt.contains("\nRust:\n"),
                "language notes are part of the built-in guide, not of the checkout's"
            );
        }
    }

    /// A layer is read from the path, and only for languages written in
    /// layers. Telling the model that a Rust file is in the wrong layer reads
    /// as a verdict about where its logic lives, under a convention the
    /// checkout does not use.
    #[test]
    fn a_file_role_is_read_from_the_path_of_a_layered_language() {
        assert_eq!(review_style_file_role("src/app/actions.rs"), "unclassified");
        assert_eq!(review_style_file_role("cmd/server/main.go"), "unclassified");
        assert_eq!(
            review_style_file_role("src/main/kotlin/App.kt"),
            "unclassified"
        );
        assert_eq!(
            review_style_file_role("src/main/kotlin/BalanceService.kt"),
            "service-layer"
        );
        assert_eq!(
            review_style_file_role("src/main/java/OrderController.java"),
            "controller"
        );
        assert_eq!(
            review_style_file_role("Api/Controllers/OrdersController.cs"),
            "controller"
        );
        assert_eq!(
            review_style_file_role("src/orders/orders.repository.ts"),
            "persistence-layer"
        );
        assert_eq!(review_style_file_role("src/routes/orders.js"), "controller");
    }

    #[test]
    fn review_style_flag_prompt_classifies_service_file_names() {
        let prompt = build_review_style_flag_prompt(
            "src/main/kotlin/CompletePendingTransactionService.kt",
            "pendingTransactionsRepository.fetchTransaction(...)",
            &RepoSettings::default(),
        );

        assert!(prompt.contains("File role: service-layer"));
        assert!(prompt.contains("Treat the File role below as authoritative"));
        assert!(prompt.contains("hold business rules by its placement"));
    }

    fn no_sides() -> crate::git::ConflictSides {
        crate::git::ConflictSides {
            ours: None,
            theirs: None,
        }
    }

    /// Two CI commits that each bumped the same image tag is the commonest
    /// conflict there is, and the text alone cannot say which tag to keep. The
    /// commits can, so they travel with the hunk.
    #[test]
    fn a_conflict_prompt_says_which_commit_each_side_came_from() {
        let hunk = crate::git::ConflictHunk {
            ours_label: "HEAD".to_string(),
            ours: "tag: sha-0e337ed\n".to_string(),
            base: None,
            theirs: "tag: sha-1685629\n".to_string(),
            theirs_label: "origin/main".to_string(),
        };
        let sides = crate::git::ConflictSides {
            ours: Some(crate::git::ConflictSideCommit {
                hash: "0ac40f2".to_string(),
                author: "github-actions[bot]".to_string(),
                date: "2026-09-04T15:38:20Z".to_string(),
                subject: "chore: deploy sha-0e337ed".to_string(),
            }),
            theirs: Some(crate::git::ConflictSideCommit {
                hash: "9a64534".to_string(),
                author: "github-actions[bot]".to_string(),
                date: "2026-09-05T03:15:00Z".to_string(),
                subject: "chore: deploy sha-1685629".to_string(),
            }),
        };

        let prompt = build_conflict_hunk_prompt(".halvnais/app.yaml", &hunk, &sides, "", "");

        assert!(prompt.contains("our side: 0ac40f2"));
        assert!(prompt.contains("2026-09-04T15:38:20Z"));
        assert!(prompt.contains("their side: 9a64534"));
        assert!(prompt.contains("chore: deploy sha-1685629"));
        assert!(prompt.contains("whose commit is newer"));
    }

    #[test]
    fn a_conflict_prompt_shows_both_sides_without_showing_a_marker() {
        let hunk = crate::git::ConflictHunk {
            ours_label: "HEAD".to_string(),
            ours: "let timeout = 30;\n".to_string(),
            base: None,
            theirs: "let retries = 3;\n".to_string(),
            theirs_label: "origin/main".to_string(),
        };

        let prompt =
            build_conflict_hunk_prompt("src/app.rs", &hunk, &no_sides(), "fn main() {\n", "}\n");

        assert!(prompt.contains("src/app.rs"));
        assert!(prompt.contains("Our side (HEAD)"));
        assert!(prompt.contains("let timeout = 30;"));
        assert!(prompt.contains("Their side (origin/main)"));
        assert!(prompt.contains("let retries = 3;"));
        assert!(prompt.contains("fn main() {"));
        assert!(prompt.contains(GIVE_UP_PHRASE));
        assert!(
            !crate::git::holds_conflict_marker(&prompt),
            "a marker in the prompt is a marker the model will echo back"
        );
    }

    #[test]
    fn a_diff3_conflict_prompt_carries_the_common_ancestor() {
        let hunk = crate::git::ConflictHunk {
            ours_label: String::new(),
            ours: "a\n".to_string(),
            base: Some("b\n".to_string()),
            theirs: "c\n".to_string(),
            theirs_label: String::new(),
        };

        let prompt = build_conflict_hunk_prompt("doc.md", &hunk, &no_sides(), "", "");

        assert!(prompt.contains("Common ancestor:\nb"));
        assert!(prompt.contains("Our side (ours)"));
        assert!(prompt.contains("Their side (theirs)"));
    }

    #[test]
    fn conventions_prompt_asks_for_the_format_not_the_topics() {
        let prompt = build_conventions_prompt("fix: tighten retry window\n");

        assert!(prompt.contains("shape:"));
        assert!(prompt.contains("language:"));
        assert!(prompt.contains("Do not describe the topics the commits are about."));
    }
}
