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
//!
//! Little is typed: a switch turns, a number moves with the arrow keys, and
//! a setting with choices (a font, a shell, a provider) opens a list of them
//! ([`super::choices`]).

use std::io::{self, Write};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::{cursor, execute, queue, terminal};
use unicode_width::UnicodeWidthStr as _;
use unifyterm_config::{File, KEYS, Key, Kind, default_of};

use super::choices::{Choice, Choices, Pick};
use super::takes;

/// Columns before a setting's name, and between the longest name and the values.
const INDENT: usize = 2;
const GAP: usize = 2;
/// The lines under the list: a rule, what the setting does, what it takes, what happened, the keys.
const FOOT: usize = 5;
/// The lines above it: the title, and the search.
const HEAD: usize = 2;
const KEYS_HELP: &str =
    "↑↓ move   Enter change   ←→ smaller, larger   e type   d default   u undo   / search   q leave";
const PICK_ONE_HELP: &str = "type to narrow   ↑↓ move   Enter choose   Esc leave it as it was";
const PICK_MANY_HELP: &str =
    "type to narrow   ↑↓ move   Space mark or unmark (in order)   Enter keep   Esc leave it as it was";
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
    /// A block of this color: a color setting's value, shown.
    Swatch(u8, u8, u8),
}

/// One line of the screen: pieces, each with its look.
pub(crate) type Line = Vec<(String, Look)>;

/// What the keys do now.
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Move,
    /// A value is being typed.
    Edit(String),
    /// The words to narrow the list by are being typed.
    Search,
    /// A value is being chosen from a list.
    Pick(Picker),
}

/// The list a setting's value is chosen from.
#[derive(Debug, Clone, PartialEq)]
struct Picker {
    key: &'static Key,
    items: Vec<Choice>,
    pick: Pick,
    /// Only items with this in them are shown.
    filter: String,
    /// The item the cursor is on: an index into the items shown.
    at: usize,
    /// For several: the values marked, in the order they were marked.
    marked: Vec<String>,
}

impl Picker {
    fn shown(&self) -> Vec<&Choice> {
        let wanted = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|item| item.shown.to_lowercase().contains(&wanted) || item.note.to_lowercase().contains(&wanted))
            .collect()
    }

    fn step(&mut self, by: isize) {
        let last = self.shown().len().saturating_sub(1);
        self.at = self.at.saturating_add_signed(by).min(last);
    }

    /// Marks the item under the cursor, or takes its mark away.
    fn toggle(&mut self) {
        let Some(value) = self.shown().get(self.at).map(|item| item.value.clone()) else { return };
        match self.marked.iter().position(|marked| *marked == value) {
            Some(place) => drop(self.marked.remove(place)),
            None => self.marked.push(value),
        }
    }
}

/// How far ← and → move a number setting.
fn nudge_step(key: &Key) -> f64 {
    match key.name {
        "font.line_height" => 0.05,
        "terminal.scrollback" => 1000.0,
        "agents.poll_ms" => 500.0,
        _ => 1.0,
    }
}

/// A setting's words as they are shown (`a, b`, or TOML's own form when one has a comma).
fn words_of(shown: &str) -> Vec<String> {
    let shown = shown.trim();
    if let Some(inner) = shown.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        return inner
            .split("\", \"")
            .map(|word| word.trim().trim_matches('"').to_owned())
            .filter(|word| !word.is_empty())
            .collect();
    }
    shown.split(',').map(str::trim).filter(|word| !word.is_empty()).map(str::to_owned).collect()
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
    /// What a setting is chosen from (see [`super::choices::for_key`]).
    choices: fn(&Key) -> Option<Choices>,
}

impl Screen {
    pub(crate) fn new(file: File) -> Self {
        Self::with_choices(file, super::choices::for_key)
    }

    /// A screen whose settings are chosen from what `choices` gives: the tests give their own fonts.
    pub(crate) fn with_choices(file: File, choices: fn(&Key) -> Option<Choices>) -> Self {
        Self { file, at: 0, top: 0, mode: Mode::Move, search: String::new(), said: None, before: Vec::new(), choices }
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

    /// Enter on a setting: a switch turns, one with choices opens their
    /// list, anything else is typed.
    fn change(&mut self) -> Next {
        let Some(key) = self.chosen() else { return Next::Stay };
        let now = self.file.in_use(key);
        self.said = None;
        if key.kind == Kind::Switch {
            return self.set(key, if now == "true" { "false" } else { "true" });
        }
        // The themes are those beside this settings file: what there is to choose depends on the file.
        let choices = if key.name == super::choices::THEME {
            Some(super::choices::themes(self.file.path()))
        } else {
            (self.choices)(key)
        };
        let Some(Choices { items, pick }) = choices else {
            self.mode = Mode::Edit(now);
            return Next::Stay;
        };
        let (at, marked) = match pick {
            Pick::One => (items.iter().position(|item| item.value == now).unwrap_or(0), Vec::new()),
            Pick::Many => (0, words_of(&now)),
        };
        self.mode = Mode::Pick(Picker { key, items, pick, filter: String::new(), at, marked });
        Next::Stay
    }

    /// ← or →: a number setting one step smaller or larger, within its limits.
    fn nudge(&mut self, by: f64) {
        let Some(key) = self.chosen() else { return };
        let (least, most) = match key.kind {
            // The settings' whole numbers are far below u32's limit.
            Kind::Whole { least, most } => {
                let exact = |number: u64| f64::from(u32::try_from(number).unwrap_or(u32::MAX));
                (exact(least), exact(most))
            }
            Kind::Number { least, most } => (f64::from(least), f64::from(most)),
            _ => return,
        };
        let Ok(now) = self.file.in_use(key).parse::<f64>() else { return };
        let next = (now + by * nudge_step(key)).clamp(least, most);
        // Hundredths are as fine as a step goes: 1.35 + 0.05 is 1.4, not 1.4000000000000001.
        let next = (next * 100.0).round() / 100.0;
        if (next - now).abs() < f64::EPSILON {
            return;
        }
        let text = if matches!(key.kind, Kind::Whole { .. }) { format!("{next:.0}") } else { format!("{next}") };
        self.set(key, &text);
    }

    /// A key while a value is chosen from a list.
    fn picking(&mut self, mut picker: Picker, key: KeyEvent) -> Next {
        const PAGE: isize = 10;
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Next::Stay,
            KeyCode::Enter => {
                let chosen = match picker.pick {
                    Pick::One => picker.shown().get(picker.at).map(|item| item.value.clone()),
                    Pick::Many => Some(picker.marked.join(", ")),
                };
                // A typing mistake must not become the font: a name the list lacks is typed with e.
                let Some(value) = chosen else {
                    let says = "nothing in the list has that: Esc, then e to type a value of your own";
                    self.said = Some((says.into(), Look::Wrong));
                    self.mode = Mode::Pick(picker);
                    return Next::Stay;
                };
                self.set(picker.key, &value);
                // A value that was refused leaves the list open to choose again.
                if matches!(self.said, Some((_, Look::Wrong))) {
                    self.mode = Mode::Pick(picker);
                }
                return Next::Stay;
            }
            KeyCode::Up => picker.step(-1),
            KeyCode::Down => picker.step(1),
            KeyCode::PageUp => picker.step(-PAGE),
            KeyCode::PageDown => picker.step(PAGE),
            KeyCode::Char(' ') if picker.pick == Pick::Many => picker.toggle(),
            KeyCode::Backspace => {
                picker.filter.pop();
                picker.at = 0;
            }
            KeyCode::Char('u') if control => {
                picker.filter.clear();
                picker.at = 0;
            }
            KeyCode::Char(typed) if !control => {
                picker.filter.push(typed);
                picker.at = 0;
            }
            _ => {}
        }
        self.mode = Mode::Pick(picker);
        Next::Stay
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
            Mode::Pick(picker) => return self.picking(picker, key),
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
            KeyCode::Left | KeyCode::Char('h') => self.nudge(-1.0),
            KeyCode::Right | KeyCode::Char('l') => self.nudge(1.0),
            KeyCode::Char('d') | KeyCode::Delete => self.reset(),
            // Typed, whatever the setting: a font the list lacks, a value of one's own.
            KeyCode::Char('e') => {
                if let Some(key) = self.chosen().filter(|key| key.kind != Kind::Switch) {
                    self.mode = Mode::Edit(self.file.in_use(key));
                    self.said = None;
                }
            }
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
            let mut line = vec![(name, if chosen { Look::Chosen } else { Look::Plain })];
            if key.kind == Kind::Color
                && let Some([red, green, blue]) = unifyterm_config::parse_color(&value.0)
            {
                line.push(("██ ".to_owned(), Look::Swatch(red, green, blue)));
            }
            line.push(value);
            list.push((line, Some(index)));
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
        if let Mode::Pick(picker) = &mut self.mode {
            let (picked, at) = picker_lines(picker, room);
            lines[1] = vec![
                (format!("{} /{}", picker.key.name, picker.filter), Look::Plain),
                (format!("   {} of {}", picker.shown().len(), picker.items.len()), Look::Dim),
            ];
            cursor = Some((picker.key.name.width() + 2 + picker.filter.width(), 1));
            let _ = at;
            lines.extend(picked);
        } else {
            for (offset, (line, index)) in list.into_iter().skip(self.top).take(room).enumerate() {
                if let (Mode::Edit(text), true) = (&self.mode, index == Some(self.at)) {
                    cursor = Some((value_at + text.width(), HEAD + offset));
                }
                lines.push(line);
            }
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
        let help = match &self.mode {
            Mode::Move => KEYS_HELP,
            Mode::Edit(_) => EDIT_HELP,
            Mode::Search => SEARCH_HELP,
            Mode::Pick(Picker { pick: Pick::One, .. }) => PICK_ONE_HELP,
            Mode::Pick(Picker { pick: Pick::Many, .. }) => PICK_MANY_HELP,
        };
        lines.push(vec![(help.to_owned(), Look::Dim)]);
        (lines.into_iter().map(|line| cut(line, width)).collect(), cursor)
    }
}

/// The lines of the list a value is chosen from, `room` of them at most,
/// the item under the cursor among them; and that item's line.
fn picker_lines(picker: &Picker, room: usize) -> (Vec<Line>, usize) {
    let shown = picker.shown();
    if shown.is_empty() {
        let says = match picker.pick {
            Pick::One if !picker.filter.trim().is_empty() => {
                "nothing has that: Esc, then e to type a value of your own".to_owned()
            }
            _ => "nothing has that".to_owned(),
        };
        return (vec![vec![(says, Look::Dim)]], 0);
    }
    let width = shown.iter().map(|item| item.shown.width()).max().unwrap_or(0);
    let top = picker.at.saturating_sub(room.saturating_sub(1));
    let lines = shown
        .iter()
        .enumerate()
        .skip(top)
        .take(room)
        .map(|(index, item)| {
            let here = index == picker.at;
            let mark = match picker.pick {
                Pick::One => String::new(),
                Pick::Many => match picker.marked.iter().position(|marked| *marked == item.value) {
                    Some(place) => format!("[{}] ", place + 1),
                    None => "[ ] ".to_owned(),
                },
            };
            let name = format!("{}{mark}{:width$}", if here { "› " } else { "  " }, item.shown);
            // Padded only to line the notes up.
            let name = if item.note.is_empty() { name.trim_end().to_owned() } else { name };
            let mut line = vec![(name, if here { Look::Chosen } else { Look::Plain })];
            if !item.note.is_empty() {
                line.push((format!("  {}", item.note), Look::Dim));
            }
            line
        })
        .collect();
    (lines, picker.at - top)
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
                Look::Swatch(r, g, b) => queue!(out, SetForegroundColor(Color::Rgb { r: *r, g: *g, b: *b }))?,
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
        (directory, Screen::with_choices(File::open(&path).unwrap(), test_choices))
    }

    /// What the tests choose from: the real lists, but fonts of their own.
    fn test_choices(key: &Key) -> Option<Choices> {
        let font = |name: &str, note: &str| Choice { value: name.into(), shown: name.into(), note: note.into() };
        let fonts = vec![font("Cascadia Code", "monospace"), font("JetBrains Mono", "monospace"), font("Segoe UI", "")];
        match key.name {
            "font.family" => Some(Choices { items: fonts, pick: Pick::One }),
            "font.fallbacks" => Some(Choices { items: fonts, pick: Pick::Many }),
            _ => super::super::choices::for_key(key),
        }
    }

    fn named(screen: &mut Screen, name: &str) {
        screen.at = screen.shown().iter().position(|key| key.name == name).unwrap();
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
            // The whole name: `ai.ask` is not `ai.ask_enter`.
            let named = |line: &&String| line.split_whitespace().next() == Some(key.name);
            assert_eq!(lines.iter().filter(named).count(), 1, "{}", key.name);
        }
        assert_eq!(lines.iter().filter(|line| line.starts_with('[')).count(), 10, "ten sections");
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
        assert_eq!(words_of("a, b"), ["a", "b"]);
        assert_eq!(words_of(r#"["keepane", "-L", "a, b"]"#), ["keepane", "-L", "a, b"]);
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
    fn a_font_is_chosen_from_a_list_and_one_it_lacks_can_be_typed() {
        let (_directory, mut screen) = opened("");
        press(&mut screen, &[KeyCode::Enter]);
        let Mode::Pick(picker) = &screen.mode else { panic!("Enter on font.family opens the list: {:?}", screen.mode) };
        assert_eq!((picker.items.len(), picker.at), (3, 0), "the cursor on the font in use (Cascadia Code)");
        let lines = text(&mut screen, 100, 30);
        assert_eq!(lines[1], "font.family /   3 of 3");
        assert_eq!(lines[HEAD], "› Cascadia Code   monospace", "{lines:#?}");
        assert_eq!(lines[lines.len() - 1], PICK_ONE_HELP);
        typed(&mut screen, "jet");
        assert_eq!(text(&mut screen, 100, 30)[1], "font.family /jet   1 of 3", "typing narrows the list");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(
            (screen.mode.clone(), on_disk(&screen)),
            (Mode::Move, "[font]\nfamily = \"JetBrains Mono\"\n".into())
        );
        // A name the list lacks is not taken by mistake: it is typed with e.
        press(&mut screen, &[KeyCode::Enter]);
        typed(&mut screen, "Iosevka Term");
        assert!(text(&mut screen, 100, 30)[HEAD].contains("e to type"));
        press(&mut screen, &[KeyCode::Enter]);
        assert!(
            matches!(screen.mode, Mode::Pick(_)) && matches!(&screen.said, Some((_, Look::Wrong))),
            "refused, still open"
        );
        assert_eq!(on_disk(&screen), "[font]\nfamily = \"JetBrains Mono\"\n", "nothing written");
        press(&mut screen, &[KeyCode::Esc, KeyCode::Char('e')]);
        assert_eq!(screen.mode, Mode::Edit("JetBrains Mono".into()));
        screen.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typed(&mut screen, "Iosevka Term");
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(on_disk(&screen), "[font]\nfamily = \"Iosevka Term\"\n");
        press(&mut screen, &[KeyCode::Enter, KeyCode::Down, KeyCode::Esc]);
        assert_eq!(on_disk(&screen), "[font]\nfamily = \"Iosevka Term\"\n", "Esc leaves it as it was");
    }

    #[test]
    fn several_are_marked_in_order() {
        let (_directory, mut screen) = opened("[font]\nfallbacks = [\"Segoe UI\"]\n");
        named(&mut screen, "font.fallbacks");
        press(&mut screen, &[KeyCode::Enter]);
        let lines = text(&mut screen, 100, 30);
        assert_eq!(
            &lines[HEAD..HEAD + 3],
            ["› [ ] Cascadia Code   monospace", "  [ ] JetBrains Mono  monospace", "  [1] Segoe UI"]
        );
        press(&mut screen, &[KeyCode::Down, KeyCode::Char(' '), KeyCode::Up, KeyCode::Char(' ')]);
        press(&mut screen, &[KeyCode::Enter]);
        assert_eq!(
            on_disk(&screen),
            "[font]\nfallbacks = [\"Segoe UI\", \"JetBrains Mono\", \"Cascadia Code\"]\n",
            "in the order marked"
        );
        press(&mut screen, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Char(' '), KeyCode::Enter]);
        assert_eq!(
            on_disk(&screen),
            "[font]\nfallbacks = [\"JetBrains Mono\", \"Cascadia Code\"]\n",
            "space again unmarks"
        );
    }

    #[test]
    fn a_choice_that_needs_another_setting_says_so_and_stays_open() {
        let (_directory, mut screen) = opened("");
        named(&mut screen, "ai.provider");
        press(&mut screen, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
        assert!(
            matches!(&screen.said, Some((why, Look::Wrong)) if why.contains("ai.model is empty")),
            "anthropic needs a model first: {:?}",
            screen.said
        );
        assert!(matches!(screen.mode, Mode::Pick(_)), "the list stays open");
        assert_eq!(on_disk(&screen), "");
        press(&mut screen, &[KeyCode::Esc]);
        assert_eq!(screen.mode, Mode::Move);
        typed(&mut screen, "/provider");
        press(&mut screen, &[KeyCode::Enter, KeyCode::Esc]);
        assert_eq!(screen.shown().len(), KEYS.len(), "Esc shows all again");
        assert_eq!(press(&mut screen, &[KeyCode::Esc]), Next::Leave, "and Esc again leaves");
    }

    #[test]
    fn the_arrow_keys_move_a_number_within_its_limits() {
        let (_directory, mut screen) = opened("");
        named(&mut screen, "font.size");
        press(&mut screen, &[KeyCode::Right, KeyCode::Right]);
        assert_eq!(on_disk(&screen), "[font]\nsize = 16.0\n");
        named(&mut screen, "font.line_height");
        press(&mut screen, &[KeyCode::Right]);
        assert_eq!(
            screen.file.value(unifyterm_config::key("font.line_height").unwrap()).as_deref(),
            Some("1.4"),
            "hundredths, not 1.4000000000000001"
        );
        named(&mut screen, "history.limit");
        for _ in 0..30 {
            press(&mut screen, &[KeyCode::Right]);
        }
        assert_eq!(
            screen.file.in_use(unifyterm_config::key("history.limit").unwrap()),
            "20",
            "no further than its most"
        );
        named(&mut screen, "terminal.scrollback");
        press(&mut screen, &[KeyCode::Left]);
        assert_eq!(screen.file.in_use(unifyterm_config::key("terminal.scrollback").unwrap()), "9000", "a step of 1000");
        let changes = screen.before.len();
        named(&mut screen, "font.family");
        press(&mut screen, &[KeyCode::Right]);
        assert_eq!(screen.before.len(), changes, "a setting that is not a number does not move");
    }

    #[test]
    fn a_color_is_shown_as_itself() {
        let (_directory, mut screen) = opened("[theme]\nbackground = \"#102030\"\n");
        let (lines, _) = screen.lines(120, 100);
        let line =
            lines.iter().find(|line| line.first().is_some_and(|(text, _)| text.contains("theme.background"))).unwrap();
        assert_eq!(line[1], ("██ ".to_owned(), Look::Swatch(0x10, 0x20, 0x30)));
        let cursor =
            lines.iter().find(|line| line.first().is_some_and(|(text, _)| text.contains("theme.cursor"))).unwrap();
        assert_eq!(cursor.len(), 2, "not set: nothing to show");
    }

    #[test]
    fn the_search_narrows_the_list_and_says_how_many() {
        let (_directory, mut screen) = opened("");
        typed(&mut screen, "/color");
        let lines = text(&mut screen, 100, 30);
        assert_eq!(lines[1], "/color   12 of 49");
        assert!(lines.iter().any(|line| line == "[theme]") && !lines.iter().any(|line| line == "[font]"));
        typed(&mut screen, "zzz");
        let lines = text(&mut screen, 100, 30);
        assert_eq!(lines[1], "/colorzzz   0 of 49");
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
