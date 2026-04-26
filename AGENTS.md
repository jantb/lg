# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 crate named `lg`. The executable entry point is `src/main.rs`, which delegates to `lg::app::App`. Shared modules live under `src/`: `app.rs` coordinates runtime behavior, `git.rs` wraps Git operations, `state.rs` holds application state, `ui.rs` handles terminal rendering, and `config.rs` / `ollama.rs` cover configuration and model integration. Panel-specific TUI code is grouped in `src/panel/`, with one file per panel such as `files.rs`, `commit.rs`, and `push.rs`. The auxiliary harness binary is `src/bin/harness.rs`. Integration tests live in `tests/`.

## Build, Test, and Development Commands

Use the Makefile for the standard workflow:

- `make check`: runs `cargo check --all-targets`.
- `make test`: runs `cargo test --all-targets`.
- `make clippy`: runs Clippy for all targets with warnings denied.
- `make fmt`: formats Rust code with `cargo fmt`.
- `make fmt-check`: verifies formatting without changing files.
- `make all`: runs check, tests, clippy, and formatting checks.
- `make harness`: runs the harness binary with `cargo run --bin harness`.
- `make release` and `make install`: build and install the optimized `lg` binary.

## Coding Style & Naming Conventions

Follow idiomatic Rust and the repository's `rustfmt` defaults. Use four-space indentation, `snake_case` for functions and modules, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Keep UI panel logic in `src/panel/` and avoid mixing rendering, state mutation, and Git command logic in one module unless the existing pattern already does so.

## Testing Guidelines

Tests use Rust's built-in test framework plus `tempfile` for hermetic repositories. Add integration coverage under `tests/` for user-visible Git and TUI flows; name tests after the behavior being protected, for example `end_to_end_commit_flow` or `mm_file_appears_in_both_lists`. Run `make test` before submitting behavior changes, and run `make all` before larger changes.

## Commit & Pull Request Guidelines

This repository currently has no committed history, so there is no established local commit convention. Use clear, imperative commit subjects such as `Add push panel validation`; Conventional Commit prefixes like `feat:` or `fix:` are acceptable when helpful. Pull requests should describe the user-facing change, list verification commands run, link related issues, and include terminal screenshots or recordings for TUI layout changes.

## Agent-Specific Instructions

Do not edit generated build output in `target/`. Preserve the managed Terrarium guidance in `CLAUDE.md`. Treat compiler warnings and Clippy findings as blockers.
