//! A description put into words: what each word of a line is, and the
//! description as text for a person or a model to read.

use std::fmt::Write as _;

use crate::walk::{Role, Step, flags_under, program_of, walk, words};
use crate::{Argument, Command, Flag, Program, sentence};

/// What the words of a line are, from the description of the line's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    /// The program's name and version, as its description gives them.
    pub program: String,
    pub version: String,
    /// Each part of the line (a word, or an option with its value) and what it is.
    pub parts: Vec<(String, String)>,
}

/// The longest telling of one word: the first sentence of its description, cut here.
const TOLD: usize = 110;

fn of_flag(flag: &Flag) -> String {
    match (flag.description.is_empty(), flag.arguments.first()) {
        (false, _) => sentence(&flag.description, TOLD),
        (true, Some(value)) if !value.name.is_empty() => format!("an option; takes {}", value.name),
        (true, _) => "an option".to_owned(),
    }
}

fn of_argument(argument: Option<&Argument>) -> String {
    match argument {
        Some(argument) if !argument.name.is_empty() && !argument.description.is_empty() => {
            format!("{}: {}", argument.name, sentence(&argument.description, TOLD))
        }
        Some(argument) if !argument.name.is_empty() => argument.name.clone(),
        _ => "a value".to_owned(),
    }
}

/// What the word `said` is, which the line's walk gave `step` for.
fn meaning(program: &Program, step: &Step<'_>, said: &str) -> String {
    match &step.role {
        Role::Program if !program.summary.is_empty() => sentence(&program.summary, TOLD),
        Role::Program if !program.root.description.is_empty() => sentence(&program.root.description, TOLD),
        Role::Program => "the program".to_owned(),
        Role::Command(command) if !command.description.is_empty() => sentence(&command.description, TOLD),
        Role::Command(_) => format!("a command of {}", program.name),
        Role::Flag { flag, .. } => of_flag(flag),
        // Each letter as it was typed, not the option's long name.
        Role::Flags(flags) => said
            .chars()
            .skip(1)
            .zip(flags)
            .map(|(letter, flag)| format!("-{letter}: {}", of_flag(flag)))
            .collect::<Vec<_>>()
            .join("; "),
        Role::ValueOf(flag) => of_argument(flag.arguments.first()),
        Role::Argument(argument) => of_argument(*argument),
        Role::Rest => "what follows are arguments, not options".to_owned(),
        Role::NoCommand | Role::NoFlag => format!("not in {}'s list", program.name),
    }
}
/// What each word of `line` is, from the description of its program. An
/// option and the value after it are one part. `None` when the program has
/// no description or the line is not one simple command.
#[must_use]
pub fn explain(programs: &[Program], line: &str) -> Option<Explained> {
    let words = words(line)?;
    let program = program_of(programs, &words.first()?.said)?;
    let (steps, _) = walk(program, &words);
    let mut parts = Vec::with_capacity(steps.len());
    let mut at = 0;
    while at < steps.len() {
        // An option with the value after it: one part.
        let with_value = matches!(steps[at].role, Role::Flag { attached: false, .. })
            && steps.get(at + 1).is_some_and(|next| matches!(next.role, Role::ValueOf(_)));
        let last = if with_value { at + 1 } else { at };
        parts.push((line[words[at].start..words[last].end].to_owned(), meaning(program, &steps[at], &words[at].said)));
        at = last + 1;
    }
    Some(Explained { program: program.name.clone(), version: program.version.clone(), parts })
}

fn value_names(arguments: &[Argument]) -> String {
    arguments
        .iter()
        .map(|argument| if argument.name.is_empty() { "value" } else { &argument.name })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A command with what is under it, as text: its description, its options
/// (and those that come down to it), its arguments, its commands.
fn described(program: &Program, under: &[&Command]) -> String {
    let path: Vec<&str> = under.iter().map(|command| command.name.as_str()).collect();
    let command = under[under.len() - 1];
    let mut text = path.join(" ");
    if under.len() == 1 && !program.version.is_empty() {
        let _ = write!(text, " {}", program.version);
    }
    let about = if under.len() == 1 && !program.summary.is_empty() { &program.summary } else { &command.description };
    if !about.is_empty() {
        let _ = write!(text, ": {about}");
    }
    text.push('\n');
    let flags: Vec<&Flag> = flags_under(under).into_iter().filter(|flag| !flag.hidden).collect();
    if !flags.is_empty() {
        text.push_str("options:\n");
        for flag in flags {
            let names: Vec<&str> = std::iter::once(&flag.name).chain(&flag.aliases).map(String::as_str).collect();
            let _ = write!(text, "  {}", names.join(", "));
            if !flag.arguments.is_empty() {
                let _ = write!(text, " <{}>", value_names(&flag.arguments));
            }
            let accepted: Vec<&str> =
                flag.arguments.iter().flat_map(|value| &value.accepted).map(String::as_str).collect();
            if !accepted.is_empty() {
                let _ = write!(text, " (one of {})", accepted.join(", "));
            }
            let _ = writeln!(text, "{}{}", if flag.description.is_empty() { "" } else { ": " }, flag.description);
        }
    }
    if !command.arguments.is_empty() {
        text.push_str("arguments:\n");
        for argument in &command.arguments {
            let _ = write!(text, "  {}", if argument.name.is_empty() { "value" } else { &argument.name });
            if !argument.accepted.is_empty() {
                let _ = write!(text, " (one of {})", argument.accepted.join(", "));
            }
            let _ =
                writeln!(text, "{}{}", if argument.description.is_empty() { "" } else { ": " }, argument.description);
        }
    }
    let commands: Vec<&Command> = command.commands.iter().filter(|inner| !inner.hidden).collect();
    if !commands.is_empty() {
        text.push_str("commands:\n");
        for inner in commands {
            let _ = writeln!(text, "  {}", one_line(inner, Detail::NamesOnly));
        }
    }
    text
}

/// `text`, with whole lines taken off its end until it is no longer than `most` bytes.
fn cut_to(mut text: String, most: usize) -> String {
    while text.len() > most {
        let without_last = text.trim_end_matches('\n').rfind('\n').map_or(0, |at| at + 1);
        text.truncate(without_last);
    }
    text
}

/// The commands a line goes into, by its words alone: the program's own first.
fn entered<'p>(program: &'p Program, rest: &[&str]) -> Vec<&'p Command> {
    let mut under = vec![&program.root];
    let mut is_a_value = false;
    for word in rest {
        if std::mem::take(&mut is_a_value) {
            continue;
        }
        if word.starts_with('-') {
            // An option that takes a value: the next word is that, not a command.
            is_a_value = flags_under(&under).into_iter().any(|flag| {
                flag.takes_a_value() && (flag.name == *word || flag.aliases.iter().any(|alias| alias == word))
            });
            continue;
        }
        let level = under[under.len() - 1];
        let Some(inner) =
            level.commands.iter().find(|inner| inner.name == *word || inner.aliases.iter().any(|alias| alias == word))
        else {
            break;
        };
        under.push(inner);
    }
    under
}

/// The description of one command, for `ut program show NAME [WORD...]`:
/// `None` when a word is no command where it stands.
#[must_use]
pub fn show(program: &Program, path: &[String]) -> Option<String> {
    let rest: Vec<&str> = path.iter().map(String::as_str).collect();
    let under = entered(program, &rest);
    (under.len() == path.len() + 1).then(|| described(program, &under))
}

/// What the first described program on `line` says of the command the
/// line uses, for a model that is to explain the line: at most `most`
/// bytes. The line may be any line: only its words are looked at.
#[must_use]
pub fn help(programs: &[Program], line: &str, most: usize) -> Option<String> {
    let all: Vec<&str> = line.split_whitespace().collect();
    let (at, program) = all.iter().enumerate().find_map(|(at, word)| Some((at, program_of(programs, word)?)))?;
    let under = entered(program, &all[at + 1..]);
    Some(cut_to(
        format!("From {}'s own description of its commands.\n{}", program.name, described(program, &under)),
        most,
    ))
}

/// How much of a command a line about it tells.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    NamesOnly,
    WithOptions,
}

/// A command on one line: `name (alias) [-h] [-c dir] [argument] {inner, inner}: what it does`.
fn one_line(command: &Command, detail: Detail) -> String {
    let mut line = command.name.clone();
    if !command.aliases.is_empty() {
        let _ = write!(line, " ({})", command.aliases.join(", "));
    }
    if detail == Detail::WithOptions {
        for flag in command.options.iter().filter(|flag| !flag.hidden) {
            let value =
                if flag.arguments.is_empty() { String::new() } else { format!(" {}", value_names(&flag.arguments)) };
            let _ = write!(line, " [{}{value}]", flag.name);
        }
        for argument in &command.arguments {
            let _ = write!(line, " [{}]", if argument.name.is_empty() { "value" } else { &argument.name });
        }
        let inner: Vec<&str> =
            command.commands.iter().filter(|inner| !inner.hidden).map(|inner| inner.name.as_str()).collect();
        if !inner.is_empty() {
            let _ = write!(line, " {{{}}}", inner.join(", "));
        }
    }
    if !command.description.is_empty() {
        let _ = write!(line, ": {}", sentence(&command.description, 80));
    }
    line
}

/// Whether `text` has `name` as a word of its own (letters' case aside).
fn names(text: &str, name: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .any(|word| word == name || word.strip_suffix(".exe") == Some(name))
}

/// The descriptions as text for a model that is to write a command: one
/// line a program, and for each program that `mentions` names (the user's
/// sentence, the commands run lately) its commands, with their options
/// while that fits in `most` bytes. Empty when there are no descriptions.
#[must_use]
pub fn brief(programs: &[Program], mentions: &[&str], most: usize) -> String {
    if programs.is_empty() {
        return String::new();
    }
    let mut text = String::from("Programs on this machine that describe their own commands:\n");
    for program in programs {
        let about =
            if program.summary.is_empty() { sentence(&program.root.description, 80) } else { program.summary.clone() };
        let _ =
            writeln!(text, "- {} {}{}{about}", program.name, program.version, if about.is_empty() { "" } else { ": " });
    }
    let named: Vec<&Program> =
        programs.iter().filter(|program| mentions.iter().any(|text| names(text, &program.key))).collect();
    let tables = |detail: Detail| {
        let mut tables = String::new();
        for program in &named {
            let _ = writeln!(tables, "The commands of {} (use no command or option that is not listed):", program.name);
            for command in program.root.commands.iter().filter(|command| !command.hidden) {
                let _ = writeln!(tables, "  {}", one_line(command, detail));
            }
        }
        tables
    };
    let room = most.saturating_sub(text.len());
    let full = tables(Detail::WithOptions);
    let fitting = if full.len() <= room { full } else { tables(Detail::NamesOnly) };
    text.push_str(&fitting);
    cut_to(text, most)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panes;

    fn parts(line: &str) -> Vec<(String, String)> {
        explain(&[panes()], line).expect("panes has a description").parts
    }

    fn pair(text: &str, meaning: &str) -> (String, String) {
        (text.to_owned(), meaning.to_owned())
    }

    #[test]
    fn each_word_is_told_from_the_description() {
        assert_eq!(
            parts("panes -L work  split-window -h -c /tmp top"),
            [
                pair("panes", "Panes in a terminal"),
                pair("-L work", "The server to talk to"),
                pair("split-window", "Split a pane in two"),
                pair("-h", "Side by side"),
                pair("-c /tmp", "The folder the new pane starts in"),
                pair("top", "command: What runs in the new pane"),
            ]
        );
        let told = explain(&[panes()], "panes kill-pane").expect("described");
        assert_eq!((told.program.as_str(), told.version.as_str()), ("panes", "1.2.0"));
    }

    #[test]
    fn the_other_forms_and_the_unknown_words_are_told_too() {
        assert_eq!(parts("panes splitw -hv")[2], pair("-hv", "-h: Side by side; -v: One over the other"));
        assert_eq!(parts("panes web --port=80")[2], pair("--port=80", "The port to listen on"));
        assert_eq!(parts("panes web --mode lan")[2], pair("--mode lan", "an option; takes mode"));
        assert_eq!(parts("panes completion zsh")[2], pair("zsh", "shell"));
        assert_eq!(
            parts("panes kill-pane -- x")[2..],
            [pair("--", "what follows are arguments, not options"), pair("x", "a value")]
        );
        assert_eq!(
            parts("panes frob --nosuch")[1..],
            [pair("frob", "not in panes's list"), pair("--nosuch", "not in panes's list")]
        );
        assert_eq!(
            parts("panes web --port")[2],
            pair("--port", "The port to listen on"),
            "an option whose value is not typed yet"
        );
    }

    #[test]
    fn nothing_is_told_of_what_is_not_described_or_not_simple() {
        assert_eq!(explain(&[panes()], "other x"), None);
        assert_eq!(explain(&[panes()], "panes kill-pane | cat"), None);
        assert_eq!(explain(&[panes()], ""), None);
        assert_eq!(explain(&[], "panes kill-pane"), None);
    }

    #[test]
    fn a_command_is_shown_with_what_is_under_it() {
        let program = panes();
        let web = show(&program, &["web".to_owned()]).expect("web is a command");
        assert_eq!(
            web,
            "panes web: The page for a phone.\n\
             options:\n  --port, -p <port>: The port to listen on.\n  --quiet: Print nothing.\n  --mode <mode> (one of lan, local)\n\
             commands:\n  start: Start it\n  stop: Stop it\n"
        );
        let root = show(&program, &[]).expect("the program itself");
        assert!(root.starts_with(
            "panes 1.2.0: Panes in a terminal.\noptions:\n  -L <socket>: The server to talk to.\ncommands:\n"
        ));
        assert!(!root.contains("self-test"), "hidden commands are not shown");
        let start = show(&program, &["web".to_owned(), "start".to_owned()]).expect("a command in a command");
        assert_eq!(
            start, "panes web start: Start it.\noptions:\n  --quiet: Print nothing.\n",
            "the option that comes down"
        );
        assert_eq!(show(&program, &["nosuch".to_owned()]), None);
        let completion = show(&program, &["completion".to_owned()]).expect("a command");
        assert!(completion.ends_with("arguments:\n  shell (one of bash, fish, zsh)\n"), "{completion}");
    }

    #[test]
    fn the_help_for_a_model_is_about_the_command_the_line_uses() {
        let programs = [panes()];
        let help_for = |line: &str| help(&programs, line, 8 * 1024).expect("panes is on the line");
        assert!(
            help_for("panes -L x web start | cat").contains("panes web start: Start it."),
            "any line: its words are enough"
        );
        assert!(help_for("sudo panes splitw -h").contains("panes split-window: Split a pane in two."));
        assert!(help_for("panes nosuch").starts_with("From panes's own description of its commands.\npanes 1.2.0"));
        assert_eq!(help(&programs, "ls -l", 8 * 1024), None);
        let short = help(&programs, "panes", 120).expect("panes is on the line");
        assert!(short.len() <= 120 && short.ends_with('\n'), "cut at a line's end: {short:?}");
    }

    #[test]
    fn the_brief_names_every_program_and_lists_the_ones_spoken_of() {
        let programs = [panes()];
        let unnamed = brief(&programs, &["split this in two"], 16 * 1024);
        assert_eq!(
            unnamed,
            "Programs on this machine that describe their own commands:\n- panes 1.2.0: Panes in a terminal.\n"
        );
        let named = brief(&programs, &["用 Panes 把窗格左右分开"], 16 * 1024);
        assert!(named.contains("The commands of panes (use no command or option that is not listed):\n"));
        assert!(
            named.contains("  split-window (splitw) [-h] [-v] [-c dir] [command]: Split a pane in two\n"),
            "{named}"
        );
        assert!(
            named.contains("  web [--port port] [--quiet] [--mode mode] {start, stop}: The page for a phone\n"),
            "{named}"
        );
        assert!(!named.contains("self-test") && !named.contains("--secret"), "nothing hidden");
        assert!(
            brief(&programs, &["x", "panes.exe kill-pane"], 16 * 1024).contains("The commands of panes"),
            "a command run lately"
        );
        assert!(
            !brief(&programs, &["windowpanes are glass"], 16 * 1024).contains("The commands of"),
            "a word of its own"
        );
        assert_eq!(brief(&[], &["panes"], 16 * 1024), "");
    }

    #[test]
    fn a_brief_too_long_loses_the_options_and_then_whole_lines() {
        let programs = [panes()];
        let full = brief(&programs, &["panes"], 16 * 1024);
        let tighter = brief(&programs, &["panes"], full.len() - 1);
        assert!(tighter.contains("  split-window (splitw): Split a pane in two\n"), "names only: {tighter}");
        assert!(!tighter.contains("[-h]"));
        let tightest = brief(&programs, &["panes"], 200);
        assert!(tightest.len() <= 200 && tightest.ends_with('\n'), "{tightest:?}");
        assert!(tightest.starts_with("Programs on this machine"));
    }
}
