//! The settings as a list on the terminal's screen.
//!
//! [`Screen`] is the list and what a key does to it: no terminal is needed
//! to make one or to press its keys, so the tests do both. [`run`] is the
//! part with a terminal: it draws the lines [`Screen::lines`] gives and
//! passes the keys on.
//!
//! A change is written to the file at once. UnifyTerm reads the file again
//! when it changes, so a font's size is seen as it is chosen; `u` takes the
//! last change back.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::{cursor, execute, queue, terminal};
use unicode_width::UnicodeWidthStr as _;
use unifyterm_config::{File, KEYS, Key, Kind, default_of};

use super::takes;

/// Columns before a setting's name, and between the longest name and the values.
const INDENT: usize = 2;
const GAP: usize = 2;
/// The lines under the list: a rule, what the setting does, what it takes, what happened, the keys.
const FOOT: usize = 5;
/// The lines above it: the title, and the search.
const HEAD: usize = 2;
const KEYS_HELP: &str = "↑↓ move   Enter change   d default   u undo   / search   q leave";
const EDIT_HELP: &str = "Enter keep   Esc leave it as it was   Ctrl-U empty the line";
const SEARCH_HELP: &str = "type to narrow the list   Enter keep   Esc show all";

/// How a piece of a line looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Look {
    Plain,
    Dim,
    Title,
    /// A value the file sets.
    Set,
    /// The line the cursor is on.
    Chosen,
    Good,
    Wrong,
}

/// One line of the screen: pieces, each with its look.
pub(crate) type Line = Vec<(String, Look)>;

/// What the keys do now.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Move,
    /// A value is being typed.
    Edit(String),
    /// The words to narrow the list by are being typed.
    Search,
}

/// What a key leaves to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    Stay,
    Leave,
}

pub(crate) struct Screen {
    file: File,
    /// The setting the cursor is on: an index into the settings shown.
    at: usize,
    /// The first line of the list on the screen.
    top: usize,
    mode: Mode,
    /// Only settings with this in their name or their line of help are shown.
    search: String,
    /// What the last key did, and whether it went well.
    said: Option<(String, Look)>,
    /// The file before each change, the last one last.
    before: Vec<File>,
}

impl Screen {
    pub(crate) const fn new(file: File) -> Self {
        Self { file, at: 0, top: 0, mode: Mode::Move, search: String::new(), said: None, before: Vec::new() }
    }

    /// The settings the search leaves.
    fn shown(&self) -> Vec<&'static Key> {
        let wanted = self.search.to_lowercase();
        KEYS.iter().filter(|key| key.name.contains(&wanted) || key.says.to_lowercase().contains(&wanted)).collect()
    }

    fn chosen(&self) -> Option<&'static Key> {
        self.shown().get(self.at).copied()
    }

    /// Writes a change that was accepted, and says so. A file that cannot
    /// be written leaves the settings as they were.
    fn keep(&mut self, was: File, what: String) {
        match self.file.save() {
            Ok(()) => {
                self.before.push(was);
                self.said = Some((what, Look::Good));
            }
            Err(error) => {
                self.file = was;
                self.said = Some((format!("{error:#}"), Look::Wrong));
            }
        }
    }

    /// Gives the chosen setting `value`; says why when it cannot have it.
    fn set(&mut self, key: &'static Key, value: &str) -> Next {
        let was = self.file.clone();
        match self.file.set(key, value) {
            Ok(()) => self.keep(was, format!("{} = {}", key.name, self.file.in_use(key))),
            Err(error) => self.said = Some((format!("{error:#}"), Look::Wrong)),
        }
        Next::Stay
    }

    /// Enter on a setting: a switch turns, a choice goes to its next one,
    /// anything else is typed.
    fn change(&mut self) -> Next {
        let Some(key) = self.chosen() else { return Next::Stay };
        let now = self.file.in_use(key);
        match key.kind {
            Kind::Switch => self.set(key, if now == "true" { "false" } else { "true" }),
            Kind::OneOf(choices) => {
                let place = choices.iter().position(|choice| *choice == now).map_or(0, |place| place + 1);
                self.set(key, choices[place % choices.len()])
            }
            _ => {
                self.mode = Mode::Edit(now);
                self.said = None;
                Next::Stay
            }
        }
    }

    fn reset(&mut self) {
        let Some(key) = self.chosen() else { return };
        let was = self.file.clone();
        match self.file.reset(key) {
            Ok(()) => self.keep(was, format!("{} is the default again: {}", key.name, default_of(key))),
            Err(error) => self.said = Some((format!("{error:#}"), Look::Wrong)),
        }
    }

    fn undo(&mut self) {
        let Some(mut was) = self.before.pop() else {
            self.said = Some(("nothing to take back".into(), Look::Dim));
            return;
        };
        match was.save() {
            Ok(()) => {
                self.file = was;
                self.said = Some(("the last change is taken back".into(), Look::Good));
            }
            Err(error) => self.said = Some((format!("{error:#}"), Look::Wrong)),
        }
    }

    fn step(&mut self, by: isize) {
        let last = self.shown().len().saturating_sub(1);
        self.at = self.at.saturating_add_signed(by).min(last);
    }

    /// What a key does.
    pub(crate) fn key(&mut self, key: KeyEvent) -> Next {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('c') {
            return Next::Leave;
        }
        match std::mem::replace(&mut self.mode, Mode::Move) {
            Mode::Move => return self.moving(key.code),
            Mode::Edit(mut text) => {
                match key.code {
                    KeyCode::Enter => {
                        if let Some(setting) = self.chosen() {
                            self.set(setting, &text);
                        }
                        // A value that was refused stays to be mended.
                        if matches!(self.said, Some((_, Look::Wrong))) {
                            self.mode = Mode::Edit(text);
                        }
                        return Next::Stay;
                    }
                    KeyCode::Esc => {
                        self.said = None;
                        return Next::Stay;
                    }
                    KeyCode::Backspace => drop(text.pop()),
                    KeyCode::Char('u') if control => text.clear(),
                    KeyCode::Char(typed) if !control => text.push(typed),
                    _ => {}
                }
                self.mode = Mode::Edit(text);
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Enter => return Next::Stay,
                    KeyCode::Esc => {
                        self.search.clear();
                        self.at = 0;
                        return Next::Stay;
                    }
                    KeyCode::Backspace => drop(self.search.pop()),
                    KeyCode::Char('u') if control => self.search.clear(),
                    KeyCode::Char(typed) if !control => self.search.push(typed),
                    _ => {}
                }
                self.at = 0;
                self.mode = Mode::Search;
            }
        }
        Next::Stay
    }

    fn moving(&mut self, code: KeyCode) -> Next {
        const PAGE: isize = 10;
        match code {
            KeyCode::Char('q') => return Next::Leave,
            KeyCode::Esc if self.search.is_empty() => return Next::Leave,
            KeyCode::Esc => {
                self.search.clear();
                self.at = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::PageUp => self.step(-PAGE),
            KeyCode::PageDown => self.step(PAGE),
            KeyCode::Home => self.at = 0,
            KeyCode::End => self.at = self.shown().len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Char(' ') => return self.change(),
            KeyCode::Char('d') | KeyCode::Delete => self.reset(),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.said = None;
            }
            _ => {}
        }
        Next::Stay
    }

    /// The screen's lines for a terminal `width` columns by `height` lines,
    /// and where the cursor is while something is typed.
    pub(crate) fn lines(&mut self, width: usize, height: usize) -> (Vec<Line>, Option<(usize, usize)>) {
        let shown = self.shown();
        let names = KEYS.iter().map(|key| key.name.len()).max().unwrap_or(0);
        let value_at = INDENT + names + GAP;
        // The list: a heading before each section's first setting.
        let mut list: Vec<(Line, Option<usize>)> = Vec::new();
        for (index, key) in shown.iter().enumerate() {
            if index == 0 || shown[index - 1].section() != key.section() {
                if index > 0 {
                    list.push((Vec::new(), None));
                }
                list.push((vec![(format!("[{}]", key.section()), Look::Title)], None));
            }
            let chosen = index == self.at;
            let name = format!("{:INDENT$}{:names$}{:GAP$}", "", key.name, "");
            let value = match (&self.mode, chosen) {
                (Mode::Edit(text), true) => (text.clone(), Look::Plain),
                _ => match self.file.value(key) {
                    Some(value) => (value, Look::Set),
                    None => (default_of(key), Look::Dim),
                },
            };
            list.push((vec![(name, if chosen { Look::Chosen } else { Look::Plain }), value], Some(index)));
        }
        // The chosen setting stays on the screen.
        let room = height.saturating_sub(HEAD + FOOT).max(1);
        let chosen_line = list.iter().position(|(_, index)| *index == Some(self.at)).unwrap_or(0);
        if chosen_line < self.top + 1 {
            self.top = chosen_line.saturating_sub(1);
        } else if chosen_line >= self.top + room {
            self.top = chosen_line + 1 - room;
        }
        let mut lines: Vec<Line> = vec![
            vec![("UnifyTerm settings".into(), Look::Title), (format!("   {}", self.file.path().display()), Look::Dim)],
            if self.search.is_empty() && self.mode != Mode::Search {
                Vec::new()
            } else {
                vec![
                    (format!("/{}", self.search), Look::Plain),
                    (format!("   {} of {}", shown.len(), KEYS.len()), Look::Dim),
                ]
            },
        ];
        let mut cursor = (self.mode == Mode::Search).then(|| (1 + self.search.width(), 1));
        for (offset, (line, index)) in list.into_iter().skip(self.top).take(room).enumerate() {
            if let (Mode::Edit(text), true) = (&self.mode, index == Some(self.at)) {
                cursor = Some((value_at + text.width(), HEAD + offset));
            }
            lines.push(line);
        }
        lines.resize(HEAD + room, Vec::new());
        lines.push(vec![("─".repeat(width), Look::Dim)]);
        let chosen = self.chosen();
        lines.push(chosen.map_or_else(Vec::new, |key| vec![(key.says.to_owned(), Look::Plain)]));
        lines.push(chosen.map_or_else(
            || vec![("no setting has that in its name or its help".to_owned(), Look::Dim)],
            |key| {
                let about = format!(
                    "takes {}   default: {}   used {}",
                    takes(key.kind),
                    shown_default(key),
                    key.applies().says()
                );
                vec![(about, Look::Dim)]
            },
        ));
        lines.push(self.said.clone().map_or_else(Vec::new, |said| vec![said]));
        let help = match self.mode {
            Mode::Move => KEYS_HELP,
            Mode::Edit(_) => EDIT_HELP,
            Mode::Search => SEARCH_HELP,
        };
        lines.push(vec![(help.to_owned(), Look::Dim)]);
        (lines.into_iter().map(|line| cut(line, width)).collect(), cursor)
    }
}

fn shown_default(key: &Key) -> String {
    let default = default_of(key);
    if default.is_empty() { "not set".into() } else { default }
}

/// `line`, no wider than `width` columns.
fn cut(line: Line, width: usize) -> Line {
    let mut left = width;
    let mut kept = Vec::new();
    for (text, look) in line {
        if left == 0 {
            break;
        }
        let mut piece = String::new();
        for symbol in text.chars() {
            let wide = unicode_width::UnicodeWidthChar::width(symbol).unwrap_or(0);
            if wide > left {
                left = 0;
                break;
            }
            left -= wide;
            piece.push(symbol);
        }
        kept.push((piece, look));
    }
    kept
}

fn draw(out: &mut impl Write, lines: &[Line], at: Option<(usize, usize)>) -> io::Result<()> {
    queue!(out, cursor::Hide, cursor::MoveTo(0, 0))?;
    for (row, line) in lines.iter().enumerate() {
        queue!(out, cursor::MoveTo(0, u16::try_from(row).unwrap_or(u16::MAX)))?;
        for (text, look) in line {
            match look {
                Look::Plain => {}
                Look::Dim => queue!(out, SetForegroundColor(Color::DarkGrey))?,
                Look::Title => queue!(out, SetAttribute(Attribute::Bold))?,
                Look::Set => queue!(out, SetForegroundColor(Color::Cyan))?,
                Look::Chosen => queue!(out, SetAttribute(Attribute::Reverse))?,
                Look::Good => queue!(out, SetForegroundColor(Color::Green))?,
                Look::Wrong => queue!(out, SetForegroundColor(Color::Red))?,
            }
            queue!(out, Print(text), SetAttribute(Attribute::Reset), ResetColor)?;
        }
        queue!(out, terminal::Clear(terminal::ClearType::UntilNewLine))?;
    }
    if let Some((column, row)) = at {
        let place = |at: usize| u16::try_from(at).unwrap_or(u16::MAX);
        queue!(out, cursor::MoveTo(place(column), place(row)), cursor::Show)?;
    }
    out.flush()
}

/// The terminal as the screen needs it, and as it was again afterwards,
/// whatever ends the screen: the terminal is never left without its echo.
struct Borrowed;

impl Borrowed {
    fn take() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for Borrowed {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Shows the settings and takes keys until the user leaves.
pub(crate) fn run(file: File) -> Result<()> {
    let mut screen = Screen::new(file);
    let _terminal = Borrowed::take()?;
    let mut out = io::stdout().lock();
    loop {
        let (width, height) = terminal::size()?;
        let (lines, at) = screen.lines(usize::from(width), usize::from(height));
        draw(&mut out, &lines, at)?;
        // Windows tells of a key's release too: only its press is a key.
        if let Event::Key(key) = crossterm::event::read()?
            && key.kind != KeyEventKind::Release
            && screen.key(key) == Next::Leave
        {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(text: &str) -> (tempfile::TempDir, Screen) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        if !text.is_empty() {
            std::fs::write(&path, text).unwrap();
        }
        (directory, Screen::new(File::open(&path).unwrap()))
    }

    fn press(screen: &mut Screen, keys: &[KeyCode]) -> Next {
        // Every key in order; what the last one leaves to do.
        let mut next = Next::Stay;
        for code in keys {
            next = screen.key(KeyEvent::new(*code, KeyModifiers::NONE));
        }
        next
    }

    fn typed(screen: &mut Screen, text: &str) {
        for symbol in text.chars() {
            screen.key(KeyEvent::new(KeyCode::Char(symbol), KeyModifiers::NONE));
        }
    }

    fn text(screen: &mut Screen, width: usize, height: usize) -> Vec<String> {
        screen.lines(width, height).0.into_iter().map(|line| line.into_iter().map(|(text, _)| text).collect()).collect()
    }

    fn on_disk(screen: &Screen) -> String {
        std::fs::read_to_string(screen.file.path()).unwrap_or_default()
    }

    #[test]
    fn the_list_names_every_setting_under_its_section() {
        let (_directory, mut screen) = opened("[font]\nsize = 16\n");
        let lines = text(&mut screen, 120, 100);
        assert!(lines[0].starts_with("UnifyTerm settings   ") && lines[0].ends_with("config.toml"), "{}", lines[0]);
        for key in &KEYS {
            assert_eq!(lines.iter().filter(|line| line.trim_start().starts_with(key.name)).count(), 1, "{}", key.name);
        }
        assert_eq!(lines.iter().filter(|line| line.starts_with('[')).count(), 9, "nine sections");
        assert!(
            lines.iter().any(|line| line == "  font.size                16"),
            "the value as the file has it: {lines:#?}"
        );
        assert!(lines.iter().any(|line| line == "  font.ligatures           true"), "a default");
        let (styled, _) = screen.lines(120, 100);
        let size = styled.iter().find(|line| line.first().is_some_and(|(text, _)| text.contains("font.size"))).unwrap();
        let ligatures =
            styled.iter().find(|line| line.first().is_some_and(|(text, _)| text.contains("ligatures"))).unwrap();
        assert_eq!((size[1].1, ligatures[1].1), (Look::Set, Look::Dim), "what the file sets is told from a default");
        let foot = &lines[lines.len() - FOOT..];
        assert_eq!(foot[0], "─".repeat(120), "a rule under the list");
        assert_eq!(foot[1], "The font.", "the chosen setting's help");
        assert!(foot[2].starts_with("takes text   default: ") && foot[2].ends_with("   used at once"), "{}", foot[2]);
        assert_eq!(foot[4], KEYS_HELP);
    }

    #[test]
    fn enter_turns_a_switch_and_writes_the_file_at_once() {
        let (_directory, mut screen) = opened("");
        press(&mut screen, &[KeyCode::Down, KeyCode::Down, KeyCode::Down]);
        assert_eq!(screen.chosen().unwrap().name, "font.ligatures");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(on_disk(&screen), "[font]\nligatures = false\n");
        assert_eq!(screen.said, Some(("font.ligatures = false".into(), Look::Good)));
        press(&mut screen, &[KeyCode::Char(' ')]);
        assert_eq!(on_disk(&screen), "[font]\nligatures = true\n", "space turns it too");
        press(&mut screen, &[KeyCode::Char('d')]);
        assert_eq!(on_disk(&screen), "", "d: the default again, and the emptied section is gone");
        press(&mut screen, &[KeyCode::Char('u'), KeyCode::Char('u')]);
        assert_eq!(on_disk(&screen), "[font]\nligatures = false\n", "u takes back one change each time");
        press(&mut screen, &[KeyCode::Char('u'), KeyCode::Char('u')]);
        assert_eq!(on_disk(&screen), "");
        assert_eq!(screen.said, Some(("nothing to take back".into(), Look::Dim)));
    }

    #[test]
    fn a_value_is_typed_and_a_wrong_one_stays_to_be_mended() {
        let (_directory, mut screen) = opened("");
        press(&mut screen, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(screen.mode, Mode::Edit("14.0".into()), "the value now is there to change");
        let (lines, cursor) = screen.lines(120, 40);
        let row =
            lines.iter().position(|line| line.first().is_some_and(|(text, _)| text.contains("font.size"))).unwrap();
        assert_eq!(cursor, Some((27 + 4, row)), "the cursor is after what is typed");
        press(&mut screen, &[KeyCode::Backspace, KeyCode::Backspace, KeyCode::Backspace, KeyCode::Backspace]);
        typed(&mut screen, "500");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(screen.mode, Mode::Edit("500".into()), "refused: still being typed");
        assert!(matches!(&screen.said, Some((why, Look::Wrong)) if why.contains("from 6 to 72")), "{:?}", screen.said);
        assert_eq!(on_disk(&screen), "", "nothing was written");
        screen.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typed(&mut screen, "18");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!((screen.mode.clone(), on_disk(&screen)), (Mode::Move, "[font]\nsize = 18.0\n".into()));
        press(&mut screen, &[KeyCode::Enter]);
        typed(&mut screen, "9");
        press(&mut screen, &[KeyCode::Esc]);
        assert_eq!(
            (screen.mode.clone(), on_disk(&screen)),
            (Mode::Move, "[font]\nsize = 18.0\n".into()),
            "Esc leaves it"
        );
    }

    #[test]
    fn a_choice_goes_round_and_a_setting_that_needs_another_says_so() {
        let (_directory, mut screen) = opened("");
        typed(&mut screen, "/provider");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(screen.shown().len(), 1);
        assert_eq!(screen.chosen().unwrap().name, "ai.provider");
        press(&mut screen, &[KeyCode::Enter]);
        assert!(
            matches!(&screen.said, Some((why, Look::Wrong)) if why.contains("ai.model is empty")),
            "anthropic needs a model first: {:?}",
            screen.said
        );
        assert_eq!(on_disk(&screen), "");
        press(&mut screen, &[KeyCode::Esc]);
        assert_eq!(screen.shown().len(), KEYS.len(), "Esc shows all again");
        assert_eq!(press(&mut screen, &[KeyCode::Esc]), Next::Leave, "and Esc again leaves");
    }

    #[test]
    fn the_search_narrows_the_list_and_says_how_many() {
        let (_directory, mut screen) = opened("");
        typed(&mut screen, "/color");
        let lines = text(&mut screen, 100, 30);
        assert_eq!(lines[1], "/color   5 of 31");
        assert!(lines.iter().any(|line| line == "[theme]") && !lines.iter().any(|line| line == "[font]"));
        typed(&mut screen, "zzz");
        let lines = text(&mut screen, 100, 30);
        assert_eq!(lines[1], "/colorzzz   0 of 31");
        assert_eq!(lines[lines.len() - 3], "no setting has that in its name or its help");
        assert_eq!(press(&mut screen, &[KeyCode::Enter, KeyCode::Enter]), Next::Stay, "Enter on nothing does nothing");
    }

    #[test]
    fn the_chosen_setting_stays_on_a_small_screen_and_lines_fit_its_width() {
        let (_directory, mut screen) = opened("");
        press(&mut screen, &[KeyCode::End]);
        let lines = text(&mut screen, 40, 12);
        assert_eq!(lines.len(), 12, "as many lines as the screen has");
        assert!(lines.iter().all(|line| line.width() <= 40), "{lines:#?}");
        assert!(lines.iter().any(|line| line.trim_start().starts_with("agents.focus")), "the last one is in view");
        press(&mut screen, &[KeyCode::Home]);
        let lines = text(&mut screen, 40, 12);
        assert_eq!(lines[HEAD], "[font]", "back at the top, with its heading");
        assert_eq!(press(&mut screen, &[KeyCode::Char('q')]), Next::Leave);
        let stop = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        press(&mut screen, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(screen.key(stop), Next::Leave, "Ctrl-C leaves from anywhere");
    }
}
