// Synced from ~/dev/codex/codex-rs/tui/src/slash_command.rs and
// bottom_pane/command_popup.rs on 2026-06-25.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub alias: bool,
    pub hidden_from_popup: bool,
    pub supports_inline_args: bool,
}

pub const SLASH_COMMANDS: &[SlashCommand] = &[
    command(
        "model",
        "choose what model and reasoning effort to use",
        false,
    ),
    command(
        "ide",
        "include current selection, open files, and other context from your IDE",
        true,
    ),
    command("permissions", "choose what Codex is allowed to do", false),
    command("keymap", "remap TUI shortcuts", true),
    command("vim", "toggle Vim mode for the composer", false),
    command(
        "setup-default-sandbox",
        "set up elevated agent sandbox",
        false,
    ),
    command(
        "sandbox-add-read-dir",
        "let sandbox read a directory: /sandbox-add-read-dir <absolute_path>",
        true,
    ),
    command("experimental", "toggle experimental features", false),
    command(
        "approve",
        "approve one retry of a recent auto-review denial",
        false,
    ),
    command("memories", "configure memory use and generation", false),
    command(
        "skills",
        "use skills to improve how Codex performs specific tasks",
        false,
    ),
    command(
        "import",
        "import setup, this project, and recent chats from Claude Code",
        false,
    ),
    command("hooks", "view and manage lifecycle hooks", false),
    command("review", "review my current changes and find issues", true),
    command("rename", "rename the current thread", true),
    command("new", "start a new chat during a conversation", false),
    command("archive", "archive this session and exit", false),
    command("delete", "permanently delete this session and exit", false),
    command("resume", "resume a saved chat", true),
    command("fork", "fork the current chat", false),
    command("app", "continue this session in Codex Desktop", false),
    command(
        "init",
        "create an AGENTS.md file with instructions for Codex",
        false,
    ),
    command(
        "compact",
        "summarize conversation to prevent hitting the context limit",
        false,
    ),
    command("plan", "switch to Plan mode", true),
    command("goal", "set or view the goal for a long-running task", true),
    command("agent", "switch the active agent thread", false),
    command(
        "side",
        "start a side conversation in an ephemeral fork",
        true,
    ),
    alias(
        "btw",
        "start a side conversation in an ephemeral fork",
        true,
    ),
    command("copy", "copy last response as markdown", false),
    command(
        "raw",
        "toggle raw scrollback mode for copy-friendly terminal selection",
        true,
    ),
    command("diff", "show git diff (including untracked files)", false),
    command("mention", "mention a file", false),
    command(
        "status",
        "show current session configuration and token usage",
        false,
    ),
    command("usage", "show account usage activity", true),
    hidden(
        "debug-config",
        "show config layers and requirement sources for debugging",
        false,
    ),
    command(
        "title",
        "configure which items appear in the terminal title",
        false,
    ),
    command(
        "statusline",
        "configure which items appear in the status line",
        false,
    ),
    command("theme", "choose a syntax highlighting theme", false),
    command("pets", "choose or hide the terminal pet", true),
    alias("pet", "choose or hide the terminal pet", true),
    command(
        "mcp",
        "list configured MCP tools; use /mcp verbose for details",
        true,
    ),
    hidden("apps", "manage apps", false),
    command("plugins", "browse plugins", false),
    command("logout", "log out of Codex", false),
    alias("quit", "exit Codex", false),
    command("exit", "exit Codex", false),
    command("feedback", "send logs to maintainers", false),
    command("rollout", "print the rollout file path", false),
    command("ps", "list background terminals", false),
    command("stop", "stop all background terminals", false),
    alias("clean", "stop all background terminals", false),
    command("clear", "clear the terminal and start a new chat", false),
    command(
        "personality",
        "choose a communication style for Codex",
        false,
    ),
    command("test-approval", "test approval request", false),
    command("subagents", "switch the active agent thread", false),
    hidden("debug-m-drop", "DO NOT USE", false),
    hidden("debug-m-update", "DO NOT USE", false),
];

pub fn popup_matches(query: &str) -> Vec<&'static SlashCommand> {
    let filter = query.trim().to_ascii_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|command| {
            if command.hidden_from_popup {
                return false;
            }
            if filter.is_empty() {
                return !command.alias;
            }
            command.name == filter || command.name.starts_with(&filter)
        })
        .collect()
}

pub fn has_command_prefix(query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || SLASH_COMMANDS
            .iter()
            .any(|command| !command.hidden_from_popup && fuzzy_match(command.name, &query))
}

fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let mut chars = haystack.chars().flat_map(char::to_lowercase);
    needle
        .chars()
        .flat_map(char::to_lowercase)
        .all(|needle_char| chars.any(|candidate| candidate == needle_char))
}

const fn command(
    name: &'static str,
    description: &'static str,
    supports_inline_args: bool,
) -> SlashCommand {
    SlashCommand {
        name,
        description,
        alias: false,
        hidden_from_popup: false,
        supports_inline_args,
    }
}

const fn alias(
    name: &'static str,
    description: &'static str,
    supports_inline_args: bool,
) -> SlashCommand {
    SlashCommand {
        name,
        description,
        alias: true,
        hidden_from_popup: false,
        supports_inline_args,
    }
}

const fn hidden(
    name: &'static str,
    description: &'static str,
    supports_inline_args: bool,
) -> SlashCommand {
    SlashCommand {
        name,
        description,
        alias: false,
        hidden_from_popup: true,
        supports_inline_args,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m_prefix_matches_codex_order() {
        let names = popup_matches("m")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["model", "memories", "mention", "mcp"]);
    }

    #[test]
    fn default_popup_hides_aliases_and_hidden_commands() {
        let names = popup_matches("")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"exit"));
        assert!(!names.contains(&"quit"));
        assert!(!names.contains(&"btw"));
        assert!(!names.contains(&"debug-config"));
        assert!(!names.contains(&"apps"));
    }

    #[test]
    fn typed_alias_can_match() {
        let names = popup_matches("bt")
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["btw"]);
    }

    #[test]
    fn activation_uses_codex_fuzzy_match() {
        assert!(has_command_prefix("ac"));
        assert!(!has_command_prefix("zzz"));
    }
}
