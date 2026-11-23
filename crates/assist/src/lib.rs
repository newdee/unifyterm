//! UnifyTerm's help with commands from a language model: a suggestion while
//! typing, a fix for a command that failed, the meaning of each part of a
//! command, a command for a sentence. Also the fix that needs no model: a
//! misspelt command name.
//!
//! Every function here takes what it sends through [`clean`] first: what
//! looks like a secret is replaced, the home directory becomes `~`. An
//! answer that is not what was asked for (not JSON of the right shape, a
//! suggestion that does not begin with what was typed, an explanation whose
//! parts do not make up the command) is dropped: `Err(Error::BadAnswer)`.
//!
//! No model is called unless the user configured one; see [`Provider`]. A
//! model is a service ([`Anthropic`], [`OpenAiCompatible`]) or a program on
//! this machine ([`Command`]).

mod command;
mod local;
mod provider;
mod reads;

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

pub use command::Command;
pub use local::{distance, local_fix};
pub use provider::{Anthropic, OpenAiCompatible, Provider, Tokens};
pub use reads::only_reads;

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
    /// The program that answers was not found: its name.
    NoProgram(String),
    /// The program that answers did not start, failed, or took too long.
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(why) => write!(f, "the model could not be reached: {why}"),
            Self::Status(code, body) => write!(f, "the model's service said {code}: {body}"),
            Self::Refused => write!(f, "the model declined"),
            Self::BadAnswer(why) => write!(f, "the model's answer was not usable: {why}"),
            Self::NoProgram(name) => write!(f, "{name} was not found: is it installed, and in PATH?"),
            Self::Failed(why) => write!(f, "{why}"),
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
const ASK_TIMEOUT: Duration = Duration::from_secs(30);
/// Where a model's thoughts end, when it writes them before its answer (`<think>…</think>`).
const THOUGHTS_END: &str = "</think>";
/// The most output lines of a failed command sent with it.
const OUTPUT_LINES: usize = 40;
/// What a secret is replaced with in what is sent.
pub const REDACTED: &str = "‹redacted›";

/// Text as it may be sent: secrets replaced, the home directory as `~`.
#[must_use]
pub fn clean(text: &str) -> String {
    let redacted = unifyterm_history::redact(text, REDACTED);
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
    // A small model may write its thoughts first, braces and all: the answer is what follows them.
    let text = text.rfind(THOUGHTS_END).map_or(text, |end| &text[end + THOUGHTS_END.len()..]);
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

/// A sentence that says what a command is to do, and where it will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask<'a> {
    /// What the user wrote after `#`.
    pub words: &'a str,
    pub cwd: &'a str,
    pub shell: &'a str,
    /// The operating system, as a person calls it: "Windows", "macOS", "Linux".
    pub system: &'a str,
    /// Programs that are there besides the system's own (UnifyTerm's commands), by name.
    pub programs: &'a [String],
    /// The last commands, newest first.
    pub recent: &'a [String],
}

/// The command a sentence was turned into, and what it does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asked {
    pub command: String,
    pub note: String,
}

/// The command that does what a sentence says; `None` when the model has none.
pub fn ask(provider: &dyn Provider, ask: &Ask<'_>) -> Result<Option<Asked>, Error> {
    let recent: Vec<String> = ask.recent.iter().take(5).map(|command| clean(command)).collect();
    let programs = if ask.programs.is_empty() {
        String::new()
    } else {
        format!("\nThese programs are there too, whatever the system: {}", ask.programs.join(" "))
    };
    let request = Request {
        system: format!(
            "You write one command line for a terminal user who says in words what it is to do. The shell is {} \
             on {}. Answer with JSON {{\"command\": the command line, \"note\": one short sentence that says \
             what it does, in the language of the request}}. The command is one line with no line break, ready \
             to run in that shell, made of programs that are on such a system. If no command does what is \
             asked, answer with an empty command.",
            ask.shell, ask.system
        ),
        user: format!(
            "Directory: {}{programs}\nRecent commands, newest first:\n{}\nRequest: {}",
            clean(ask.cwd),
            recent.join("\n"),
            clean(ask.words)
        ),
        schema: object_schema(
            &json!({ "command": { "type": "string" }, "note": { "type": "string" } }),
            &["command", "note"],
        ),
        max_tokens: 4_000,
        effort: Effort::Low,
        timeout: ASK_TIMEOUT,
    };
    let answer: Asked = parse(&provider.complete(&request)?)?;
    let command = answer.command.trim();
    // A comment is no command either: on the input line, Enter would hand it to the model again.
    if command.is_empty() || command.starts_with('#') {
        return Ok(None);
    }
    // A line break would run what is before it as soon as it reaches the shell.
    if command.contains(['\n', '\r']) {
        return Err(Error::BadAnswer("the command has a line break".into()));
    }
    Ok(Some(Asked { command: command.to_owned(), note: answer.note.trim().to_owned() }))
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
        let good = Fixed::new(vec![Ok(r#"{"completion": "cargo test -p unifyterm"}"#.into())]);
        assert_eq!(suggest(&good, &context("car", &recent)).unwrap().as_deref(), Some("cargo test -p unifyterm"));
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

    fn asking<'a>(words: &'a str, recent: &'a [String], programs: &'a [String]) -> Ask<'a> {
        Ask { words, cwd: "C:/work", shell: PWSH, system: "Windows", programs, recent }
    }

    #[test]
    fn a_sentence_becomes_one_line_or_nothing() {
        let recent = vec!["cargo build".to_owned()];
        let programs = vec!["cat".to_owned(), "grep".to_owned()];
        let good = Fixed::new(vec![Ok(r#"{"command": " ls -la ", "note": " lists the files "}"#.into())]);
        assert_eq!(
            ask(&good, &asking("list the files", &recent, &programs)).unwrap(),
            Some(Asked { command: "ls -la".into(), note: "lists the files".into() }),
            "without the spaces around it"
        );
        let sent = good.sent();
        for part in [
            "pwsh on Windows",
            "Directory: C:/work",
            "cargo build",
            "Request: list the files",
            "too, whatever the system: cat grep",
        ] {
            assert!(sent.contains(part), "{part} is not in:\n{sent}");
        }
        let alone = Fixed::new(vec![Ok(r#"{"command": "ls", "note": ""}"#.into())]);
        ask(&alone, &asking("x", &[], &[])).unwrap();
        assert!(!alone.sent().contains("too, whatever"), "no other programs: nothing is said of them");
        let none = Fixed::new(vec![Ok(r#"{"command": "  ", "note": "that is not a command's work"}"#.into())]);
        assert_eq!(ask(&none, &asking("write me a poem", &recent, &[])).unwrap(), None);
        let comment = Fixed::new(vec![Ok(r##"{"command": "# no command does that", "note": ""}"##.into())]);
        assert_eq!(
            ask(&comment, &asking("write me a poem", &recent, &[])).unwrap(),
            None,
            "a comment is not a command"
        );
        for lines in [r#"{"command": "ls\nrm -rf /", "note": ""}"#, r#"{"command": "ls\r", "note": ""}"#] {
            let two = Fixed::new(vec![Ok(lines.into())]);
            let said = ask(&two, &asking("list", &recent, &[]));
            // A line break at the very end is only space around the command; inside it, it would run the first part.
            if lines.contains("rm") {
                assert!(matches!(said, Err(Error::BadAnswer(_))), "{lines}: {said:?}");
            } else {
                assert_eq!(said.unwrap().unwrap().command, "ls");
            }
        }
        for wrong in ["not json", r#"{"command": "ls"}"#, r#"{"note": "x"}"#] {
            let bad = Fixed::new(vec![Ok(wrong.into())]);
            assert!(matches!(ask(&bad, &asking("list", &recent, &[])), Err(Error::BadAnswer(_))), "{wrong}");
        }
        let down = Fixed::new(vec![Err(Error::NoProgram("claude".into()))]);
        assert_eq!(ask(&down, &asking("list", &recent, &[])), Err(Error::NoProgram("claude".into())));
        let secret = Fixed::new(vec![Ok(r#"{"command": "", "note": ""}"#.into())]);
        let _ = ask(&secret, &asking("log in to mysql with --password hunter2", &recent, &[]));
        assert!(!secret.sent().contains("hunter2") && secret.sent().contains(REDACTED), "{}", secret.sent());
    }

    #[test]
    fn thoughts_before_the_answer_are_passed_over() {
        let thinking =
            "<think>the user typed {car}; maybe {\"completion\": \"wrong\"}</think>\n{\"completion\": \"cargo test\"}";
        let recent = Vec::new();
        let model = Fixed::new(vec![Ok(thinking.into())]);
        assert_eq!(suggest(&model, &context("car", &recent)).unwrap().as_deref(), Some("cargo test"));
        // Thoughts that never end: no answer followed them.
        let cut = Fixed::new(vec![Ok("<think>still thinking about {car".into())]);
        assert!(matches!(suggest(&cut, &context("car", &recent)), Err(Error::BadAnswer(_))));
    }

    #[test]
    fn each_error_says_what_went_wrong() {
        assert_eq!(
            Error::NoProgram("claude".into()).to_string(),
            "claude was not found: is it installed, and in PATH?"
        );
        assert_eq!(
            Error::Failed("codex gave no answer in 90 s: it was stopped".into()).to_string(),
            "codex gave no answer in 90 s: it was stopped"
        );
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
