//! One tab: a session and the view that paints it and feeds it input.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, Font,
    FontFallbacks, FontFeatures, FontStyle, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rgba, ScrollDelta, ScrollWheelEvent, SharedString, Size, TextRun, UTF16Selection,
    Window, canvas, div, fill, point, prelude::*, px, rgb, size,
};
use smol::channel::Receiver;

use crate::config::Config;
use crate::frame::{self, CursorMark, Frame, StrokeKind};
use crate::keys::{self, Mods, MouseAction};
use crate::session::{CursorShape, Notice, PtyEvent, SelectBy, Session};
use crate::theme::{Rgb, Theme};
use crate::{Copy, Paste, boxdraw};

/// The settings every tab shares.
pub struct Settings {
    pub config: Config,
    pub theme: Theme,
}

/// What a tab tells the window that holds it.
pub enum TerminalEvent {
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

pub struct TerminalView {
    session: Session,
    settings: Arc<Settings>,
    pub focus: FocusHandle,
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

impl TerminalView {
    /// Starts `command` in a session of its own.
    pub fn start(command: portable_pty::CommandBuilder, settings: &Settings) -> anyhow::Result<(Session, Receiver<PtyEvent>)> {
        Session::spawn(command, START_COLUMNS, START_LINES, settings.config.terminal.scrollback, settings.theme.clone())
    }

    pub fn new(session: Session, events: Receiver<PtyEvent>, settings: Arc<Settings>, cx: &mut Context<Self>) -> Self {
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

    /// The tab's title: what the program set, else `fallback`.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref().filter(|title| !title.trim().is_empty())
    }

    /// Puts a line of uni-term's own on the screen (a problem with the settings file).
    pub fn announce(&mut self, text: &str) {
        self.session.feed(announcement(text).as_bytes());
    }

    fn output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
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
    fn send(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Err(error) = self.session.write(bytes) {
            eprintln!("uni-term: {error:#}");
        }
        self.session.scroll_to_bottom();
        cx.notify();
    }

    fn font(&self, bold: bool, italic: bool) -> Font {
        let font = &self.settings.config.font;
        Font {
            family: font.family.clone().into(),
            // Said outright, never left to the default: GPUI's Windows
            // text layout applies an empty feature list as "no features",
            // which turns ligatures off.
            features: FontFeatures(Arc::new(vec![("calt".into(), u32::from(font.ligatures))])),
            fallbacks: (!font.fallbacks.is_empty()).then(|| FontFallbacks::from_fonts(font.fallbacks.clone())),
            weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        }
    }

    fn font_size(&self) -> Pixels {
        px(self.settings.config.font.size)
    }

    /// Fits the screen to `bounds` and lays out what is on it.
    fn prepare(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Prepared {
        let font = &self.settings.config.font;
        let text = window.text_system();
        let advance = text.em_advance(text.resolve_font(&self.font(false, false)), self.font_size());
        let cell = size(advance.unwrap_or(px(font.size * FALLBACK_ADVANCE)), px((font.size * font.line_height).round()));
        let padding = px(self.settings.config.terminal.padding);
        let origin = bounds.origin + point(padding, padding);
        let room = size(bounds.size.width - padding * 2., bounds.size.height - padding * 2.);
        let columns = (room.width / cell.width) as u16;
        let lines = (room.height / cell.height) as u16;
        // A window with no room for a screen (minimized, or dragged that
        // small) keeps the size it had: resizing would make the program
        // rewrap everything for a size nobody reads.
        if columns >= MIN_COLUMNS
            && lines >= MIN_LINES
            && (columns, lines) != self.session.size()
            && let Err(error) = self.session.resize(columns, lines)
        {
            eprintln!("uni-term: {error:#}");
        }
        let focused = self.focus.is_focused(window) && window.is_window_active();
        if let Err(error) = self.session.focus(focused) {
            eprintln!("uni-term: {error:#}");
        }
        let frame = frame::layout(&self.session.snapshot(), &self.settings.theme, focused);
        (self.origin, self.cell) = (origin, cell);
        if let Some(cursor) = &frame.cursor {
            self.cursor_cell = (cursor.line, cursor.column);
        }
        Prepared { frame, origin, cell, composing: self.composing.clone() }
    }

    /// The cell under a point of the window, as (line, column), and whether
    /// the point is in the cell's right half. Points outside count as the
    /// nearest cell.
    fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, bool) {
        let (columns, lines) = self.session.size();
        let x = ((position.x - self.origin.x) / self.cell.width).clamp(0.0, f32::from(columns) - 0.01);
        let y = ((position.y - self.origin.y) / self.cell.height).clamp(0.0, f32::from(lines) - 0.01);
        (y as usize, x as usize, x.fract() >= 0.5)
    }

    /// Reports a mouse event to a program that asked for the mouse. `false`
    /// when the program did not ask, or Shift is held to select text anyway.
    fn report_mouse(&mut self, button: keys::MouseButton, action: MouseAction, position: Point<Pixels>, mods: gpui::Modifiers) -> bool {
        let modes = self.session.modes();
        if !modes.mouse || mods.shift {
            return false;
        }
        let (line, column, _) = self.cell_at(position);
        if action == MouseAction::Move {
            let wanted = modes.mouse_motion || (modes.mouse_drag && button != keys::MouseButton::None);
            if !wanted || self.reported == Some((line, column)) {
                return true;
            }
        }
        self.reported = Some((line, column));
        let mods = Mods { shift: mods.shift, control: mods.control, alt: mods.alt };
        if let Some(bytes) = keys::mouse(button, action, column, line, mods, modes.sgr_mouse)
            && let Err(error) = self.session.write(&bytes)
        {
            eprintln!("uni-term: {error:#}");
        }
        true
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
        // No bytes: a key that types text, which comes through `replace_text_in_range`.
        if let Some(bytes) = keys::encode(&key.key, key.key_char.as_deref(), mods, self.session.modes().app_cursor) {
            self.session.clear_selection();
            self.send(&bytes, cx);
            cx.stop_propagation();
        }
    }

    /// Puts the selected text on the clipboard; `false` when nothing is selected.
    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.session.selection() else { return false };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        self.paste_clipboard(cx);
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        if let Err(error) = self.session.paste(&text) {
            eprintln!("uni-term: {error:#}");
        }
        self.session.clear_selection();
        self.session.scroll_to_bottom();
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let button = match event.button {
            MouseButton::Left => keys::MouseButton::Left,
            MouseButton::Middle => keys::MouseButton::Middle,
            MouseButton::Right => keys::MouseButton::Right,
            MouseButton::Navigate(_) => return,
        };
        self.reported = None;
        if self.report_mouse(button, MouseAction::Press, event.position, event.modifiers) {
            return;
        }
        match button {
            keys::MouseButton::Left => {
                let (line, column, right_half) = self.cell_at(event.position);
                let by = match event.click_count {
                    0 | 1 => SelectBy::Cell,
                    2 => SelectBy::Word,
                    _ => SelectBy::Line,
                };
                self.session.select(line, column, right_half, by);
                self.selecting = true;
            }
            // The right button copies what is selected, else pastes, as the Windows console does.
            keys::MouseButton::Right if self.copy_selection(cx) => self.session.clear_selection(),
            _ => self.paste_clipboard(cx),
        }
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            if event.pressed_button != Some(MouseButton::Left) {
                self.selecting = false; // Released outside the window.
                return;
            }
            let (line, column, right_half) = self.cell_at(event.position);
            self.session.select_to(line, column, right_half);
            cx.notify();
            return;
        }
        let button = match event.pressed_button {
            Some(MouseButton::Left) => keys::MouseButton::Left,
            Some(MouseButton::Middle) => keys::MouseButton::Middle,
            Some(MouseButton::Right) => keys::MouseButton::Right,
            _ => keys::MouseButton::None,
        };
        self.report_mouse(button, MouseAction::Move, event.position, event.modifiers);
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            // A click with no drag selects nothing.
            if self.session.selection().is_none() {
                self.session.clear_selection();
            }
            cx.notify();
            return;
        }
        let button = match event.button {
            MouseButton::Left => keys::MouseButton::Left,
            MouseButton::Middle => keys::MouseButton::Middle,
            MouseButton::Right => keys::MouseButton::Right,
            MouseButton::Navigate(_) => return,
        };
        self.reported = None;
        self.report_mouse(button, MouseAction::Release, event.position, event.modifiers);
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.wheel += match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y / self.cell.height,
        };
        let lines = self.wheel.trunc();
        self.wheel -= lines;
        let (up, count) = (lines > 0.0, lines.abs() as usize);
        if count == 0 {
            return;
        }
        let modes = self.session.modes();
        if modes.mouse && !event.modifiers.shift {
            let button = if up { keys::MouseButton::WheelUp } else { keys::MouseButton::WheelDown };
            for _ in 0..count.div_ceil(LINES_PER_NOTCH) {
                self.reported = None;
                self.report_mouse(button, MouseAction::Press, event.position, event.modifiers);
            }
        } else if modes.alt_screen && modes.alternate_scroll {
            // A full-screen program with no mouse (less, man): the wheel is the arrow keys.
            let arrow = keys::encode(if up { "up" } else { "down" }, None, Mods::default(), modes.app_cursor).unwrap_or_default();
            if let Err(error) = self.session.write(&arrow.repeat(count)) {
                eprintln!("uni-term: {error:#}");
            }
        } else {
            self.session.scroll(if up { count as i32 } else { -(count as i32) });
            cx.notify();
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        // An empty selection: the terminal takes text, with nothing to replace.
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composing.is_empty()).then(|| 0..self.composing.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composing.clear();
        cx.notify();
    }

    /// Finished text: a key's character, or what the input method composed.
    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
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
        cx: &mut Context<Self>,
    ) {
        self.composing = new_text.to_owned();
        cx.notify();
    }

    /// Where the input method puts its window: at the cursor's cell.
    fn bounds_for_range(&mut self, _: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let (line, column) = self.cursor_cell;
        let origin = self.origin + point(self.cell.width * column as f32, self.cell.height * line as f32);
        Some(Bounds::new(origin, self.cell))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (view, painted) = (cx.entity(), cx.entity());
        let (settings, focus) = (self.settings.clone(), self.focus.clone());
        let fonts = [self.font(false, false), self.font(true, false), self.font(false, true), self.font(true, true)];
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
                        Painter { prepared: &prepared, theme: &settings.theme, fonts: &fonts, font_size }.paint(window, cx);
                    },
                )
                .size_full(),
            )
    }
}

/// Turns a frame's cells into pixels.
struct Painter<'a> {
    prepared: &'a Prepared,
    theme: &'a Theme,
    /// Regular, bold, italic, bold italic.
    fonts: &'a [Font; 4],
    font_size: Pixels,
}

impl Painter<'_> {
    /// A length rounded to whole device pixels, so neighbors share an edge.
    fn snap(value: Pixels, scale: f32) -> Pixels {
        px((value / px(1.) * scale).round() / scale)
    }

    fn paint(&self, window: &mut Window, cx: &mut App) {
        let Prepared { frame, origin, cell, composing } = self.prepared;
        let scale = window.scale_factor();
        let x = |column: usize| Self::snap(origin.x + cell.width * column as f32, scale);
        let y = |line: usize| Self::snap(origin.y + cell.height * line as f32, scale);
        let span = |line: usize, column: usize, cells: usize| Bounds::from_corners(point(x(column), y(line)), point(x(column + cells), y(line + 1)));

        for item in &frame.fills {
            window.paint_quad(fill(span(item.line, item.column, item.cells), hsla(item.color)));
        }
        for text in &frame.texts {
            let font = &self.fonts[usize::from(text.bold) + 2 * usize::from(text.italic)];
            let width = cell.width * text.cells as f32;
            self.text(&text.text, font, text.color, point(x(text.column), y(text.line)), text.grid.then_some(cell.width), width, window, cx);
        }
        for drawn in &frame.drawn {
            self.drawn(drawn.c, drawn.color, span(drawn.line, drawn.column, 1), scale, window);
        }
        let thickness = Self::snap(px(1.), scale).max(px(1.) / scale);
        for stroke in &frame.strokes {
            let at = match stroke.kind {
                StrokeKind::Underline => UNDERLINE_AT,
                StrokeKind::Strike => STRIKE_AT,
            };
            let top = Self::snap(y(stroke.line) + cell.height * at, scale);
            let bounds = Bounds::from_corners(point(x(stroke.column), top), point(x(stroke.column + stroke.cells), top + thickness));
            window.paint_quad(fill(bounds, hsla(stroke.color)));
        }
        if let Some(cursor) = &frame.cursor {
            self.cursor(cursor, span(cursor.line, cursor.column, cursor.cells), scale, window, cx);
        }
        if !composing.is_empty() {
            // The text being composed, over the screen at the cursor, underlined.
            let (line, column) = frame.cursor.as_ref().map_or((0, 0), |cursor| (cursor.line, cursor.column));
            let at = point(x(column), y(line));
            let Some(shaped) = self.shape(composing, &self.fonts[0], self.theme.foreground, None, window) else { return };
            let bounds = Bounds::new(at, size(shaped.width, cell.height));
            window.paint_quad(fill(bounds, hsla(self.theme.background)));
            let _ = shaped.paint(at, cell.height, window, cx);
            let under = Bounds::new(at + point(px(0.), cell.height - thickness), size(shaped.width, thickness));
            window.paint_quad(fill(under, hsla(self.theme.foreground)));
        }
    }

    fn shape(&self, text: &str, font: &Font, color: Rgb, cell_width: Option<Pixels>, window: &mut Window) -> Option<gpui::ShapedLine> {
        let run = TextRun { len: text.len(), font: font.clone(), color: hsla(color), background_color: None, underline: None, strikethrough: None };
        Some(window.text_system().shape_line(SharedString::from(text.to_owned()), self.font_size, &[run], cell_width))
    }

    /// Paints text at a cell's corner. `cell_width`: put each glyph at its
    /// own cell. Without it the text is centered in `width`.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, text: &str, font: &Font, color: Rgb, at: Point<Pixels>, cell_width: Option<Pixels>, width: Pixels, window: &mut Window, cx: &mut App) {
        let Some(shaped) = self.shape(text, font, color, cell_width, window) else { return };
        let inset = if cell_width.is_none() && shaped.width < width { (width - shaped.width) / 2. } else { px(0.) };
        let _ = shaped.paint(at + point(inset, px(0.)), self.prepared.cell.height, window, cx);
    }

    /// Paints a box-drawing or block character into a cell's bounds.
    fn drawn(&self, c: char, color: Rgb, bounds: Bounds<Pixels>, scale: f32, window: &mut Window) {
        let device = |value: Pixels| (value / px(1.) * scale).round();
        let Some((rects, opacity)) = boxdraw::rects(c, device(bounds.size.width), device(bounds.size.height)) else { return };
        let color = Rgba { a: opacity, ..rgb(color.hex()) };
        for [left, top, right, bottom] in rects {
            let corner = |dx: f32, dy: f32| bounds.origin + point(px(dx / scale), px(dy / scale));
            window.paint_quad(fill(Bounds::from_corners(corner(left, top), corner(right, bottom)), color));
        }
    }

    fn cursor(&self, cursor: &CursorMark, bounds: Bounds<Pixels>, scale: f32, window: &mut Window, cx: &mut App) {
        let color = hsla(cursor.color);
        let strip = |origin: Point<Pixels>, size: Size<Pixels>| fill(Bounds::new(origin, size), color);
        let (w, h) = (bounds.size.width, bounds.size.height);
        let thick = Self::snap(px(CURSOR_THICKNESS), scale);
        match cursor.shape {
            CursorShape::Block => {
                window.paint_quad(fill(bounds, color));
                if let Some(over) = &cursor.over {
                    if over.chars().next().is_some_and(boxdraw::draws) {
                        self.drawn(over.chars().next().unwrap_or(' '), cursor.over_color, bounds, scale, window);
                    } else {
                        self.text(over, &self.fonts[0], cursor.over_color, bounds.origin, None, w, window, cx);
                    }
                }
            }
            CursorShape::Beam => window.paint_quad(strip(bounds.origin, size(thick, h))),
            CursorShape::Underline => window.paint_quad(strip(bounds.origin + point(px(0.), h - thick), size(w, thick))),
            CursorShape::Hollow => {
                let border = Self::snap(px(HOLLOW_BORDER), scale).max(px(1.) / scale);
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
    use super::announcement;

    #[test]
    fn an_announcement_cannot_drive_the_screen() {
        assert_eq!(announcement("line one\nline two"), "\x1b[33mline one\r\nline two\x1b[0m\r\n");
        // An escape sequence, a bell and a carriage return quoted from a file are dropped.
        assert_eq!(announcement("bad \x1b[2Jkey\x07 here\r"), "\x1b[33mbad [2Jkey here\x1b[0m\r\n");
    }
}