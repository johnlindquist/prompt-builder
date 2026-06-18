# prompt-builder

Rust terminal prompt builder that uses Codex's real `codex_tui::ComposerInput`.

## Run

```sh
cd ~/dev/prompt-builder
cargo run
```

The TUI loads skills from `~/.agents/skills`, shows them next to the composer, and submits the composed prompt to:

```sh
codex --dangerously-bypass-approvals-and-sandbox -C <cwd> <prompt>
```

## Terminal Workflows

Prefill the composer:

```sh
cargo run -- "fix the focus in the main window"
```

Pipe into a noninteractive handoff:

```sh
printf 'fix the focus in the main window\n' | cargo run -- --submit
```

Preview without launching Codex:

```sh
cargo run -- --dry-run --print-command -C ~/dev/codex \
  --developer-instructions "You're an expert debugger who starts with the \$fusion skill." \
  "the focus in the main window"
```

Print a composed prompt for shell pipelines:

```sh
printf 'explain this\n' | cargo run -- --stdin --print-prompt
```

Use a harmless binary to verify argv handoff:

```sh
PROMPT_BUILDER_CODEX_BIN=/bin/echo cargo run -- --submit "hello"
```

## Codex Options

Pass Codex behavior options through:

```sh
cargo run -- \
  -C ~/dev/codex \
  --profile fixit \
  --model gpt-5.5 \
  -c 'instructions="Be concise."' \
  -c 'developer_instructions="Inspect source first."'
```

Shorthands:

```sh
cargo run -- --instructions "Base instructions"
cargo run -- --developer-instructions "Developer instructions"
```

Example zsh function:

```sh
fixit() {
  (cd ~/dev/prompt-builder && cargo run --quiet -- \
    -C "$PWD" \
    --developer-instructions "You're an expert debugger who always starts with the \$fusion skill. The user will pass a terse bug; investigate the behavior and fix it.")
}
```

## Notes

This project path-depends on `../codex/codex-rs/tui` and mirrors Codex's patched `ratatui`, `crossterm`, `tokio-tungstenite`, and `tungstenite` dependencies so the real composer types match.
