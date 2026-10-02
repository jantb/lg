# lg

`lg` is a terminal UI for git. It covers the everyday loop of staging,
committing, branching and pushing, and adds a local LLM for commit messages
and reviews, embedded coding-agent sessions, and a sandbox for running those
agents safely.

## Features

- **Staging and committing.** Browse the working tree, stage and unstage files
  or hunks, and commit with a message suggested by a local model. Message
  language, subject length and prompt are configurable.
- **Hunks.** A file's diff shows what is staged above what is not. In the diff
  pane `]`/`[` move between hunks (the current one is highlighted and named in
  the title), `space` stages an unstaged hunk or unstages a staged one, and `d`
  discards an unstaged hunk from the working tree after asking. A hunk whose
  file changed since it was shown is refused rather than applied.
- **Amend.** `Ctrl+T` in the commit modal, or `A` in Commits, puts the last
  commit's message in the editor; `Ctrl+S` then replaces that commit with the
  message and whatever is staged. It always asks first, and says so when the
  commit is already pushed and a force push will be needed.
- **Stash.** `s` in Files stashes every change, untracked files included, under
  an optional message; `S` lists the stash to apply (`space`), pop (`g`) or
  drop (`d`, after asking). Stashes lg took itself before a pull or a flow that
  did not finish are marked, so leftover work is easy to find.
- **Branches, commits and worktrees.** List, switch, create, rename, merge and
  delete branches; browse history; work in several checkouts at once. In
  Commits, `y` copies a commit's SHA (`y` in Branches copies the branch's) and
  `t` reverts a commit. A branch's log in the diff pane (select it in Branches,
  then `0`) has a commit cursor too, and `C` cherry-picks that commit onto the
  checked-out branch. Both ask first, and a conflict opens the conflict editor
  as a merge does.
- **Push and release flow.** Push with remote tracking, and run a guarded
  release flow that refuses to act on protected branches. When a branch has
  diverged from its remote, the push modal offers to merge upstream, or `f` to
  force-push with `--force-with-lease` after naming the remote branch and the
  commits it would overwrite. lg never offers a plain `--force`, nor any force
  push of a protected or deploy branch.
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
`0` focuses the diff, `c` commits, `p`/`P` pull and push, `E` opens the
environments pipeline, `H` opens GitHub, `V` starts a guided review, `w` opens
worktrees and `,` opens settings. In the diff pane `]`/`[`, `space` and `d`
work on hunks; in Files `s`/`S` stash; in Commits `y`, `t` and `A` copy,
revert and amend. `/` filters Files, Branches and Commits as you type (`Esc`
clears it). `:` searches every documented action by what it does and runs it
where it can, and `/` inside `?` filters the key reference. When the status
bar cuts an error short it says `! details`: `!` shows the whole of the last
error, scrollable and copyable with `y`.

## LLM setup

Commit messages and reviews are requested from an OpenAI-compatible chat
endpoint. The default is a local server at `http://localhost:8000/v1/chat/completions`.
Change the model and endpoint under `[models]` in the settings screen or with
`lg config`. Nothing is sent anywhere unless you point it at a remote endpoint.
If nothing answers there, lg says so once, names the endpoint, and stops
asking on its own for the rest of the session; `Ctrl+R` in the commit modal
still asks.

To have Claude answer everything instead, set `models.provider` to `claude`
(Settings → models, or `LG_LLM_PROVIDER=claude` for one run). Every request then
runs through the `claude` CLI with its tools, MCP servers and settings files
turned off, using your Claude Code login. `models.claude_model` picks `sonnet`,
`opus`, `haiku` or a full model id; empty uses the CLI's default.

## Installing

```
just install    # builds and installs into $PREFIX/bin (default ~/.cargo/bin)
```

