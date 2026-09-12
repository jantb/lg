# Terrarium Python Tools

This project uses the `python` terrarium preset. It has no dedicated Python MCP build tools; run test and lint commands through `make_run`, `just_run`, or `docker_run` when a wrapper exists, and use the shared terrarium MCP tools for everything else.

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

The sandbox grants the pip, uv, and Poetry caches plus the usual virtualenv locations, and the proxy allows PyPI. Install into a project-local virtualenv rather than the system interpreter.

Keep `pyproject.toml` and `requirements.txt` aligned with the work. If dependencies changed, update them in the same change.
