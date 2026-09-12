## Shared tools

- `git_status`, `git_log`, `git_diff`, `git_show`, `git_unmerged`
- `list_files`, `directory_structure`, `search_files`, `create_directory`
- `make_install`, `make_run`, `just_run`, `docker_run`, `command_log_read`, `spenn_list_scenarios`, `spenn_run_scenario`, `report_violation`

All build tools accept an optional `project` parameter (relative path) to target a specific sub-project.
When only one sub-project uses a given toolchain, the `project` parameter can be omitted.

You are running inside a macOS sandbox with a deny-default policy. When a denial blocks work that should legitimately be allowed, call `report_violation` with what was attempted and the error, so the profile can be reviewed. For an incidental denial, take another approach and move on rather than reporting it.

Use the built-in git MCP tools for read-only repository inspection instead of direct shell `git` commands.
For linked repositories, use the git tool `path` parameter to select the repository; use `pathspec` only for files inside that repository.
A symlinked project root is still a terrarium project: pass the symlink path in `path`, or its configured name in `project` under workspace mode, rather than falling back to shell commands.
The MCP tools are scoped to the terrarium project. A repository outside it is not reachable through them and the sandbox blocks shell builds there, so ask the user to run those commands.
Never modify tests to make them pass — tests are the source of truth. Fix the implementation to satisfy the tests.
Always fix all compiler warnings and lints — treat warnings as errors.
