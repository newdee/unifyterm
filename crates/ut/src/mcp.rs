//! `ut mcp`: an MCP server on standard input and output, for an agent that
//! works in uni-term. One JSON-RPC message a line. It answers questions
//! about the command history and about `ut`'s commands; it runs nothing.
//!
//! | tool             | arguments                                    | answer                                   |
//! |------------------|----------------------------------------------|------------------------------------------|
//! | `history_search` | `query`; `cwd`, `limit` (20, at most 100)    | commands: text, runs, last run, last exit |
//! | `history_recent` | `cwd`, `limit`                               | the same, newest first                    |
//! | `list_commands`  | none                                         | `ut`'s command names                      |
//! | `command_help`   | `name`                                       | that command's `--help`                   |
//!
//! The history is opened read-only and nothing is made when there is none.

use std::ffi::OsString;
use std::io::{self, BufRead as _, Write as _};

use serde_json::{Value, json};
use uni_history::{Candidate, History, Query};

/// Protocol versions this server speaks, newest first (as keepane's).
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_LIMIT: u64 = 20;
const MOST: u64 = 100;
/// The most of a command's help given back, in characters.
const HELP_CHARS: usize = 16 * 1024;

const INSTRUCTIONS: &str = "You work in uni-term, a terminal that keeps the commands its shells run. \
Ask history_search what was run before (with the directory as cwd to rank that directory's first), \
history_recent for the last commands, list_commands for the Unix commands uni-term provides on every \
system (cat, grep, jq, find, sed, ...), and command_help for how one of them is used. Nothing here \
runs a command: use your own shell for that.";

fn tool(name: &str, description: &str, properties: &Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

fn tools() -> Value {
    let limit = json!({ "type": "integer", "description": "How many commands, at most (default 20, at most 100)" });
    let cwd = json!({ "type": "string", "description": "A directory, with / (C:/work): its commands first, or only" });
    json!([
        tool(
            "history_search",
            "Commands from the history that begin with or contain the query, best first",
            &json!({ "query": { "type": "string", "description": "Part of a command" }, "cwd": cwd, "limit": limit }),
            &["query"],
        ),
        tool("history_recent", "The commands last run, newest first", &json!({ "cwd": cwd, "limit": limit }), &[]),
        tool("list_commands", "The names of the commands uni-term provides (ut's)", &json!({}), &[]),
        tool(
            "command_help",
            "How one of uni-term's commands is used: its --help",
            &json!({ "name": { "type": "string", "description": "A command's name, as list_commands gives it" } }),
            &["name"],
        ),
    ])
}

/// What answers the tools' questions: the history file, and `ut` itself.
pub(crate) struct Sources {
    /// `None` when there is no history file.
    pub(crate) history: Option<History>,
    pub(crate) commands: Vec<String>,
    /// A command's help text by its name.
    pub(crate) help: fn(&str) -> Option<String>,
}

fn candidates(found: &[Candidate]) -> Value {
    Value::Array(
        found
            .iter()
            .map(|candidate| {
                json!({
                    "command": candidate.text,
                    "runs": candidate.count,
                    "failed": candidate.fail_count,
                    "last_run_ms": candidate.last_at_ms,
                    "last_exit": candidate.last_exit,
                })
            })
            .collect(),
    )
}

/// A tool's text result.
fn text(value: &Value) -> Value {
    json!({ "content": [{ "type": "text", "text": value.to_string() }] })
}

fn failure(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// The answer to one `tools/call`.
fn call(sources: &Sources, name: &str, arguments: &Value) -> Value {
    let limit = arguments["limit"].as_u64().unwrap_or(DEFAULT_LIMIT).clamp(1, MOST);
    // At most MOST (100): it fits any usize.
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let cwd = arguments["cwd"].as_str().map(|cwd| cwd.replace('\\', "/"));
    match name {
        "history_search" => {
            let Some(query) = arguments["query"].as_str() else { return failure("query is needed") };
            let Some(history) = &sources.history else { return text(&json!([])) };
            let cwd = cwd.unwrap_or_default();
            match history.search(&Query { text: query, cwd: &cwd, now_ms: uni_history::now_ms(), limit }) {
                Ok(found) => text(&candidates(&found)),
                Err(error) => failure(&format!("the history could not be read: {error:#}")),
            }
        }
        "history_recent" => {
            let Some(history) = &sources.history else { return text(&json!([])) };
            match history.recent(limit, cwd.as_deref()) {
                Ok(found) => text(&candidates(&found)),
                Err(error) => failure(&format!("the history could not be read: {error:#}")),
            }
        }
        "list_commands" => text(&json!(sources.commands)),
        "command_help" => {
            let Some(name) = arguments["name"].as_str() else { return failure("name is needed") };
            if !sources.commands.iter().any(|known| known == name) {
                return failure(&format!("{name} is not one of uni-term's commands (list_commands names them)"));
            }
            match (sources.help)(name) {
                Some(help) => {
                    json!({ "content": [{ "type": "text", "text": help.chars().take(HELP_CHARS).collect::<String>() }] })
                }
                None => failure(&format!("{name} gave no help")),
            }
        }
        _ => failure(&format!("no tool named {name}")),
    }
}

/// The answer to one message; `None` for a notification.
pub(crate) fn answer(sources: &Sources, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => {
            let asked = message["params"]["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOLS.iter().find(|known| **known == asked).unwrap_or(&PROTOCOLS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "uni-term", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            Ok(call(sources, name, &message["params"]["arguments"]))
        }
        _ => Err(json!({ "code": -32601, "message": format!("no method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

pub(crate) fn main(args: &[OsString], commands: Vec<String>, help: fn(&str) -> Option<String>) -> i32 {
    if !args.is_empty() {
        let _ = writeln!(io::stderr(), "usage: ut mcp   (an MCP server on standard input and output; no arguments)");
        return 2;
    }
    let history = match uni_history::default_path().and_then(|path| History::open_read_only(&path)) {
        Ok(history) => history,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut mcp: the history is not in use: {error:#}");
            None
        }
    };
    let sources = Sources { history, commands, help };
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => answer(&sources, &message),
            Err(error) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {error}") } }),
            ),
        };
        if let Some(reply) = reply
            && writeln!(out, "{reply}").and_then(|()| out.flush()).is_err()
        {
            break;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use uni_history::Run;

    use super::*;

    fn sources(history: Option<History>) -> Sources {
        Sources {
            history,
            commands: vec!["cat".into(), "grep".into()],
            help: |name| (name == "cat").then(|| "usage: cat [OPTION]... [FILE]...".to_owned()),
        }
    }

    fn ask(sources: &Sources, method: &str, params: &Value) -> Value {
        answer(sources, &json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })).unwrap()
    }

    fn tool_text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn the_handshake_and_the_tool_list() {
        let sources = sources(None);
        let known = ask(&sources, "initialize", &json!({ "protocolVersion": "2025-03-26" }));
        assert_eq!(known["result"]["protocolVersion"], "2025-03-26");
        let unknown = ask(&sources, "initialize", &json!({}));
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOLS[0], "an unknown version gets the newest");
        assert!(answer(&sources, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let listed = ask(&sources, "tools/list", &json!({}));
        let names: Vec<&str> =
            listed["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["history_search", "history_recent", "list_commands", "command_help"]);
        assert!(
            listed["result"]["tools"].as_array().unwrap().iter().all(|tool| tool["inputSchema"]["type"] == "object")
        );
        assert_eq!(ask(&sources, "nope", &json!({}))["error"]["code"], -32601);
        assert_eq!(ask(&sources, "ping", &json!({}))["result"], json!({}));
    }

    #[test]
    fn the_history_is_searched_and_listed() {
        let mut history = History::in_memory().unwrap();
        let run = |text: &str, cwd: &str, exit| Run {
            text: text.into(),
            at_ms: uni_history::now_ms(),
            cwd: cwd.into(),
            exit,
            duration_ms: None,
            shell: "pwsh".into(),
        };
        history.record(&run("git status", "C:/work", Some(0))).unwrap();
        history.record(&run("git push", "C:/other", Some(1))).unwrap();
        let sources = sources(Some(history));
        let found: Value = serde_json::from_str(&tool_text(&ask(
            &sources,
            "tools/call",
            &json!({ "name": "history_search", "arguments": { "query": "git", "cwd": "C:\\work" } }),
        )))
        .unwrap();
        assert_eq!(found[0]["command"], "git status", "the directory's own first: {found}");
        assert_eq!(found[1]["last_exit"], 1);
        let recent: Value = serde_json::from_str(&tool_text(&ask(
            &sources,
            "tools/call",
            &json!({ "name": "history_recent", "arguments": { "cwd": "C:/other", "limit": 500 } }),
        )))
        .unwrap();
        assert_eq!(recent.as_array().unwrap().len(), 1);
        let missing = ask(&sources, "tools/call", &json!({ "name": "history_search", "arguments": {} }));
        assert_eq!(missing["result"]["isError"], true);
    }

    #[test]
    fn commands_and_their_help() {
        let sources = sources(None);
        assert_eq!(tool_text(&ask(&sources, "tools/call", &json!({ "name": "list_commands" }))), r#"["cat","grep"]"#);
        assert_eq!(
            tool_text(&ask(&sources, "tools/call", &json!({ "name": "command_help", "arguments": { "name": "cat" } }))),
            "usage: cat [OPTION]... [FILE]..."
        );
        let unknown =
            ask(&sources, "tools/call", &json!({ "name": "command_help", "arguments": { "name": "rm -rf" } }));
        assert_eq!(unknown["result"]["isError"], true, "only uni-term's commands are asked for help");
        assert_eq!(ask(&sources, "tools/call", &json!({ "name": "nothing" }))["result"]["isError"], true);
        let empty: Value =
            serde_json::from_str(&tool_text(&ask(&sources, "tools/call", &json!({ "name": "history_recent" }))))
                .unwrap();
        assert_eq!(empty, json!([]), "no history: an empty list, not an error");
    }
}
