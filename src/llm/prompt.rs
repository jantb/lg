//! The prompt each task sends, and the repo style guide they share.

use crate::settings::RepoSettings;

use super::diff::{diff_excerpt, summarize_diff};

const REVIEW_REPO_STYLE_GUIDE: &str = "\
Established repo style:
- Kotlin/Spring, but immutable code by default: prefer val, immutable collections, data-class .copy(), focused functions, and pure helper functions.
- Constructor injection only. Inject narrow interfaces/services, not broad infrastructure.
- Controllers stay thin: auth, validation, DTO assembly, ResponseEntity. Business decisions go in service-layer files/classes whose path or name contains Service, or in explicit hub flow code.
- Treat business rules in controllers, adapters, Kafka consumers/listeners, repositories, DTOs, configuration, or other non-Service/non-flow files as a style issue unless the shown code only delegates or translates data.
- Flow start state construction may call repositories/services to load initial data before the flow begins. Once a flow has started, later state constructors/steps should stay pure; flag direct repository/service calls there.
- Domain IDs use inline value classes like UserId, MembershipId; wrap raw primitives at repository boundaries.
- Names should describe domain intent and behavior. Flag vague, misleading, or overly generic names and suggest a concrete replacement.
- Use sealed interfaces/classes for variants with different data; enums only for simple tags.
- JSON uses the shared configuredJson; avoid Jackson in app code except generated/Spring/Avro internals.
- Time uses kotlinx.datetime; java.time only at interop edges.
- Logging uses private val log by Logger(), not direct LoggerFactory.
- Outbound HTTP uses Ktor CIO adapters. Each external system gets one adapter.
- Persistence is PostgreSQL via Exposed + Flyway.
- Kafka/outbound side effects from flows go through the outbox, not direct Kafka publishing.
- Tests prefer real small fakes over mocks. Use Mockk only when a fake is impractical; never Mockito.
- Integration tests use @SpringBootTest + TestConfiguration + Testcontainers.
- Do not edit generated code under target/generated-sources.
- Run the repo formatter/lint before declaring work done; linter wins on formatting.";

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
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         {}\n\
         Selected review subtree:\n{context}",
        crate::settings::language_instruction(settings)
    )
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
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         {}\n\
         Review context:\n{context}",
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
         Use FAIL for clear violations such as business logic in controllers or other non-Service/non-flow files,\n\
         direct Kafka side effects, Jackson app-code usage, Mockito, java.time away from interop edges,\n\
         or generated code edits.\n\n\
         Do not flag repository/service calls used to build the initial/start state before a flow begins;\n\
         only flag direct repository/service calls in later states or steps after the flow has started.\n\n\
         Treat the File role below as authoritative. For service-layer or flow files, repository/service calls\n\
         and business rule orchestration are allowed by layer placement; do not flag them merely as\n\
         non-Service/non-flow violations. Return OK for that concern unless another concrete style rule is violated.\n\n\
         For naming issues, include a concrete rename suggestion in the reason.\n\n\
         {REVIEW_REPO_STYLE_GUIDE}\n\n\
         {}\n\
         File: {path}\n\
         File role: {file_role}\n\
         Review context:\n{context}",
        crate::settings::language_instruction(settings)
    )
}

pub fn review_style_file_role(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.contains("service") {
        "service-layer"
    } else if lower.contains("flow") {
        "flow"
    } else {
        "non-service/non-flow"
    }
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
        assert!(prompt.contains("Constructor injection only"));
        assert!(prompt.contains("configuredJson"));
        assert!(prompt.contains("path or name contains Service"));
        assert!(prompt.contains("Flow start state construction may call repositories/services"));
        assert!(prompt.contains("Selected review subtree:\nsrc/main/kotlin/App.kt"));
    }

    #[test]
    fn review_chat_system_prompt_includes_repo_style() {
        let prompt =
            build_review_chat_system_prompt("full review context", &RepoSettings::default());

        assert!(prompt.contains("commit subjects/bodies"));
        assert!(prompt.contains("patch intent"));
        assert!(prompt.contains("intentional behavior change"));
        assert!(prompt.contains("Ktor CIO adapters"));
        assert!(prompt.contains("never Mockito"));
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
        assert!(prompt.contains("non-Service/non-flow files"));
        assert!(prompt.contains(
            "Do not flag repository/service calls used to build the initial/start state"
        ));
        assert!(prompt.contains("after the flow has started"));
        assert!(prompt.contains("File: src/main/kotlin/App.kt"));
        assert!(prompt.contains("File role: non-service/non-flow"));
        assert!(prompt.contains("updates controller logic"));
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
        assert!(prompt.contains("business rule orchestration are allowed"));
    }

    #[test]
    fn conventions_prompt_asks_for_the_format_not_the_topics() {
        let prompt = build_conventions_prompt("fix: tighten retry window\n");

        assert!(prompt.contains("shape:"));
        assert!(prompt.contains("language:"));
        assert!(prompt.contains("Do not describe the topics the commits are about."));
    }
}
