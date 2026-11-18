//! One program in a pty and the screen it draws: the part of a terminal
//! that needs no window, so tests drive it directly.
//!
//! Bytes the program prints come out of [`Session::spawn`]'s channel; the
//! owner hands them to [`Session::feed`], which updates the screen. Keys go
//! the other way through [`Session::write`]. [`Session::snapshot`] is what
//! is on screen, in this crate's own types: nothing outside this file knows
//! which library keeps the screen.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::{self, CursorShape as VteCursor, NamedColor, Processor};
use anyhow::{Context as _, Result, anyhow};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use smol::channel::{Receiver, bounded};

use crate::graphics::{self, Effect, Graphics, Image};
use crate::keys::{CursorKeys, MouseProtocol};
use crate::marks::{Mark, Scanner};
use crate::prompt::{Finished, Phase, Prompt};
use crate::theme::{Rgb, Theme};

/// Most bytes taken from the pty in one read.
const READ_CHUNK: usize = 8192;
/// Reads that may wait for the session's owner. When it is full the reader
/// waits, and so does the program: a flood of output cannot fill memory.
const OUTPUT_QUEUE: usize = 256;
/// After the program ends, its last output is waited for until none has come for this long ...
const QUIET_AFTER_END: std::time::Duration = std::time::Duration::from_millis(100);
/// ... or this long has passed (a program it left behind may print on and on).
const LONGEST_AFTER_END: std::time::Duration = std::time::Duration::from_secs(1);
/// How often the waiter looks whether the output has gone quiet.
const QUIET_LOOK: std::time::Duration = std::time::Duration::from_millis(10);

/// What the pty's threads tell the session's owner.
pub(crate) enum PtyEvent {
    /// The program printed these bytes.
    Output(Vec<u8>),
    /// The program ended.
    Exited,
}

/// What the program asked of the terminal, for the session's owner to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    /// The window's title (`None`: back to the default).
    Title(Option<String>),
    /// Text for the clipboard (OSC 52).
    Copy(String),
    /// A command the shell ran has ended (its hook said so).
    Finished(Finished),
}

/// What the user has typed at the shell's prompt so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Input {
    /// From where the prompt ends to the cursor: not what is right of the
    /// cursor, which may be the shell's own grey suggestion.
    pub(crate) text: String,
    /// The cursor's cell in the view, as (line, column).
    pub(crate) cursor: (usize, usize),
    /// The column the input starts at on the cursor's line (0 when it started on a line above).
    pub(crate) start_column: usize,
}

/// A cell's color before the theme gives it a value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Color {
    Foreground,
    Background,
    /// One of xterm's 256.
    Indexed(u8),
    Rgb(Rgb),
}

/// A cell's attributes, as bits of [`Cell::attrs`].
pub(crate) mod attr {
    pub(crate) const BOLD: u16 = 1;
    pub(crate) const ITALIC: u16 = 1 << 1;
    pub(crate) const UNDERLINE: u16 = 1 << 2;
    pub(crate) const STRIKE: u16 = 1 << 3;
    pub(crate) const DIM: u16 = 1 << 4;
    pub(crate) const INVERSE: u16 = 1 << 5;
    pub(crate) const HIDDEN: u16 = 1 << 6;
    /// The character takes this cell and the next.
    pub(crate) const WIDE: u16 = 1 << 7;
    /// The second cell of a wide character: nothing of its own to draw.
    pub(crate) const SPACER: u16 = 1 << 8;
    pub(crate) const SELECTED: u16 = 1 << 9;
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Cell {
    pub(crate) c: char,
    /// Combining characters on top of `c`.
    pub(crate) combining: Option<String>,
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) attrs: u16,
}

impl Cell {
    pub(crate) fn has(&self, attribute: u16) -> bool {
        self.attrs & attribute != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CursorShape {
    Block,
    Beam,
    Underline,
    Hollow,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cursor {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) shape: CursorShape,
}

/// What is on screen: `lines` rows of `columns` cells, top row first.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Snapshot {
    pub(crate) columns: usize,
    pub(crate) lines: usize,
    pub(crate) cells: Vec<Cell>,
    /// `None` when the program hid it or it is scrolled out of view.
    pub(crate) cursor: Option<Cursor>,
    /// How many lines of history the view is scrolled back by.
    pub(crate) scrolled: usize,
}

impl Snapshot {
    pub(crate) fn row(&self, line: usize) -> &[Cell] {
        &self.cells[line * self.columns..(line + 1) * self.columns]
    }
}

/// How pasted text is sent to the program.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Paste {
    /// As if typed: each line end is the Enter key.
    #[default]
    Typed,
    /// Between the marks that tell the program "this is a paste".
    Bracketed,
}

/// How much of the mouse the program asked to be told about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum MouseTracking {
    #[default]
    Off,
    /// Presses and releases.
    Clicks,
    /// ... and movement while a button is held.
    Drags,
    /// ... and all movement.
    Motion,
}

/// Whether the window has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Focus {
    Focused,
    Unfocused,
}

/// Which half of a cell a point is in. A selection that starts in a cell's
/// right half starts after that cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Half {
    Left,
    Right,
}

/// The modes the program set that change what input the terminal sends.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Modes {
    pub(crate) cursor_keys: CursorKeys,
    pub(crate) paste: Paste,
    pub(crate) mouse: MouseTracking,
    pub(crate) mouse_protocol: MouseProtocol,
    pub(crate) alt_screen: bool,
    /// On the alternate screen the wheel sends arrow keys.
    pub(crate) alternate_scroll: bool,
    pub(crate) focus_events: bool,
}

/// How much a click selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SelectBy {
    Cell,
    Word,
    Line,
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// The terminal library's events. Its answers to the program (where the
/// cursor is, what a color is) are written back at once: ConPTY asks for
/// the cursor's position before it passes anything on. The rest waits in
/// `notices` for [`Session::take_notices`].
pub(crate) struct Listener {
    /// None for a screen with no program behind it (a replay).
    writer: Option<Writer>,
    notices: Arc<Mutex<Vec<Notice>>>,
    theme: Theme,
}

impl Listener {
    fn reply(&self, text: &str) {
        if let Some(writer) = &self.writer {
            // A failed write means the program is gone; `PtyEvent::Exited` says so.
            let _ = writer.lock().expect(LOCK_POISONED).write_all(text.as_bytes());
        }
    }
}

/// Why a lock cannot be taken: a thread panicked while holding it, and
/// what it guards may be half written. There is nothing to carry on with.
const LOCK_POISONED: &str = "lock poisoned";

/// The library's number for a color it knows by name.
#[expect(clippy::as_conversions, reason = "the discriminant of a fieldless enum, which has no `From`")]
const fn number(name: NamedColor) -> usize {
    name as usize
}

/// The library's numbers for the colors that are not one of the 256.
const FOREGROUND_INDEX: usize = number(NamedColor::Foreground);
const BACKGROUND_INDEX: usize = number(NamedColor::Background);
const CURSOR_INDEX: usize = number(NamedColor::Cursor);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let notice = match event {
            Event::PtyWrite(text) => return self.reply(&text),
            Event::ColorRequest(index, format) => {
                let color = match index {
                    FOREGROUND_INDEX => self.theme.foreground,
                    BACKGROUND_INDEX => self.theme.background,
                    CURSOR_INDEX => self.theme.cursor,
                    // One of the 256. A larger number is no color.
                    other => match u8::try_from(other) {
                        Ok(indexed) => self.theme.indexed(indexed),
                        Err(_) => return,
                    },
                };
                return self.reply(&format(ansi::Rgb { r: color.0, g: color.1, b: color.2 }));
            }
            Event::Title(title) => Notice::Title(Some(title)),
            Event::ResetTitle => Notice::Title(None),
            Event::ClipboardStore(_, text) => Notice::Copy(text),
            _ => return,
        };
        self.notices.lock().expect(LOCK_POISONED).push(notice);
    }
}

/// A terminal's screen: bytes in, cells out. It has no program of its own;
/// [`Session`] is a screen with one.
pub(crate) struct Screen {
    term: Term<Listener>,
    parser: Processor,
    notices: Arc<Mutex<Vec<Notice>>>,
    /// Where every byte fed in is also written, when `UNIFYTERM_RAW_LOG`
    /// names a file: what `--replay` reads back.
    raw_log: Option<std::fs::File>,
    /// The shell's marks, and what they say it is doing.
    scanner: Scanner,
    prompt: Prompt,
    /// The images programs sent.
    graphics: Graphics,
    /// A cell's size in pixels, as last drawn: how many cells an image takes.
    cell_pixels: (u32, u32),
    /// Where answers to the program go (a graphics command's `OK`).
    replies: Option<Writer>,
}

/// A cell's size in pixels until the window says.
const CELL_PIXELS: (u32, u32) = (9, 19);

/// A program in a pty, and its screen.
pub(crate) struct Session {
    screen: Screen,
    writer: Writer,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl std::ops::Deref for Session {
    type Target = Screen;
    fn deref(&self) -> &Screen {
        &self.screen
    }
}

impl std::ops::DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }
}

/// The most lines of a command's output kept with it.
const OUTPUT_LINES: usize = 200;

/// A row's text without the spaces at its end, and a line break unless it runs on into the next.
fn row_text(row: &alacritty_terminal::grid::Row<alacritty_terminal::term::cell::Cell>, columns: usize) -> String {
    let mut text = String::new();
    for column in 0..columns {
        let cell = &row[Column(column)];
        if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            text.push(cell.c);
            text.extend(cell.zerowidth().into_iter().flatten());
        }
    }
    let mut text = text.trim_end().to_owned();
    if !row[Column(columns - 1)].flags.contains(Flags::WRAPLINE) {
        text.push('\n');
    }
    text
}

/// The environment variable naming the file a session's output is also
/// written to.
const RAW_LOG_VARIABLE: &str = "UNIFYTERM_RAW_LOG";
/// How many sessions this process has started.
static SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// The raw log of the process's `number`th session (from 1): the named
/// file for the first, the name with `.2`, `.3` added for the rest. Each
/// session has its own, so that replaying one draws one screen.
fn raw_log_path(name: &str, number: usize) -> String {
    if number <= 1 { name.to_owned() } else { format!("{name}.{number}") }
}

impl Session {
    /// Starts `command` in a pty of `columns` by `lines`, keeping
    /// `scrollback` lines of history. The channel carries what the program
    /// prints, then that it ended.
    pub(crate) fn spawn(
        command: CommandBuilder,
        columns: u16,
        lines: u16,
        scrollback: usize,
        theme: Theme,
    ) -> Result<(Self, Receiver<PtyEvent>)> {
        let pair = native_pty_system().openpty(pty_size(columns, lines)).map_err(|e| anyhow!("open a pty: {e}"))?;
        let mut child = pair.slave.spawn_command(command).map_err(|e| anyhow!("start the program: {e}"))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow!("read the pty: {e}"))?;
        let writer: Writer =
            Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| anyhow!("write the pty: {e}"))?));

        let (tx, rx) = bounded(OUTPUT_QUEUE);
        let output = tx.clone();
        // When output last came (milliseconds since `born`), and whether the reader saw the pipe's end.
        let born = std::time::Instant::now();
        let last_output = Arc::new(AtomicU64::new(0));
        let ended = Arc::new(AtomicBool::new(false));
        let (reader_last, reader_ended) = (Arc::clone(&last_output), Arc::clone(&ended));
        let since_born = move || u64::try_from(born.elapsed().as_millis()).unwrap_or(u64::MAX);
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; READ_CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if output.send_blocking(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                                break;
                            }
                            reader_last.store(since_born(), Ordering::Release);
                        }
                    }
                }
                reader_ended.store(true, Ordering::Release);
            })
            .context("start the pty reader")?;
        // ConPTY keeps its output pipe open after the program ends, so the
        // reader never sees the end: the program's exit is waited for here.
        // ConPTY also draws the program's last output a moment after it has
        // ended: the end is reported only when that output is in (the reader
        // saw the pipe close, or nothing came for QUIET_AFTER_END), so that
        // nothing a program printed is lost after it.
        std::thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || {
                let _ = child.wait();
                let exited = std::time::Instant::now();
                loop {
                    let quiet = since_born().saturating_sub(last_output.load(Ordering::Acquire));
                    let quiet_long_enough =
                        u128::from(quiet) >= QUIET_AFTER_END.as_millis() && exited.elapsed() >= QUIET_AFTER_END;
                    if ended.load(Ordering::Acquire) || quiet_long_enough || exited.elapsed() >= LONGEST_AFTER_END {
                        break;
                    }
                    std::thread::sleep(QUIET_LOOK);
                }
                let _ = tx.send_blocking(PtyEvent::Exited);
            })
            .context("start the pty waiter")?;

        let mut screen = Screen::new(columns, lines, scrollback, theme, Some(Arc::clone(&writer)));
        if let Some(path) = std::env::var_os(RAW_LOG_VARIABLE) {
            let path = raw_log_path(&path.to_string_lossy(), SESSIONS.fetch_add(1, Ordering::Relaxed) + 1);
            let file = std::fs::OpenOptions::new().create(true).append(true).open(&path);
            screen.raw_log = Some(file.with_context(|| format!("open the raw log {path}"))?);
        }
        Ok((Self { screen, writer, master: pair.master, killer }, rx))
    }

    /// Sends bytes to the program, as if typed.
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().expect(LOCK_POISONED);
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Sends pasted text: marked as a paste when the program asked for
    /// that, else with each line end as the Enter key.
    pub(crate) fn paste(&self, text: &str) -> Result<()> {
        self.write(&paste_bytes(text, self.modes().paste))
    }

    /// Changes the screen's size and tells the program.
    pub(crate) fn resize(&mut self, columns: u16, lines: u16) -> Result<()> {
        self.master.resize(pty_size(columns, lines)).map_err(|e| anyhow!("resize the pty: {e}"))?;
        self.screen.set_size(columns, lines);
        Ok(())
    }

    /// Tells the program the window gained or lost focus, if it asked to know.
    pub(crate) fn focus(&mut self, focus: Focus) -> Result<()> {
        let focused = focus == Focus::Focused;
        if self.screen.term.is_focused == focused {
            return Ok(());
        }
        self.screen.term.is_focused = focused;
        if self.modes().focus_events {
            self.write(if focused { b"\x1b[I" } else { b"\x1b[O" })?;
        }
        Ok(())
    }
}

impl Screen {
    /// An empty screen. `replies`: where its answers to the program's
    /// questions go; `None` when there is no program.
    pub(crate) fn new(columns: u16, lines: u16, scrollback: usize, theme: Theme, replies: Option<Writer>) -> Self {
        let notices = Arc::new(Mutex::new(Vec::new()));
        let listener = Listener { writer: replies.clone(), notices: Arc::clone(&notices), theme };
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Term::new(config, &TermSize::new(columns.into(), lines.into()), listener);
        Self {
            term,
            parser: Processor::new(),
            notices,
            raw_log: None,
            scanner: Scanner::default(),
            prompt: Prompt::default(),
            graphics: Graphics::default(),
            cell_pixels: CELL_PIXELS,
            replies,
        }
    }

    /// Takes bytes the program printed and updates the screen. Where a
    /// shell's mark ends, the cursor's place is taken for it.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if let Some(log) = self.raw_log.as_mut() {
            // A log that cannot be written is given up, not the session.
            if log.write_all(bytes).is_err() {
                self.raw_log = None;
            }
        }
        let mut done = 0;
        for (end, mark) in self.scanner.scan(bytes) {
            self.parser.advance(&mut self.term, &bytes[done..end]);
            done = end;
            if let Mark::Graphics(text) = mark {
                self.graphics_command(&text);
                continue;
            }
            if let Some(mut finished) = self.prompt.mark(mark, self.cursor_place(), std::time::Instant::now()) {
                if let Some(line) = finished.line {
                    finished.output = self.lines_between(line + 1, self.cursor_place().0);
                }
                self.notices.lock().expect(LOCK_POISONED).push(Notice::Finished(finished));
            }
        }
        self.parser.advance(&mut self.term, &bytes[done..]);
    }

    /// Does what a graphics command asks: answers, and placeholders at the cursor.
    fn graphics_command(&mut self, text: &str) {
        for effect in self.graphics.command(text, self.cell_pixels) {
            match effect {
                Effect::Reply(answer) => {
                    if let Some(writer) = &self.replies {
                        // A failed write means the program is gone.
                        let _ = writer.lock().expect(LOCK_POISONED).write_all(answer.as_bytes());
                    }
                }
                Effect::Place { id, cells: (columns, rows), cursor } => {
                    // No wider than the room right of the cursor: a wrapped row would break the picture.
                    let start = self.term.grid().cursor.point.column.0;
                    let room = self.term.columns().saturating_sub(start).max(1);
                    let text = graphics::placeholders(id, (columns.min(room), rows), start, cursor);
                    self.parser.advance(&mut self.term, text.as_bytes());
                }
            }
        }
    }

    /// A cell's size in pixels, as the window draws it.
    pub(crate) fn set_cell_pixels(&mut self, width: u32, height: u32) {
        self.cell_pixels = (width.max(1), height.max(1));
    }

    /// A cell's size in pixels, as last drawn.
    pub(crate) fn cell_pixels(&self) -> (u32, u32) {
        self.cell_pixels
    }

    /// The image numbered `id`, if the program sent one.
    pub(crate) fn image(&self, id: u32) -> Option<&Image> {
        self.graphics.image(id)
    }

    /// The cursor's cell, its line counted from the top of the history.
    fn cursor_place(&self) -> (usize, usize) {
        let grid = self.term.grid();
        let line = usize::try_from(grid.cursor.point.line.0).unwrap_or(0);
        (grid.history_size() + line, grid.cursor.point.column.0)
    }

    /// The text of lines `from..to` (counted from the top of the history),
    /// at most the last [`OUTPUT_LINES`] of them; lines no longer kept are left out.
    fn lines_between(&self, from: usize, to: usize) -> String {
        let grid = self.term.grid();
        let (history, columns) = (grid.history_size(), self.term.columns());
        let mut text = String::new();
        for line in from.max(to.saturating_sub(OUTPUT_LINES))..to {
            // Lines above the screen are in the history, at negative numbers.
            let Ok(at) = i32::try_from(i64::try_from(line).unwrap_or(i64::MAX) - i64::try_from(history).unwrap_or(0))
            else {
                continue;
            };
            if at < -i32::try_from(history).unwrap_or(i32::MAX) {
                continue;
            }
            text.push_str(&row_text(&grid[Line(at)], columns));
        }
        text
    }

    /// Whether the cursor's line is empty from the cursor on: room to show a suggestion without
    /// covering the shell's own (PowerShell's grey prediction, for one).
    pub(crate) fn blank_right_of_cursor(&self) -> bool {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        let row = &grid[point.line];
        (point.column.0..self.term.columns()).all(|column| row[Column(column)].c == ' ')
    }

    /// The shell's directory, with `/`, if its hook said it.
    pub(crate) fn cwd(&self) -> Option<&str> {
        self.prompt.cwd()
    }

    /// Which shell runs, if its hook said it.
    pub(crate) fn shell(&self) -> Option<&str> {
        self.prompt.shell()
    }

    /// The user pressed Enter: what was on the input line is the command.
    pub(crate) fn enter(&mut self) {
        let typed = self.input().map(|input| input.text);
        self.prompt.enter(typed, std::time::Instant::now());
    }

    /// What is typed at the prompt; `None` when the shell is not at its
    /// input line, a program has the whole screen, the view is scrolled
    /// back, or the input runs over more than one line.
    pub(crate) fn input(&self) -> Option<Input> {
        let Phase::Input((start_line, start_column)) = self.prompt.phase() else { return None };
        if self.modes().alt_screen || self.term.grid().display_offset() != 0 {
            return None;
        }
        let grid = self.term.grid();
        let columns = self.term.columns();
        let (cursor_line, cursor_column) = self.cursor_place();
        if cursor_line < start_line {
            return None;
        }
        let history = grid.history_size();
        let mut text = String::new();
        for line in start_line..=cursor_line {
            let screen_line = i32::try_from(line.checked_sub(history)?).ok()?;
            let row = &grid[Line(screen_line)];
            // A line the input does not run on into is a line break: more than one line of input.
            if line < cursor_line && !row[Column(columns - 1)].flags.contains(Flags::WRAPLINE) {
                return None;
            }
            let from = if line == start_line { start_column } else { 0 };
            let to = if line == cursor_line { cursor_column } else { columns };
            for column in from..to.min(columns) {
                let cell = &row[Column(column)];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                text.push(cell.c);
                text.extend(cell.zerowidth().into_iter().flatten());
            }
        }
        let start_column = if start_line == cursor_line { start_column } else { 0 };
        Some(Input { text, cursor: (cursor_line - history, cursor_column), start_column })
    }

    /// The screen's text, one line of the screen a line, without the
    /// spaces at each line's end: what `--replay` prints.
    pub(crate) fn text(&self) -> String {
        let snapshot = self.snapshot();
        let mut text = String::new();
        for line in 0..snapshot.lines {
            let mut row = String::new();
            for cell in snapshot.row(line).iter().filter(|cell| !cell.has(attr::SPACER)) {
                row.push(cell.c);
                row.extend(cell.combining.as_deref());
            }
            text.push_str(row.trim_end());
            text.push('\n');
        }
        text
    }

    /// What the program asked for since the last call.
    pub(crate) fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut *self.notices.lock().expect(LOCK_POISONED))
    }

    /// The screen's size as (columns, lines).
    pub(crate) fn size(&self) -> (u16, u16) {
        (narrow(self.term.columns()), narrow(self.term.screen_lines()))
    }

    /// Changes the screen's size. A session's `resize` also tells its program.
    pub(crate) fn set_size(&mut self, columns: u16, lines: u16) {
        self.term.resize(TermSize::new(columns.into(), lines.into()));
    }

    pub(crate) fn modes(&self) -> Modes {
        let mode = self.term.mode();
        let has = |flag: TermMode| mode.contains(flag);
        // The widest of the mouse modes that is on.
        let mouse = if has(TermMode::MOUSE_MOTION) {
            MouseTracking::Motion
        } else if has(TermMode::MOUSE_DRAG) {
            MouseTracking::Drags
        } else if has(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Clicks
        } else {
            MouseTracking::Off
        };
        Modes {
            cursor_keys: if has(TermMode::APP_CURSOR) { CursorKeys::Application } else { CursorKeys::Normal },
            paste: if has(TermMode::BRACKETED_PASTE) { Paste::Bracketed } else { Paste::Typed },
            mouse,
            mouse_protocol: if has(TermMode::SGR_MOUSE) { MouseProtocol::Sgr } else { MouseProtocol::Legacy },
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
        }
    }

    /// Scrolls the view `lines` back into history (negative: toward now).
    pub(crate) fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Starts a selection at a cell of the view, in the half of it that
    /// was clicked: in the right half, the cell itself is not yet in.
    pub(crate) fn select(&mut self, line: usize, column: usize, half: Half, by: SelectBy) {
        let kind = match by {
            SelectBy::Cell => SelectionType::Simple,
            SelectBy::Word => SelectionType::Semantic,
            SelectBy::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(kind, self.point(line, column), side(half)));
    }

    /// Moves the selection's end to a cell of the view.
    pub(crate) fn select_to(&mut self, line: usize, column: usize, half: Half) {
        let point = self.point(line, column);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side(half));
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// The selected text; `None` when nothing is selected.
    pub(crate) fn selection(&self) -> Option<String> {
        self.term.selection_to_string().filter(|text| !text.is_empty())
    }

    /// A cell of the view as the library's point, which counts history.
    fn point(&self, line: usize, column: usize) -> Point {
        let line = line.min(self.term.screen_lines().saturating_sub(1));
        let column = column.min(self.term.columns().saturating_sub(1));
        viewport_to_point(self.term.grid().display_offset(), Point::new(line, Column(column)))
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let (columns, lines) = (self.term.columns(), self.term.screen_lines());
        let content = self.term.renderable_content();
        let scrolled = content.display_offset;
        let blank = Cell { c: ' ', combining: None, fg: Color::Foreground, bg: Color::Background, attrs: 0 };
        let mut cells = vec![blank; columns * lines];
        // A line of the grid as a line of the view: history lines are negative.
        let scrolled_lines = i32::try_from(scrolled).unwrap_or(i32::MAX);
        let view_line = |line: Line| usize::try_from(line.0.saturating_add(scrolled_lines)).ok().filter(|&l| l < lines);
        for indexed in content.display_iter {
            let Some(line) = view_line(indexed.point.line) else { continue };
            let Some(slot) = cells.get_mut(line * columns + indexed.point.column.0) else { continue };
            let cell = indexed.cell;
            let flags = cell.flags;
            let mut attrs = 0;
            for (flag, attribute) in [
                (Flags::BOLD, attr::BOLD),
                (Flags::ITALIC, attr::ITALIC),
                (Flags::ALL_UNDERLINES, attr::UNDERLINE),
                (Flags::STRIKEOUT, attr::STRIKE),
                (Flags::DIM, attr::DIM),
                (Flags::INVERSE, attr::INVERSE),
                (Flags::HIDDEN, attr::HIDDEN),
                (Flags::WIDE_CHAR, attr::WIDE),
                (Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER, attr::SPACER),
            ] {
                if flags.intersects(flag) {
                    attrs |= attribute;
                }
            }
            if content.selection.is_some_and(|range| range.contains(indexed.point)) {
                attrs |= attr::SELECTED;
            }
            let (fg, dim) = color(cell.fg, content.colors);
            let (bg, _) = color(cell.bg, content.colors);
            if dim {
                attrs |= attr::DIM;
            }
            *slot = Cell { c: cell.c, combining: cell.zerowidth().map(|chars| chars.iter().collect()), fg, bg, attrs };
        }
        let shape = match content.cursor.shape {
            VteCursor::Block => Some(CursorShape::Block),
            VteCursor::Beam => Some(CursorShape::Beam),
            VteCursor::Underline => Some(CursorShape::Underline),
            VteCursor::HollowBlock => Some(CursorShape::Hollow),
            VteCursor::Hidden => None,
        };
        let cursor = shape.and_then(|shape| {
            let line = view_line(content.cursor.point.line)?;
            Some(Cursor { line, column: content.cursor.point.column.0, shape })
        });
        Snapshot { columns, lines, cells, cursor, scrolled }
    }

    /// The screen's text, one string per line, top first, each as wide as the screen.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<String> {
        let snapshot = self.snapshot();
        (0..snapshot.lines)
            .map(|line| {
                let mut text = String::new();
                for cell in snapshot.row(line).iter().filter(|cell| !cell.has(attr::SPACER)) {
                    text.push(cell.c);
                    text.extend(cell.combining.as_deref());
                }
                text
            })
            .collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Already ended is fine: nothing is left to stop.
        let _ = self.killer.kill();
    }
}

fn side(half: Half) -> Side {
    match half {
        Half::Left => Side::Left,
        Half::Right => Side::Right,
    }
}

/// A screen's width or height in cells as the pty takes it. No screen is
/// 65 536 cells a side; one that claimed to be is cut to the most there is.
fn narrow(cells: usize) -> u16 {
    u16::try_from(cells).unwrap_or(u16::MAX)
}

/// The first of the eight dim colors in the library's numbering.
const DIM_START: usize = number(NamedColor::DimBlack);
/// How many colors have names (black to bright white) and dim twins (black to white).
const NAMED_COLORS: u8 = 16;
const DIM_COLORS: u8 = 8;

/// A cell's color in this crate's terms, and whether it is a dim one. A
/// color the program redefined (OSC 4, 10, 11) comes back as its new value.
fn color(color: ansi::Color, redefined: &alacritty_terminal::term::color::Colors) -> (Color, bool) {
    let rgb = |c: ansi::Rgb| Color::Rgb(Rgb(c.r, c.g, c.b));
    match color {
        ansi::Color::Spec(c) => (rgb(c), false),
        ansi::Color::Indexed(index) => (redefined[usize::from(index)].map_or(Color::Indexed(index), rgb), false),
        ansi::Color::Named(name) => {
            let index = number(name);
            if let Some(c) = redefined[index] {
                return (rgb(c), false);
            }
            let below = |limit: u8, from: usize| {
                index.checked_sub(from).and_then(|offset| u8::try_from(offset).ok()).filter(|offset| *offset < limit)
            };
            if let Some(named) = below(NAMED_COLORS, 0) {
                (Color::Indexed(named), false)
            } else if let Some(dim) = below(DIM_COLORS, DIM_START) {
                (Color::Indexed(dim), true)
            } else if index == BACKGROUND_INDEX {
                (Color::Background, false)
            } else {
                (Color::Foreground, name == NamedColor::DimForeground)
            }
        }
    }
}

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Pasted text as the bytes to send. Bracketed: between the paste marks,
/// with any escape character taken out so the text cannot end the paste
/// itself. Typed: each line end becomes a carriage return (Enter).
fn paste_bytes(text: &str, paste: Paste) -> Vec<u8> {
    match paste {
        Paste::Bracketed => {
            let clean = text.replace('\x1b', "");
            [PASTE_START, clean.as_bytes(), PASTE_END].concat()
        }
        Paste::Typed => text.replace("\r\n", "\r").replace('\n', "\r").into_bytes(),
    }
}

fn pty_size(columns: u16, lines: u16) -> PtySize {
    PtySize { rows: lines, cols: columns, pixel_width: 0, pixel_height: 0 }
}

/// The program a new tab runs: the configured one, else PowerShell 7 when
/// it is installed, else Windows PowerShell; `$SHELL` elsewhere. It starts
/// in the current directory.
pub(crate) fn shell_command(program: Option<&str>, args: &[String]) -> Result<CommandBuilder> {
    let mut command = CommandBuilder::new(program.map_or_else(default_shell, str::to_owned));
    command.args(args);
    command.cwd(std::env::current_dir().context("the current directory")?);
    // UnifyTerm started from a multiplexer's pane is not in that pane: what
    // the pane told its shell (which pane, whose history) is not passed on.
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(names_a_pane) {
            command.env_remove(name);
        }
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "unifyterm");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    Ok(command)
}

/// Whether an environment variable says which multiplexer pane a shell is
/// in: keepane's, tmux's, screen's.
fn names_a_pane(variable: &str) -> bool {
    let name = variable.to_ascii_uppercase();
    name.starts_with("KEEPANE") || matches!(name.as_str(), "TMUX" | "TMUX_PANE" | "STY")
}

#[cfg(windows)]
fn default_shell() -> String {
    let on_path = |name: &str| {
        std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
    };
    if on_path("pwsh.exe") { "pwsh.exe".into() } else { "powershell.exe".into() }
}

#[cfg(not(windows))]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(15);
    const HISTORY: usize = 100;

    fn echo(text: &str) -> CommandBuilder {
        let (program, flag) = if cfg!(windows) { ("cmd.exe", "/c") } else { ("/bin/sh", "-c") };
        let mut command = CommandBuilder::new(program);
        command.args([flag, &format!("echo {text}")]);
        command
    }

    fn start(command: CommandBuilder) -> (Session, Receiver<PtyEvent>) {
        Session::spawn(command, 80, 24, HISTORY, Theme::default()).unwrap()
    }

    /// A screen with no program: what tests feed it is all that is on it.
    pub(crate) fn quiet(columns: u16, lines: u16) -> Screen {
        Screen::new(columns, lines, HISTORY, Theme::default(), None)
    }

    #[test]
    fn shell_marks_leave_nothing_on_the_screen() {
        let mut screen = quiet(40, 3);
        let marks = "a\x1b]133;A\x07b\x1b]133;B\x1b\\c\x1b]633;E;ls -l\x5cx3b x\x07d\x1b]7;file://h/C:/x\x1b\\e\x1b]9;9;C:\\x\x1b\\f\x1b]133;D;1\x07g\x1b]633;P;Shell=pwsh\x07h";
        screen.feed(marks.as_bytes());
        assert_eq!(screen.text().lines().next(), Some("abcdefgh"), "the screen's library passes over them");
        let notices = screen.take_notices();
        let [Notice::Finished(finished)] = notices.as_slice() else { panic!("one command ended: {notices:?}") };
        assert_eq!(
            (finished.text.as_str(), finished.exit, finished.cwd.as_deref()),
            ("ls -l; x", Some(1), Some("C:/x")),
            "the marks were read: the text, how it ended, the last directory said"
        );
        assert_eq!(screen.shell(), Some("pwsh"));
    }

    #[test]
    fn graphics_sequences_leave_nothing_on_the_screen() {
        let mut screen = quiet(40, 3);
        screen.feed(b"a\x1b_Gf=100,a=T,m=1;iVBORw0KGgo=\x1b\\b\x1b_Gm=0;AAAA\x1b\\c");
        assert_eq!(screen.text().lines().next(), Some("abc"), "APC is passed over by the screen's library");
    }

    #[test]
    fn a_placed_image_is_placeholders_in_the_grid_that_scroll_with_the_text() {
        use base64::Engine as _;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(18, 38))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        let mut screen = quiet(20, 6);
        screen.set_cell_pixels(9, 19);
        screen.feed(format!("ab\x1b_Ga=T,f=100,i=513,q=2;{data}\x1b\\cd").as_bytes());
        // 18 × 38 pixels on 9 × 19 cells: 2 columns, 2 rows, from column 2.
        let snapshot = screen.snapshot();
        let cell = |line: usize, column: usize| snapshot.row(line)[column].clone();
        assert_eq!(cell(0, 2).c, graphics::PLACEHOLDER);
        assert_eq!(cell(0, 2).fg, Color::Rgb(Rgb(0, 2, 1)), "the number 513 is the color 0,2,1");
        let marks = |line, column| {
            cell(line, column).combining.unwrap_or_default().chars().filter_map(graphics::number_of).collect::<Vec<_>>()
        };
        assert_eq!(
            (marks(0, 2), marks(0, 3), marks(1, 2), marks(1, 3)),
            (vec![0, 0], vec![0, 1], vec![1, 0], vec![1, 1])
        );
        let text: Vec<String> = screen.text().lines().take(2).map(str::to_owned).collect();
        assert_eq!(
            text,
            [
                "ab\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\u{0305}\u{030D}",
                "  \u{10EEEE}\u{030D}\u{0305}\u{10EEEE}\u{030D}\u{030D}cd"
            ],
            "the text after the image goes on after its last row"
        );
        assert!(screen.image(513).is_some());
        // Six lines down from line 1: both rows of the image are above the screen now.
        screen.feed(b"\r\n\n\n\n\n\n\n\n");
        assert_eq!(screen.snapshot().row(0)[2].c, ' ', "scrolled away with the text");
        screen.feed(b"\x1b[2J");
        assert!(screen.snapshot().cells.iter().all(|cell| cell.c != graphics::PLACEHOLDER), "cleared with the screen");
        // An image that reaches the last column: every row starts in the same column.
        screen.feed(b"\x1b[H  \x1b_Ga=p,i=513,c=30,r=3,q=2\x1b\\");
        let snapshot = screen.snapshot();
        for line in 0..3 {
            let row = snapshot.row(line);
            assert_eq!(
                (row[1].c, row[2].c, row[19].c),
                (' ', graphics::PLACEHOLDER, graphics::PLACEHOLDER),
                "row {line}"
            );
        }
    }

    #[test]
    fn the_input_is_read_from_the_screen_between_the_prompt_and_the_cursor() {
        // 30 columns: the suggestion fits on the line, the long input below does not.
        let mut screen = quiet(30, 4);
        assert_eq!(screen.input(), None, "no prompt yet");
        screen.feed(b"\x1b]133;A\x07PS> \x1b]133;B\x07git st");
        let input = screen.input().unwrap();
        assert_eq!((input.text.as_str(), input.cursor, input.start_column), ("git st", (0, 10), 4));
        // A shell's grey suggestion right of the cursor is not input.
        screen.feed(b"atus\x1b[90m --short\x1b[0m\x1b[8D");
        assert_eq!(screen.input().unwrap().text, "git status");
        // Input that runs past the line's end goes on on the next line.
        screen.feed(b"\x1b[K --porcelain --branch");
        assert_eq!(screen.input().unwrap().text, "git status --porcelain --branch");
        // Chinese takes two cells a character, and is read once.
        screen.feed(b"\r\n\x1b]133;A\x07> \x1b]133;B\x07echo \xe4\xb8\xad\xe6\x96\x87");
        assert_eq!(screen.input().unwrap().text, "echo 中文");
        screen.enter();
        assert_eq!(screen.input(), None, "after Enter a command runs");
        screen.feed("\r\n中文\r\nsecond line\r\n\x1b]133;D;0\x07".as_bytes());
        let notices = screen.take_notices();
        let [Notice::Finished(finished)] = notices.as_slice() else { panic!("one command ended: {notices:?}") };
        assert_eq!((finished.text.as_str(), finished.exit), ("echo 中文", Some(0)));
        assert_eq!(finished.output, "中文\nsecond line\n", "what it printed, from the line below the command");
        // A program with the whole screen: no input line.
        screen.feed(b"\x1b]133;A\x07> \x1b]133;B\x07vi\x1b[?1049h");
        assert_eq!(screen.input(), None);
    }

    /// Feeds the session what arrives until `done` says so; the screen at the end.
    fn pump(session: &mut Session, events: &Receiver<PtyEvent>, done: impl Fn(&Session) -> bool) -> Vec<String> {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline && !done(session) {
            match events.try_recv() {
                Ok(PtyEvent::Output(bytes)) => session.feed(&bytes),
                Ok(PtyEvent::Exited) | Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        session.rows()
    }

    fn shows(session: &Session, text: &str) -> bool {
        session.rows().iter().any(|row| row.contains(text))
    }

    #[test]
    fn what_a_program_prints_reaches_the_screen() {
        let (mut session, events) = start(echo("unifyterm-ok"));
        let rows = pump(&mut session, &events, |s| shows(s, "unifyterm-ok"));
        assert!(shows(&session, "unifyterm-ok"), "{rows:?}");
        assert_eq!(rows.len(), 24);
    }

    #[test]
    fn the_end_of_the_program_is_reported() {
        let (mut session, events) = start(echo("bye"));
        let deadline = Instant::now() + WAIT;
        loop {
            match events.try_recv() {
                Ok(PtyEvent::Exited) => break,
                // Fed, not dropped: ConPTY holds the program back until its
                // question about the cursor is answered, which feeding does.
                Ok(PtyEvent::Output(bytes)) => session.feed(&bytes),
                Err(_) => {
                    assert!(Instant::now() < deadline, "no exit reported");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn the_last_output_comes_before_the_end() {
        // Twenty short programs: each one's last words must be in before its end is reported.
        let mut lost = Vec::new();
        for round in 0..20 {
            let (mut session, events) = start(echo("last-words"));
            let deadline = Instant::now() + WAIT;
            loop {
                match events.try_recv() {
                    Ok(PtyEvent::Exited) => break,
                    Ok(PtyEvent::Output(bytes)) => session.feed(&bytes),
                    Err(_) => {
                        assert!(Instant::now() < deadline, "no exit reported");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            if !shows(&session, "last-words") {
                lost.push(round);
            }
        }
        assert!(lost.is_empty(), "the last output came after the end in rounds {lost:?}");
    }

    #[test]
    fn typed_text_reaches_the_program() {
        // Not the user's own shell: PowerShell and bash would keep the typed line in the user's
        // history file. cmd.exe and sh keep none.
        let (program, line): (&str, &[u8]) =
            if cfg!(windows) { ("cmd.exe", b"set /a 6*7\r") } else { ("sh", b"echo $((6*7))\r") };
        let args = if cfg!(windows) { vec!["/q".to_owned()] } else { Vec::new() };
        let (mut session, events) = start(shell_command(Some(program), &args).unwrap());
        // 6 * 7 is typed, 42 is only ever printed.
        session.write(line).unwrap();
        let rows = pump(&mut session, &events, |s| s.rows().iter().any(|row| row.trim() == "42"));
        assert!(rows.iter().any(|row| row.trim() == "42"), "{rows:?}");
    }

    #[test]
    fn a_resize_changes_the_screen() {
        let (mut session, _events) = start(echo("x"));
        session.resize(100, 30).unwrap();
        assert_eq!(session.size(), (100, 30));
        assert_eq!(session.rows().len(), 30);
        assert_eq!(session.snapshot().cells.len(), 3000);
    }

    #[test]
    fn colors_and_attributes_are_in_the_snapshot() {
        let mut session = quiet(20, 4);
        session
            .feed(b"\x1b[H\x1b[2J\x1b[1;31ma\x1b[0;4;38;5;196mb\x1b[0;7;48;2;1;2;3mc\x1b[0;2;9md\x1b[0;3;8me\x1b[0mf");
        let snapshot = session.snapshot();
        let row = snapshot.row(0);
        assert_eq!((row[0].c, row[0].fg, row[0].attrs), ('a', Color::Indexed(1), attr::BOLD));
        assert_eq!((row[1].c, row[1].fg, row[1].attrs), ('b', Color::Indexed(196), attr::UNDERLINE));
        assert_eq!((row[2].c, row[2].bg, row[2].attrs), ('c', Color::Rgb(Rgb(1, 2, 3)), attr::INVERSE));
        assert_eq!((row[3].c, row[3].attrs), ('d', attr::DIM | attr::STRIKE));
        assert_eq!((row[4].c, row[4].attrs), ('e', attr::ITALIC | attr::HIDDEN));
        assert_eq!((row[5].c, row[5].fg, row[5].bg, row[5].attrs), ('f', Color::Foreground, Color::Background, 0));
        assert_eq!(snapshot.cursor, Some(Cursor { line: 0, column: 6, shape: CursorShape::Block }));
    }

    #[test]
    fn a_wide_character_takes_two_cells() {
        let mut session = quiet(20, 4);
        session.feed("\x1b[H\x1b[2J中a\u{301}".as_bytes());
        let snapshot = session.snapshot();
        let row = snapshot.row(0);
        assert_eq!((row[0].c, row[0].attrs), ('中', attr::WIDE));
        assert!(row[1].has(attr::SPACER));
        assert_eq!((row[2].c, row[2].combining.as_deref()), ('a', Some("\u{301}")));
        assert_eq!(session.rows()[0].trim_end(), "中a\u{301}");
    }

    #[test]
    fn the_view_scrolls_back_into_history() {
        let mut session = quiet(10, 3);
        session.feed(b"\x1b[H\x1b[2J1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(session.rows(), ["3         ", "4         ", "5         "]);
        session.scroll(2);
        let snapshot = session.snapshot();
        assert_eq!((snapshot.scrolled, snapshot.cursor), (2, None), "the cursor's line is out of view");
        assert_eq!(session.rows()[0].trim_end(), "1");
        // No further back than there is history (clearing the screen put a blank line there too).
        session.scroll(100);
        let oldest = session.snapshot().scrolled;
        assert!((2..10).contains(&oldest), "{oldest}");
        session.scroll(100);
        assert_eq!(session.snapshot().scrolled, oldest);
        session.scroll_to_bottom();
        assert_eq!(session.rows()[2].trim_end(), "5");
    }

    #[test]
    fn a_selection_gives_its_text() {
        let mut session = quiet(20, 3);
        session.feed(b"\x1b[H\x1b[2Jhello world\r\nsecond line");
        assert_eq!(session.selection(), None);
        session.select(0, 6, Half::Left, SelectBy::Cell);
        session.select_to(0, 10, Half::Right);
        assert_eq!(session.selection().as_deref(), Some("world"));
        let selected: Vec<bool> = session.snapshot().row(0).iter().map(|cell| cell.has(attr::SELECTED)).collect();
        assert_eq!(selected.iter().filter(|&&on| on).count(), 5);
        assert!(selected[6] && selected[10] && !selected[5] && !selected[11]);

        session.select(0, 2, Half::Left, SelectBy::Word);
        assert_eq!(session.selection().as_deref(), Some("hello"));
        session.select(1, 3, Half::Left, SelectBy::Line);
        assert_eq!(session.selection().as_deref(), Some("second line\n"));
        session.select(0, 0, Half::Left, SelectBy::Cell);
        session.select_to(1, 5, Half::Right);
        assert_eq!(session.selection().as_deref(), Some("hello world\nsecond"));
        session.clear_selection();
        assert_eq!(session.selection(), None);
    }

    #[test]
    fn the_programs_requests_become_notices() {
        let mut session = quiet(20, 3);
        // A title, a bell (which UnifyTerm does nothing with), text for the clipboard, the title reset.
        session.feed(b"\x1b]0;my title\x07\x07\x1b]52;c;aGk=\x07\x1b]2;\x07");
        let notices = session.take_notices();
        assert_eq!(notices[0], Notice::Title(Some("my title".into())));
        assert!(notices.contains(&Notice::Copy("hi".into())), "{notices:?}");
        assert!(session.take_notices().is_empty(), "taken once");
    }

    #[test]
    fn modes_follow_what_the_program_set() {
        let mut session = quiet(20, 3);
        assert_eq!(session.modes(), Modes { alternate_scroll: true, ..Modes::default() }, "a new screen");
        session.feed(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?1049h\x1b[?1004h");
        let modes = session.modes();
        assert_eq!(modes.cursor_keys, CursorKeys::Application);
        assert_eq!(modes.paste, Paste::Bracketed);
        assert_eq!(modes.mouse, MouseTracking::Clicks);
        assert_eq!(modes.mouse_protocol, MouseProtocol::Sgr);
        assert!(modes.alt_screen && modes.focus_events, "{modes:?}");
        session.feed(b"\x1b[?1002h");
        assert_eq!(session.modes().mouse, MouseTracking::Drags);
        session.feed(b"\x1b[?1003h");
        assert_eq!(session.modes().mouse, MouseTracking::Motion, "the widest mode that is on");
        session.feed(b"\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1006l");
        assert_eq!(
            (session.modes().mouse, session.modes().mouse_protocol),
            (MouseTracking::Off, MouseProtocol::Legacy)
        );
    }

    #[test]
    fn a_panes_variables_are_not_passed_on() {
        for name in ["KEEPANE", "KEEPANE_PANE", "KEEPANE_SHELL_HISTORY", "keepane_env_file", "TMUX", "TMUX_PANE", "STY"]
        {
            assert!(names_a_pane(name), "{name}");
        }
        for name in ["PATH", "TERM", "TMUXINATOR_CONFIG", "STYLE", "HOME"] {
            assert!(!names_a_pane(name), "{name}");
        }
        let command = shell_command(Some("sh"), &[]).unwrap();
        let passed: Vec<_> = command.iter_full_env_as_str().map(|(name, _)| name.to_owned()).collect();
        assert!(passed.iter().all(|name| !names_a_pane(name)), "{passed:?}");
        assert!(passed.iter().any(|name| name == "TERM_PROGRAM"));
    }

    #[test]
    fn each_session_has_its_own_raw_log() {
        assert_eq!(raw_log_path("out.raw", 1), "out.raw");
        assert_eq!(raw_log_path("out.raw", 2), "out.raw.2");
        assert_eq!(raw_log_path("out.raw", 13), "out.raw.13");
    }

    #[test]
    fn pasted_text_is_marked_or_given_enter_keys() {
        assert_eq!(paste_bytes("a\r\nb\nc", Paste::Typed), b"a\rb\rc");
        assert_eq!(paste_bytes("a\nb", Paste::Bracketed), b"\x1b[200~a\nb\x1b[201~");
        let sneaky = paste_bytes("x\x1b[201~rm -rf /", Paste::Bracketed);
        assert_eq!(sneaky, b"\x1b[200~x[201~rm -rf /\x1b[201~", "the text cannot end the paste");
    }
}
