# Terrarium Swift Tools

This project uses the `swift` terrarium preset. During `terrarium run`, use the built-in terrarium MCP tools for Swift workflow commands instead of direct shell commands when possible.

Prefer the terrarium MCP tools over direct shell commands for normal Swift build workflows:

- `swift_build`
- `swift_test`
- `swift_format_check`
- `make_install` (only when Makefile exists)
- `make_run`
- `just_run`
- `docker_run`
- `command_log_read`
- `spenn_list_scenarios`
- `spenn_run_scenario`
- `git_status`
- `git_unmerged`
- `git_log`
- `git_diff`
- `git_show`

You are running inside a macOS sandbox with a deny-default policy. When a denial blocks work that should legitimately be allowed, call `report_violation` with what was attempted and the error, so the profile can be reviewed. For an incidental denial, take another approach and move on rather than reporting it.

Use the built-in git MCP tools for read-only repository inspection instead of direct shell `git` commands.
For linked repositories, use the git tool `path` parameter to select the repository; use `pathspec` only for files inside that repository.
A symlinked project root is still a terrarium project: pass the symlink path in `path`, or its configured name in `project` under workspace mode, rather than falling back to shell commands.
The MCP tools are scoped to the terrarium project. A repository outside it is not reachable through them and the sandbox blocks shell builds there, so ask the user to run those commands.
When implementing a Swift feature, keep `Package.swift` aligned with the work. If dependencies changed, update `Package.swift`.
After implementing a Swift feature, run the tests with `swift_test`.
Tests are the source of truth for intended behavior. Never edit a test just to turn it green — fix the implementation. Do update a test when the intended behavior itself changed or the test no longer compiles, and say plainly which tests you changed and why.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and lints — treat warnings as errors.
