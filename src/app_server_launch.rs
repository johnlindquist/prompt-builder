use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::process::Child;
use std::process::ChildStdin;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use serde_json::json;
use serde_json::Value;

use crate::cli::LaunchConfig;
use crate::codex_spawn;

pub fn named_thread_argv(config: &LaunchConfig, prompt: &str, thread_name: &str) -> Vec<String> {
    let _ = thread_name;
    codex_spawn::resume_argv(config, "<created-thread-id>", prompt)
}

pub fn print_command(config: &LaunchConfig, prompt: &str, thread_name: &str) {
    let env = codex_spawn::env_prefix(&config.env);
    let app_server = [config.codex_bin.as_str(), "app-server", "--stdio"]
        .into_iter()
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let resume = codex_spawn::resume_argv(config, "<created-thread-id>", prompt)
        .into_iter()
        .map(|arg| shell_quote(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "{env}{app_server}  # JSON-RPC: thread/start; thread/name/set {}; then {env}{resume}",
        shell_quote(thread_name)
    );
}

pub fn launch(config: &LaunchConfig, prompt: &str, thread_name: &str) -> anyhow::Result<()> {
    let thread_id = create_named_thread(config, thread_name)?;
    codex_spawn::launch_resume(config, &thread_id, prompt, Some(thread_name))
}

fn create_named_thread(config: &LaunchConfig, thread_name: &str) -> anyhow::Result<String> {
    let mut server = AppServerClient::spawn(config)?;
    server.initialize()?;
    let thread_id = server.thread_start(config)?;
    server.thread_set_name(&thread_id, thread_name)?;
    server.shutdown();
    Ok(thread_id)
}

struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl AppServerClient {
    fn spawn(config: &LaunchConfig) -> anyhow::Result<Self> {
        let mut child = Command::new(&config.codex_bin)
            .arg("app-server")
            .arg("--stdio")
            .envs(config.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to launch {} app-server", config.codex_bin))?;
        let stdin = child
            .stdin
            .take()
            .context("failed to open app-server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to open app-server stdout")?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    fn initialize(&mut self) -> anyhow::Result<()> {
        self.request(json!({
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "prompt-builder",
                    "title": "prompt-builder",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }))?;
        self.notify(json!({ "method": "initialized" }))
    }

    fn thread_start(&mut self, config: &LaunchConfig) -> anyhow::Result<String> {
        let mut params = json!({
            "cwd": config.cwd.to_string_lossy(),
            "approvalPolicy": "never",
            "sandbox": "danger-full-access",
            "threadSource": "local"
        });
        if let Some(model) = &config.model {
            params["model"] = json!(model);
        }

        let response = self.request(json!({
            "method": "thread/start",
            "params": params
        }))?;
        response
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("thread/start response did not include thread.id")
    }

    fn thread_set_name(&mut self, thread_id: &str, thread_name: &str) -> anyhow::Result<()> {
        self.request(json!({
            "method": "thread/name/set",
            "params": {
                "threadId": thread_id,
                "name": thread_name
            }
        }))?;
        Ok(())
    }

    fn request(&mut self, mut value: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        value["id"] = json!(id);
        self.write_json(&value)?;

        loop {
            let message = self.read_json()?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                anyhow::bail!("app-server request failed: {error}");
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn notify(&mut self, value: Value) -> anyhow::Result<()> {
        self.write_json(&value)
    }

    fn write_json(&mut self, value: &Value) -> anyhow::Result<()> {
        serde_json::to_writer(&mut self.stdin, value)?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_json(&mut self) -> anyhow::Result<Value> {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line)?;
        if bytes == 0 {
            anyhow::bail!("app-server exited before responding");
        }
        serde_json::from_str(line.trim_end()).context("failed to parse app-server JSON response")
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn named_thread_argv_documents_app_server_flow_and_resume() {
        let config = LaunchConfig {
            codex_bin: "codex".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            profile: None,
            model: None,
            config: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
        };

        assert_eq!(
            named_thread_argv(&config, "fix", "project:Fix"),
            vec![
                "codex".to_string(),
                "resume".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "-C".to_string(),
                "/tmp/project".to_string(),
                "--".to_string(),
                "<created-thread-id>".to_string(),
                "fix".to_string(),
            ]
        );
    }
}
