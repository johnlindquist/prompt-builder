#![cfg(unix)]

use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    // Unix-domain socket paths have a small platform limit (104 bytes on macOS),
    // so keep this integration-test directory deliberately short.
    let path = PathBuf::from("/tmp").join(format!("pb-herdr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create temp directory");
    path
}

fn serve_herdr(socket_path: PathBuf) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
    std::thread::spawn(move || {
        let listener = UnixListener::bind(&socket_path).expect("bind fake Herdr socket");
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut line = String::new();
            BufReader::new(stream.try_clone().expect("clone stream"))
                .read_line(&mut line)
                .expect("read request");
            let request: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            eprintln!("fake Herdr request: {}", request["method"]);
            let id = request["id"].as_str().expect("request id");
            let response = if index == 0 {
                serde_json::json!({
                    "id": id,
                    "result": { "pane": { "tab_id": "w_test:3" } }
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

fn prompt_builder(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_prompt-builder"));
    command
        .env("HOME", home)
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PANE_ID");
    command
}

#[test]
fn real_pi_launch_renames_containing_herdr_tab() {
    let dir = temp_dir("success");
    let project = dir.join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let socket_path = dir.join("herdr.sock");
    let server = serve_herdr(socket_path.clone());
    while !socket_path.exists() {
        std::thread::yield_now();
    }

    let output = prompt_builder(&dir)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", &socket_path)
        .env("HERDR_PANE_ID", "p_42")
        .args([
            "--target",
            "pi",
            "--pi-bin",
            "/bin/echo",
            "-C",
            project.to_str().expect("UTF-8 project path"),
            "--submit",
            "--name",
            "Fix parser",
            "hello",
        ])
        .output()
        .expect("run prompt-builder");
    eprintln!(
        "prompt-builder status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().expect("server exits");

    assert!(output.status.success());
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "pane.get");
    assert_eq!(requests[0]["params"]["pane_id"], "p_42");
    assert_eq!(requests[1]["method"], "tab.rename");
    assert_eq!(requests[1]["params"]["tab_id"], "w_test:3");
    assert_eq!(requests[1]["params"]["label"], "Fix parser");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--name project:Fix parser hello"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Herdr tab"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn herdr_failure_warns_once_but_pi_still_launches() {
    let dir = temp_dir("failure");
    let missing_socket = dir.join("missing.sock");

    let output = prompt_builder(&dir)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", missing_socket)
        .env("HERDR_PANE_ID", "p_42")
        .args([
            "--target",
            "pi",
            "--pi-bin",
            "/bin/echo",
            "--submit",
            "--name",
            "Fix parser",
            "hello",
        ])
        .output()
        .expect("run prompt-builder");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("--name prompt-builder:Fix parser hello")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("warning: failed to rename current Herdr tab:")
            .count(),
        1
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dry_run_and_blank_name_do_not_touch_herdr() {
    let dir = temp_dir("no-mutation");
    let missing_socket = dir.join("missing.sock");

    let dry_run = prompt_builder(&dir)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", &missing_socket)
        .env("HERDR_PANE_ID", "p_42")
        .args([
            "--target",
            "pi",
            "--pi-bin",
            "/bin/echo",
            "--submit",
            "--dry-run",
            "--name",
            "Fix parser",
            "hello",
        ])
        .output()
        .expect("run dry-run");
    assert!(dry_run.status.success());
    assert!(!String::from_utf8_lossy(&dry_run.stderr).contains("Herdr tab"));

    let blank = prompt_builder(&dir)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", &missing_socket)
        .env("HERDR_PANE_ID", "p_42")
        .args([
            "--target",
            "pi",
            "--pi-bin",
            "/bin/echo",
            "--submit",
            "--name",
            " ",
            "hello",
        ])
        .output()
        .expect("run blank-name launch");
    assert!(blank.status.success());
    assert!(!String::from_utf8_lossy(&blank.stderr).contains("Herdr tab"));
    assert_eq!(String::from_utf8_lossy(&blank.stdout).trim(), "hello");
    let _ = std::fs::remove_dir_all(dir);
}
