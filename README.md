# prompt-builder

Rust terminal prompt builder for handing composed prompts to Codex.

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

Compose first, then hand the submitted prompt to another command:

```sh
cargo run -- --handoff-command xfc
cargo run -- --handoff-command x --handoff-arg fork --handoff-arg=--last
```

The composed prompt is passed as the final argv element, not through a shell string.

Name a handoff conversation:

```sh
cargo run -- --name "Fix focused fork" --handoff-command xfc
cargo run -- --submit --dry-run --name "Fix focused fork" --handoff-command xfc "continue"
```

The TUI starts in a single-line Name field above the prompt field. Press Tab to
move between Name and Prompt. Submitted names are prefixed with the cwd basename
as `<cwd-name>:<name>` and exported to handoff commands as `CODEX_THREAD_NAME`;
the prompt remains the final argv element.

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

This project intentionally keeps its TUI small. It mirrors Codex's paste and submit behavior locally instead of depending on Codex's full TUI crate.
