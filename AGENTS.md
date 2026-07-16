# Repository Guidelines

## Project Structure & Module Organization

This is a Rust terminal prompt builder for handing composed prompts to Pi, Codex, or Claude Code. Source lives in `src/`:

- `main.rs` wires CLI parsing, stdin handling, TUI launch, target dispatch, and submission.
- `cli.rs` defines flags, the `target` management subcommands, pass-through Codex options, and unit-tested config helpers.
- `targets.rs` loads and saves launch targets (profiles) from `~/.prompt-builder/targets.toml`; each target picks pi, codex, or claude plus bin/env/args overrides. Fresh configurations default to Pi.
- `app.rs`, `skill_popup.rs`, and `skills.rs` implement the Ratatui UI (including the options-row target selector) and skill loading; `target_popup.rs` is the modal target manager (browse/select, reload, and hand off to `$EDITOR` for TOML edits).
- `history.rs` merges cross-session prompt history from `~/.codex/history.jsonl` and `~/.prompt-builder/history.jsonl` for Up-arrow recall and Ctrl+R search.
- `external_editor.rs` runs `$VISUAL`/`$EDITOR` on the draft (Ctrl+G).
- `pi_spawn.rs`, `codex_spawn.rs`, and `claude_spawn.rs` build and launch commands for their respective targets.

`examples/fixit.zsh` contains a shell workflow example. There is no separate assets or integration test directory currently. The composer is implemented locally in `composer_input.rs`; changes around input behavior should preserve the paste and submit behavior users expect from `~/dev/codex`.

## Build, Test, and Development Commands

- `cargo run` starts the interactive TUI.
- `cargo run -- "prefilled prompt"` opens the TUI with initial text.
- `printf 'prompt\n' | cargo run -- --submit` submits noninteractively.
- `cargo run -- --dry-run --print-command "hello"` prints the selected target argv without launching it.
- `cargo test` runs unit tests.
- `cargo fmt` formats the crate using rustfmt.
- `cargo clippy --all-targets --all-features` checks for common Rust issues before review.

Use `PROMPT_BUILDER_PI_BIN=/bin/echo cargo run -- --submit "hello"` to verify the default command handoff without invoking Pi. Use an explicit `--target codex` or `--target claude` with the corresponding binary override for regression checks.

## Coding Style & Naming Conventions

Use Rust 2021 idioms and rustfmt defaults. Prefer small modules with clear ownership of behavior. Name files and modules with `snake_case`; types and enums with `PascalCase`; functions, variables, and CLI helper methods with `snake_case`. Return `anyhow::Result` at command boundaries where context-rich errors are useful.

## Testing Guidelines

Place narrow unit tests beside the code under `#[cfg(test)]`, as in `src/cli.rs`. Focus tests on argument transformation, command construction, and skill parsing logic. Use descriptive test names such as `shorthand_config_values_are_toml_strings`. Run `cargo test` before submitting changes.

## Commit & Pull Request Guidelines

Recent commits use short imperative messages, for example `Add skill autocomplete` and `Remove cwd label from header`. Follow that style: describe the change, not the process.

Pull requests should include a concise summary, relevant command output from `cargo test` or `cargo clippy`, and screenshots or terminal recordings for visible TUI changes. Mention any assumptions about the local Codex checkout or patched dependencies.
