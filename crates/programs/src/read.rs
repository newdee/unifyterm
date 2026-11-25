//! Reading an OpenCLI description (<https://github.com/spectreconsole/open-cli>, version 0.1)
//! into the tree the rest of this crate works on.
//!
//! Only what is used is read; other fields are passed over, and a field
//! given as `null` is read as one left out (a program whose JSON writer puts
//! `null` for what it has not got). Words that may be put on an input line
//! (names, aliases, accepted values) are left out when they hold a character
//! a shell reads: a description is data, it must never become a command.

use serde::Deserialize;

use crate::nulls::or_empty;
use crate::{Argument, Command, Flag, Program, bare};

/// The largest description read, in bytes (ffmpeg's is 2.2 MB).
pub const MOST_BYTES: usize = 16 * 1024 * 1024;
/// How deep commands may be inside commands.
const DEEPEST: usize = 8;
/// The longest name, alias or accepted value kept.
const LONGEST_NAME: usize = 64;
/// The longest description kept: its first paragraph, cut here.
const LONGEST_DESCRIPTION: usize = 400;
/// Characters a shell reads: no word with one of them is kept.
const SHELL_READS: &[char] = &[';', '|', '&', '$', '`', '<', '>', '(', ')', '{', '}', '\'', '"', '\\', '!', '*', '?'];

/// Why a description was not read.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    TooLarge(usize),
    /// Not JSON, or not shaped as a description: what the parser said.
    NotADescription(String),
    /// No program name, or one that cannot be a file's name.
    NoName(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes: more than the {MOST_BYTES} a description may have"),
            Self::NotADescription(why) => write!(f, "not an OpenCLI description: {why}"),
            Self::NoName(name) => write!(f, "the program's name ('{name}') cannot be used: it must be a plain word"),
        }
    }
}

impl std::error::Error for Error {}

/// A description that was read, and how many of its entries were left out
/// (names with characters a shell reads, commands too deep).
#[derive(Debug)]
pub struct Read {
    pub program: Program,
    pub left_out: usize,
}

#[derive(Deserialize, Default)]
struct RawInfo {
    #[serde(default, deserialize_with = "or_empty")]
    title: String,
    #[serde(default, deserialize_with = "or_empty")]
    summary: String,
    #[serde(default, deserialize_with = "or_empty")]
    description: String,
    #[serde(default, deserialize_with = "or_empty")]
    version: String,
}

#[derive(Deserialize)]
struct RawDocument {
    /// The version of the specification: its presence says what the file is.
    opencli: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "or_empty")]
    info: RawInfo,
    /// The root command (since 2026-04-19).
    command: Option<RawCommand>,
    // Before that, the root command's parts were the document's own.
    #[serde(default, deserialize_with = "or_empty")]
    commands: Vec<RawCommand>,
    #[serde(default, deserialize_with = "or_empty")]
    options: Vec<RawOption>,
    #[serde(default, deserialize_with = "or_empty")]
    arguments: Vec<RawArgument>,
}

#[derive(Deserialize, Default)]
struct RawCommand {
    #[serde(default, deserialize_with = "or_empty")]
    name: String,
    #[serde(default, deserialize_with = "or_empty")]
    aliases: Vec<String>,
    #[serde(default, deserialize_with = "or_empty")]
    description: String,
    #[serde(default, deserialize_with = "or_empty")]
    hidden: bool,
    #[serde(default, deserialize_with = "or_empty")]
    options: Vec<RawOption>,
    #[serde(default, deserialize_with = "or_empty")]
    arguments: Vec<RawArgument>,
    #[serde(default, deserialize_with = "or_empty")]
    commands: Vec<RawCommand>,
}

#[derive(Deserialize)]
struct RawOption {
    #[serde(default, deserialize_with = "or_empty")]
    name: String,
    #[serde(default, deserialize_with = "or_empty")]
    aliases: Vec<String>,
    #[serde(default, deserialize_with = "or_empty")]
    description: String,
    #[serde(default, deserialize_with = "or_empty")]
    hidden: bool,
    #[serde(default, deserialize_with = "or_empty")]
    recursive: bool,
    #[serde(default, deserialize_with = "or_empty")]
    arguments: Vec<RawArgument>,
}

#[derive(Deserialize)]
struct RawArgument {
    #[serde(default, deserialize_with = "or_empty")]
    name: String,
    #[serde(default, deserialize_with = "or_empty")]
    description: String,
    #[serde(default, rename = "acceptedValues", deserialize_with = "or_empty")]
    accepted: Vec<String>,
    arity: Option<RawArity>,
}

#[derive(Deserialize)]
struct RawArity {
    minimum: Option<u64>,
    maximum: Option<u64>,
}

/// Whether `word` may be put on an input line as it is: no blank, no
/// control character, nothing a shell reads.
fn plain(word: &str) -> bool {
    !word.is_empty()
        && word.chars().count() <= LONGEST_NAME
        && !word.chars().any(|c| c.is_whitespace() || c.is_control() || SHELL_READS.contains(&c))
}

/// A description as it is shown: its first paragraph on one line, without
/// control characters, not longer than [`LONGEST_DESCRIPTION`].
fn shown(description: &str) -> String {
    let unix = description.replace("\r\n", "\n");
    let paragraph = unix.trim().split("\n\n").next().unwrap_or_default();
    let line = paragraph
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.chars().count() <= LONGEST_DESCRIPTION {
        return line;
    }
    let mut cut: String = line.chars().take(LONGEST_DESCRIPTION - 1).collect();
    cut.push('…');
    cut
}

/// Keeps the plain words of `words`, counting the others in `left_out`.
fn plain_words(words: Vec<String>, left_out: &mut usize) -> Vec<String> {
    let before = words.len();
    let kept: Vec<String> = words.into_iter().filter(|word| plain(word)).collect();
    *left_out += before - kept.len();
    kept
}

fn argument(raw: RawArgument, left_out: &mut usize) -> Argument {
    let (minimum, maximum) = match raw.arity {
        None => (1, Some(1)),
        Some(arity) => (arity.minimum.unwrap_or(1), arity.maximum),
    };
    Argument {
        name: shown(&raw.name),
        description: shown(&raw.description),
        accepted: plain_words(raw.accepted, left_out),
        minimum,
        maximum,
    }
}

fn options(raw: Vec<RawOption>, left_out: &mut usize) -> Vec<Flag> {
    let mut kept = Vec::with_capacity(raw.len());
    for option in raw {
        if !plain(&option.name) {
            *left_out += 1;
            continue;
        }
        kept.push(Flag {
            name: option.name,
            aliases: plain_words(option.aliases, left_out),
            description: shown(&option.description),
            hidden: option.hidden,
            recursive: option.recursive,
            arguments: option.arguments.into_iter().map(|raw| argument(raw, left_out)).collect(),
        });
    }
    kept
}

fn command(raw: RawCommand, depth: usize, left_out: &mut usize) -> Command {
    let mut commands = Vec::with_capacity(raw.commands.len());
    for inner in raw.commands {
        if depth >= DEEPEST || !plain(&inner.name) {
            *left_out += 1;
            continue;
        }
        commands.push(command(inner, depth + 1, left_out));
    }
    Command {
        name: raw.name,
        aliases: plain_words(raw.aliases, left_out),
        description: shown(&raw.description),
        hidden: raw.hidden,
        options: options(raw.options, left_out),
        arguments: raw.arguments.into_iter().map(|raw| argument(raw, left_out)).collect(),
        commands,
    }
}

/// Reads a description from its JSON text.
pub fn read(text: &str) -> Result<Read, Error> {
    if text.len() > MOST_BYTES {
        return Err(Error::TooLarge(text.len()));
    }
    let text = text.trim_start_matches('\u{feff}').trim_start();
    // A description is an object: an array would be read too, its items taken for the fields in order.
    if !text.starts_with('{') {
        return Err(Error::NotADescription("it is not a JSON object".into()));
    }
    let document: RawDocument =
        serde_json::from_str(text).map_err(|error| Error::NotADescription(error.to_string()))?;
    if document.opencli.is_none() {
        return Err(Error::NotADescription("it has no 'opencli' field".into()));
    }
    let root = match document.command {
        Some(root) => root,
        None => RawCommand {
            name: document.info.title.clone(),
            commands: document.commands,
            options: document.options,
            arguments: document.arguments,
            ..RawCommand::default()
        },
    };
    let name = root.name.trim().to_owned();
    // The name is also a file's name: no folder, no option.
    if !plain(&name) || name.starts_with(['-', '.']) || name.contains(['/', ':']) {
        return Err(Error::NoName(name));
    }
    let mut left_out = 0;
    let mut root = command(root, 0, &mut left_out);
    root.name.clone_from(&name);
    if root.description.is_empty() {
        root.description = shown(&document.info.description);
    }
    let program = Program {
        key: bare(&name),
        name,
        summary: shown(&document.info.summary),
        version: shown(&document.info.version),
        root,
    };
    Ok(Read { program, left_out })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r#"{
        "opencli": "0.1",
        "info": {"title": "Panes", "summary": "Panes in a terminal.", "version": "1.2.0"},
        "command": {
            "name": "panes",
            "options": [{"name": "-L", "arguments": [{"name": "socket"}], "description": "Which server."}],
            "commands": [
                {"name": "split-window", "aliases": ["splitw"], "description": "Split a pane in two.\n\nMore about it.",
                 "options": [{"name": "-h", "description": "Side by side."}],
                 "arguments": [{"name": "command", "arity": {"minimum": 0}}]},
                {"name": "kill; rm -rf ~", "description": "A name a shell would read."},
                {"name": "inner", "hidden": true}
            ]
        }
    }"#;

    #[test]
    fn a_description_becomes_a_tree() {
        let read = read(SMALL).expect("it is a description");
        let program = read.program;
        assert_eq!((program.key.as_str(), program.version.as_str()), ("panes", "1.2.0"));
        assert_eq!(program.summary, "Panes in a terminal.");
        assert_eq!(read.left_out, 1, "the command whose name a shell would read");
        let names: Vec<&str> = program.root.commands.iter().map(|command| command.name.as_str()).collect();
        assert_eq!(names, ["split-window", "inner"]);
        let split = &program.root.commands[0];
        assert_eq!(split.description, "Split a pane in two.", "the first paragraph only");
        assert_eq!(split.aliases, ["splitw"]);
        assert_eq!((split.arguments[0].minimum, split.arguments[0].maximum), (0, None), "an arity with no maximum");
        let socket = &program.root.options[0].arguments[0];
        assert_eq!((socket.minimum, socket.maximum), (1, Some(1)), "no arity: exactly one");
        assert!(program.root.commands[1].hidden, "hidden is kept: it is not offered, but it is known");
    }

    #[test]
    fn the_older_shape_is_read_too() {
        let text = r#"{"opencli": "0.1", "info": {"title": "old", "version": "1"},
            "commands": [{"name": "run"}], "options": [{"name": "--quiet"}]}"#;
        let program = read(text).expect("the shape before 2026-04-19").program;
        assert_eq!(program.name, "old");
        assert_eq!(program.root.commands[0].name, "run");
        assert_eq!(program.root.options[0].name, "--quiet");
    }

    /// Every field this crate reads, given as `null`: the same tree as with the fields left out.
    #[test]
    fn a_field_given_as_null_is_read_as_left_out() {
        let nulls = r#"{"opencli": "0.1",
            "info": {"title": null, "summary": null, "description": null, "version": null},
            "commands": null, "options": null, "arguments": null,
            "command": {"name": "panes", "aliases": null, "description": null, "hidden": null,
                "options": [{"name": "-L", "aliases": null, "description": null, "hidden": null, "recursive": null,
                             "arguments": [{"name": "socket", "description": null, "acceptedValues": null, "arity": null}]},
                            {"name": "-q", "arguments": null}],
                "arguments": null,
                "commands": [{"name": "run", "options": null, "arguments": null, "commands": null}]}}"#;
        let left_out = r#"{"opencli": "0.1", "info": {},
            "command": {"name": "panes",
                "options": [{"name": "-L", "arguments": [{"name": "socket"}]}, {"name": "-q"}],
                "commands": [{"name": "run"}]}}"#;
        let (with_nulls, without) = (read(nulls).expect("nulls are read"), read(left_out).expect("it is read"));
        assert_eq!(with_nulls.program, without.program);
        assert_eq!((with_nulls.left_out, without.program.root.options.len()), (0, 2));
        let no_info = read(r#"{"opencli": "0.1", "info": null, "command": {"name": "x"}}"#).expect("no info");
        assert_eq!((no_info.program.name.as_str(), no_info.program.version.as_str()), ("x", ""));
        // What is there and of the wrong kind is still refused, and the place is still named.
        let wrong = read("{\"opencli\": \"0.1\", \"command\": {\"name\": \"x\",\n \"hidden\": \"yes\"}}");
        let Err(Error::NotADescription(said)) = wrong.map(|read| read.left_out) else { panic!("it was read") };
        assert!(said.contains("expected a boolean") && said.contains("line 2"), "{said}");
        assert_eq!(
            read(r#"{"opencli": "0.1", "command": {"name": null}}"#).map(|read| read.left_out),
            Err(Error::NoName(String::new())),
            "a name is needed all the same"
        );
    }

    #[test]
    fn what_is_no_description_is_refused_and_says_why() {
        assert!(matches!(read("{"), Err(Error::NotADescription(_))), "not JSON");
        for text in ["", "[\"0.1\", {}, {\"name\": \"x\"}]", "null", "\"opencli\""] {
            assert_eq!(
                read(text).map(|read| read.left_out),
                Err(Error::NotADescription("it is not a JSON object".into())),
                "{text:?}"
            );
        }
        assert_eq!(
            read(r#"{"command": {"name": "x"}}"#).map(|read| read.left_out),
            Err(Error::NotADescription("it has no 'opencli' field".into()))
        );
        for name in ["", "a b", "../x", "-x", "a/b", "c:x", "x;y"] {
            let text = format!(r#"{{"opencli": "0.1", "info": {{}}, "command": {{"name": {name:?}}}}}"#);
            assert_eq!(read(&text).map(|read| read.left_out), Err(Error::NoName(name.into())), "the name {name:?}");
        }
        assert!(Error::NoName("a b".into()).to_string().contains("'a b'"));
    }

    #[test]
    fn nothing_a_shell_reads_is_kept_as_a_word() {
        let text = r#"{"opencli": "0.1", "info": {}, "command": {"name": "x",
            "options": [{"name": "--ok", "aliases": ["-o", "$(id)"], "arguments": [{"name": "v", "acceptedValues": ["a", "b c", "`d`"]}]},
                        {"name": "--bad>file"}],
            "commands": [{"name": "fine", "aliases": ["f&g"], "description": "a\u001b[31mb\u0007"}]}}"#;
        let read = read(text).expect("it is a description");
        assert_eq!(read.left_out, 5, "an alias, two values, an option, an alias");
        let root = &read.program.root;
        assert_eq!(root.options.len(), 1);
        assert_eq!(root.options[0].aliases, ["-o"]);
        assert_eq!(root.options[0].arguments[0].accepted, ["a"]);
        assert!(root.commands[0].aliases.is_empty());
        assert_eq!(root.commands[0].description, "a[31mb", "control characters are taken out of what is shown");
    }

    #[test]
    fn commands_too_deep_and_text_too_long_are_cut() {
        let mut inner = String::from(r#"{"name": "leaf"}"#);
        for level in 0..12 {
            inner = format!(r#"{{"name": "level{level}", "commands": [{inner}]}}"#);
        }
        let long = "word ".repeat(200);
        let text = format!(
            r#"{{"opencli": "0.1", "info": {{}}, "command": {{"name": "deep", "description": "{long}", "commands": [{inner}]}}}}"#
        );
        let read = read(&text).expect("it is a description");
        let mut depth = 0;
        let mut at = &read.program.root;
        while let Some(next) = at.commands.first() {
            depth += 1;
            at = next;
        }
        assert_eq!(depth, DEEPEST, "no deeper than that");
        assert_eq!(read.left_out, 1, "the first command too deep, with all in it");
        assert_eq!(read.program.root.description.chars().count(), LONGEST_DESCRIPTION);
        assert!(read.program.root.description.ends_with('…'));
    }
}
