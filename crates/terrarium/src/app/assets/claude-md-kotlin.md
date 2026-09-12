# Terrarium Kotlin Tools

This project uses the `kotlin` terrarium preset. During `terrarium run`, use the built-in terrarium MCP tools for Gradle workflow commands instead of direct shell commands when possible.

Prefer the terrarium MCP tools over direct shell commands for normal Kotlin build workflows:

- `gradle_check`
- `gradle_test`
- `gradle_build`
- `gradle_format_check`
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
When implementing a Kotlin feature, inspect `settings.gradle`, `settings.gradle.kts`, and the relevant `build.gradle` or `build.gradle.kts` files before choosing which module to change. Keep edits scoped to the affected module unless the build graph or shared convention plugins require a wider change.
Keep Gradle build files aligned with dependency, plugin, Java toolchain, and Kotlin compiler option changes. Check `gradle/libs.versions.toml`, `gradle.properties`, and wrapper files when the project uses them.
Infer whether the project is Kotlin/JVM, Android, or multiplatform from plugins and source sets before editing code. Do not assume Android-specific layouts unless the build files show Android plugins.
Do not edit generated output or Gradle build directories. Fix source files, test files, or build configuration instead.
For verification, prefer the smallest useful Gradle MCP command first: `gradle_check` for compile feedback, `gradle_test` with a filter for focused test feedback, then full `gradle_test` when the change is ready.
Prefer small real test implementations over mocks. For repository or service tests, use simple backing data structures such as maps or in-memory fakes so behavior and state can be inspected directly; do not mock returned results with Mockito when a small implementation would make the test clearer.
Test observable effects and outcomes, not internal call sequences or implementation details.
Keep unit test scope tight. Give each function a clear responsibility, prefer pure functions where practical, and test those pure functions directly.
`gradle_test` clears the project-local Gradle state and forces a fresh test execution.
After implementing a Kotlin feature, run the tests with `gradle_test`.
Tests are the source of truth for intended behavior. Never edit a test just to turn it green — fix the implementation. Do update a test when the intended behavior itself changed or the test no longer compiles, and say plainly which tests you changed and why.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and lints — treat warnings as errors.
