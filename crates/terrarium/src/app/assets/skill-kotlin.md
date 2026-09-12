---
name: terrarium-kotlin-tools
description: >
  Use when you need to run built-in terrarium MCP commands in Kotlin projects initialized
  with `terrarium`. The terrarium MCP tools run Gradle commands inside a strict macOS sandbox
  and return compact structured results.
  TRIGGER: terrarium-managed Kotlin project + any of: compile errors, run tests, check formatting.
---

{version_tag}

When working in a terrarium-managed Kotlin project, prefer the built-in MCP tools over Bash:

| Goal | Use |
|------|-----|
| Compile check | `gradle_check` |
| Tests | `gradle_test` (optional: single test name or subset filter) |
| Build | `gradle_build` |
| Formatting | `gradle_format_check` |
| Run Makefile install | `make_install` (only when Makefile exists) |
| Run Make target/args | `make_run` |
| Run Just recipe/args | `just_run` |
| Run Docker command | `docker_run` |
| Read command log | `command_log_read` |
| List Spenn scenarios | `spenn_list_scenarios` |
| Run Spenn scenario | `spenn_run_scenario` |
| Git status | `git_status` |
| Unmerged files | `git_unmerged` |
| Git history | `git_log` |
| Git diff summary | `git_diff` |
| Git revision details | `git_show` |
| Report sandbox violation | `report_violation` |

`make_install`, `make_run`, `just_run`, and `docker_run` always write command output live to a returned log path under `~/.terrarium/projects/<project>/output/`. Use `command_log_read` to inspect those logs. For long rebuilds or log-following commands, call `make_run`, `just_run`, or `docker_run` with `background=true` so the MCP request returns immediately with a PID and live log path.

You are running inside a macOS sandbox with a deny-default policy. When a denial blocks work that should legitimately be allowed, call `report_violation` with what was attempted and the error, so the profile can be reviewed. For an incidental denial, take another approach and move on rather than reporting it.

Results are compact: only errors/failures are returned where possible.
Use the git MCP tools for read-only repository inspection instead of direct Bash git commands.
For linked repositories, use the git tool `path` parameter to select the repository; use `pathspec` only for files inside that repository.
A symlinked project root is still a terrarium project: pass the symlink path in `path`, or its configured name in `project` under workspace mode, rather than falling back to Bash commands.
The MCP tools are scoped to the terrarium project. A repository outside it is not reachable through them and the sandbox blocks shell builds there, so ask the user to run those commands.
When implementing a Kotlin feature, inspect `settings.gradle`, `settings.gradle.kts`, and the relevant `build.gradle` or `build.gradle.kts` files before choosing which module to change. Keep edits scoped to the affected module unless the build graph or shared convention plugins require a wider change.
Keep Gradle build files aligned with dependency, plugin, Java toolchain, and Kotlin compiler option changes. Check `gradle/libs.versions.toml`, `gradle.properties`, and wrapper files when the project uses them.
Infer whether the project is Kotlin/JVM, Android, or multiplatform from plugins and source sets before editing code. Do not assume Android-specific layouts unless the build files show Android plugins.
Do not edit generated output or Gradle build directories. Fix source files, test files, or build configuration instead.
For verification, prefer the smallest useful Gradle MCP command first: `gradle_check` for compile feedback, `gradle_test` with a filter for focused test feedback, then full `gradle_test` when the change is ready.
Prefer small real test implementations over mocks. For repository or service tests, use simple backing data structures such as maps or in-memory fakes so behavior and state can be inspected directly; do not mock returned results with Mockito when a small implementation would make the test clearer.
Test observable effects and outcomes, not internal call sequences or implementation details.
Keep unit test scope tight. Give each function a clear responsibility, prefer pure functions where practical, and test those pure functions directly.
After implementing a Kotlin feature, run the tests with `gradle_test`.
Tests are the source of truth for intended behavior. Never edit a test just to turn it green — fix the implementation. Do update a test when the intended behavior itself changed or the test no longer compiles, and say plainly which tests you changed and why.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and lints — treat warnings as errors.
Do not bypass the terrarium MCP tools with direct Gradle or read-only git Bash commands in projects managed by terrarium.
