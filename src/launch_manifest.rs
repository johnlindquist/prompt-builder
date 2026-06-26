use crate::app_server_launch;
use crate::cli::enabled_option_argv;
use crate::cli::HandoffConfig;
use crate::cli::LaunchConfig;
use crate::cli::ToggleOption;
use crate::codex_spawn;
use crate::handoff;

pub fn codex_launch_json(config: &LaunchConfig, prompt: &str, thread_name: Option<&str>) -> String {
    render_manifest(
        "codex",
        &codex_spawn::codex_argv(config, prompt),
        prompt,
        thread_name,
        None,
    )
}

pub fn default_launch_json(
    config: &LaunchConfig,
    prompt: &str,
    thread_name: Option<&str>,
) -> String {
    match thread_name {
        Some(thread_name) => named_thread_launch_json(config, prompt, thread_name),
        None => codex_launch_json(config, prompt, None),
    }
}

pub fn named_thread_launch_json(config: &LaunchConfig, prompt: &str, thread_name: &str) -> String {
    render_manifest(
        "app-server-named-thread",
        &app_server_launch::named_thread_argv(config, prompt, thread_name),
        prompt,
        Some(thread_name),
        Some(("app_server", named_thread_json(thread_name))),
    )
}

pub fn handoff_launch_json(
    config: &HandoffConfig,
    prompt: &str,
    thread_name: Option<&str>,
    toggle_options: &[ToggleOption],
) -> String {
    let enabled_argv = enabled_option_argv(toggle_options);
    render_manifest(
        "handoff",
        &handoff::handoff_argv(config, prompt, &enabled_argv),
        prompt,
        thread_name,
        Some(("handoff", handoff_json(config, toggle_options))),
    )
}

fn render_manifest(
    mode: &str,
    argv: &[String],
    prompt: &str,
    thread_name: Option<&str>,
    extra_json: Option<(&str, String)>,
) -> String {
    let command = argv.first().map(String::as_str).unwrap_or_default();
    let extra_field = extra_json
        .map(|(key, json)| format!(",\n  \"{key}\": {json}"))
        .unwrap_or_default();
    format!(
        "{{\n  \"schema_version\": 1,\n  \"mode\": {},\n  \"command\": {},\n  \"argv\": {},\n  \"env\": {},\n  \"prompt\": {}{}\n}}",
        json_string(mode),
        json_string(command),
        json_string_array(argv),
        env_json(thread_name),
        json_string(prompt),
        extra_field
    )
}

fn env_json(thread_name: Option<&str>) -> String {
    match thread_name {
        Some(thread_name) => format!("{{\"CODEX_THREAD_NAME\": {}}}", json_string(thread_name)),
        None => "{}".to_string(),
    }
}

fn toggle_options_json(options: &[ToggleOption]) -> String {
    let items = options
        .iter()
        .filter(|option| option.enabled)
        .map(|option| {
            format!(
                "{{\"label\": {}, \"argv\": {}}}",
                json_string(&option.label),
                json_string_array(&option.argv)
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn handoff_json(config: &HandoffConfig, options: &[ToggleOption]) -> String {
    format!(
        "{{\"command\": {}, \"args\": {}, \"enabled_launch_options\": {}}}",
        json_string(&config.command),
        json_string_array(&config.args),
        toggle_options_json(options)
    )
}

fn named_thread_json(thread_name: &str) -> String {
    format!(
        "{{\"transport\": \"stdio\", \"create\": \"thread/start\", \"name_request\": \"thread/name/set\", \"thread_name\": {}}}",
        json_string(thread_name)
    )
}

fn json_string_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>();
    format!("[{}]", items.join(", "))
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn codex_json_uses_raw_argv_and_escapes_prompt() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            profile: Some("fixit".to_string()),
            model: None,
            config: vec!["developer_instructions=debug carefully".to_string()],
        };

        let json = codex_launch_json(&config, "fix \"x\"\n\t诶\u{1}", Some("project:Fix"));

        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"mode\": \"codex\""));
        assert!(json.contains("\"command\": \"codex\""));
        assert!(json.contains("\"argv\": [\"codex\", \"--dangerously-bypass-approvals-and-sandbox\", \"-C\", \"/tmp/project\", \"--profile\", \"fixit\", \"-c\", \"developer_instructions=debug carefully\", \"--\", \"fix \\\"x\\\"\\n\\t诶\\u0001\"]"));
        assert!(json.contains("\"CODEX_THREAD_NAME\": \"project:Fix\""));
    }

    #[test]
    fn handoff_json_includes_enabled_toggle_metadata() {
        let config = HandoffConfig {
            command: "x".to_string(),
            args: vec!["resume".to_string()],
        };
        let options = vec![
            ToggleOption {
                label: "compact".to_string(),
                argv: vec!["--compact".to_string()],
                enabled: true,
            },
            ToggleOption {
                label: "trace".to_string(),
                argv: vec!["--trace".to_string()],
                enabled: false,
            },
        ];

        let json = handoff_launch_json(&config, "continue", None, &options);

        assert!(json.contains("\"mode\": \"handoff\""));
        assert!(json.contains("\"argv\": [\"x\", \"resume\", \"--compact\", \"continue\"]"));
        assert!(json.contains("\"handoff\": {\"command\": \"x\", \"args\": [\"resume\"], \"enabled_launch_options\": [{\"label\": \"compact\", \"argv\": [\"--compact\"]}]}"));
        assert!(!json.contains("\"trace\""));
    }

    #[test]
    fn default_json_uses_app_server_mode_for_named_threads() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            profile: None,
            model: None,
            config: Vec::new(),
        };

        let json = default_launch_json(&config, "fix", Some("project:Fix"));

        assert!(json.contains("\"mode\": \"app-server-named-thread\""));
        assert!(json.contains("\"thread/name/set\""));
        assert!(json.contains("\"thread_name\": \"project:Fix\""));
        assert!(json.contains("\"app_server\""));
        assert!(json.contains("\"<created-thread-id>\""));
    }
}
