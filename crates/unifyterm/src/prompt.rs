//! What a shell is doing, from its marks ([`crate::marks`]) and from the
//! Enter key: at its prompt, being typed into, or running a command. When
//! a command ends, a [`Finished`] says what it was, where it ran and how it
//! ended.
//!
//! Shells differ in what they say. PowerShell's hook says the command's
//! text, its exit code and the prompt (no "command starts"); bash, zsh and
//! fish say all of it; cmd.exe says only where the prompt is. So:
//! - the command starts at Enter (or `133;C`, if that comes first);
//! - its text is what `633;E` says, else what was on the input line at Enter;
//! - it ends at `133;D`, or, for a shell that never says so, at the next prompt.

use std::time::Instant;

use crate::marks::Mark;

/// A cell of the screen, as (line, column): lines counted from the top of
/// the screen's history, so that they stay put while the screen scrolls.
pub(crate) type Place = (usize, usize);

/// What the shell is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// No mark seen yet, or a command ended and no prompt has come.
    Unknown,
    /// The prompt is being printed.
    Prompt,
    /// The user types; the input starts at this place.
    Input(Place),
    /// A command runs.
    Running,
}

/// A command that ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finished {
    pub(crate) text: String,
    /// `None` when no hook said where the shell is.
    pub(crate) cwd: Option<String>,
    pub(crate) exit: Option<i32>,
    pub(crate) duration_ms: Option<i64>,
    /// `None` when no hook said which shell this is.
    pub(crate) shell: Option<String>,
    /// The line the command was typed on (counted from the top of the
    /// history), when known: its output is on the lines below.
    pub(crate) line: Option<usize>,
    /// What it printed, as the screen shows it; filled in by the screen.
    pub(crate) output: String,
}

/// Where a prompt is on the screen: from its start (`133;A`) to where the input begins (`133;B`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptSpan {
    pub(crate) start: Place,
    pub(crate) input: Place,
}

/// One session's shell, as its marks tell it.
#[derive(Debug)]
pub(crate) struct Prompt {
    phase: Phase,
    /// Where the prompt being printed began.
    began: Option<Place>,
    /// The newest prompt that was printed whole.
    newest: Option<PromptSpan>,
    cwd: Option<String>,
    shell: Option<String>,
    /// The command's text from `633;E`.
    said: Option<String>,
    /// The input line at Enter.
    typed: Option<String>,
    started: Option<Instant>,
    /// The line the input was on at Enter.
    line: Option<usize>,
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            phase: Phase::Unknown,
            began: None,
            newest: None,
            cwd: None,
            shell: None,
            said: None,
            typed: None,
            started: None,
            line: None,
        }
    }
}

impl Prompt {
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub(crate) fn shell(&self) -> Option<&str> {
        self.shell.as_deref()
    }

    /// The newest prompt printed whole: where it began and where its input begins.
    pub(crate) fn newest(&self) -> Option<PromptSpan> {
        self.newest
    }

    /// A mark arrived; `cursor` is where the cursor was just after it.
    /// A command that ended with it is returned.
    pub(crate) fn mark(&mut self, mark: Mark, cursor: Place, now: Instant) -> Option<Finished> {
        match mark {
            Mark::PromptStart => {
                // A shell that never says a command ended (cmd.exe): it ended now.
                let ended = if self.phase == Phase::Running { self.finish(None, now) } else { None };
                self.phase = Phase::Prompt;
                self.began = Some(cursor);
                return ended;
            }
            Mark::InputStart => {
                self.phase = Phase::Input(cursor);
                // A prompt whose start was not marked: from the start of the input's line.
                let start = self.began.take().unwrap_or((cursor.0, 0));
                self.newest = Some(PromptSpan { start, input: cursor });
                self.said = None;
                self.typed = None;
                self.started = None;
            }
            Mark::CommandStart => {
                self.phase = Phase::Running;
                self.started.get_or_insert(now);
            }
            Mark::CommandEnd(exit) => {
                let ended = self.finish(exit, now);
                self.phase = Phase::Unknown;
                return ended;
            }
            Mark::Directory(path) => self.cwd = Some(path),
            Mark::CommandLine(text) => self.said = Some(text),
            Mark::Shell(name) => self.shell = Some(name),
            // The screen's, not the prompt's.
            Mark::Graphics(_) => {}
        }
        None
    }

    /// The user pressed Enter with `input` on the input line. Only at the
    /// input line: Enter in a running program is the program's.
    pub(crate) fn enter(&mut self, input: Option<String>, now: Instant) {
        if let Phase::Input((line, _)) = self.phase {
            self.typed = input;
            self.started = Some(now);
            self.line = Some(line);
            self.phase = Phase::Running;
        }
    }

    /// The command that was under way, if there was one with any text.
    fn finish(&mut self, exit: Option<i32>, now: Instant) -> Option<Finished> {
        let text = self.said.take().or_else(|| self.typed.take()).map(|text| text.trim().to_owned());
        let started = self.started.take();
        let line = self.line.take();
        self.typed = None;
        let text = text.filter(|text| !text.is_empty())?;
        let duration_ms =
            started.map(|started| i64::try_from(now.duration_since(started).as_millis()).unwrap_or(i64::MAX));
        let (cwd, shell) = (self.cwd.clone(), self.shell.clone());
        Some(Finished { text, cwd, exit, duration_ms, shell, line, output: String::new() })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_newest_prompt_is_from_its_start_to_its_input() {
        let now = Instant::now();
        let mut prompt = Prompt::default();
        assert_eq!(prompt.newest(), None, "no prompt yet");
        prompt.mark(Mark::PromptStart, (40, 0), now);
        assert_eq!(prompt.newest(), None, "not printed whole yet");
        prompt.mark(Mark::InputStart, (41, 2), now);
        assert_eq!(prompt.newest(), Some(PromptSpan { start: (40, 0), input: (41, 2) }), "a prompt of two lines");
        prompt.mark(Mark::CommandStart, (41, 9), now);
        assert_eq!(prompt.newest(), Some(PromptSpan { start: (40, 0), input: (41, 2) }), "kept while a command runs");
        // A shell that marks only where the input begins: the prompt is that line's start to it.
        prompt.mark(Mark::InputStart, (44, 8), now);
        assert_eq!(prompt.newest(), Some(PromptSpan { start: (44, 0), input: (44, 8) }));
    }

    const AT: Place = (3, 10);

    fn later(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn a_shell_that_says_everything() {
        let (mut prompt, t) = (Prompt::default(), Instant::now());
        assert_eq!(prompt.mark(Mark::Shell("bash".into()), AT, t), None);
        assert_eq!(prompt.mark(Mark::Directory("/home/me".into()), AT, t), None);
        prompt.mark(Mark::PromptStart, AT, t);
        assert_eq!(prompt.phase(), Phase::Prompt);
        prompt.mark(Mark::InputStart, AT, t);
        assert_eq!(prompt.phase(), Phase::Input(AT));
        prompt.enter(Some("ls -l ".into()), t);
        prompt.mark(Mark::CommandLine("ls -l".into()), AT, later(t, 1));
        prompt.mark(Mark::CommandStart, AT, later(t, 2));
        assert_eq!(prompt.phase(), Phase::Running);
        let ended = prompt.mark(Mark::CommandEnd(Some(2)), AT, later(t, 250)).unwrap();
        assert_eq!(
            ended,
            Finished {
                text: "ls -l".into(),
                cwd: Some("/home/me".into()),
                exit: Some(2),
                duration_ms: Some(250),
                shell: Some("bash".into()),
                line: Some(3),
                output: String::new(),
            }
        );
        assert_eq!(prompt.phase(), Phase::Unknown);
        assert_eq!(prompt.mark(Mark::PromptStart, AT, t), None, "nothing twice");
    }

    #[test]
    fn powershell_says_the_text_and_the_end_but_no_start() {
        let (mut prompt, t) = (Prompt::default(), Instant::now());
        prompt.mark(Mark::PromptStart, AT, t);
        prompt.mark(Mark::InputStart, AT, t);
        prompt.enter(Some("git status".into()), t);
        // The hook's prompt function: the text, the end, the directory, the next prompt.
        prompt.mark(Mark::CommandLine("git status".into()), AT, later(t, 40));
        let ended = prompt.mark(Mark::CommandEnd(Some(0)), AT, later(t, 40)).unwrap();
        assert_eq!((ended.text.as_str(), ended.exit, ended.duration_ms), ("git status", Some(0), Some(40)));
    }

    #[test]
    fn cmd_says_only_the_prompt() {
        let (mut prompt, t) = (Prompt::default(), Instant::now());
        prompt.mark(Mark::PromptStart, AT, t);
        prompt.mark(Mark::InputStart, AT, t);
        prompt.enter(Some("dir /b".into()), t);
        let ended = prompt.mark(Mark::PromptStart, AT, later(t, 15)).unwrap();
        assert_eq!((ended.text.as_str(), ended.exit, ended.duration_ms), ("dir /b", None, Some(15)));
        assert_eq!(prompt.phase(), Phase::Prompt);
    }

    #[test]
    fn nothing_is_made_of_an_empty_line_or_of_enter_in_a_program() {
        let (mut prompt, t) = (Prompt::default(), Instant::now());
        prompt.enter(Some("not at a prompt".into()), t);
        assert_eq!(prompt.phase(), Phase::Unknown, "Enter outside the input line is the program's");
        prompt.mark(Mark::InputStart, AT, t);
        prompt.enter(Some("   ".into()), t);
        assert_eq!(prompt.mark(Mark::CommandEnd(Some(0)), AT, t), None, "an empty line is no command");
        assert_eq!(prompt.mark(Mark::CommandEnd(Some(0)), AT, t), None, "an end with nothing under way");
        // A program that drew its own screen and read its own Enter keys (vim): still running.
        prompt.mark(Mark::InputStart, AT, t);
        prompt.enter(Some("vim x".into()), t);
        prompt.enter(Some("ignored".into()), t);
        assert_eq!(prompt.mark(Mark::CommandEnd(Some(0)), AT, t).unwrap().text, "vim x");
    }

    #[test]
    fn a_new_prompt_forgets_what_the_last_input_was() {
        let (mut prompt, t) = (Prompt::default(), Instant::now());
        prompt.mark(Mark::InputStart, AT, t);
        prompt.mark(Mark::CommandLine("stale".into()), AT, t);
        prompt.mark(Mark::InputStart, (4, 0), t);
        prompt.enter(Some("fresh".into()), t);
        assert_eq!(prompt.mark(Mark::CommandEnd(None), AT, t).unwrap().text, "fresh");
    }
}
