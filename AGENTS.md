# Repository Guidelines

## Project Structure & Module Organization

This is a Rust terminal prompt builder for handing composed prompts to Codex. Source lives in `src/`:

- `main.rs` wires CLI parsing, stdin handling, TUI launch, and submission.
- `cli.rs` defines flags, pass-through Codex options, and unit-tested config helpers.
- `app.rs`, `skill_popup.rs`, and `skills.rs` implement the Ratatui UI and skill loading.
- `codex_spawn.rs` builds and launches the Codex command.

`examples/fixit.zsh` contains a shell workflow example. There is no separate assets or integration test directory currently. The composer is implemented locally in `composer_input.rs`; changes around input behavior should preserve the paste and submit behavior users expect from `~/dev/codex`.

## Build, Test, and Development Commands

- `cargo run` starts the interactive TUI.
- `cargo run -- "prefilled prompt"` opens the TUI with initial text.
- `printf 'prompt\n' | cargo run -- --submit` submits noninteractively.
- `cargo run -- --dry-run --print-command "hello"` prints the Codex argv without launching it.
- `cargo test` runs unit tests.
- `cargo fmt` formats the crate using rustfmt.
- `cargo clippy --all-targets --all-features` checks for common Rust issues before review.

Use `PROMPT_BUILDER_CODEX_BIN=/bin/echo cargo run -- --submit "hello"` to verify command handoff without invoking Codex.

## Coding Style & Naming Conventions

Use Rust 2021 idioms and rustfmt defaults. Prefer small modules with clear ownership of behavior. Name files and modules with `snake_case`; types and enums with `PascalCase`; functions, variables, and CLI helper methods with `snake_case`. Return `anyhow::Result` at command boundaries where context-rich errors are useful.

## Testing Guidelines

Place narrow unit tests beside the code under `#[cfg(test)]`, as in `src/cli.rs`. Focus tests on argument transformation, command construction, and skill parsing logic. Use descriptive test names such as `shorthand_config_values_are_toml_strings`. Run `cargo test` before submitting changes.

## Commit & Pull Request Guidelines

Recent commits use short imperative messages, for example `Add skill autocomplete` and `Remove cwd label from header`. Follow that style: describe the change, not the process.

Pull requests should include a concise summary, relevant command output from `cargo test` or `cargo clippy`, and screenshots or terminal recordings for visible TUI changes. Mention any assumptions about the local Codex checkout or patched dependencies.
