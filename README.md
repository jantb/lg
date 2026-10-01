# lg

`lg` is a terminal UI for git. It covers the everyday loop of staging,
committing, branching and pushing, and adds a local LLM for commit messages
and reviews, embedded coding-agent sessions, and a sandbox for running those
agents safely.

## Features

- **Staging and committing.** Browse the working tree, stage and unstage files
  or hunks, and commit with a message suggested by a local model. Message
  language, subject length and prompt are configurable.
- **Branches, commits and worktrees.** List, switch, create, rename, merge and
  delete branches; browse history; work in several checkouts at once.
- **Push and release flow.** Push with remote tracking, and run a guarded
  release flow that refuses to act on protected branches.
- **Environments as a pipeline.** Map branches to deploy environments and see
  how far each one is behind, with explicit promotions drawn as the route a
  change travels.
- **GitHub.** Through the GitHub CLI (`gh`): list the checkout's pull requests
  with their checks and reviews, check one out in place or in a new worktree,
  approve, request changes, comment, merge (method, branch cleanup,
  auto-merge), close or reopen, and open a new pull request for the current
  branch — prefilled from its commits, or from the PR text review mode wrote.
  A second tab lists your repositories, or those of any organization you
  belong to, to clone into the workspace. Needs `gh auth login` once.
- **Assisted review.** Review the current branch against `main` in a
  finding-by-finding tree, either from the local model or from a Claude Code
  session that reads the checkout itself. `lg review` prints the same review
  and exits.
- **Guided review.** Walk the branch against `main` (`V`; on `main` itself,
  your uncommitted changes), or a pull request (`v` in GitHub), one hunk at a
  time. `?` inside it lists every key. Each step comes with the model's read of
  it, and the next steps are read ahead so the walk does not wait. Move a line
  cursor, pin notes to lines, ask follow-up questions, and fix things as you
  go: `e` opens `$EDITOR` at the line with lg suspended, and `f` has Claude
  Code make the change in place. Edits show up in the walk at once. For a pull
  request, `s` submits every note as inline comments in one review (comment,
  approve or request changes). Progress and notes live in the git directory,
  so leaving and coming back resumes where you stopped.
- **Conflict assistance.** An in-app conflict editor with model-suggested
  resolutions.
- **Embedded sessions.** Run a coding agent (claude, codex or pi) or your own
  shell inside `lg`: one agent of each kind per checkout, and as many shells as
  you like. Sessions keep running while you look at something else, and `lg`
  shows what the agent is doing.
- **Sandboxing.** Agents and dev tools can run inside a macOS Seatbelt profile
  with a filtering network proxy, via `lg sandbox ...`.
- **Scoped configuration.** Settings live at user, repository or worktree scope
  and are editable from a Settings screen or with `lg config`.
- **Some fun.** While the model is thinking, the commit panel draws an animated
  scene. Animations can be turned off.

## Usage

```
lg              Start the interactive TUI in the current repository
lg config ...   Show, edit, export or import scoped configuration
lg sandbox ...  Run the bundled Terrarium sandbox tools
lg review       Print an assisted review of this branch against main, then exit
lg --help       Show this message
lg --version    Show the version
```

Press `?` inside the TUI for the full key reference. `1`–`4` switch panes,
`c` commits, `p`/`P` push, `E` opens the environments pipeline, `H` opens
GitHub, `V` starts a guided review, `w` opens worktrees and `,` opens settings.

## LLM setup

Commit messages and reviews are requested from an OpenAI-compatible chat
endpoint. The default is a local server at `http://localhost:8000/v1/chat/completions`.
Change the model and endpoint under `[models]` in the settings screen or with
`lg config`. Nothing is sent anywhere unless you point it at a remote endpoint.

To have Claude answer everything instead, set `models.provider` to `claude`
(Settings → models, or `LG_LLM_PROVIDER=claude` for one run). Every request then
runs through the `claude` CLI with its tools, MCP servers and settings files
turned off, using your Claude Code login. `models.claude_model` picks `sonnet`,
`opus`, `haiku` or a full model id; empty uses the CLI's default.

## Installing

```
just install    # builds and installs into $PREFIX/bin (default ~/.cargo/bin)
```

