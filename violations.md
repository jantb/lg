# Sandbox Violations

## 2026-04-19 17:16:36 UTC

**Command:** `target/debug/harness`

Running target/debug/harness via Bash tool (dangerouslyDisableSandbox: true) — the harness calls git subprocesses (init, commit, etc.) in tempdirs. Git fails with 'fatal: Invalid path /private: Operation not permitted' or 'fatal: cannot mkdir /Users/jantb/dev/priv/lg/.testgit: Operation not permitted'. The sandbox blocks git subprocesses from creating directories even in allowed paths (/tmp, /Users/jantb/dev/priv/lg). The git binary at /Applications/Xcode.app/Contents/Developer/usr/bin/git itself runs but cannot mkdir anywhere accessible. This blocks running the harness binary via Bash.

