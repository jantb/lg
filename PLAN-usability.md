# Usability and configuration improvement plan

Status: direction accepted; based on a source review, not an interactive usability test. Small-terminal optimization is deferred. Terrarium will be copied into this repository and fully bundled; both codebases are owned by the same team.

## What exists today

- `src/panel/model.rs` already provides model and writing settings, but combines global model configuration and checkout-specific writing preferences in one form. Small terminals get a “Terminal too small” message.
- `src/panel/author.rs` edits Git identity separately. Enter saves a folder-wide rule, while another shortcut saves a repository override. Scope is consequential but selected through shortcuts.
- `src/settings.rs` stores writing preferences outside the repository, keyed by checkout path. Linked worktrees therefore do not automatically share these preferences.
- `src/session.rs` and `src/panel/agent.rs` support Claude, Codex, and Pi through a fixed enum and picker. Launch behavior and capabilities differ by agent.
- `src/terrarium.rs` derives worktree profiles from an existing main-worktree Terrarium profile. The launcher invokes an external `terrarium` executable. Neither the engine nor first-time profile setup is bundled here.
- `src/config.rs` hardcodes `origin`, `main`, `develop`/`dev`, and `test`; `src/git/release.rs` models two release environments. Branch roles and protections depend on these names.
- The “Deployment Status” panel derives status from Git history. This establishes branch inclusion, not whether a deployment actually succeeded.
- Editor selection is hardcoded to RustRover or IDEA in `src/git/config.rs`.

## 1. A unified Settings screen — highest priority

Add a discoverable Settings command, with searchable categories: Identity, Writing, Models, Agents, Sandbox, Branches & Environments, and Interface & Tools. Keep existing shortcuts as direct links to the relevant category.

Use a category list and scrollable form designed for the normal desktop terminal workflow. Defer narrow-terminal layouts and small-screen optimization. Every setting shows its effective value, source, and editing scope. Provide Save, Cancel, Reset this override, inline validation, and an unsaved-changes indicator. Reset should explain which inherited value will become active.

Introduce a typed, versioned configuration model before expanding the forms. For lg-owned settings, resolve defaults → user → repository → worktree → supported environment-variable overrides. Show environment-controlled fields as overridden rather than implying a save will change the active value. Repository identity should use the Git common directory so linked worktrees share defaults, with explicit worktree overrides.

Keep Git identity in Git configuration and preserve Git's own precedence. Keep credentials out of shareable configuration. Initially store lg settings locally; add an explicit export/import flow for portable project preferences later. Project-supplied executable commands and expanded permissions need a separate local trust decision.

Migration must preserve existing model settings, writing files, and per-checkout customizations. Detect conflicting worktree preferences instead of silently choosing one. Use atomic file replacement, report parse errors with the affected field, and preserve a migration backup.

Done when: existing users retain their effective settings, can explain where each value comes from, and can edit all categories through consistent navigation and save/cancel controls.

## 2. Make identity and writing configuration understandable

- Identity: effective name/email and source; explicit Repository or Folder scope selector; folder impact preview before saving; clear override action. Display signing configuration initially, then add editing if needed.
- Writing: language, subject/body limits, commit prompt, review guide, and optional conventions. Separate commit formatting from general prose style where their requirements differ.
- Provide a preview against a selected recent commit or staged diff. History-derived suggestions should be reviewable before application.
- Show the effective author in the commit flow, with a direct link to change it.
- Make editor/terminal commands configurable, storing executable and argument arrays rather than interpolated shell strings.

Done when: changing this repository's author cannot accidentally create a folder-wide rule, and users can preview writing preferences without creating a commit.

## 3. Agent inventory and a clear session launcher

Replace the fixed display list with configured agent profiles backed by adapters for the existing agents. Each profile has a name, executable, argument array, default status, optional model selection where supported, and sandbox preference.

Show executable detection, resolved path, detected version, and capability information: initial prompt, activity reporting, and resume support. Do not infer authentication readiness solely from an executable being installed. Run bounded checks in the background and offer refresh and diagnostics. Missing agents remain visible with an explanation.

The launcher shows repository, branch, worktree path, agent, and confinement mode before starting. Preserve a fast path using saved defaults. Allow choosing an existing worktree or creating one in the same flow. Custom commands can use a generic terminal adapter without claiming unsupported capabilities.

Replace the ambiguous sandbox boolean with explicit modes such as Terrarium, agent-managed permissions, and direct execution, offered only where supported. The current “unsandboxed” Codex path still requests its own workspace sandbox, so a single shared label is misleading.

Done when: users can tell which agents can launch, where they will run, and which confinement mechanism applies before a failed launch or a new process.

## 4. Fully bundle Terrarium inside lg

Decision: bring a workable source copy of `/Users/jantb/dev/priv/sx` into this repository. Both codebases are ours; bundling is committed scope, not a feasibility investigation or an optional external installation.

The source is a Rust 2024 application containing macOS Seatbelt execution, profile resolution/presets, a network proxy, MCP services, project setup, and a CLI. Copy the complete runtime and required assets/tests first, preserving working behavior before simplifying its interface. Exclude Git metadata, build output, personal profiles, credentials, and runtime state. Record the source revision and local changes so future fixes can be ported deliberately.

Implementation sequence:

1. Add the source under `crates/terrarium` as a local workspace crate. Expose its dispatch/runtime through a library entry point while retaining its CLI for development and tests. No sibling-checkout path dependency, submodule checkout, or runtime download is required.
2. Include that crate in lg's build and expose a dedicated `lg sandbox ...` subprocess entry point. Sandboxed PTY sessions invoke the current lg executable through that entry point instead of finding `terrarium` on PATH. Dispatch sandbox commands before initializing the TUI. Keep proxy/MCP/child-process lifecycle behavior intact.
3. Audit executable references in generated agent configuration, callbacks, documentation, and subprocess launches so they use the bundled entry point. Bundle presets and assets at build time; verify that copying/installing the lg binary is sufficient.
4. Connect profile discovery, first-time creation, validation, and permission inspection to Settings. Reuse existing Terrarium profiles compatibly and preserve managed `CLAUDE.md` guidance. Keep runtime state outside the checkout and avoid automatically rewriting unrelated existing installations.
5. Extend `just` checks to the workspace and verify `just release`/`just install` include everything required by the bundled runtime. Confinement targets macOS initially, matching the existing Seatbelt backend; additional OS backends are separate work.

The Sandbox page shows the bundled engine version, active profile/source, writable paths, shared Git metadata access, network rules, and credential access supported by the engine. Explain worktree profile inheritance and the shared Git-directory exception: worktrees do not have independent Git metadata boundaries. Offer profile validation and a diagnostic probe. Never silently fall back to direct execution when confinement fails.

Done when: lg built from this repository can initialize a fresh profile and launch a confined agent or shell on a supported Mac with no standalone Terrarium executable or source checkout available. Test permitted worktree writes, denied writes outside allowed paths, shared Git operations, network policy, and cleanup of subprocesses/proxy/MCP services. Existing profiles and managed guidance remain intact.

## 5. Explicit branch strategy and environment mappings

Replace fixed environment enums with repository configuration containing:

- Integration/base branch and default remote.
- Branch roles and protection policy, independently configurable from deployment targets.
- Any number of environments, each with a stable ID, display name, remote/ref, and optional URL.
- Explicit allowed promotion edges and a merge strategy for each supported operation.

Offer initial templates for trunk-based development, an integration branch with environment branches, and custom setup. Detect existing branches as suggestions. Do not assume production must be `main`, or that environments must form a linear development → test → production chain. Tag/artifact-based deployments should remain representable as unconfigured deployment sources rather than being forced into branch mappings.

Add “Assign environment…” to branch actions and the same editor in Settings. For example, a user could map Development to `origin/develop`, Staging to `origin/qa`, and Production to `origin/release`, while retaining `main` as the integration branch. These are examples, not new defaults.

Show environment and role badges in branch lists. Give environments a dedicated view reachable regardless of the repository tree's height. Distinguish “included in target branch,” “missing commits,” and “deployment unknown.” Actual deployed revision, pipeline result, and deployment time require a future CI/deployment connector.

Before promotion, show the exact source/target refs, commits, merge method, remote, and whether push is included. Recheck refs before mutation. Validate missing refs, ambiguous mappings, invalid promotion cycles, and local/remote divergence. Changes to mappings must not move branches themselves.

Apply the same configuration to branch creation, sync, landing, release, deletion guards, status caches, and UI labels. Replacing only the display names would leave incorrect Git behavior underneath.

Done when: a repository using `trunk`, `qa`, and `production` works without code changes, and all actions honor its configured roles and targets. Existing repositories retain their current flow through migration.

## 6. Supporting usability improvements

- A searchable action menu showing context, shortcut, and why an action is unavailable.
- A short setup checklist for missing identity, agent, model endpoint, sandbox, and branch configuration; ordinary Git operations remain usable independently.
- Persistent activity/error history with retry where meaningful; short status messages alone are easy to miss.
- Consistent save/cancel behavior and context-sensitive help across forms.
- Clear active repository/branch/worktree context while switching between sessions and Git panels.
- Session lifecycle controls: rename, restart, exit reason, and capability-aware resume. Do not promise restored live processes after restarting lg.

## Delivery order and validation

1. **Configuration foundation + Settings shell:** schema, scopes, migration, category navigation; move existing settings without changing their meaning.
2. **Identity + agent launch:** scope selection, inventory, diagnostics, saved launch defaults, configurable editor.
3. **Bundled sandbox:** in-repository Terrarium source/runtime, bundled subprocess entry point, first-run profiles, explicit modes, permission display, and failed-launch recovery.
4. **Branch/environment configuration:** dynamic model and Git operation integration, then badges and promotion previews.
5. **Deployment integrations and polish:** real deployment status, portable configuration export/import, and remaining discovery improvements. Sandbox bundling ships in milestone 3.

Action-menu discoverability and persistent errors can accompany the earlier milestones. Ship each milestone as independently usable work rather than waiting for the whole plan.

Test config precedence, migration conflicts, malformed files, and round trips. Use hermetic repositories for nonstandard branch names, multiple remotes, worktree inheritance, protection rules, and promotion behavior. Test missing agent executables and failed sandbox setup without launching real agents. Verify confinement with backend integration tests on supported platforms before describing a bundled backend as working.

Add TUI flow coverage for scoped identity edits, save/cancel, launcher mode selection, and environment assignment. Include terminal screenshots for layout changes. Run `just test` for behavior changes and `just` for these larger milestones; compiler and Clippy warnings block completion.

Recommended first implementation slice: Settings shell, scope/source display, and moving the existing identity/model/writing controls into it. This delivers an immediate usability gain and establishes the configuration foundation needed by agents, sandboxing, and environments.
