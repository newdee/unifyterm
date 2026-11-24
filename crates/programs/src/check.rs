//! The word of a line that is no command or option of its program, and the
//! one near it that is.

use crate::walk::{Role, flags_under, program_of, walk, words};
use crate::{Program, nearest};

/// A word that the program's description does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The program's name, as its description gives it.
    pub program: String,
    /// The word, as typed.
    pub word: String,
    /// The line with the nearest word of the description in its place, when one is near.
    pub corrected: Option<String>,
    /// What to tell the user.
    pub says: String,
}

/// The first word of `line` that should be a command or an option of its
/// program and is none the description has. `None` when the program has no
/// description, the line is not one simple command, or every word is known.
///
/// Said only where the description can be taken at its word: a word where
/// only commands may stand, and a `--long` option of a command that has
/// options described. Not said: one-letter options (nothing is near one
/// letter), and `--no-x` when `--x` is described.
#[must_use]
pub fn problem(programs: &[Program], line: &str) -> Option<Problem> {
    let words = words(line)?;
    let program = program_of(programs, &words.first()?.said)?;
    let (steps, _) = walk(program, &words);
    for (word, step) in words.iter().zip(&steps) {
        let (typed, known, kind): (&str, Vec<&str>, &str) = match step.role {
            // Where a command may stand: a word that is none, or an argument that is near one.
            Role::NoCommand | Role::Argument(_) if step.where_a_command_may_be => {
                let level = step.under[step.under.len() - 1];
                let names: Vec<&str> = level
                    .commands
                    .iter()
                    .flat_map(|command| std::iter::once(&command.name).chain(&command.aliases))
                    .map(String::as_str)
                    .collect();
                // A command that also takes arguments: the word is one of those, unless a command is near it.
                if step.role != Role::NoCommand && nearest(&word.said, names.iter().copied()).is_none() {
                    continue;
                }
                (word.said.as_str(), names, "command")
            }
            Role::NoFlag if word.said.starts_with("--") => {
                let typed = word.said.split_once('=').map_or(word.said.as_str(), |(name, _)| name);
                let names: Vec<&str> = flags_under(&step.under)
                    .into_iter()
                    .flat_map(|flag| std::iter::once(&flag.name).chain(&flag.aliases))
                    .map(String::as_str)
                    .filter(|name| name.starts_with("--"))
                    .collect();
                let negated =
                    typed.strip_prefix("--no-").is_some_and(|rest| names.contains(&format!("--{rest}").as_str()));
                if names.is_empty() || negated {
                    continue;
                }
                (typed, names, "option")
            }
            _ => continue,
        };
        let near = nearest(typed, known.iter().copied());
        // In quotes, where the word's letters are in the line is not known: it is named, not replaced.
        let as_typed = line[word.start..word.end] == word.said;
        let corrected = near.filter(|_| as_typed).map(|(name, _)| {
            // Only the name is replaced: a value after `=` stays.
            format!("{}{name}{}", &line[..word.start], &line[word.start + typed.len()..])
        });
        let says = match near {
            Some((name, 1)) => {
                format!("{}'s list has no {kind} '{typed}'; '{name}' is one keystroke away.", program.name)
            }
            Some((name, _)) => {
                format!("{}'s list has no {kind} '{typed}'; '{name}' is two keystrokes away.", program.name)
            }
            None => format!("{}'s list has no {kind} '{typed}'.", program.name),
        };
        return Some(Problem { program: program.name.clone(), word: typed.to_owned(), corrected, says });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panes;

    /// A program that takes a file, or one of its commands.
    fn tool() -> Program {
        let text = r#"{"opencli": "0.1", "info": {}, "command": {"name": "tool",
            "arguments": [{"name": "file", "arity": {"minimum": 0}}],
            "commands": [{"name": "build"}, {"name": "test"}]}}"#;
        crate::read(text).expect("a description").program
    }

    #[test]
    fn where_a_file_or_a_command_may_stand_only_a_near_miss_is_a_problem() {
        let programs = [tool()];
        let found = problem(&programs, "tool biuld -c Release").expect("near a command");
        assert_eq!(found.corrected.as_deref(), Some("tool build -c Release"));
        assert_eq!(problem(&programs, "tool app.dll"), None, "far from every command: the file it may be");
        assert_eq!(problem(&programs, "tool app.dll tset"), None, "after an argument, words are arguments");
        assert_eq!(problem(&programs, "tool build tset"), None, "under a command, its own words");
    }

    fn corrected(line: &str) -> Option<String> {
        problem(&[panes()], line).and_then(|problem| problem.corrected)
    }

    #[test]
    fn a_mistyped_command_is_put_right_and_the_rest_kept() {
        assert_eq!(corrected("panes splt-window -h"), Some("panes split-window -h".into()));
        assert_eq!(corrected("panes  -L work  kil-pane"), Some("panes  -L work  kill-pane".into()));
        assert_eq!(corrected("panes web strat"), Some("panes web start".into()), "a command inside a command");
        assert_eq!(corrected("panes spiltw"), Some("panes splitw".into()), "near an alias");
        let found = problem(&[panes()], "panes splt-window").expect("no such command");
        assert_eq!(found.says, "panes's list has no command 'splt-window'; 'split-window' is one keystroke away.");
        assert_eq!((found.program.as_str(), found.word.as_str()), ("panes", "splt-window"));
    }

    #[test]
    fn a_mistyped_long_option_is_put_right() {
        assert_eq!(corrected("panes web --prot 80"), Some("panes web --port 80".into()));
        assert_eq!(
            corrected("panes web --prot=80 --quiet"),
            Some("panes web --port=80 --quiet".into()),
            "its value stays"
        );
        assert_eq!(
            corrected("panes web start --quite"),
            Some("panes web start --quiet".into()),
            "an option that goes down"
        );
        let found = problem(&[panes()], "panes web --qiuet").expect("no such option");
        assert_eq!(found.says, "panes's list has no option '--qiuet'; '--quiet' is one keystroke away.");
    }

    #[test]
    fn a_word_far_from_all_is_named_and_not_replaced() {
        let found = problem(&[panes()], "panes frobnicate now").expect("no such command");
        assert_eq!(found.corrected, None);
        assert_eq!(found.says, "panes's list has no command 'frobnicate'.");
        let found = problem(&[panes()], "panes web --frobnicate").expect("no such option");
        assert_eq!((found.corrected, found.says.as_str()), (None, "panes's list has no option '--frobnicate'."));
    }

    #[test]
    fn what_may_be_right_is_not_called_wrong() {
        let fine = [
            "panes split-window -h -c /tmp top",
            "panes splitw -hv",
            "panes web --port=80",
            "panes web -x",                // one letter: nothing is near it
            "panes split-window -x",       // the same
            "panes web --no-quiet",        // the opposite of a described option
            "panes kill-pane --force",     // a command with no options described
            "panes kill-pane anything",    // nothing described: a value
            "panes completion tcsh",       // a value not among the accepted ones is the program's to refuse
            "panes split-window sp",       // an argument, not a command
            "panes kill-pane -- --nosuch", // after --, arguments
            "other frobnicate",            // no description
            "panes frobnicate | cat",      // not one simple command
            "panes",
            "",
        ];
        for line in fine {
            assert_eq!(problem(&[panes()], line), None, "{line}");
        }
    }

    #[test]
    fn the_first_problem_is_the_one_told_and_quotes_keep_the_line_as_it_is() {
        let found = problem(&[panes()], "panes web --prot 80 strat").expect("two problems");
        assert_eq!(found.word, "--prot", "the first one");
        let quoted = problem(&[panes()], "panes 'splt-window'").expect("no such command");
        assert_eq!(quoted.corrected, None, "a word in quotes is named, not replaced");
        assert!(quoted.says.contains("'split-window'"));
    }
}
