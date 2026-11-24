//! Going along a command line word by word with a program's description:
//! which word is a command, which an option, which a value.
//!
//! Only a line that is one simple command is gone along: with a pipe, a
//! redirection, a variable or the like in it, what the shell makes of the
//! words is not known here, and nothing is said about the line.

use crate::{Argument, Command, Flag, Program, bare};

/// Outside quotes, a line with one of these is not one simple command.
const NOT_SIMPLE: &[char] = &['|', '&', ';', '<', '>', '(', ')', '{', '}', '$', '`'];

/// A word of the line: where it is, and what it says without its quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) said: String,
}

/// The words of a line that is one simple command; `None` for any other
/// line (a pipe, a variable, a quote left open).
pub(crate) fn words(line: &str) -> Option<Vec<Word>> {
    let mut words = Vec::new();
    let mut open: Option<Word> = None;
    let mut quote: Option<char> = None;
    for (at, c) in line.char_indices() {
        match quote {
            Some(closing) => {
                // In double quotes a shell still reads variables and commands.
                if closing == '"' && (c == '$' || c == '`') {
                    return None;
                }
                if c == closing {
                    quote = None;
                } else if let Some(word) = open.as_mut() {
                    word.said.push(c);
                }
            }
            None if c.is_whitespace() => {
                if let Some(mut word) = open.take() {
                    word.end = at;
                    words.push(word);
                }
            }
            None if NOT_SIMPLE.contains(&c) => return None,
            None => {
                let word = open.get_or_insert(Word { start: at, end: at, said: String::new() });
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else {
                    word.said.push(c);
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if let Some(mut word) = open.take() {
        word.end = line.len();
        words.push(word);
    }
    Some(words)
}

/// The program whose description is for the command named `word`.
pub(crate) fn program_of<'p>(programs: &'p [Program], word: &str) -> Option<&'p Program> {
    if word.contains(['/', '\\']) {
        return None;
    }
    let key = bare(word);
    programs.iter().find(|program| program.key == key)
}

/// What a word of the line is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Role<'p> {
    Program,
    Command(&'p Command),
    /// An option. `attached`: the word holds the option's value too (`--name=value`, `-n5`).
    Flag {
        flag: &'p Flag,
        attached: bool,
    },
    /// Several one-letter options in one word (`-hv`).
    Flags(Vec<&'p Flag>),
    /// The value of the option before it.
    ValueOf(&'p Flag),
    /// An argument: the described one it is, or `None` for a value the description does not tell of.
    Argument(Option<&'p Argument>),
    /// `--`: what follows are arguments.
    Rest,
    /// A word where a command of the description should be, that is none of them.
    NoCommand,
    /// A word that begins with `-` and is no option of the description.
    NoFlag,
}

/// A word with what it is, and the commands it was read under (the program's own first).
#[derive(Debug, Clone)]
pub(crate) struct Step<'p> {
    pub(crate) role: Role<'p>,
    pub(crate) under: Vec<&'p Command>,
    /// The word stands where a command of the last of `under` could: no argument was given there yet.
    pub(crate) where_a_command_may_be: bool,
}

/// Where the line stands after its words: what the next word would be read as.
#[derive(Debug, Clone)]
pub(crate) struct Stand<'p> {
    /// The commands entered, the program's own first.
    pub(crate) under: Vec<&'p Command>,
    /// The option whose value the next word is.
    pub(crate) awaiting: Option<&'p Flag>,
    /// The argument the next word that is no command would be.
    pub(crate) argument: Option<&'p Argument>,
    pub(crate) after_rest: bool,
    /// A command may come next: no argument was given to the last command yet.
    pub(crate) a_command_may_follow: bool,
}

/// The options a word may be under these commands: the last command's own,
/// and those of the commands above that go down to their subcommands.
pub(crate) fn flags_under<'p>(under: &[&'p Command]) -> Vec<&'p Flag> {
    let Some((last, above)) = under.split_last() else { return Vec::new() };
    let mut flags: Vec<&Flag> = last.options.iter().collect();
    for command in above.iter().rev() {
        flags.extend(command.options.iter().filter(|flag| flag.recursive));
    }
    flags
}

fn flag_named<'p>(under: &[&'p Command], name: &str) -> Option<&'p Flag> {
    flags_under(under).into_iter().find(|flag| flag.name == name || flag.aliases.iter().any(|alias| alias == name))
}

/// Which argument of `command` a word is, with `taken` words given to arguments before it.
fn argument_at(command: &Command, taken: usize) -> Option<&Argument> {
    let mut left = taken;
    for argument in &command.arguments {
        let Some(most) = argument.maximum else { return Some(argument) };
        let most = usize::try_from(most).unwrap_or(usize::MAX);
        if left < most {
            return Some(argument);
        }
        left -= most;
    }
    None
}

/// What a word that begins with `-` is, and the option whose value comes next, if one does.
fn dashed<'p>(under: &[&'p Command], said: &str) -> (Role<'p>, Option<&'p Flag>) {
    if let Some(flag) = flag_named(under, said) {
        return (Role::Flag { flag, attached: false }, flag.takes_a_value().then_some(flag));
    }
    if let Some((name, _)) = said.split_once('=')
        && let Some(flag) = flag_named(under, name)
    {
        return (Role::Flag { flag, attached: true }, None);
    }
    if said.starts_with("--") {
        return (Role::NoFlag, None);
    }
    // One dash and more than one letter: letters that are each an option, or an option with its value.
    let letters: Vec<char> = said.chars().skip(1).collect();
    let each: Option<Vec<&Flag>> = letters.iter().map(|letter| flag_named(under, &format!("-{letter}"))).collect();
    if let Some(each) = each.filter(|each| each.len() > 1) {
        let awaiting = each.last().copied().filter(|flag| flag.takes_a_value());
        // Only the last may take a value: before it, a letter that takes one makes the rest its value.
        if each[..each.len() - 1].iter().all(|flag| !flag.takes_a_value()) {
            return (Role::Flags(each), awaiting);
        }
    }
    if let Some(first) = letters.first()
        && letters.len() > 1
        && let Some(flag) = flag_named(under, &format!("-{first}"))
        && !flag.arguments.is_empty()
    {
        return (Role::Flag { flag, attached: true }, None);
    }
    (Role::NoFlag, None)
}

/// Goes along `words` (the program's name first) with `program`'s description.
pub(crate) fn walk<'p>(program: &'p Program, words: &[Word]) -> (Vec<Step<'p>>, Stand<'p>) {
    let mut under = vec![&program.root];
    let mut steps = Vec::with_capacity(words.len());
    let mut awaiting: Option<&Flag> = None;
    let mut taken = 0;
    let mut after_rest = false;
    // The word after an option that is not described may be that option's value.
    let mut after_no_flag = false;
    for (index, word) in words.iter().enumerate() {
        let said = word.said.as_str();
        let may_be_a_value = std::mem::take(&mut after_no_flag);
        let mut where_a_command_may_be = false;
        let role = if index == 0 {
            Role::Program
        } else if let Some(flag) = awaiting.take() {
            Role::ValueOf(flag)
        } else if after_rest {
            taken += 1;
            Role::Argument(argument_at(under[under.len() - 1], taken - 1))
        } else if said == "--" {
            after_rest = true;
            Role::Rest
        } else if said.starts_with('-') && said.len() > 1 {
            let (role, next) = dashed(&under, said);
            awaiting = next;
            after_no_flag = role == Role::NoFlag;
            role
        } else {
            let level = under[under.len() - 1];
            // After an argument, the words that follow are arguments too, whatever they are called.
            let command = level
                .commands
                .iter()
                .filter(|_| taken == 0)
                .find(|command| command.name == said || command.aliases.iter().any(|alias| alias == said));
            where_a_command_may_be = taken == 0 && !may_be_a_value && !level.commands.is_empty();
            if let Some(command) = command {
                steps.push(Step { role: Role::Command(command), under: under.clone(), where_a_command_may_be });
                under.push(command);
                taken = 0;
                continue;
            } else if may_be_a_value {
                Role::Argument(None)
            } else if !level.arguments.is_empty() || level.commands.is_empty() {
                taken += 1;
                Role::Argument(argument_at(level, taken - 1))
            } else {
                Role::NoCommand
            }
        };
        steps.push(Step { role, under: under.clone(), where_a_command_may_be });
    }
    let argument = argument_at(under[under.len() - 1], taken);
    let a_command_may_follow = taken == 0 && !after_rest;
    (steps, Stand { under, awaiting, argument, after_rest, a_command_may_follow })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panes;

    fn said(line: &str) -> Option<Vec<String>> {
        words(line).map(|words| words.into_iter().map(|word| word.said).collect())
    }

    #[test]
    fn a_simple_line_is_split_into_its_words() {
        assert_eq!(said("panes  split-window -h"), Some(vec!["panes".into(), "split-window".into(), "-h".into()]));
        assert_eq!(said("a 'b c' \"d e\"f"), Some(vec!["a".into(), "b c".into(), "d ef".into()]));
        assert_eq!(said(""), Some(vec![]));
        let line = "x  yy ";
        let found = words(line).expect("simple");
        assert_eq!((found[1].start, found[1].end), (3, 5));
        assert_eq!(&line[found[1].start..found[1].end], "yy");
    }

    #[test]
    fn a_line_the_shell_reads_more_into_is_not_split() {
        for line in
            ["a | b", "a > f", "a; b", "a && b", "echo $HOME", "a `b`", "a $(b)", "a \"$x\"", "a 'open", "{ a }"]
        {
            assert_eq!(said(line), None, "{line}");
        }
        assert!(said("a '$x | y'").is_some(), "in single quotes nothing is read");
    }

    fn roles(line: &str) -> Vec<String> {
        let program = panes();
        let words = words(line).expect("simple");
        let (steps, _) = walk(&program, &words);
        steps
            .iter()
            .map(|step| match &step.role {
                Role::Program => "program".to_owned(),
                Role::Command(command) => format!("command {}", command.name),
                Role::Flag { flag, attached: false } => format!("flag {}", flag.name),
                Role::Flag { flag, attached: true } => format!("flag {} with its value", flag.name),
                Role::Flags(flags) => {
                    format!("flags {}", flags.iter().map(|flag| flag.name.as_str()).collect::<String>())
                }
                Role::ValueOf(flag) => format!("value of {}", flag.name),
                Role::Argument(Some(argument)) => format!("argument {}", argument.name),
                Role::Argument(None) => "a value".to_owned(),
                Role::Rest => "rest".to_owned(),
                Role::NoCommand => "no command".to_owned(),
                Role::NoFlag => "no flag".to_owned(),
            })
            .collect()
    }

    #[test]
    fn each_word_is_read_as_what_the_description_says() {
        assert_eq!(
            roles("panes -L work split-window -h -c /tmp top now"),
            [
                "program",
                "flag -L",
                "value of -L",
                "command split-window",
                "flag -h",
                "flag -c",
                "value of -c",
                "argument command",
                "argument command"
            ],
            "an argument that takes any number of words takes the words that follow"
        );
    }

    #[test]
    fn options_in_their_other_forms() {
        assert_eq!(roles("panes splitw -hv"), ["program", "command split-window", "flags -h-v"]);
        assert_eq!(roles("panes web --port=80"), ["program", "command web", "flag --port with its value"]);
        assert_eq!(roles("panes web -p80"), ["program", "command web", "flag --port with its value"]);
        assert_eq!(
            roles("panes web --port 80 --quiet"),
            ["program", "command web", "flag --port", "value of --port", "flag --quiet"]
        );
        assert_eq!(roles("panes web --prot 80"), ["program", "command web", "no flag", "a value"]);
        assert_eq!(roles("panes web -x"), ["program", "command web", "no flag"]);
        assert_eq!(roles("panes kill-pane -- -h"), ["program", "command kill-pane", "rest", "a value"]);
        assert_eq!(roles("panes -"), ["program", "no command"], "a lone dash is a word, not an option");
    }

    #[test]
    fn a_word_that_is_no_command_is_told_from_a_value() {
        assert_eq!(roles("panes splt-window -h"), ["program", "no command", "no flag"]);
        assert_eq!(
            roles("panes kill-pane now"),
            ["program", "command kill-pane", "a value"],
            "nothing described: a value"
        );
        assert_eq!(roles("panes completion zsh fish"), ["program", "command completion", "argument shell", "a value"]);
        assert_eq!(roles("panes web --quiet start"), ["program", "command web", "flag --quiet", "command start"]);
        assert_eq!(
            roles("panes web start --quiet"),
            ["program", "command web", "command start", "flag --quiet"],
            "--quiet goes down to web's commands"
        );
        assert_eq!(
            roles("panes web start --port 1"),
            ["program", "command web", "command start", "no flag", "a value"]
        );
    }

    #[test]
    fn where_the_line_stands_says_what_comes_next() {
        let program = panes();
        let stand = |line: &str| walk(&program, &words(line).expect("simple")).1;
        assert_eq!(stand("panes web --port").awaiting.map(|flag| flag.name.as_str()), Some("--port"));
        assert_eq!(stand("panes completion").argument.map(|argument| argument.name.as_str()), Some("shell"));
        assert_eq!(stand("panes completion zsh").argument.map(|argument| argument.name.as_str()), None);
        assert_eq!(stand("panes web start").under.len(), 3);
        assert!(stand("panes kill-pane --").after_rest);
        assert_eq!(
            program_of(std::slice::from_ref(&program), "Panes.EXE").map(|program| program.key.as_str()),
            Some("panes")
        );
        assert!(program_of(std::slice::from_ref(&program), "./panes").is_none(), "a path is the user's own program");
    }
}
