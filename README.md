# prompt-builder

Terminal prompt builder for composing a prompt before handing it to Pi, Codex,
or Claude Code.

`prompt-builder` opens a small Ratatui composer with skill lookup, Codex-like
paste handling, optional conversation naming, launch targets (profiles) for
multiple Pi, Codex, and Claude Code profiles, and a generic handoff mode for
shell workflows.

## Install

Requirements:

- Rust toolchain from `rustup`; this repo pins Rust `1.95.0`.
- Pi CLI on `PATH` for the default submission target.
- Codex and Claude Code CLIs are optional when using those targets.

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

By default, submitted prompts launch Pi with its normal interactive behavior:

```sh
pi <prompt>
```

Pi does not add a permission-bypass flag; use Pi's own isolation, trust, and
extension settings for the safety policy you want. Explicit Codex targets launch
with `--dangerously-bypass-approvals-and-sandbox`, and Claude targets launch:

```sh
claude --dangerously-skip-permissions -- <prompt>
```

Only use these launchers in directories where you are comfortable letting the
selected agent modify files and run commands.

Before launching an agent, inspect what would run:

```sh
prompt-builder --dry-run --print-command "hello"
PROMPT_BUILDER_PI_BIN=/bin/echo prompt-builder --submit "hello"
```

For scripts that should avoid parsing shell-quoted command text, print the raw
launch manifest:

```sh
prompt-builder --print-launch-json "hello"
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

## Composer Shortcuts

| Key | Action |
| --- | --- |
| `Enter` | Submit the prompt |
| `Shift+Enter` / `Ctrl+J` | Insert newline |
| `Tab` | Move focus Name → Prompt → target → options |
| `Space` / `←` / `→` | Cycle the focused target selector |
| `Enter` on `Target ‹name›` | Open the target manager popup |
| `Ctrl+G` on `Target ‹name›` | Edit `targets.toml` in `$VISUAL`/`$EDITOR` |
| `@` | Fuzzy file search popup (inserts the path) |
| `/` | Slash command popup (first line only) |
| `$` | Skill mention popup |
| `↑` / `↓` | Recall prompt history when the composer is empty |
| `Ctrl+R` | Reverse-search prompt history (type to filter, `Ctrl+R` for older, `Enter` accept, `Esc` cancel) |
| `Ctrl+G` | Edit the prompt in `$VISUAL`/`$EDITOR` |
| `Ctrl+C` | Clear the focused field, then quit (cleared drafts stay in history) |
| `Ctrl+D` | Quit when the composer is empty |

Prompt history is cross-session: `prompt-builder` reads Codex's own
`~/.codex/history.jsonl` alongside its `~/.prompt-builder/history.jsonl`, so
prompts submitted in either tool are recallable. Long lines wrap at word
boundaries, pastes over 1000 chars collapse into a `[Pasted Content N chars]`
placeholder that expands on submit, and rapid keystroke bursts (terminals
without bracketed paste) treat Enter as a pasted newline instead of submitting
early.

## Skills

Press `$` in the composer to select a skill mention. By default,
`prompt-builder` loads user skills from `~/.agents/skills` and project skills
from `.agents/skills` at `--cwd` and each ancestor through the nearest Git
repository root. Project directories are searched nearest-first, and a project
skill overrides a same-name user skill.

Each skill normally lives at `.agents/skills/<skill-name>/SKILL.md`. Passing one
or more `--skills-dir` values replaces the default user directory but does not
disable project-local discovery.

## Targets (Profiles)

A target names a launcher: Pi, Codex, or Claude Code, plus the binary,
environment variables, and default options it should use. Targets make it easy
to keep several agent profiles side by side (for example `PI_CODING_AGENT_DIR`,
`CODEX_HOME`, or `CLAUDE_CONFIG_DIR` variants).

Three targets are built in, in default order: `pi`, `codex`, and `claude`.
Define your own in
`~/.prompt-builder/targets.toml`, either by hand or with the `target`
subcommands:

```sh
prompt-builder target list
prompt-builder target add pi-work --kind pi \
  --env 'PI_CODING_AGENT_DIR=~/.pi-work/agent' \
  --model openai/gpt-4o \
  --arg=--thinking --arg high
prompt-builder target add egghead --kind codex \
  --env 'CODEX_HOME=~/.codex-egghead' \
  -c 'cli_auth_credentials_store="file"'
prompt-builder target add claude-second --kind claude \
  --env 'CLAUDE_CONFIG_DIR=~/.claude-second'
prompt-builder target remove claude-second
prompt-builder target path
```

The written file looks like:

```toml
[[targets]]
name = "pi"
kind = "pi"

[[targets]]
name = "codex"
kind = "codex"

[[targets]]
name = "egghead"
kind = "codex"
config = ['cli_auth_credentials_store="file"']

[targets.env]
CODEX_HOME = "~/.codex-egghead"

[[targets]]
name = "claude-second"
kind = "claude"

[targets.env]
CLAUDE_CONFIG_DIR = "~/.claude-second"
```

Each target supports `name`, `kind` (`pi`, `codex`, or `claude`), `bin`
(executable override), `env` (launch environment; `~/` expands to the home directory),
`model`, `args` (extra argv before the prompt), and for Codex targets
`profile` and `config` (repeatable `-c` overrides). CLI flags win over target
values; target `config` entries come first so CLI `-c` entries override them.

Pick a target with `--target`/`-t`, or in the TUI: a `Target ‹name›`
selector appears in the options row. Tab to it and press Space or the arrow
keys to cycle targets before submitting.

Press Enter on the selector to open the target manager popup: ↑/↓ browse,
Enter switches the active target, `r` reloads `targets.toml` from disk, and
`e` (or Ctrl+G, also directly from the selector) opens the whole targets file
in `$VISUAL`/`$EDITOR` to add, edit, remove, or reorder targets without
leaving the composer. Saved edits are validated before they are committed:
invalid TOML, duplicate names, or unknown fields never touch the live file,
and the manager shows the error while keeping your draft so `e` reopens
exactly what you wrote. Edits made through the editor are written back
byte-for-byte, so comments and formatting in `targets.toml` survive; the
`target add`/`target remove` CLI commands rewrite the file normalized and drop
comments. If another process changes `targets.toml` while the editor is open,
the commit is refused — press `r` to reload, then re-apply your edit.

```sh
prompt-builder --target claude "explain this repo"
prompt-builder -t egghead --submit --dry-run "fix it"
```

The first target in the file is the default. Fresh or empty configurations use
Pi first. Existing non-empty `targets.toml` files remain authoritative and are
not rewritten; add a `kind = "pi"` target and move it first to migrate the
default. `target add NAME` now defaults to Pi, so automation creating Codex
targets should pass `--kind codex` explicitly.

Pi receives `--model`, `env`, extra `args`, and conversation names through
`--name`. Because Pi has no `--` delimiter, prompts beginning with `-` or `@`
receive a transport-only leading space; the launch manifest preserves the
original prompt. For unknown Pi extension boolean flags in target `args`, prefer
`--flag=true` so the following prompt is not consumed as the flag's value.
Interactive Pi may ask whether to trust project-local resources; noninteractive
behavior follows Pi's configured fallback unless you explicitly add `--approve`
or `--no-approve`.

Claude targets currently support prompt handoff with `--model`, `env`, and extra `args`; Codex-only options
(`--profile`, `-c`, `--instructions`, `--developer-instructions`) and
conversation names are ignored with a warning. Fork/resume orchestration for
Claude Code sessions is future work — use handoff mode with your own wrapper
if you need it today.

## Conversation Name

Use `--name` to prefill or submit an optional conversation name:

```sh
prompt-builder --name "Fix focused fork"
prompt-builder --submit --dry-run --name "Fix focused fork" "continue"
```

The TUI starts in a single-line Name field above the Prompt field. Press Tab to
move between fields. Submitted names are prefixed with the cwd basename as
`<cwd-name>:<name>`.

On a real submission from a Herdr-managed pane, a nonblank Name also renames the
containing Herdr tab to the submitted display name. The target/session name
remains cwd-prefixed. Print-only and dry-run modes do not rename the tab.

For default non-handoff submissions with a Pi target, names are passed directly
as `--name <cwd-name>:<name>`. For a Codex target, named prompts create
the Codex thread through `codex app-server`, call `thread/name/set`, then
resume the named thread with the composed prompt. Unnamed prompts keep the direct `codex ... <prompt>`
launch path. Claude targets ignore names with a warning. Handoff submissions
export the name to child commands as
`CODEX_THREAD_NAME` so wrappers can apply their own naming behavior.

## Handoff Mode

Use handoff mode when `prompt-builder` should compose first, then call another
command with the submitted prompt as the final argv element.

```sh
prompt-builder --handoff-command xfc
prompt-builder --handoff-command x --handoff-arg fork --handoff-arg=--last
prompt-builder --submit --dry-run --handoff-command /bin/echo "hello"
```

The prompt is passed with `Command::arg`, not through shell eval.

Shortcut and wrapper flows can prefill bottom-bar launch toggles:

```sh
prompt-builder --handoff-command xfc --fork-from last --compact
prompt-builder --submit --dry-run --handoff-command /bin/echo \
  --fork-from 019edc7e-e0ed-7883-9e03-348c64771b1f --compact "continue"
prompt-builder --handoff-command my-wrapper \
  --launch-option "app-server trace=--trace-app-server"
```

In the TUI, these options appear below the prompt as checkbox-style toggles.
Press Tab to move Name → Prompt → each option, then Space to toggle the focused
option on or off. Enabled option argv is inserted after static handoff args and
before the final prompt argument. The wrapper owns any fork/compact app-server
orchestration; `prompt-builder` only composes, displays, and forwards the
selected option tokens. Use repeated `--launch-option "Label=arg,arg"` entries
for wrapper-specific options beyond `--fork-from` and `--compact`.

## Codex Options

Pass Codex behavior options through:

```sh
prompt-builder \
  --target codex \
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
    --target codex \
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
