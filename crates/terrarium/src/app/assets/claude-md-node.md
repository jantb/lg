# Terrarium Node Tools

This project uses the `node` terrarium preset. During `terrarium run`, use the built-in terrarium MCP tools for Node workflow commands instead of direct shell commands when possible. They run outside the sandbox and resolve the package manager (npm, pnpm, yarn, or bun) from the project's lockfile, so you never name it yourself.

Prefer the terrarium MCP tools over direct shell commands for normal Node build workflows:

- `npm_install`
- `npm_run`
- `npm_test`
- `node_run`
- `npx_run`
- `make_install` (only when Makefile exists)
- `make_run`
- `just_run`
- `docker_run`
- `command_log_read`
- `git_status`
- `git_unmerged`
- `git_log`
- `git_diff`
- `git_show`

`npm_install`, `npm_run`, `node_run`, and `npx_run` write command output live to a returned log path under `~/.terrarium/projects/<project>/output/`. Use `command_log_read` to inspect those logs. A dev server or watch mode never exits, so start it with `background=true` and read its log rather than waiting on the call.

You are running inside a macOS sandbox with a deny-default policy. When a denial blocks work that should legitimately be allowed, call `report_violation` with what was attempted and the error, so the profile can be reviewed. For an incidental denial, take another approach and move on rather than reporting it.

Keep `package.json` aligned with the work. If dependencies changed, update `package.json` and the lockfile in the same change — `npm_install` with `packages` does both.
