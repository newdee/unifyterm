//! `ut mcp` as an agent sees it: JSON-RPC lines in, lines out, with the
//! history in a directory of the test's own (`UNI_TERM_DATA_DIR`).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Sends `messages` to one `ut mcp`, one a line, and gives its answers.
fn talk(data: &Path, messages: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ut"))
        .arg("mcp")
        .env("UNI_TERM_DATA_DIR", data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start ut mcp");
    let mut input = child.stdin.take().expect("its input");
    for message in messages {
        writeln!(input, "{message}").expect("send");
    }
    writeln!(input, "this is not JSON").expect("send");
    drop(input);
    let output = child.wait_with_output().expect("ut mcp ends with its input");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout)
        .expect("text")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect()
}

fn call(id: u64, name: &str, arguments: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } })
}

fn result_text(answer: &Value) -> &str {
    answer["result"]["content"][0]["text"].as_str().expect("a text result")
}

#[test]
fn an_agent_asks_the_history() {
    let data = tempfile::tempdir().expect("a directory");
    let mut history = uni_history::History::open(&data.path().join("history.db")).expect("a history");
    for (text, cwd) in [("cargo build --release", "C:/work"), ("cargo test", "C:/work"), ("git push", "C:/other")] {
        let run = uni_history::Run {
            text: text.into(),
            at_ms: uni_history::now_ms(),
            cwd: cwd.into(),
            exit: Some(0),
            duration_ms: Some(5),
            shell: "pwsh".into(),
        };
        history.record(&run).expect("record");
    }
    drop(history);
    let answers = talk(
        data.path(),
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05" } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            call(3, "history_search", &json!({ "query": "cargo", "cwd": "C:\\work" })),
            call(4, "history_recent", &json!({ "limit": 1000 })),
        ],
    );
    assert_eq!(answers.len(), 5, "4 answers and the parse error; none for the notification: {answers:?}");
    assert_eq!(answers[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(answers[1]["result"]["tools"].as_array().map(Vec::len), Some(4));
    let found: Value = serde_json::from_str(result_text(&answers[2])).expect("a list");
    let texts: Vec<&str> = found.as_array().expect("a list").iter().filter_map(|row| row["command"].as_str()).collect();
    assert_eq!(texts.len(), 2);
    assert!(texts.iter().all(|text| text.starts_with("cargo")), "{texts:?}");
    let recent: Value = serde_json::from_str(result_text(&answers[3])).expect("a list");
    assert_eq!(recent.as_array().map(Vec::len), Some(3), "a limit over 100 is taken as 100");
    assert_eq!(answers[4]["error"]["code"], -32700);
}

#[test]
fn no_history_is_an_empty_list_and_makes_no_file() {
    let data = tempfile::tempdir().expect("a directory");
    let answers = talk(
        data.path(),
        &[call(1, "history_recent", &json!({})), call(2, "history_search", &json!({ "query": "x" }))],
    );
    assert_eq!(result_text(&answers[0]), "[]");
    assert_eq!(result_text(&answers[1]), "[]");
    assert_eq!(std::fs::read_dir(data.path()).expect("the directory").count(), 0, "nothing was made");
}

#[test]
fn every_command_has_help() {
    let data = tempfile::tempdir().expect("a directory");
    let listed = talk(data.path(), &[call(1, "list_commands", &json!({}))]);
    let names: Vec<String> = serde_json::from_str(result_text(&listed[0])).expect("names");
    assert!(names.len() >= 99, "{} commands", names.len());
    let asks: Vec<Value> =
        names.iter().zip(1..).map(|(name, id)| call(id, "command_help", &json!({ "name": name }))).collect();
    let answers = talk(data.path(), &asks);
    let without: Vec<&String> = names
        .iter()
        .zip(&answers)
        .filter(|(_, answer)| answer["result"]["isError"] == true || result_text(answer).trim().is_empty())
        .map(|(name, _)| name)
        .collect();
    assert!(without.is_empty(), "no help from {without:?}");
    let refused = talk(data.path(), &[call(1, "command_help", &json!({ "name": "powershell" }))]);
    assert_eq!(refused[0]["result"]["isError"], true, "only ut's own commands");
}

#[test]
fn the_skill_is_printed_and_installed() {
    let dir = tempfile::tempdir().expect("a directory");
    let printed = Command::new(env!("CARGO_BIN_EXE_ut")).arg("skill").output().expect("ut skill");
    let text = String::from_utf8(printed.stdout).expect("text");
    assert!(text.starts_with("---\nname: uni-term\n"), "{}", &text[..40.min(text.len())]);
    let installed = Command::new(env!("CARGO_BIN_EXE_ut"))
        .args(["skill", "--install"])
        .arg(dir.path())
        .output()
        .expect("ut skill --install");
    assert!(installed.status.success());
    assert_eq!(std::fs::read_to_string(dir.path().join("uni-term/SKILL.md")).expect("the file"), text);
    assert!(!String::from_utf8_lossy(&installed.stdout).contains('\\'), "the path is said with /");
    assert_eq!(
        Command::new(env!("CARGO_BIN_EXE_ut")).args(["skill", "--other"]).status().expect("ut skill").code(),
        Some(2)
    );
}
