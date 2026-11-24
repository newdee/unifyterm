//! The words that may come next on the line, for the list under it.

use crate::walk::{Role, flags_under, program_of, walk, words};
use crate::{Argument, Program, sentence};

/// The longest note beside an offered word.
const NOTE: usize = 56;

/// A word offered: the whole line with the word in place, and a few words about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub line: String,
    pub note: String,
}

fn values(argument: Option<&Argument>) -> impl Iterator<Item = (&str, String)> {
    argument
        .into_iter()
        .flat_map(|argument| argument.accepted.iter().map(|value| (value.as_str(), argument.name.clone())))
}

/// The words the descriptions have for the word being typed at the end of
/// `line`, at most `most` of them, in the descriptions' own order: commands
/// (or an argument's accepted values), options once a `-` is typed, an
/// option's accepted values after it. Hidden ones are not offered. Empty
/// when the line's program has no description, or the line is not one
/// simple command.
#[must_use]
pub fn complete(programs: &[Program], line: &str, most: usize) -> Vec<Offer> {
    let Some(mut before) = words(line) else { return Vec::new() };
    // The word being typed: the last one, or an empty one after a blank.
    let typing = if line.ends_with(char::is_whitespace) { None } else { before.pop() };
    let (start, typed) = match &typing {
        Some(word) => (word.start, word.said.as_str()),
        None => (line.len(), ""),
    };
    // A word in quotes is the user's own value.
    if typing.as_ref().is_some_and(|word| line[word.start..word.end] != word.said) {
        return Vec::new();
    }
    let Some(program) = before.first().and_then(|first| program_of(programs, &first.said)) else { return Vec::new() };
    let (steps, stand) = walk(program, &before);
    // After a word the description does not know, what comes next is not known either.
    if steps.iter().any(|step| matches!(step.role, Role::NoCommand | Role::NoFlag)) {
        return Vec::new();
    }
    let level = stand.under[stand.under.len() - 1];
    let found: Vec<(&str, String)> = if let Some(flag) = stand.awaiting {
        values(flag.arguments.first()).collect()
    } else if stand.after_rest {
        values(stand.argument).collect()
    } else if typed.starts_with('-') {
        flags_under(&stand.under)
            .into_iter()
            .filter(|flag| !flag.hidden)
            .filter_map(|flag| {
                let name = std::iter::once(&flag.name).chain(&flag.aliases).find(|name| name.starts_with(typed))?;
                Some((name.as_str(), sentence(&flag.description, NOTE)))
            })
            .collect()
    } else {
        // After an argument was given, no command comes any more.
        let may_follow = stand.a_command_may_follow;
        let commands = level.commands.iter().filter(|command| may_follow && !command.hidden).filter_map(|command| {
            let name = std::iter::once(&command.name).chain(&command.aliases).find(|name| name.starts_with(typed))?;
            Some((name.as_str(), sentence(&command.description, NOTE)))
        });
        commands.chain(values(stand.argument)).collect()
    };
    found
        .into_iter()
        .filter(|(word, _)| word.starts_with(typed) && *word != typed)
        .take(most)
        .map(|(word, note)| Offer { line: format!("{}{word}", &line[..start]), note })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panes;

    fn offered(line: &str) -> Vec<String> {
        complete(&[panes()], line, 6).into_iter().map(|offer| offer.line).collect()
    }

    #[test]
    fn commands_are_offered_for_the_word_being_typed() {
        assert_eq!(
            offered("panes s"),
            ["panes split-window", "panes swap-window"],
            "the hidden self-test is not offered"
        );
        assert_eq!(offered("panes sp"), ["panes split-window"]);
        assert_eq!(offered("panes splitw"), Vec::<String>::new(), "an alias typed in full needs nothing");
        assert_eq!(offered("panes spli"), ["panes split-window"], "the name before its alias");
        assert_eq!(offered("panes  -L work  k"), ["panes  -L work  kill-pane"], "what was typed stays as typed");
        assert_eq!(offered("panes web st"), ["panes web start", "panes web stop"]);
        let first = &complete(&[panes()], "panes sp", 6)[0];
        assert_eq!(first.note, "Split a pane in two", "the first sentence");
        assert_eq!(offered("panes ").len(), 5, "after a blank: the commands that are not hidden");
        assert_eq!(complete(&[panes()], "panes ", 2).len(), 2, "no more than asked for");
    }

    #[test]
    fn options_come_once_a_dash_is_typed() {
        assert_eq!(offered("panes split-window "), Vec::<String>::new(), "no options before a dash");
        assert_eq!(
            offered("panes split-window -"),
            ["panes split-window -h", "panes split-window -v", "panes split-window -c"]
        );
        assert_eq!(
            offered("panes web --"),
            ["panes web --port", "panes web --quiet", "panes web --mode"],
            "not the hidden one"
        );
        assert_eq!(
            offered("panes web -"),
            ["panes web --port", "panes web --quiet", "panes web --mode"],
            "each option once"
        );
        assert_eq!(offered("panes web start --"), ["panes web start --quiet"], "the option that goes down");
        assert_eq!(offered("panes web --port"), Vec::<String>::new(), "typed in full");
    }

    #[test]
    fn accepted_values_are_offered_where_they_go() {
        assert_eq!(
            offered("panes completion "),
            ["panes completion bash", "panes completion fish", "panes completion zsh"]
        );
        assert_eq!(offered("panes completion z"), ["panes completion zsh"]);
        assert_eq!(offered("panes completion zsh "), Vec::<String>::new(), "the argument takes one word");
        assert_eq!(offered("panes web --mode l"), ["panes web --mode lan", "panes web --mode local"]);
        assert_eq!(complete(&[panes()], "panes web --mode ", 6)[0].note, "mode", "the note names the value");
        assert_eq!(offered("panes web --port "), Vec::<String>::new(), "a value nothing is known of");
    }

    #[test]
    fn nothing_is_offered_where_nothing_is_known() {
        assert_eq!(offered("pan"), Vec::<String>::new(), "the program's own name is not this list's");
        assert_eq!(offered("other sp"), Vec::<String>::new(), "a program with no description");
        assert_eq!(offered("panes sp | grep x"), Vec::<String>::new(), "not one simple command");
        assert_eq!(offered("panes nosuch s"), Vec::<String>::new(), "after a word that is no command");
        assert_eq!(offered("panes web --nosuch s"), Vec::<String>::new(), "after an option that is none");
        assert_eq!(offered("panes 'sp"), Vec::<String>::new(), "a quote left open");
        assert_eq!(offered("panes \"sp\""), Vec::<String>::new(), "a word in quotes");
        assert_eq!(offered(""), Vec::<String>::new());
        assert_eq!(complete(&[], "panes sp", 6), Vec::new(), "no descriptions at all");
        assert_eq!(offered("Panes.exe sp"), ["Panes.exe split-window"], "the program's name as Windows may write it");
    }
}
