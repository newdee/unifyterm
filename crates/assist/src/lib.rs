//! uni-term's help with commands from a language model: a suggestion while
//! typing, a fix for a command that failed, the meaning of each part of a
//! command. Also the fix that needs no model: a misspelt command name.
//!
//! Every function here takes what it sends through [`clean`] first: what
//! looks like a secret is replaced, the home directory becomes `~`. An
//! answer that is not what was asked for (not JSON of the right shape, a
//! suggestion that does not begin with what was typed, an explanation whose
//! parts do not make up the command) is dropped: `Err(Error::BadAnswer)`.
//!
//! No model is called unless the user configured one; see [`Provider`].

mod local;
mod provider;

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

pub use local::{distance, local_fix};
pub use provider::{Anthropic, OpenAiCompatible, Provider};

/// Why there is no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The model could not be reached, or did not answer in time.
    Network(String),
    /// The service answered with an error status.
    Status(u16, String),
    /// The model declined.
    Refused,
    /// The answer was not what was asked for.
    BadAnswer(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(why) => write!(f, "the model could not be reached: {why}"),
            Self::Status(code, body) => write!(f, "the model's service said {code}: {body}"),
            Self::Refused => write!(f, "the model declined"),
            Self::BadAnswer(why) => write!(f, "the model's answer was not usable: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// How much thought the model is to give an answer (Claude's `effort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
}

/// One question to a model: instructions, the question, and the JSON
/// shape the answer must have.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
    pub max_tokens: u32,
    pub effort: Effort,
    pub timeout: Duration,
}

/// How long each kind of answer may take.
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(3);
const FIX_TIMEOUT: Duration = Duration::from_secs(15);
const EXPLAIN_TIMEOUT: Duration = Duration::from_secs(20);
/// The most output lines of a failed command sent with it.
const OUTPUT_LINES: usize = 40;
/// What a secret is replaced with in what is sent.
pub const REDACTED: &str = "‹redacted›";

/// Text as it may be sent: secrets replaced, the home directory as `~`.
#[must_use]
pub fn clean(text: &str) -> String {
    let redacted = uni_history::redact(text, REDACTED);
    let Some(home) = dirs::home_dir() else { return redacted };
    let home = home.to_string_lossy().into_owned();
    if home.len() < 3 {
        return redacted;
    }
    redacted.replace(&home, "~").replace(&home.replace('\\', "/"), "~")
}

/// Where the shell is and what it ran last: what a suggestion is based on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context<'a> {
    pub typed: &'a str,
    pub cwd: &'a str,
    pub shell: &'a str,
    /// The last commands, newest first.
    pub recent: &'a [String],
}

fn object_schema(properties: &serde_json::Value, required: &[&str]) -> serde_json::Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

/// The answer's text as JSON of type `T`.
fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, Error> {
    // A model may wrap JSON in a code fence: what is between the first { and the last } is the answer.
    let start = text.find('{').ok_or_else(|| Error::BadAnswer("no JSON object".into()))?;
    let end = text.rfind('}').ok_or_else(|| Error::BadAnswer("no JSON object".into()))?;
    serde_json::from_str(&text[start..=end]).map_err(|error| Error::BadAnswer(error.to_string()))
}

/// The rest of the command the user is typing; `None` when the model has none.
pub fn suggest(provider: &dyn Provider, context: &Context<'_>) -> Result<Option<String>, Error> {
    #[derive(Deserialize)]
    struct Answer {
        completion: String,
    }
    let typed = clean(context.typed);
    let recent: Vec<String> = context.recent.iter().take(5).map(|command| clean(command)).collect();
    let request = Request {
        system: format!(
            "You complete a command line in a terminal. The shell is {}. Answer with JSON: \
             {{\"completion\": the whole command line}}. It must begin with exactly what was typed. \
             If you have nothing better, repeat what was typed.",
            context.shell
        ),
        user: format!(
            "Directory: {}\nRecent commands, newest first:\n{}\nTyped so far: {typed}",
            clean(context.cwd),
            recent.join("\n")
        ),
        schema: object_schema(&json!({ "completion": { "type": "string" } }), &["completion"]),
        max_tokens: 2_000,
        effort: Effort::Low,
        timeout: SUGGEST_TIMEOUT,
    };
    let answer: Answer = parse(&provider.complete(&request)?)?;
    if !answer.completion.starts_with(&typed) {
        return Err(Error::BadAnswer("the suggestion does not begin with what was typed".into()));
    }
    Ok((answer.completion.len() > typed.len() && !answer.completion.contains(['\n', '\r']))
        .then_some(answer.completion))
}

/// A command that failed: what to send about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure<'a> {
    pub command: &'a str,
    pub exit: Option<i32>,
    /// What it printed (the last lines are sent).
    pub output: &'a str,
    pub shell: &'a str,
}

/// A corrected command and why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Fix {
    pub command: String,
    pub reason: String,
}

/// A fix for a command that failed; `None` when the model sees none.
pub fn fix(provider: &dyn Provider, failure: &Failure<'_>) -> Result<Option<Fix>, Error> {
    let lines: Vec<&str> = failure.output.lines().collect();
    let tail = lines[lines.len().saturating_sub(OUTPUT_LINES)..].join("\n");
    let command = clean(failure.command);
    let request = Request {
        system: format!(
            "A command failed in a terminal; the shell is {}. If the command has a mistake you can correct \
             (a misspelt name, a wrong option, a missing quote), answer with JSON {{\"command\": the corrected \
             command line, \"reason\": one short sentence}}. If you cannot tell, answer with an empty command.",
            failure.shell
        ),
        user: format!(
            "Command: {command}\nExit code: {}\nLast output:\n{}",
            failure.exit.map_or_else(|| "unknown".to_owned(), |exit| exit.to_string()),
            clean(&tail)
        ),
        schema: object_schema(
            &json!({ "command": { "type": "string" }, "reason": { "type": "string" } }),
            &["command", "reason"],
        ),
        max_tokens: 4_000,
        effort: Effort::Low,
        timeout: FIX_TIMEOUT,
    };
    let answer: Fix = parse(&provider.complete(&request)?)?;
    let usable = !answer.command.trim().is_empty() && answer.command.trim() != command.trim();
    Ok((usable && !answer.command.contains(['\n', '\r'])).then_some(answer))
}

/// One part of a command and what it means.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Part {
    pub text: String,
    pub meaning: String,
}

/// What each part of a command means. `help`: the command's own help
/// text, when it has one, sent along so the model need not guess.
pub fn explain(provider: &dyn Provider, command: &str, shell: &str, help: Option<&str>) -> Result<Vec<Part>, Error> {
    #[derive(Deserialize)]
    struct Answer {
        parts: Vec<Part>,
    }
    let command = clean(command);
    let help = help.map(|help| format!("\nThe program's help:\n{}", clean(help))).unwrap_or_default();
    let request = Request {
        system: format!(
            "Explain a command line for a terminal user; the shell is {shell}. Split it into its parts \
             (the program, each option with its value, each argument, pipes and redirections) in order, \
             and say in one short sentence what each does. Answer with JSON {{\"parts\": [{{\"text\": the \
             part exactly as written, \"meaning\": the sentence}}]}}. Joined with spaces, the parts' texts \
             must give the command back."
        ),
        user: format!("Command: {command}{help}"),
        schema: object_schema(
            &json!({ "parts": { "type": "array", "items": object_schema(&json!({ "text": { "type": "string" }, "meaning": { "type": "string" } }),
                &["text", "meaning"],
            ) } }),
            &["parts"],
        ),
        max_tokens: 8_000,
        effort: Effort::Medium,
        timeout: EXPLAIN_TIMEOUT,
    };
    let answer: Answer = parse(&provider.complete(&request)?)?;
    let squeeze = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let joined = squeeze(&answer.parts.iter().map(|part| part.text.as_str()).collect::<Vec<_>>().join(" "));
    if answer.parts.is_empty() || joined != squeeze(&command) {
        return Err(Error::BadAnswer("the parts do not make up the command".into()));
    }
    Ok(answer.parts)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::Mutex;

    use super::*;

    /// A provider that gives fixed answers and keeps what it was asked.
    struct Fixed {
        answers: Mutex<Vec<Result<String, Error>>>,
        asked: Mutex<Vec<Request>>,
    }

    impl Fixed {
        fn new(answers: Vec<Result<String, Error>>) -> Self {
            Self { answers: Mutex::new(answers), asked: Mutex::new(Vec::new()) }
        }

        fn sent(&self) -> String {
            self.asked.lock().unwrap().iter().fold(String::new(), |mut all, request| {
                writeln!(all, "{}\n{}", request.system, request.user).unwrap();
                all
            })
        }
    }

    impl Provider for Fixed {
        fn complete(&self, request: &Request) -> Result<String, Error> {
            self.asked.lock().unwrap().push(request.clone());
            self.answers.lock().unwrap().remove(0)
        }
    }

    const PWSH: &str = "pwsh";

    fn context<'a>(typed: &'a str, recent: &'a [String]) -> Context<'a> {
        Context { typed, cwd: "C:/work", shell: PWSH, recent }
    }

    #[test]
    fn a_suggestion_must_begin_with_what_was_typed() {
        let recent = vec!["cargo build".to_owned()];
        let good = Fixed::new(vec![Ok(r#"{"completion": "cargo test -p uni-term"}"#.into())]);
        assert_eq!(suggest(&good, &context("car", &recent)).unwrap().as_deref(), Some("cargo test -p uni-term"));
        let fenced = Fixed::new(vec![Ok("```json\n{\"completion\": \"git status\"}\n```".into())]);
        assert_eq!(suggest(&fenced, &context("git", &recent)).unwrap().as_deref(), Some("git status"));
        let off = Fixed::new(vec![Ok(r#"{"completion": "ls -la"}"#.into())]);
        assert!(matches!(suggest(&off, &context("car", &recent)), Err(Error::BadAnswer(_))));
        let same = Fixed::new(vec![Ok(r#"{"completion": "car"}"#.into())]);
        assert_eq!(suggest(&same, &context("car", &recent)).unwrap(), None, "nothing more than typed: no suggestion");
        let lines = Fixed::new(vec![Ok(r#"{"completion": "car\nrm -rf /"}"#.into())]);
        assert_eq!(suggest(&lines, &context("car", &recent)).unwrap(), None, "a line break would run it");
        for wrong in ["not json", r#"{"other": 1}"#, "{"] {
            let bad = Fixed::new(vec![Ok(wrong.into())]);
            assert!(matches!(suggest(&bad, &context("car", &recent)), Err(Error::BadAnswer(_))), "{wrong}");
        }
        let down = Fixed::new(vec![Err(Error::Network("timed out".into()))]);
        assert_eq!(suggest(&down, &context("car", &recent)), Err(Error::Network("timed out".into())));
    }

    #[test]
    fn a_fix_is_a_different_single_line() {
        let failure = Failure { command: "gti status", exit: Some(1), output: "gti: not found", shell: PWSH };
        let good = Fixed::new(vec![Ok(r#"{"command": "git status", "reason": "gti is git misspelt"}"#.into())]);
        assert_eq!(
            fix(&good, &failure).unwrap(),
            Some(Fix { command: "git status".into(), reason: "gti is git misspelt".into() })
        );
        for none in [r#"{"command": "", "reason": "no idea"}"#, r#"{"command": "gti status", "reason": "same"}"#] {
            assert_eq!(fix(&Fixed::new(vec![Ok(none.into())]), &failure).unwrap(), None, "{none}");
        }
        let long_output = (0..100).fold(String::new(), |mut all, n| {
            writeln!(all, "line {n}").unwrap();
            all
        });
        let provider = Fixed::new(vec![Ok(r#"{"command": "", "reason": ""}"#.into())]);
        fix(&provider, &Failure { output: &long_output, ..failure.clone() }).unwrap();
        let sent = provider.sent();
        assert!(
            sent.contains("line 99") && sent.contains("line 60") && !sent.contains("line 59\n"),
            "the last 40 lines"
        );
    }

    #[test]
    fn an_explanation_must_make_up_the_command() {
        let command = "tar -czf out.tgz ./src";
        let good = r#"{"parts": [{"text": "tar", "meaning": "archiver"}, {"text": "-czf", "meaning": "create, gzip, to file"},
            {"text": "out.tgz", "meaning": "the archive"}, {"text": "./src", "meaning": "what goes in"}]}"#;
        let parts = explain(&Fixed::new(vec![Ok(good.into())]), command, PWSH, None).unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[1], Part { text: "-czf".into(), meaning: "create, gzip, to file".into() });
        let missing = r#"{"parts": [{"text": "tar", "meaning": "archiver"}]}"#;
        assert!(matches!(
            explain(&Fixed::new(vec![Ok(missing.into())]), command, PWSH, None),
            Err(Error::BadAnswer(_))
        ));
        let empty = r#"{"parts": []}"#;
        assert!(matches!(explain(&Fixed::new(vec![Ok(empty.into())]), "", PWSH, None), Err(Error::BadAnswer(_))));
        let with_help = Fixed::new(vec![Ok(good.into())]);
        explain(&with_help, command, PWSH, Some("usage: tar [OPTION]...")).unwrap();
        assert!(with_help.sent().contains("usage: tar"), "the help goes along");
    }

    #[test]
    fn nothing_secret_is_sent() {
        let provider =
            Fixed::new(vec![Ok(r#"{"completion": "x"}"#.into()), Ok(r#"{"command": "", "reason": ""}"#.into())]);
        let recent = vec!["export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789".to_owned()];
        let _ = suggest(&provider, &context("mysql --password hunter2 -e", &recent));
        let _ = fix(
            &provider,
            &Failure {
                command: "curl -u admin:s3cret@x",
                exit: Some(6),
                output: "Authorization: Bearer abcdefghijklmnop",
                shell: PWSH,
            },
        );
        let sent = provider.sent();
        for secret in ["hunter2", "ghp_abcdefghijklmnopqrstuvwxyz0123456789", "abcdefghijklmnop"] {
            assert!(!sent.contains(secret), "{secret} was sent:\n{sent}");
        }
        assert!(sent.contains(REDACTED));
        if let Some(home) = dirs::home_dir() {
            let home = home.to_string_lossy().replace('\\', "/");
            let provider = Fixed::new(vec![Ok(r#"{"completion": "x"}"#.into())]);
            let _ = suggest(
                &provider,
                &Context { typed: "ls", cwd: &format!("{home}/projects"), shell: PWSH, recent: &[] },
            );
            assert!(provider.sent().contains("Directory: ~/projects"), "the home directory is ~");
        }
    }
}
