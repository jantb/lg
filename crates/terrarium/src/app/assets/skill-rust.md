---
name: terrarium-rust-tools
description: >
  Use when you need to run built-in terrarium MCP commands in Rust projects initialized
  with `terrarium`. The terrarium MCP tools run Cargo commands inside a strict macOS sandbox
  and return compact structured results.
  TRIGGER: terrarium-managed Rust project + any of: compile errors, run tests, check lints, check formatting, format code, update dependencies.
---

{version_tag}

When working in a terrarium-managed Rust project, prefer the built-in MCP tools over Bash:

| Goal | Use |
|------|-----|
| Compile check | `cargo_check` |
| Tests | `cargo_test` (optional: single test name or subset filter) |
| Build | `cargo_build` (optional: `release=true`) |
| Lints | `cargo_clippy` |
| Format check | `cargo_fmt_check` |
| Auto-format | `cargo_fmt` |
| Lockfile refresh | `cargo_update` |
| Incompatible dependency upgrades | `cargo_upgrade_incompatible` |
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
When implementing a Rust feature, update `Cargo.toml` if dependencies or crate features changed as part of the work.
Use `cargo_update` to refresh `Cargo.lock`, `cargo_upgrade_incompatible` when intentionally moving Rust dependencies across breaking versions.
After implementing a Rust feature, run the tests with `cargo_test`.
Tests are the source of truth for intended behavior. Never edit a test just to turn it green — fix the implementation. Do update a test when the intended behavior itself changed or the test no longer compiles, and say plainly which tests you changed and why.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and clippy lints — treat warnings as errors.
Do not bypass the terrarium MCP tools with direct Cargo or read-only git Bash commands in projects managed by terrarium.
