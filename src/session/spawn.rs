//! How each agent and the shell is launched, sandboxed or not.

use super::*;

/// The permission mode an unsandboxed claude session runs under.
pub(super) const AUTO_PERMISSION_MODE: &str = "auto";

/// The sandbox an unsandboxed codex session confines itself to: it may work in
/// the checkout without stopping to ask, and asks before reaching outside it.
/// As far as claude's auto mode goes, in the vocabulary codex has.
pub(super) const CODEX_WORKSPACE_SANDBOX: &str = "workspace-write";

/// How to launch `program` in `cwd`: directly, or wrapped in terrarium when the
/// session is sandboxed, which confines the process to that worktree.
///
/// The returned args already end with terrarium's `--`, so everything a caller
/// appends is the program's own argument either way.
pub(super) fn confined(cwd: &Path, sandboxed: bool, program: &str) -> (String, Vec<String>) {
    if !sandboxed {
        return (program.to_string(), Vec::new());
    }
    (
        crate::terrarium::executable()
            .to_string_lossy()
            .into_owned(),
        vec![
            "sandbox".to_string(),
            "run".to_string(),
            "--project".to_string(),
            cwd.to_string_lossy().into_owned(),
            "--".to_string(),
            program.to_string(),
        ],
    )
}

/// The prompt an agent opens on, when it is worth passing at all. A blank one
/// is not: it would reach the agent as an empty first turn.
pub(super) fn opening_prompt(prompt: Option<&str>) -> Option<&str> {
    prompt.map(str::trim).filter(|prompt| !prompt.is_empty())
}

/// How to launch claude in `cwd`. Sandboxed sessions go through terrarium,
/// which confines the process to that worktree. An unsandboxed one runs in
/// auto mode: nothing is holding it back anyway, so stopping to ask buys
/// little.
///
/// `settings` is the hook settings file from [`crate::hooks::install`], which is
/// how the session comes to report what it is doing. Without one it still runs —
/// it just says nothing.
///
/// `prompt` is what the session opens on, for one started in answer to
/// something lg already knows about — a conflict it stopped on, say. It is
/// claude's first turn, not a flag, so it goes last of all.
pub fn claude_spawn(
    cwd: &Path,
    sandboxed: bool,
    settings: Option<&Path>,
    prompt: Option<&str>,
) -> Spawn {
    // terrarium resolves the project path before looking up its profile, so the
    // path handed to it has to be resolved too.
    let cwd = &crate::terrarium::resolve(cwd);
    let (program, mut args) = confined(cwd, sandboxed, "claude");
    if !sandboxed {
        args.push("--permission-mode".to_string());
        args.push(AUTO_PERMISSION_MODE.to_string());
    }
    // After the `--` a sandboxed session goes through: these are claude's
    // arguments, not terrarium's.
    if let Some(settings) = settings {
        args.push("--settings".to_string());
        args.push(settings.to_string_lossy().into_owned());
    }
    if let Some(prompt) = opening_prompt(prompt) {
        args.push(prompt.to_string());
    }
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        env_remove: nested_claude_markers(),
    }
}

/// How to launch codex in `cwd`.
///
/// Sandboxed, terrarium is already holding the process to this checkout, and
/// codex's own sandbox would be a second one nested inside it — which is both
/// redundant and the case its bypass flag documents itself as being for.
/// Unsandboxed, nothing else is confining it, so codex confines itself to the
/// checkout: free rein inside, a question before it reaches outside.
///
/// `prompt` is what the session opens on, and goes last as codex's optional
/// positional prompt.
pub fn codex_spawn(cwd: &Path, sandboxed: bool, prompt: Option<&str>) -> Spawn {
    let cwd = &crate::terrarium::resolve(cwd);
    let (program, mut args) = confined(cwd, sandboxed, "codex");
    if sandboxed {
        args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    } else {
        args.push("--sandbox".to_string());
        args.push(CODEX_WORKSPACE_SANDBOX.to_string());
    }
    if let Some(prompt) = opening_prompt(prompt) {
        args.push(prompt.to_string());
    }
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        env_remove: nested_claude_markers(),
    }
}

/// How to launch pi in `cwd`. It has no sandbox of its own to ask for, so a
/// sandboxed session is terrarium's doing and an unsandboxed one runs as pi
/// would from a shell.
///
/// The prompt goes after `--`, which is how pi is told the rest is a message
/// rather than flags — without it a prompt opening with a dash is read as one.
pub fn pi_spawn(cwd: &Path, sandboxed: bool, prompt: Option<&str>) -> Spawn {
    let cwd = &crate::terrarium::resolve(cwd);
    let (program, mut args) = confined(cwd, sandboxed, "pi");
    if let Some(prompt) = opening_prompt(prompt) {
        args.push("--".to_string());
        args.push(prompt.to_string());
    }
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        env_remove: nested_claude_markers(),
    }
}

/// Shell to run a terminal session in: the one the user has chosen, falling
/// back to something every unix has. It is started with no arguments, which on
/// a pty is an interactive shell and so reads the usual rc file.
pub(super) fn login_shell() -> String {
    let configured = crate::preferences::load().config.tools.terminal;
    if !configured.is_empty() {
        return configured;
    }
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| FALLBACK_SHELL.to_string())
}

/// How to launch a shell in `cwd`. A terminal is the user's own shell and is
/// never put through terrarium: the sandbox is for agents, and a confined
/// shell only blocks the work the user came to do by hand.
pub fn shell_spawn(cwd: &Path) -> Spawn {
    // Resolved the same way the agent sessions beside it are, so the shell
    // opens in the checkout they show.
    let cwd = &crate::terrarium::resolve(cwd);
    let program = login_shell();
    let args = crate::preferences::load().config.tools.terminal_args;
    Spawn {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env: session_env(),
        // A shell is where a claude gets started by hand, so it needs the
        // markers dropped for the same reason a claude session does.
        env_remove: nested_claude_markers(),
    }
}

/// What every session is told about the terminal it is drawn on.
pub(super) fn session_env() -> Vec<(String, String)> {
    vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ]
}

/// Variables that tell claude it is running inside another claude. lg may
/// itself have been started from inside a session; without dropping these the
/// child would think it is nested in one.
pub(super) fn nested_claude_markers() -> Vec<String> {
    vec![
        "CLAUDECODE".to_string(),
        "CLAUDE_CODE_CHILD_SESSION".to_string(),
        "CLAUDE_CODE_ENTRYPOINT".to_string(),
    ]
}
