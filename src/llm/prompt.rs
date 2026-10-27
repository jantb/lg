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

/// The same for every step of a walk, so the prompt cache serves it once and each step pays only for its file.
pub fn build_guided_step_system_prompt(overview: &str, settings: &RepoSettings) -> String {
    format!(
        "You are walking a reviewer through a change one file at a time. The overview of the\n\
         whole change is below; each message then gives every hunk of one file with the source\n\
         around them. Review the file's change as a whole, not each hunk on its own.\n\
         Reply in exactly this shape, one item a line, and nothing else:\n\
         Summary: <what the change to the file does, in one or two sentences, in terms of behavior rather than lines>\n\
         Fixed: <a finding from your earlier read that the code now fixes, only when one is given>\n\
         L<n> issue: <what is wrong and why it matters>\n\
         L<n> nit: <a smaller point>\n\
         L<n> question: <what the code shown cannot settle, asked as a question>\n\
         Verdict: ok|nit|issue\n\
         Write one `L<n>` line per finding, most severe first, where <n> is the number the diff\n\
         shows for the line it is about. A finding is a bug, an unhandled case, a broken\n\
         contract, an invariant the change can break on some path, a missing test, a sentinel\n\
         value (-1, 0, MAX, an empty string standing in for \"none\") that can collide with a\n\
         real value or leak into arithmetic, indexing or display, or a concrete violation of\n\
         the repo style below. A missing comment is never a finding.\n\
         Ask a question rather than guess when the answer depends on code not shown; a question\n\
         does not change the verdict. Leave out the `L` lines when there is nothing to flag.\n\
         Keep each line under 25 words. Keep the words Summary, Fixed, Verdict, issue, nit and\n\
         question in English. Do not restate the diff, use code fences, or invent code that is not shown.\n\n\
         {}\n\n\
         {}\n\
         Change overview:\n{overview}",
        review_style(settings, overview),
        crate::settings::language_instruction(settings)
    )
}

pub fn build_guided_chat_prompt(title: &str, review: &str, settings: &RepoSettings) -> String {
    format!(
        "You are pairing with a reviewer on {title}, in this checkout. lg's guided review walks\n\
         the change file by file; what is still open in the file on screen is below, and when the\n\
         reviewer moves to another file you will be told what is open there.\n\
         Read the code they point at, but change nothing until the reviewer says what to fix, one\n\
         at a time or all at once. Keep each change minimal, match the surrounding style, and never\n\
         commit or otherwise change git state. Start by saying in two or three lines what is open.\n\n\
         {}\n\n\
         {}\n\
         {review}",
        review_style(settings, review),
        crate::settings::language_instruction(settings)
    )
}

pub fn build_guided_chat_file_read(brief: &str) -> String {
    format!(
        "lg has read the file the reviewer is on now. {brief}\n\
         Say in two or three lines what is open there, then wait."
    )
}

pub fn build_guided_chat_redo(brief: &str) -> String {
    format!(
        "The reviewer has changed the file and lg has reviewed it again. {brief}\n\
         Say in two or three lines what is fixed and what is still open, then wait."
    )
}

/// The opening of a guided review: the plan every walk mode orders the files
/// by, from the whole diff — its concerns, reading order, risk and what to skim.
pub fn build_guided_overview_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "You are about to walk a reviewer through this change one file at a time. Read the whole\n\
         diff below and plan the walk. Reply in exactly this shape, one item a line, and nothing else:\n\
         Summary: <what the change is for, in two or three sentences>\n\
         Concern: <a short title for one part of the change, such as one feature, fix or refactor>\n\
         Why: <what that part does and why, in one sentence>\n\
         Files: <its files, comma separated, as the diff names them>\n\
         (one Concern, Why and Files group per part of the change, the part to read first first)\n\
         Order: <every file, comma separated, in the order best read: what explains the rest first>\n\
         Risk: <file> high|medium|low: <why, in a few words>\n\
         Skim: <file>: <why it is safe to skim: generated, a lockfile, a pure rename, formatting>\n\
         Watch: <one thing most worth the reviewer's attention, naming its file>\n\
         Put every file in exactly one concern; a file that serves several goes in the one it\n\
         serves most. Give one Risk line per file that is not skimmed, and up to three Watch lines.\n\
         Keep the words Summary, Concern, Why, Files, Order, Risk, Skim and Watch, and high, medium\n\
         and low, in English. No code fences. Do not invent behavior that is not shown.\n\n\
         {}\n\
         {context}",
        crate::settings::language_instruction(settings)
    )
}

/// The last step of a guided review: the whole changeset read as one change,
/// in context, for what reading it file by file misses.
pub fn build_guided_final_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "You have walked a reviewer through this change one file at a time. Now step back and\n\
         review the whole changeset below as one change, the way a reviewer reads it before\n\
         approving: read every file's diff together and judge it in context, not file by file.\n\
         Ask whether the pieces hang together and do what the overview says and no more; whether\n\
         the design is sound or a simpler shape would do; and what no single file shows: a caller\n\
         or test not updated for a changed signature or contract, a value one file produces and\n\
         another reads differently, the same thing handled inconsistently in two places,\n\
         duplicated logic, an invariant that spans files and is broken on some path, leftover\n\
         debugging or dead code, and whether the tests cover the change as a whole.\n\
         What the file-by-file reads raised is listed after the diff. Repeat one only when the\n\
         wider context changes it: makes it worse, settles it, or shows it is part of a pattern.\n\
         Reply in exactly this shape, one item a line, and nothing else:\n\
         Summary: <the change as a whole and whether it hangs together, in two to four sentences>\n\
         <path>:L<n> issue: <what is wrong, seen in the whole change, and why it matters>\n\
         <path>:L<n> nit: <a smaller point>\n\
         <path>:L<n> question: <what the diff cannot settle, asked as a question>\n\
         Verdict: ok|nit|issue\n\
         <path> is the file as the diff names it and <n> the number the diff shows for the line.\n\
         Leave out the finding lines when there is nothing to flag. Keep each line under 30 words.\n\
         Keep the words Summary, Verdict, issue, nit and question in English. A missing comment is\n\
         never a finding. Do not restate the diff, use code fences, or invent code that is not shown.\n\n\
         {}\n\n\
         {}\n\
         {context}",
        review_style(settings, context),
        crate::settings::language_instruction(settings)
    )
}

/// The system prompt for a question asked about the step on screen.
pub fn build_guided_question_system_prompt(context: &str, settings: &RepoSettings) -> String {
    format!(
        "You are helping a reviewer understand one file of a change they are reviewing. Answer\n\
         their question from the context below; say what is missing rather than guessing. Be\n\
         brief and concrete, cite lines as `L123`, and do not use code fences unless asked for\n\
         replacement code.\n\n\
         {}\n\
         {context}",
        crate::settings::language_instruction(settings)
    )
}

/// What a headless Claude Code run is asked when the reviewer wants a step
/// fixed: the one change, in the one place, and nothing else.
pub fn build_guided_fix_prompt(context: &str, instruction: &str) -> String {
    format!(
        "You are making one small change requested during a code review of this checkout.\n\
         Apply exactly what the reviewer asks below, in the file shown, editing it in place.\n\
         Read what you need to get it right, keep the change minimal, match the surrounding style,\n\
         and do not touch unrelated code, commit, or run anything that changes git state.\n\
         When done, reply with one line saying what you changed.\n\n\
         Reviewer's request:\n{instruction}\n\n\
         {context}"
    )
}

/// What a headless Claude Code run is asked when a step's commentary raised
/// questions: settle them in the code, and keep only what the reviewer has to
/// decide.
pub fn build_guided_question_check_prompt(
    context: &str,
    questions: &str,
    settings: &RepoSettings,
) -> String {
    format!(
        "A reviewer is walking through a change in this checkout one file at a time. A model that\n\
         saw only the file's diff below raised the questions below it. Settle each one by reading the code:\n\
         open the files, follow the callers and the tests. Do not change any file.\n\
         Then reply with one line per question that still matters, in exactly this shape, and nothing else:\n\
         L<n> issue: <what is wrong and why it matters>\n\
         L<n> nit: <a smaller point>\n\
         L<n> question: <only what the code cannot settle and the reviewer must decide, such as intent; say what you found>\n\
         Leave out every question the code answers with nothing to flag: the reviewer does not need\n\
         to hear about it. Reply with nothing at all when every question is settled that way.\n\
         Keep <n> as the question had it. Keep each line under 25 words. Keep the words issue, nit\n\
         and question in English.\n\n\
         {}\n\
         Questions:\n{questions}\n\n\
         {context}",
        crate::settings::language_instruction(settings)
    )
}

/// What the review chat is told when lg's read of a file raised questions:
/// settle them in the code and write what still matters where lg reads it.
pub fn build_guided_chat_questions(path: &str, questions: &str, answer: &Path) -> String {
    format!(
        "lg's read of {path} raised the questions below. Settle each one by reading\n\
         the code: open the file, follow the callers and the tests. Change nothing.\n\
         Then write one line per question that still matters to {answer}, in exactly this shape:\n\
         L<n> issue: <what is wrong and why it matters>\n\
         L<n> nit: <a smaller point>\n\
         L<n> question: <only what the code cannot settle and the reviewer must decide; say what you found>\n\
         Leave out every question the code answers with nothing to flag, and write an empty file\n\
         when that is all of them. Keep <n> as the question had it. Then say in one line what you\n\
         found, and wait.\n\n\
         {questions}",
        answer = answer.display(),
    )
}

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

    /// The built-in guide is about code in general; what a Kotlin reviewer
    /// checks that a TypeScript reviewer does not is appended per language,
    /// and only for the languages the reviewed text names files of.
    #[test]
    fn the_built_in_guide_gains_notes_for_each_language_under_review() {
        let prompt = review_style(
            &RepoSettings::default(),
            "Files changed:\n- src/main/kotlin/App.kt\n- web/src/App.tsx",
        );

        assert!(prompt.contains("\nKotlin:\n"), "{prompt}");
        assert!(prompt.contains("\nTypeScript:\n"), "{prompt}");
        assert!(!prompt.contains("\nRust:\n"), "{prompt}");
        assert!(!prompt.contains("\nC# / .NET:\n"), "{prompt}");
    }

    #[test]
    fn the_built_in_guide_rules_out_yoda_comparisons() {
        let style = review_style(&RepoSettings::default(), "src/Order.java");

        assert!(style.contains("Boolean.TRUE.equals"), "{style}");
    }

    #[test]
    fn the_built_in_guide_rules_out_parameterized_tests() {
        let style = review_style(&RepoSettings::default(), "");

        assert!(style.contains("No parameterized tests"), "{style}");
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
            build_guided_step_system_prompt("src/lib.rs", &settings),
            build_guided_final_prompt("src/lib.rs", &settings),
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
