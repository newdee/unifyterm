//! One tab: a session and the view that paints it and feeds it input.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, Font,
    FontFallbacks, FontFeatures, FontStyle, FontWeight, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Size,
    TextRun, UTF16Selection, Window, canvas, div, fill, point, prelude::*, px, rgb, size,
};
use smol::channel::Receiver;

use crate::config::Config;
use crate::frame::{self, CursorMark, Frame, StrokeKind};
use crate::keys::{self, Mods, MouseAction};
use crate::session::{CursorShape, Focus, Half, MouseTracking, Notice, PtyEvent, SelectBy, Session};
use crate::theme::{Rgb, Theme};
use crate::{Copy, Paste, boxdraw};

/// The settings every tab shares.
#[derive(Debug)]
pub(crate) struct Settings {
    pub(crate) config: Config,
    pub(crate) theme: Theme,
}

/// What a tab tells the window that holds it.
#[derive(Debug)]
pub(crate) enum TerminalEvent {
    /// The title changed.
    Title,
    /// The program ended.
    Exited,
}

/// The fewest columns and lines a screen is given, however small the window.
const MIN_COLUMNS: u16 = 2;
const MIN_LINES: u16 = 1;
/// The screen's size until the first paint says how big the window is.
const START_COLUMNS: u16 = 100;
const START_LINES: u16 = 30;
/// A cell's width as a share of the font size, when the font will not say.
const FALLBACK_ADVANCE: f32 = 0.6;
/// Where in a line's height the underline and the strike-through are.
const UNDERLINE_AT: f32 = 0.88;
const STRIKE_AT: f32 = 0.52;
/// The beam and underline cursors' thickness, and the hollow one's border.
const CURSOR_THICKNESS: f32 = 2.0;
const HOLLOW_BORDER: f32 = 1.0;
/// Lines the system scrolls for one notch of the wheel. A program that
/// tracks the mouse gets one wheel report a notch and scrolls by its own
/// amount (nvim: 3 lines), so that many lines of wheel make one report.
const LINES_PER_NOTCH: usize = 3;
/// How far inside a cell a point at the screen's far edge is taken to be,
/// so that it counts as the last cell and not one past it.
const INSIDE_LAST_CELL: f32 = 0.01;

pub(crate) struct TerminalView {
    session: Session,
    settings: Arc<Settings>,
    pub(crate) focus: FocusHandle,
    /// The title the program set.
    title: Option<String>,
    /// Text an input method is composing, shown at the cursor, not yet sent.
    composing: String,
    /// Where the text is in the window and one cell's size, as last painted.
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    /// The cursor's cell (line, column), for placing the input method's window.
    cursor_cell: (usize, usize),
    /// A drag with the left button is selecting text.
    selecting: bool,
    /// The part of a line the wheel has moved that is not yet scrolled.
    wheel: f32,
    /// The cell last reported to a program that tracks the mouse.
    reported: Option<(usize, usize)>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

/// What `prepaint` hands `paint`.
struct Prepared {
    frame: Frame,
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    composing: String,
}

/// The bytes that put `text` on a screen in yellow. Control characters in
/// it are left out: the text may quote a file, and what a file holds must
/// not be able to drive the screen.
fn announcement(text: &str) -> String {
    let clean: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    format!("\x1b[33m{}\x1b[0m\r\n", clean.replace('\n', "\r\n"))
}

fn hsla(color: Rgb) -> gpui::Hsla {
    rgb(color.hex()).into()
}

/// A count of cells (or of device pixels in a cell) as a factor for a
/// length. A count past 65 535 is cut to that: no screen is that large.
fn factor(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// How many whole units a ratio of lengths holds: 0 for a negative ratio
/// and for one that is not a number.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "float-to-int `as` saturates and maps NaN to 0, which is what a count wants"
)]
fn whole(ratio: f32) -> usize {
    ratio as usize
}

/// A problem with the session that the user cannot act on at once: logged, and the tab goes on.
fn logged(result: anyhow::Result<()>) {
    if let Err(error) = result {
        log::error!("{error:#}");
    }
}

fn button(button: MouseButton) -> Option<keys::MouseButton> {
    match button {
        MouseButton::Left => Some(keys::MouseButton::Left),
        MouseButton::Middle => Some(keys::MouseButton::Middle),
        MouseButton::Right => Some(keys::MouseButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

impl TerminalView {
    /// Starts `command` in a session of its own.
    pub(crate) fn start(
        command: portable_pty::CommandBuilder,
        settings: &Settings,
    ) -> anyhow::Result<(Session, Receiver<PtyEvent>)> {
        Session::spawn(command, START_COLUMNS, START_LINES, settings.config.terminal.scrollback, settings.theme.clone())
    }

    pub(crate) fn new(
        session: Session,
        events: Receiver<PtyEvent>,
        settings: Arc<Settings>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            let mut running = true;
            while running && let Ok(event) = events.recv().await {
                let PtyEvent::Output(mut bytes) = event else { break };
                // Everything that has already arrived goes in as one piece: one repaint.
                loop {
                    match events.try_recv() {
                        Ok(PtyEvent::Output(more)) => bytes.extend(more),
                        Ok(PtyEvent::Exited) => running = false,
                        Err(_) => {}
                    }
                    if !running || events.is_empty() {
                        break;
                    }
                }
                if this.update(cx, |view, cx| view.output(&bytes, cx)).is_err() {
                    return; // The tab is closed.
                }
            }
            // The tab may be closed already: then nobody is left to tell.
            let _ = this.update(cx, |_, cx| cx.emit(TerminalEvent::Exited));
        })
        .detach();
        Self {
            session,
            settings,
            focus: cx.focus_handle(),
            title: None,
            composing: String::new(),
            origin: Point::default(),
            cell: size(px(1.), px(1.)),
            cursor_cell: (0, 0),
            selecting: false,
            wheel: 0.0,
            reported: None,
        }
    }

    /// The title the program set; `None` when it set none or an empty one.
    pub(crate) fn title(&self) -> Option<&str> {
        self.title.as_deref().filter(|title| !title.trim().is_empty())
    }

    /// Puts a line of uni-term's own on the screen (a problem with the settings file).
    pub(crate) fn announce(&mut self, text: &str) {
        self.session.feed(announcement(text).as_bytes());
    }

    fn output(&mut self, bytes: &[u8], cx: &mut Context<'_, Self>) {
        self.session.feed(bytes);
        for notice in self.session.take_notices() {
            match notice {
                Notice::Title(title) => {
                    self.title = title;
                    cx.emit(TerminalEvent::Title);
                }
                Notice::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            }
        }
        cx.notify();
    }

    /// Sends bytes the user's input made, and shows where they go: the
    /// view returns to the newest line.
    fn send(&mut self, bytes: &[u8], cx: &mut Context<'_, Self>) {
        logged(self.session.write(bytes));
        self.session.scroll_to_bottom();
        cx.notify();
    }

    fn font(&self, weight: FontWeight, style: FontStyle) -> Font {
        let font = &self.settings.config.font;
        Font {
            family: font.family.clone().into(),
            // Said outright, never left to the default: GPUI's Windows
            // text layout applies an empty feature list as "no features",
            // which turns ligatures off.
            features: FontFeatures(Arc::new(vec![("calt".into(), u32::from(font.ligatures))])),
            fallbacks: (!font.fallbacks.is_empty()).then(|| FontFallbacks::from_fonts(font.fallbacks.clone())),
            weight,
            style,
        }
    }

    /// The four fonts text is drawn in: regular, bold, italic, bold italic.
    fn fonts(&self) -> [Font; 4] {
        [
            self.font(FontWeight::NORMAL, FontStyle::Normal),
            self.font(FontWeight::BOLD, FontStyle::Normal),
            self.font(FontWeight::NORMAL, FontStyle::Italic),
            self.font(FontWeight::BOLD, FontStyle::Italic),
        ]
    }

    fn font_size(&self) -> Pixels {
        px(self.settings.config.font.size)
    }

    /// Fits the screen to `bounds` and lays out what is on it.
    fn prepare(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Prepared {
        let font = &self.settings.config.font;
        let text = window.text_system();
        let regular = self.font(FontWeight::NORMAL, FontStyle::Normal);
        let advance = text.em_advance(text.resolve_font(&regular), self.font_size());
        let cell =
            size(advance.unwrap_or(px(font.size * FALLBACK_ADVANCE)), px((font.size * font.line_height).round()));
        let padding = px(self.settings.config.terminal.padding);
        let origin = bounds.origin + point(padding, padding);
        let room = size(bounds.size.width - padding * 2., bounds.size.height - padding * 2.);
        let fits = |ratio: f32| u16::try_from(whole(ratio)).unwrap_or(u16::MAX);
        let (columns, lines) = (fits(room.width / cell.width), fits(room.height / cell.height));
        // A window with no room for a screen (minimized, or dragged that
        // small) keeps the size it had: resizing would make the program
        // rewrap everything for a size nobody reads.
        if columns >= MIN_COLUMNS && lines >= MIN_LINES && (columns, lines) != self.session.size() {
            logged(self.session.resize(columns, lines));
        }
        let has_keyboard = self.focus.is_focused(window) && window.is_window_active();
        let focus = if has_keyboard { Focus::Focused } else { Focus::Unfocused };
        logged(self.session.focus(focus));
        let frame = frame::layout(&self.session.snapshot(), &self.settings.theme, focus);
        (self.origin, self.cell) = (origin, cell);
        if let Some(cursor) = &frame.cursor {
            self.cursor_cell = (cursor.line, cursor.column);
        }
        Prepared { frame, origin, cell, composing: self.composing.clone() }
    }

    /// The cell under a point of the window, as (line, column), and the
    /// half of the cell the point is in. Points outside count as the
    /// nearest cell.
    fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, Half) {
        let (columns, lines) = self.session.size();
        let x = ((position.x - self.origin.x) / self.cell.width).clamp(0.0, f32::from(columns) - INSIDE_LAST_CELL);
        let y = ((position.y - self.origin.y) / self.cell.height).clamp(0.0, f32::from(lines) - INSIDE_LAST_CELL);
        let half = if x.fract() >= 0.5 { Half::Right } else { Half::Left };
        (whole(y), whole(x), half)
    }

    /// Reports a mouse event to a program that asked for the mouse. `false`
    /// when the program did not ask, or Shift is held to select text anyway.
    fn report_mouse(
        &mut self,
        button: keys::MouseButton,
        action: MouseAction,
        position: Point<Pixels>,
        held: Modifiers,
    ) -> bool {
        let modes = self.session.modes();
        if modes.mouse == MouseTracking::Off || held.shift {
            return false;
        }
        let (line, column, _) = self.cell_at(position);
        if action == MouseAction::Move {
            let wanted = match modes.mouse {
                MouseTracking::Motion => true,
                MouseTracking::Drags => button != keys::MouseButton::None,
                MouseTracking::Clicks | MouseTracking::Off => false,
            };
            if !wanted || self.reported == Some((line, column)) {
                return true;
            }
        }
        self.reported = Some((line, column));
        let held = Mods { shift: held.shift, control: held.control, alt: held.alt };
        if let Some(bytes) = keys::mouse(button, action, (column, line), held, modes.mouse_protocol) {
            logged(self.session.write(&bytes));
        }
        true
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let key = &event.keystroke;
        let held = key.modifiers;
        if held.platform {
            return; // A shortcut of the system's or the window's.
        }
        // Ctrl-C with text selected copies it; with none, it interrupts.
        if key.key == "c" && held.control && !held.shift && !held.alt && self.copy_selection(cx) {
            self.session.clear_selection();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let mods = Mods { shift: held.shift, control: held.control, alt: held.alt };
        let cursor_keys = self.session.modes().cursor_keys;
        // No bytes: a key that types text, which comes through `replace_text_in_range`.
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, cursor_keys) {
            self.session.clear_selection();
            self.send(&bytes, cx);
            cx.stop_propagation();
        }
    }

    /// Puts the selected text on the clipboard; `false` when nothing is selected.
    fn copy_selection(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(text) = self.session.selection() else { return false };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.copy_selection(cx);
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.paste_clipboard(cx);
    }

    fn paste_clipboard(&mut self, cx: &mut Context<'_, Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        logged(self.session.paste(&text));
        self.session.clear_selection();
        self.session.scroll_to_bottom();
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.focus);
        let Some(pressed) = button(event.button) else { return };
        self.reported = None;
        if self.report_mouse(pressed, MouseAction::Press, event.position, event.modifiers) {
            return;
        }
        match pressed {
            keys::MouseButton::Left => {
                let (line, column, half) = self.cell_at(event.position);
                let by = match event.click_count {
                    0 | 1 => SelectBy::Cell,
                    2 => SelectBy::Word,
                    _ => SelectBy::Line,
                };
                self.session.select(line, column, half, by);
                self.selecting = true;
            }
            // The right button copies what is selected, else pastes, as the Windows console does.
            keys::MouseButton::Right if self.copy_selection(cx) => self.session.clear_selection(),
            _ => self.paste_clipboard(cx),
        }
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.selecting {
            if event.pressed_button != Some(MouseButton::Left) {
                self.selecting = false; // Released outside the window.
                return;
            }
            let (line, column, half) = self.cell_at(event.position);
            self.session.select_to(line, column, half);
            cx.notify();
            return;
        }
        let held = event.pressed_button.and_then(button).unwrap_or(keys::MouseButton::None);
        self.report_mouse(held, MouseAction::Move, event.position, event.modifiers);
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.selecting {
            self.selecting = false;
            // A click with no drag selects nothing.
            if self.session.selection().is_none() {
                self.session.clear_selection();
            }
            cx.notify();
            return;
        }
        let Some(released) = button(event.button) else { return };
        self.reported = None;
        self.report_mouse(released, MouseAction::Release, event.position, event.modifiers);
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.wheel += match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y / self.cell.height,
        };
        let lines = self.wheel.trunc();
        self.wheel -= lines;
        let (up, count) = (lines > 0.0, whole(lines.abs()));
        if count == 0 {
            return;
        }
        let modes = self.session.modes();
        if modes.mouse != MouseTracking::Off && !event.modifiers.shift {
            let wheel = if up { keys::MouseButton::WheelUp } else { keys::MouseButton::WheelDown };
            for _ in 0..count.div_ceil(LINES_PER_NOTCH) {
                self.reported = None;
                self.report_mouse(wheel, MouseAction::Press, event.position, event.modifiers);
            }
        } else if modes.alt_screen && modes.alternate_scroll {
            // A full-screen program with no mouse (less, man): the wheel is the arrow keys.
            let arrow = if up { "up" } else { "down" };
            let bytes = keys::encode(arrow, None, Mods::default(), modes.cursor_keys).unwrap_or_default();
            logged(self.session.write(&bytes.repeat(count)));
        } else {
            let back = i32::try_from(count).unwrap_or(i32::MAX);
            self.session.scroll(if up { back } else { -back });
            cx.notify();
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<'_, Self>) -> Option<UTF16Selection> {
        // An empty selection: the terminal takes text, with nothing to replace.
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        (!self.composing.is_empty()).then(|| 0..self.composing.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.composing.clear();
        cx.notify();
    }

    /// Finished text: a key's character, or what the input method composed.
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.composing.clear();
        // Nothing typed (an input method closing with no text): nothing to send.
        if text.is_empty() {
            cx.notify();
            return;
        }
        self.session.clear_selection();
        self.send(text.as_bytes(), cx);
    }

    /// Text still being composed: shown at the cursor, sent when finished.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        new_text.clone_into(&mut self.composing);
        cx.notify();
    }

    /// Where the input method puts its window: at the cursor's cell.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, column) = self.cursor_cell;
        let origin = self.origin + point(self.cell.width * factor(column), self.cell.height * factor(line));
        Some(Bounds::new(origin, self.cell))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let (view, painted) = (cx.entity(), cx.entity());
        let (settings, focus) = (Arc::clone(&self.settings), self.focus.clone());
        let fonts = self.fonts();
        let font_size = self.font_size();
        div()
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(self.settings.theme.background.hex()))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    move |bounds, window, cx| view.update(cx, |view, _| view.prepare(bounds, window)),
                    move |bounds, prepared, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx);
                        Painter { prepared: &prepared, theme: &settings.theme, fonts: &fonts, font_size }
                            .paint(window, cx);
                    },
                )
                .size_full(),
            )
    }
}

/// How a piece of text sits in its cells.
#[derive(Clone, Copy)]
enum Placement {
    /// Each glyph at its own cell, of this width.
    Grid(Pixels),
    /// As the font spaces it, centered in this width (a wide character in its two cells).
    Centered(Pixels),
}

/// A piece of text to paint: where, in what, and how it sits in its cells.
struct Label<'a> {
    text: &'a str,
    font: &'a Font,
    color: Rgb,
    at: Point<Pixels>,
    placement: Placement,
}

/// Turns a frame's cells into pixels.
struct Painter<'a> {
    prepared: &'a Prepared,
    theme: &'a Theme,
    /// Regular, bold, italic, bold italic.
    fonts: &'a [Font; 4],
    font_size: Pixels,
}

/// A length rounded to whole device pixels, so neighbors share an edge.
fn snap(value: Pixels, scale: f32) -> Pixels {
    px((value / px(1.) * scale).round() / scale)
}

/// A length in whole device pixels.
fn device_pixels(value: Pixels, scale: f32) -> u32 {
    u32::try_from(whole((value / px(1.) * scale).round())).unwrap_or(u32::MAX)
}

/// Paints a box-drawing or block character into a cell's bounds.
fn paint_drawn(c: char, color: Rgb, bounds: Bounds<Pixels>, scale: f32, window: &mut Window) {
    let (w, h) = (device_pixels(bounds.size.width, scale), device_pixels(bounds.size.height, scale));
    let Some((rects, opacity)) = boxdraw::rects(c, w, h) else { return };
    let color = Rgba { a: opacity, ..rgb(color.hex()) };
    // A cell is far narrower than 65 536 device pixels: the count is exact as a float.
    let length = |device: u32| px(f32::from(u16::try_from(device).unwrap_or(u16::MAX)) / scale);
    for [left, top, right, bottom] in rects {
        let corner = |dx: u32, dy: u32| bounds.origin + point(length(dx), length(dy));
        window.paint_quad(fill(Bounds::from_corners(corner(left, top), corner(right, bottom)), color));
    }
}

impl Painter<'_> {
    fn paint(&self, window: &mut Window, cx: &mut App) {
        let Prepared { frame, origin, cell, composing } = self.prepared;
        let scale = window.scale_factor();
        let x = |column: usize| snap(origin.x + cell.width * factor(column), scale);
        let y = |line: usize| snap(origin.y + cell.height * factor(line), scale);
        let span = |line: usize, column: usize, cells: usize| {
            Bounds::from_corners(point(x(column), y(line)), point(x(column + cells), y(line + 1)))
        };

        for item in &frame.fills {
            window.paint_quad(fill(span(item.line, item.column, item.cells), hsla(item.color)));
        }
        for run in &frame.runs {
            let width = cell.width * factor(run.cells);
            let label = Label {
                text: &run.text,
                font: &self.fonts[usize::from(run.bold) + 2 * usize::from(run.italic)],
                color: run.color,
                at: point(x(run.column), y(run.line)),
                placement: if run.grid { Placement::Grid(cell.width) } else { Placement::Centered(width) },
            };
            self.text(&label, window, cx);
        }
        for drawn in &frame.drawn {
            paint_drawn(drawn.c, drawn.color, span(drawn.line, drawn.column, 1), scale, window);
        }
        let thickness = snap(px(1.), scale).max(px(1.) / scale);
        for stroke in &frame.strokes {
            let at = match stroke.kind {
                StrokeKind::Underline => UNDERLINE_AT,
                StrokeKind::Strike => STRIKE_AT,
            };
            let top = snap(y(stroke.line) + cell.height * at, scale);
            let bounds = Bounds::from_corners(
                point(x(stroke.column), top),
                point(x(stroke.column + stroke.cells), top + thickness),
            );
            window.paint_quad(fill(bounds, hsla(stroke.color)));
        }
        if let Some(cursor) = &frame.cursor {
            self.cursor(cursor, span(cursor.line, cursor.column, cursor.cells), scale, window, cx);
        }
        if !composing.is_empty() {
            // The text being composed, over the screen at the cursor, underlined.
            let (line, column) = frame.cursor.as_ref().map_or((0, 0), |cursor| (cursor.line, cursor.column));
            let at = point(x(column), y(line));
            let shaped = self.shape(composing, &self.fonts[0], self.theme.foreground, None, window);
            let bounds = Bounds::new(at, size(shaped.width, cell.height));
            window.paint_quad(fill(bounds, hsla(self.theme.background)));
            // Text that cannot be painted is left out of this frame: there is nothing else to do with it.
            let _ = shaped.paint(at, cell.height, window, cx);
            let under = Bounds::new(at + point(px(0.), cell.height - thickness), size(shaped.width, thickness));
            window.paint_quad(fill(under, hsla(self.theme.foreground)));
        }
    }

    /// `text` as glyphs. `cell_width`: put each glyph at its own cell.
    fn shape(
        &self,
        text: &str,
        font: &Font,
        color: Rgb,
        cell_width: Option<Pixels>,
        window: &mut Window,
    ) -> ShapedLine {
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color: hsla(color),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window.text_system().shape_line(SharedString::from(text.to_owned()), self.font_size, &[run], cell_width)
    }

    /// Paints text at a cell's corner.
    fn text(&self, label: &Label<'_>, window: &mut Window, cx: &mut App) {
        let (cell_width, inset) = match label.placement {
            Placement::Grid(cell_width) => (Some(cell_width), None),
            Placement::Centered(width) => (None, Some(width)),
        };
        let shaped = self.shape(label.text, label.font, label.color, cell_width, window);
        let inset = inset.filter(|width| shaped.width < *width).map_or(px(0.), |width| (width - shaped.width) / 2.);
        // Text that cannot be painted is left out of this frame: there is nothing else to do with it.
        let _ = shaped.paint(label.at + point(inset, px(0.)), self.prepared.cell.height, window, cx);
    }

    fn cursor(&self, cursor: &CursorMark, bounds: Bounds<Pixels>, scale: f32, window: &mut Window, cx: &mut App) {
        let color = hsla(cursor.color);
        let strip = |origin: Point<Pixels>, size: Size<Pixels>| fill(Bounds::new(origin, size), color);
        let (w, h) = (bounds.size.width, bounds.size.height);
        let thick = snap(px(CURSOR_THICKNESS), scale);
        match cursor.shape {
            CursorShape::Block => {
                window.paint_quad(fill(bounds, color));
                let Some(over) = &cursor.over else { return };
                if let Some(drawn) = over.chars().next().filter(|c| boxdraw::draws(*c)) {
                    paint_drawn(drawn, cursor.over_color, bounds, scale, window);
                } else {
                    let font = &self.fonts[0];
                    let placement = Placement::Centered(w);
                    self.text(
                        &Label { text: over, font, color: cursor.over_color, at: bounds.origin, placement },
                        window,
                        cx,
                    );
                }
            }
            CursorShape::Beam => window.paint_quad(strip(bounds.origin, size(thick, h))),
            CursorShape::Underline => {
                window.paint_quad(strip(bounds.origin + point(px(0.), h - thick), size(w, thick)));
            }
            CursorShape::Hollow => {
                let border = snap(px(HOLLOW_BORDER), scale).max(px(1.) / scale);
                window.paint_quad(strip(bounds.origin, size(w, border)));
                window.paint_quad(strip(bounds.origin + point(px(0.), h - border), size(w, border)));
                window.paint_quad(strip(bounds.origin, size(border, h)));
                window.paint_quad(strip(bounds.origin + point(w - border, px(0.)), size(border, h)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announcement_cannot_drive_the_screen() {
        assert_eq!(announcement("line one\nline two"), "\x1b[33mline one\r\nline two\x1b[0m\r\n");
        // An escape sequence, a bell and a carriage return quoted from a file are dropped.
        assert_eq!(announcement("bad \x1b[2Jkey\x07 here\r"), "\x1b[33mbad [2Jkey here\x1b[0m\r\n");
    }

    #[test]
    fn counts_come_from_ratios_without_surprises() {
        assert_eq!(whole(115.9), 115, "a part of a cell is no cell");
        assert_eq!(whole(-3.0), 0, "no negative count");
        assert_eq!(whole(f32::NAN), 0, "not a number is no cell");
        assert_eq!(whole(f32::INFINITY), usize::MAX, "cut to the most there is");
        assert!((factor(30) - 30.0).abs() < f32::EPSILON, "{}", factor(30));
        assert!((factor(1_000_000) - 65_535.0).abs() < f32::EPSILON, "past any screen: cut");
    }
}
