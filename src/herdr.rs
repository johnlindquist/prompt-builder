use std::path::PathBuf;

const HERDR_ENV_VAR: &str = "HERDR_ENV";
const HERDR_ENV_VALUE: &str = "1";
const HERDR_SOCKET_PATH_VAR: &str = "HERDR_SOCKET_PATH";
const HERDR_PANE_ID_VAR: &str = "HERDR_PANE_ID";

#[derive(Clone, Debug, PartialEq, Eq)]
struct HerdrContext {
    socket_path: PathBuf,
    pane_id: String,
}

fn context_from_env() -> Option<HerdrContext> {
    let marker = std::env::var(HERDR_ENV_VAR).ok();
    let socket_path = std::env::var_os(HERDR_SOCKET_PATH_VAR);
    let pane_id = std::env::var(HERDR_PANE_ID_VAR).ok();
    context_from_parts(
        marker.as_deref(),
        socket_path.as_deref(),
        pane_id.as_deref(),
    )
}

fn context_from_parts(
    marker: Option<&str>,
    socket_path: Option<&std::ffi::OsStr>,
    pane_id: Option<&str>,
) -> Option<HerdrContext> {
    if marker != Some(HERDR_ENV_VALUE) {
        return None;
    }
    let socket_path = socket_path.filter(|value| !value.is_empty())?;
    let pane_id = pane_id.map(str::trim).filter(|value| !value.is_empty())?;
    Some(HerdrContext {
        socket_path: PathBuf::from(socket_path),
        pane_id: pane_id.to_string(),
    })
}

/// Returns `Ok(false)` when no Herdr action applies to the current process.
pub(crate) fn rename_current_tab(label: &str) -> anyhow::Result<bool> {
    let label = label.trim();
    if label.is_empty() {
        return Ok(false);
    }
    let Some(context) = context_from_env() else {
        return Ok(false);
    };

    #[cfg(unix)]
    {
        rename_current_tab_with(&context, label)?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        let _ = (context, label);
        Ok(false)
    }
}

#[cfg(unix)]
fn rename_current_tab_with(context: &HerdrContext, label: &str) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use serde_json::json;

    let prefix = format!("prompt-builder:{}", std::process::id());
    let pane = request(
        &context.socket_path,
        &format!("{prefix}:pane-get"),
        "pane.get",
        json!({ "pane_id": context.pane_id.as_str() }),
    )?;
    let tab_id = pane
        .get("pane")
        .and_then(|pane| pane.get("tab_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|tab_id| !tab_id.is_empty())
        .context("Herdr pane.get response did not include pane.tab_id")?;
    request(
        &context.socket_path,
        &format!("{prefix}:tab-rename"),
        "tab.rename",
        json!({ "tab_id": tab_id, "label": label }),
    )?;
    Ok(())
}

#[cfg(unix)]
fn request(
    socket_path: &std::path::Path,
    id: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use anyhow::Context as _;
    use serde_json::json;
    use serde_json::Value;

    const IO_TIMEOUT: Duration = Duration::from_millis(500);

    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("connect to Herdr socket {}", socket_path.display()))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    serde_json::to_writer(
        &mut stream,
        &json!({ "id": id, "method": method, "params": params }),
    )
    .with_context(|| format!("encode Herdr {method} request"))?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    if reader.read_line(&mut line)? == 0 {
        anyhow::bail!("Herdr {method} returned an empty response");
    }
    let response: Value =
        serde_json::from_str(&line).with_context(|| format!("decode Herdr {method} response"))?;
    if response.get("id").and_then(Value::as_str) != Some(id) {
        anyhow::bail!("Herdr {method} returned a mismatched response id");
    }
    if let Some(error) = response.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        anyhow::bail!("Herdr {method} failed ({code}): {message}");
    }
    response
        .get("result")
        .cloned()
        .with_context(|| format!("Herdr {method} response did not include result"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn context_requires_all_three_herdr_signals() {
        let socket = OsStr::new("/tmp/herdr.sock");
        assert!(context_from_parts(Some("1"), Some(socket), Some("p_42")).is_some());
        assert!(context_from_parts(None, Some(socket), Some("p_42")).is_none());
        assert!(context_from_parts(Some("0"), Some(socket), Some("p_42")).is_none());
        assert!(context_from_parts(Some("1"), None, Some("p_42")).is_none());
        assert!(context_from_parts(Some("1"), Some(OsStr::new("")), Some("p_42")).is_none());
        assert!(context_from_parts(Some("1"), Some(socket), None).is_none());
        assert!(context_from_parts(Some("1"), Some(socket), Some("  ")).is_none());
    }

    #[cfg(unix)]
    fn temp_socket(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "prompt-builder-herdr-{tag}-{}.sock",
            std::process::id()
        ))
    }

    #[cfg(unix)]
    fn serve_responses(
        socket_path: PathBuf,
        rename_error: bool,
    ) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
        use std::io::BufRead;
        use std::io::BufReader;
        use std::io::Write;
        use std::os::unix::net::UnixListener;

        std::thread::spawn(move || {
            let _ = std::fs::remove_file(&socket_path);
            let listener = UnixListener::bind(&socket_path).expect("bind fake Herdr socket");
            let mut requests = Vec::new();
            for index in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut line = String::new();
                BufReader::new(stream.try_clone().expect("clone stream"))
                    .read_line(&mut line)
                    .expect("read request");
                let request: serde_json::Value =
                    serde_json::from_str(&line).expect("parse request");
                let id = request["id"].as_str().expect("request id");
                let response = if index == 0 {
                    serde_json::json!({
                        "id": id,
                        "result": { "pane": { "tab_id": "w_test:3" } }
                    })
                } else if rename_error {
                    serde_json::json!({
                        "id": id,
                        "error": { "code": "tab_not_found", "message": "tab disappeared" }
                    })
                } else {
                    serde_json::json!({ "id": id, "result": { "tab": {} } })
                };
                serde_json::to_writer(&mut stream, &response).expect("write response");
                stream.write_all(b"\n").expect("finish response");
                requests.push(request);
            }
            requests
        })
    }

    #[cfg(unix)]
    #[test]
    fn resolves_pane_then_renames_tab_with_exact_label() {
        let socket_path = temp_socket("success");
        let server = serve_responses(socket_path.clone(), false);
        while !socket_path.exists() {
            std::thread::yield_now();
        }
        let context = HerdrContext {
            socket_path: socket_path.clone(),
            pane_id: "p_42".to_string(),
        };
        let label = "Fix 'parser' $(not-a-shell) — phase 2";

        rename_current_tab_with(&context, label).expect("rename succeeds");
        let requests = server.join().expect("server exits");

        assert_eq!(requests[0]["method"], "pane.get");
        assert_eq!(requests[0]["params"]["pane_id"], "p_42");
        assert_eq!(requests[1]["method"], "tab.rename");
        assert_eq!(requests[1]["params"]["tab_id"], "w_test:3");
        assert_eq!(requests[1]["params"]["label"], label);
        let _ = std::fs::remove_file(socket_path);
    }

    #[cfg(unix)]
    #[test]
    fn returns_herdr_api_errors() {
        let socket_path = temp_socket("error");
        let server = serve_responses(socket_path.clone(), true);
        while !socket_path.exists() {
            std::thread::yield_now();
        }
        let context = HerdrContext {
            socket_path: socket_path.clone(),
            pane_id: "p_42".to_string(),
        };

        let error = rename_current_tab_with(&context, "Fix parser").expect_err("rename fails");
        assert!(error.to_string().contains("tab_not_found"));
        let _ = server.join().expect("server exits");
        let _ = std::fs::remove_file(socket_path);
    }
}
