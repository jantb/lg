# Sandbox Violations

## 2026-04-19 17:16:36 UTC

**Command:** `target/debug/harness`

Running target/debug/harness via Bash tool (dangerouslyDisableSandbox: true) — the harness calls git subprocesses (init, commit, etc.) in tempdirs. Git fails with 'fatal: Invalid path /private: Operation not permitted' or 'fatal: cannot mkdir /Users/jantb/dev/priv/lg/.testgit: Operation not permitted'. The sandbox blocks git subprocesses from creating directories even in allowed paths (/tmp, /Users/jantb/dev/priv/lg). The git binary at /Applications/Xcode.app/Contents/Developer/usr/bin/git itself runs but cannot mkdir anywhere accessible. This blocks running the harness binary via Bash.

## 2026-08-24 17:10:09 UTC

**Command:** `git status --short / git commit`

Committing in this linked worktree is blocked. The worktree root is /Users/jantb/dev/priv/lg.worktrees/feature-refactor, but its .git file points the gitdir at /Users/jantb/dev/priv/lg/.git/worktrees/feature-refactor, which is outside the sandboxed project root. Any shell git invocation fails with "fatal: Invalid path '/Users': Operation not permitted" — including /Library/Developer/CommandLineTools/usr/bin/git. /usr/bin/git additionally fails because the Xcode shim cannot load DVTSystemPrerequisites under the sandbox. The terrarium MCP git tools (git_status, git_diff) work since the MCP server runs outside the sandbox, but they are read-only, so there is no way to commit from inside. Allowing read/write on the parent repository's .git/worktrees directory would let linked worktrees be committed from within a terrarium project.

