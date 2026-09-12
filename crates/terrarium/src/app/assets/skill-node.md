---
name: terrarium-node-tools
description: >
  Use when you need to run built-in terrarium MCP commands in Node projects initialized
  with `terrarium`. The terrarium MCP tools run npm/pnpm/yarn/bun commands outside the sandbox
  and return compact structured results with a live log path.
  TRIGGER: terrarium-managed Node project + any of: install dependencies, run a package.json script,
  start a dev server, run tests, run tsc or another node_modules binary.
---

{version_tag}

When working in a terrarium-managed Node project, prefer the built-in MCP tools over Bash:

| Goal | Use |
|------|-----|
| Install dependencies | `npm_install` (omit `packages` for a lockfile-faithful install) |
| Add a dependency | `npm_install` with `packages` (and `dev=true` for a devDependency) |
| Run a package.json script | `npm_run` (`script`, optional `args`) |
| Start a dev server or watcher | `npm_run` with `background=true` |
| Tests | `npm_test` (optional `filter` as a test name pattern) |
| Run node directly | `node_run` (e.g. `args=["--test", "src/app.test.js"]`) |
| Run a node_modules binary | `npx_run` (e.g. `package="tsc"`, `args=["--noEmit"]`) |
| Run Makefile install | `make_install` (only when Makefile exists) |
| Run Make target/args | `make_run` |
| Run Just recipe/args | `just_run` |
| Run Docker command | `docker_run` |
| Read command log | `command_log_read` |
| Git status | `git_status` |
| Unmerged files | `git_unmerged` |
| Git history | `git_log` |
| Git diff summary | `git_diff` |
| Git revision details | `git_show` |
| Report sandbox violation | `report_violation` |

The package manager is resolved from the project's lockfile (`pnpm-lock.yaml`, `yarn.lock`, `bun.lockb`, `package-lock.json`) or its `packageManager` field, so name the script, not the command line.

`npm_install`, `npm_run`, `node_run`, `npx_run`, `make_run`, `just_run`, and `docker_run` always write command output live to a returned log path under `~/.terrarium/projects/<project>/output/`. Use `command_log_read` to inspect those logs. A dev server or watch mode never exits — call `npm_run` with `background=true` so the request returns immediately with a PID and live log path, then read the log.

You are running inside a macOS sandbox with a deny-default policy. When a denial blocks work that should legitimately be allowed, call `report_violation` with what was attempted and the error, so the profile can be reviewed. For an incidental denial, take another approach and move on rather than reporting it.

Results are compact: only errors/failures are returned where possible.
Use the git MCP tools for read-only repository inspection instead of direct Bash git commands.
For linked repositories, use the git tool `path` parameter to select the repository; use `pathspec` only for files inside that repository.
A symlinked project root is still a terrarium project: pass the symlink path in `path`, or its configured name in `project` under workspace mode, rather than falling back to Bash commands.
The MCP tools are scoped to the terrarium project. A repository outside it is not reachable through them and the sandbox blocks shell builds there, so ask the user to run those commands.
When implementing a Node feature, keep `package.json` aligned with the work. If dependencies changed, add them with `npm_install` so `package.json` and the lockfile move together.
After implementing a Node feature, run the tests with `npm_test`.
Tests are the source of truth for intended behavior. Never edit a test just to turn it green — fix the implementation. Do update a test when the intended behavior itself changed or the test no longer compiles, and say plainly which tests you changed and why.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and lints — treat warnings as errors.
Do not bypass the terrarium MCP tools with direct npm, node, or read-only git Bash commands in projects managed by terrarium.
