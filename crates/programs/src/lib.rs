//! Command lists: what programs say about their own commands, options and
//! arguments, and what UnifyTerm makes of it.
//!
//! A program's description is a file in the OpenCLI format ([`read`]), kept
//! in one folder ([`Programs`]). With the descriptions and the line on the
//! prompt, four things are worked out here, none of which runs a program or
//! asks a model:
//!
//! - [`complete`]: the words that may come next, for the list under the input line;
//! - [`problem`]: the word that is no command or option of the program, and the one near it;
//! - [`explain`]: what each word of the line is;
//! - [`brief`] and [`help`]: the description as text, for a model to read.
//!
//! Nothing of a description is ever run: its words are shown, put on the
//! input line by the user's own key, or sent to the model as text.

mod check;
mod names;
mod offer;
mod read;
mod store;
mod tell;
mod walk;

pub use check::{Problem, problem};
pub use names::{bare, distance, nearest, within_reach};
pub use offer::{Offer, complete};
pub use read::{Error, MOST_BYTES, Read, read};
pub use store::{Programs, directory_in, load, stamp_is_young};
pub use tell::{Explained, brief, explain, help, show};

/// A program with the description of its commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The name as command lines are compared with it: lower case, no `.exe`.
    pub key: String,
    /// The name as the description gives it.
    pub name: String,
    /// One line about the program; may be empty.
    pub summary: String,
    pub version: String,
    /// The program itself as a command: its options, arguments and commands.
    pub root: Command,
}

/// A command: the program itself, or one of the commands under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub aliases: Vec<String>,
    /// The first paragraph of its description, on one line; may be empty.
    pub description: String,
    /// Known, but not offered.
    pub hidden: bool,
    pub options: Vec<Flag>,
    pub arguments: Vec<Argument>,
    pub commands: Vec<Command>,
}

/// An option of a command: `-h`, `--port 80`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    /// As it is typed, dashes and all: `--quiet`.
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub hidden: bool,
    /// Also an option of the commands under its command.
    pub recursive: bool,
    /// The values it takes; empty for an option that stands alone.
    pub arguments: Vec<Argument>,
}

impl Flag {
    /// Whether the word after this option is its value.
    #[must_use]
    pub fn takes_a_value(&self) -> bool {
        self.arguments.first().is_some_and(|argument| argument.minimum >= 1)
    }
}

/// A value: a command's argument, or what an option takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    pub name: String,
    pub description: String,
    /// The values it may be, when the description lists them.
    pub accepted: Vec<String>,
    /// The fewest and the most words it takes; `None`: any number.
    pub minimum: u64,
    pub maximum: Option<u64>,
}

impl Program {
    /// How many commands the description has, at every depth, and how many options.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        fn count(command: &Command) -> (usize, usize) {
            command
                .commands
                .iter()
                .map(count)
                .fold((command.commands.len(), command.options.len()), |sum, inner| (sum.0 + inner.0, sum.1 + inner.1))
        }
        count(&self.root)
    }
}

/// The first sentence of a description, not longer than `most` characters.
pub(crate) fn sentence(description: &str, most: usize) -> String {
    let first = description.split_once(". ").map_or(description, |(first, _)| first).trim_end_matches('.');
    if first.chars().count() <= most {
        return first.to_owned();
    }
    let mut cut: String = first.chars().take(most.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A description with every shape the tests need: options with and without values, an
    /// option that goes down, commands inside a command, accepted values, a hidden command.
    pub(crate) const PANES: &str = r#"{
        "opencli": "0.1",
        "info": {"title": "Panes", "summary": "Panes in a terminal.", "version": "1.2.0"},
        "command": {
            "name": "panes",
            "options": [{"name": "-L", "arguments": [{"name": "socket"}], "description": "The server to talk to."}],
            "commands": [
                {"name": "split-window", "aliases": ["splitw"], "description": "Split a pane in two. The new pane is the active one.",
                 "options": [{"name": "-h", "description": "Side by side."},
                             {"name": "-v", "description": "One over the other."},
                             {"name": "-c", "arguments": [{"name": "dir"}], "description": "The folder the new pane starts in."}],
                 "arguments": [{"name": "command", "description": "What runs in the new pane.", "arity": {"minimum": 0}}]},
                {"name": "swap-window", "description": "Exchange two windows."},
                {"name": "kill-pane", "description": "Close a pane."},
                {"name": "completion", "description": "Print a completer for a shell.",
                 "arguments": [{"name": "shell", "acceptedValues": ["bash", "fish", "zsh"]}]},
                {"name": "web", "description": "The page for a phone.",
                 "options": [{"name": "--port", "aliases": ["-p"], "arguments": [{"name": "port"}], "description": "The port to listen on."},
                             {"name": "--quiet", "recursive": true, "description": "Print nothing."},
                             {"name": "--mode", "arguments": [{"name": "mode", "acceptedValues": ["lan", "local"]}]},
                             {"name": "--secret", "hidden": true}],
                 "commands": [{"name": "start", "description": "Start it."}, {"name": "stop", "description": "Stop it."}]},
                {"name": "self-test", "hidden": true, "description": "For the makers."}
            ]
        }
    }"#;

    pub(crate) fn panes() -> Program {
        read(PANES).expect("the tests' description is one").program
    }

    #[test]
    fn a_description_counts_its_commands_and_options() {
        assert_eq!(
            panes().counts(),
            (8, 8),
            "six commands and two under web; one of the program's own and seven below"
        );
    }

    #[test]
    fn a_sentence_is_the_first_one_and_not_too_long() {
        assert_eq!(sentence("Split a pane in two. The new pane is the active one.", 60), "Split a pane in two");
        assert_eq!(sentence("Close a pane.", 60), "Close a pane");
        assert_eq!(sentence("A long description of it", 10), "A long de…");
        assert_eq!(sentence("", 10), "");
    }
}
