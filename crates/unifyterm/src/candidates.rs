//! The list of commands from the history shown under the input line while
//! the user types: which input it is for, which row is chosen, what the
//! keys do. Drawing it is `terminal.rs`'s; asking the history is the
//! recorder's. This part has no window, so tests drive it.
//!
//! | key            | with the list showing                                   |
//! |----------------|---------------------------------------------------------|
//! | Ctrl-N, Down   | choose the next row (none is chosen at first)           |
//! | Ctrl-P, Up     | choose the row before                                   |
//! | Tab, Right     | put the chosen command on the input line, not run it    |
//! | Enter          | the same, when a row is chosen; else Enter is the shell's |
//! | Esc            | close the list until the input changes                  |

use unifyterm_history::Candidate;

/// A key the list may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Next,
    Previous,
    /// Tab or Right.
    Take,
    Enter,
    Close,
}

/// What came of a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Not the list's: the key goes to the shell.
    Passed,
    /// The list took it.
    Taken,
    /// Put this command on the input line.
    Fill(String),
}

/// The list's state.
#[derive(Debug, Default)]
pub(crate) struct Candidates {
    /// The input the rows are for.
    shown_for: Option<String>,
    rows: Vec<Candidate>,
    chosen: Option<usize>,
    /// The input the list was closed for, or filled with: no list until it changes.
    quiet_for: Option<String>,
    /// The input last asked about, and its number: answers to older questions are dropped.
    asked_for: Option<String>,
    asked: u64,
}

impl Candidates {
    /// The question to ask for the input now on the line, if one is needed:
    /// its number and its text. `None` input (no input line) closes the list.
    pub(crate) fn question(&mut self, input: Option<&str>) -> Option<(u64, String)> {
        let Some(text) = input.filter(|text| !text.trim().is_empty()) else {
            self.close_all();
            return None;
        };
        if self.quiet_for.as_deref() == Some(text) {
            return None;
        }
        self.quiet_for = None;
        if self.asked_for.as_deref() == Some(text) {
            return None;
        }
        self.asked += 1;
        self.asked_for = Some(text.to_owned());
        Some((self.asked, text.to_owned()))
    }

    /// The history's answer to question `number`. An answer to an older one is dropped.
    pub(crate) fn answer(&mut self, number: u64, rows: Vec<Candidate>) {
        if number != self.asked {
            return;
        }
        let input = self.asked_for.clone().unwrap_or_default();
        // The input itself is no suggestion.
        self.rows = rows.into_iter().filter(|row| row.text != input).collect();
        self.shown_for = Some(input);
        self.chosen = None;
    }

    /// The rows to draw and the chosen one; `None` when the list is closed.
    pub(crate) fn visible(&self) -> Option<(&[Candidate], Option<usize>, &str)> {
        let input = self.shown_for.as_deref()?;
        (!self.rows.is_empty() && self.quiet_for.is_none()).then_some((self.rows.as_slice(), self.chosen, input))
    }

    pub(crate) fn key(&mut self, key: Key) -> Outcome {
        let Some((rows, chosen, _)) = self.visible() else { return Outcome::Passed };
        let count = rows.len();
        match (key, chosen) {
            (Key::Next, None) => self.chosen = Some(0),
            (Key::Next, Some(at)) => self.chosen = Some((at + 1) % count),
            (Key::Previous, None) => self.chosen = Some(count - 1),
            (Key::Previous, Some(at)) => self.chosen = Some((at + count - 1) % count),
            (Key::Take | Key::Enter, Some(at)) => {
                let text = rows[at].text.clone();
                self.quiet_for = Some(text.clone());
                self.chosen = None;
                return Outcome::Fill(text);
            }
            // With no row chosen, Tab completes and Enter runs, as the shell does.
            (Key::Take | Key::Enter, None) => return Outcome::Passed,
            (Key::Close, _) => {
                self.quiet_for = self.shown_for.clone();
                self.chosen = None;
            }
        }
        Outcome::Taken
    }

    fn close_all(&mut self) {
        self.shown_for = None;
        self.rows.clear();
        self.chosen = None;
        self.quiet_for = None;
        self.asked_for = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(texts: &[&str]) -> Vec<Candidate> {
        texts
            .iter()
            .map(|text| Candidate {
                text: (*text).into(),
                count: 1,
                fail_count: 0,
                last_at_ms: 0,
                last_exit: Some(0),
                score: 0.0,
            })
            .collect()
    }

    fn shown(list: &Candidates) -> Vec<String> {
        list.visible().map(|(rows, _, _)| rows.iter().map(|row| row.text.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn a_question_per_new_input_and_only_the_last_answer_counts() {
        let mut list = Candidates::default();
        assert_eq!(list.question(None), None);
        assert_eq!(list.question(Some("   ")), None, "nothing typed: no list");
        let (first, _) = list.question(Some("gi")).unwrap();
        assert_eq!(list.question(Some("gi")), None, "the same input: asked already");
        let (second, text) = list.question(Some("git")).unwrap();
        assert_eq!(text, "git");
        list.answer(first, rows(&["gist"]));
        assert!(list.visible().is_none(), "an answer to an older question is dropped");
        list.answer(second, rows(&["git status", "git", "git push"]));
        assert_eq!(shown(&list), ["git status", "git push"], "the input itself is not offered");
        assert_eq!(list.visible().unwrap().2, "git");
        list.question(None);
        assert!(list.visible().is_none(), "no input line: no list");
    }

    #[test]
    fn keys_choose_around_the_ends_and_fill_without_running() {
        let mut list = Candidates::default();
        let (number, _) = list.question(Some("git")).unwrap();
        list.answer(number, rows(&["git status", "git push", "git log"]));
        assert_eq!(list.key(Key::Enter), Outcome::Passed, "nothing chosen: Enter is the shell's");
        assert_eq!(list.key(Key::Take), Outcome::Passed, "and Tab too");
        assert_eq!(list.key(Key::Previous), Outcome::Taken);
        assert_eq!(list.visible().unwrap().1, Some(2), "up from nothing: the last");
        list.key(Key::Next);
        assert_eq!(list.visible().unwrap().1, Some(0), "down past the end: the first");
        list.key(Key::Next);
        assert_eq!(list.key(Key::Enter), Outcome::Fill("git push".into()));
        assert!(list.visible().is_none(), "filled: closed");
        assert_eq!(list.question(Some("git push")), None, "and quiet for the line it filled");
        assert!(list.question(Some("git push ")).is_some(), "until the input changes");
    }

    #[test]
    fn escape_closes_until_the_input_changes() {
        let mut list = Candidates::default();
        let (number, _) = list.question(Some("ca")).unwrap();
        list.answer(number, rows(&["cargo test", "cat x"]));
        assert_eq!(list.key(Key::Close), Outcome::Taken);
        assert!(list.visible().is_none());
        assert_eq!(list.question(Some("ca")), None, "the same input: still closed");
        assert_eq!(list.key(Key::Next), Outcome::Passed, "closed: keys are the shell's");
        let (number, _) = list.question(Some("car")).unwrap();
        list.answer(number, rows(&["cargo test"]));
        assert_eq!(shown(&list), ["cargo test"]);
    }

    #[test]
    fn an_empty_answer_shows_nothing() {
        let mut list = Candidates::default();
        let (number, _) = list.question(Some("zz")).unwrap();
        list.answer(number, Vec::new());
        assert!(list.visible().is_none());
        assert_eq!(list.key(Key::Next), Outcome::Passed);
    }
}
