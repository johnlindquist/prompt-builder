# prompt-builder

Terminal prompt builder for composing a prompt before handing it to Codex.

`prompt-builder` opens a small Ratatui composer with skill lookup, Codex-like
paste handling, optional conversation naming, and a generic handoff mode for
shell workflows.

## Install

Requirements:

- Rust toolchain from `rustup`; this repo pins Rust `1.95.0`.
- Codex CLI on `PATH` for normal submission.

Private GitHub install:

```sh
cargo install --git ssh://git@github.com/johnlindquist/prompt-builder.git --locked
```

Public HTTPS install, once the repo is public:

```sh
cargo install --git https://github.com/johnlindquist/prompt-builder.git --locked
```

Local development install:

```sh
git clone git@github.com:johnlindquist/prompt-builder.git
cd prompt-builder
cargo install --path . --locked
```

The installed binary is `prompt-builder`. If you want the short local command
`p`, add an alias or symlink:

```sh
alias p=prompt-builder
ln -sf ~/.cargo/bin/prompt-builder ~/.cargo/bin/p
```

## Safety

By default, submitted prompts launch:

```sh
codex --dangerously-bypass-approvals-and-sandbox -C <cwd> <prompt>
```

Only use the default launcher in directories where you are comfortable letting
Codex modify files and run commands under that mode.

Before launching Codex, inspect what would run:

```sh
prompt-builder --dry-run --print-command "hello"
PROMPT_BUILDER_CODEX_BIN=/bin/echo prompt-builder --submit "hello"
```

## Quick Start

Open the interactive composer:

```sh
prompt-builder
```

Prefill the composer:

```sh
prompt-builder "fix the focus in the main window"
```

Submit noninteractively:

```sh
printf 'fix the focus in the main window\n' | prompt-builder --submit
```

Print the composed prompt for shell pipelines:

```sh
printf 'explain this\n' | prompt-builder --stdin --print-prompt
```

## Conversation Name

Use `--name` to prefill or submit an optional conversation name:

```sh
prompt-builder --name "Fix focused fork"
prompt-builder --submit --dry-run --name "Fix focused fork" "continue"
```

The TUI starts in a single-line Name field above the Prompt field. Press Tab to
move between fields. Submitted names are prefixed with the cwd basename as
`<cwd-name>:<name>` and exported to child commands as `CODEX_THREAD_NAME`.

## Handoff Mode

Use handoff mode when `prompt-builder` should compose first, then call another
command with the submitted prompt as the final argv element.

```sh
prompt-builder --handoff-command xfc
prompt-builder --handoff-command x --handoff-arg fork --handoff-arg=--last
prompt-builder --submit --dry-run --handoff-command /bin/echo "hello"
```

The prompt is passed with `Command::arg`, not through shell eval.

## Codex Options

Pass Codex behavior options through:

```sh
prompt-builder \
  -C ~/dev/codex \
  --profile fixit \
  --model gpt-5.5 \
  -c 'instructions="Be concise."' \
  -c 'developer_instructions="Inspect source first."'
```

Shorthands:

```sh
prompt-builder --instructions "Base instructions"
prompt-builder --developer-instructions "Developer instructions"
```

Example zsh function:

```sh
fixit() {
  prompt-builder \
    -C "$PWD" \
    --developer-instructions "You're an expert debugger who always starts with the \$fusion skill. The user will pass a terse bug; investigate the behavior and fix it."
}
```

## Update

From GitHub:

```sh
cargo install --git ssh://git@github.com/johnlindquist/prompt-builder.git --locked --force
```

From a local checkout:

```sh
cargo install --path . --locked --force
```

## Development

```sh
cargo fmt -- --check
cargo test
cargo clippy --all-targets --all-features
cargo install --path . --locked
```

This project intentionally keeps its TUI small. It mirrors Codex's paste and
submit behavior locally instead of depending on Codex's full TUI crate.

The current lockfile pins patched `crossterm` and `ratatui` revisions. Keep
using `--locked` for installs so the terminal behavior is reproducible.
