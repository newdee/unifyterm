//! One tab: a session and the view that paints it and feeds it input.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, Corners, ElementInputHandler, EntityInputHandler, EventEmitter,
    FocusHandle, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, InputHandler, KeyDownEvent, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage, Rgba, ScrollDelta,
    ScrollWheelEvent, ShapedLine, SharedString, Size, TextRun, UTF16Selection, Window, canvas, div, fill, point,
    prelude::*, px, rgb, size,
};
use smol::channel::Receiver;

use crate::candidates::{self, Candidates, Outcome};
use crate::config::Config;
use crate::frame::{self, CursorMark, Frame, StrokeKind};
use crate::helper::{Consent, Feature, Helper, WHAT_IS_SENT};
use crate::keys::{self, Mods, MouseAction};
use crate::recorder::Recorder;
use crate::session::{CursorShape, Focus, Half, MouseTracking, Notice, PtyEvent, SelectBy, Session};
use crate::theme::{Rgb, Theme};
use crate::{Copy, Explain, Paste, boxdraw, powerline, shine};
use std::time::{Duration, Instant};

/// What is behind a tab's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backdrop {
    /// The theme's background color: the tab draws it itself.
    Color,
    /// The window's wallpaper, with the background color under it: the
    /// window draws both, and the tab draws nothing behind its text.
    Wallpaper,
}

/// The settings every tab shares.
#[derive(Debug)]
pub(crate) struct Settings {
    pub(crate) config: Config,
    pub(crate) theme: Theme,
    pub(crate) backdrop: Backdrop,
    /// The prompt's look, put together from the settings and the theme: the window reads its light.
    pub(crate) prompt: crate::config::Prompt,
}

/// What a tab tells the window that holds it.
#[derive(Debug)]
pub(crate) enum TerminalEvent {
    /// The title changed.
    Title,
    /// The program ended.
    Exited,
    /// A command the shell ran has ended.
    Finished(crate::prompt::Finished),
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
/// The list of commands and the boxes, which lie over the text: how far
/// their text is in from their sides, and how far the mark of the row in use
/// is from that row's top and bottom. As the preview page draws them
/// (demo/appearance.html).
const OVER_TEXT_INSET: f32 = 12.0;
const CHOSEN_MARK_INSET: f32 = 4.0;

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
    /// Where the view itself is in the window: what is drawn over the text is placed from here.
    view_origin: Point<Pixels>,
    cell: Size<Pixels>,
    /// The cursor's cell (line, column), for placing the input method's window.
    cursor_cell: (usize, usize),
    /// A drag with the left button is selecting text.
    selecting: bool,
    /// The part of a line the wheel has moved that is not yet scrolled.
    wheel: f32,
    /// The cell last reported to a program that tracks the mouse.
    reported: Option<(usize, usize)>,
    /// Where the candidate list asks; `None` with the history off.
    recorder: Option<Recorder>,
    /// The commands from the history offered under the input line.
    candidates: Candidates,
    /// The images' textures, by number.
    textures: std::collections::HashMap<u32, Texture>,
    /// Textures replaced or no longer used, to free.
    stale: Vec<Arc<RenderImage>>,
    /// Help with commands: fixes, suggestions, explanations.
    helper: Arc<Helper>,
    /// The box over the text, if one is open.
    popup: Option<Popup>,
    /// Counts the popups opened: an answer meant for one that is gone is dropped.
    popup_number: u64,
    /// The model's suggestion for the input line, shown in grey after the cursor.
    suggestion: Option<String>,
    /// The input last asked about: one question per pause in typing.
    suggested_for: Option<String>,
    /// This tab's last commands, newest first.
    recent: Vec<String>,
    /// The light over the newest prompt (`prompt.shine`), when it is on.
    sweep: Option<shine::Sweep>,
    /// A wake-up is asked for, for the next pass of a light that comes again.
    sweep_waiting: bool,
}

/// What may be sent once the user agrees.
#[derive(Debug, Clone)]
enum Pending {
    Suggest(String),
    Fix {
        command: String,
        exit: Option<i32>,
        output: String,
    },
    Explain(String),
    /// A sentence to make a command of: the words after `#`, and the input line as it was.
    Ask {
        words: String,
        line: String,
    },
}

/// A box over the text, under the input line.
#[derive(Debug, Clone)]
enum Popup {
    /// "May this be sent?": Enter yes, Esc no.
    Consent(Pending),
    /// A corrected command: Enter puts it on the input line, not run.
    Fix(unifyterm_assist::Fix),
    /// The model is asked what a command means.
    Waiting(String),
    /// A command, what each part of it means, and where that is from when not from the model.
    Explained(String, Vec<unifyterm_assist::Part>, Option<String>),
    /// The command made of a sentence, and what became of it (one of the lines below).
    Asked(unifyterm_assist::Asked, &'static str, Option<String>),
    /// A short line: why there is no answer.
    Note(String),
}

/// What became of a command made of a sentence.
const ON_THE_LINE: &str = "Enter: run it    any other key: change it first";
const NOT_RUN: &str = "It may change something, so it was not run.    Enter: run it";
const NOT_PUT: &str = "The line changed meanwhile: the command was not put on it.";
const DOUBTED: &str = "The program's own list does not have that, so it was not run.    Enter: run it";
/// What a line begins with when it is a sentence for the model, not a command.
const SENTENCE: char = '#';

/// How long typing must pause before the model is asked for a suggestion.
const SUGGEST_PAUSE: std::time::Duration = std::time::Duration::from_millis(300);
/// A light that sets out within this is waited for frame by frame, not with a wake-up.
const LIGHT_SOON: Duration = Duration::from_millis(250);
/// How many of a tab's last commands are kept for the model's context.
const RECENT: usize = 5;

impl EventEmitter<TerminalEvent> for TerminalView {}

/// What `prepaint` hands `paint`.
struct Prepared {
    frame: Frame,
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    composing: String,
    /// The images to draw, by number, with the cells their placement takes (columns, rows).
    pictures: Vec<(frame::ImageArea, Arc<RenderImage>, (usize, usize))>,
    /// Textures no longer wanted: freed on the next paint.
    stale: Vec<Arc<RenderImage>>,
}

/// A texture: the image's generation and the size (device pixels) it was made from and for.
type Texture = (u64, (u32, u32), Arc<RenderImage>);

/// An image as the window's renderer takes it (blue, green, red, alpha), no
/// larger than `fit` = (width, height) device pixels: the size it is shown
/// at. A picture of 2048 × 2048 shown in four columns needs no texture of 16 MiB.
fn texture(image: &crate::graphics::Image, fit: (u32, u32)) -> Option<Arc<RenderImage>> {
    let full = image::RgbaImage::from_raw(image.width, image.height, image.rgba.to_vec())?;
    let (width, height) = (fit.0.clamp(1, image.width), fit.1.clamp(1, image.height));
    let mut shown = if (width, height) == (image.width, image.height) {
        full
    } else {
        image::imageops::resize(&full, width, height, image::imageops::FilterType::Triangle)
    };
    for pixel in shown.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(shown)])))
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
        recorder: Option<Recorder>,
        helper: Arc<Helper>,
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
            view_origin: Point::default(),
            cell: size(px(1.), px(1.)),
            cursor_cell: (0, 0),
            selecting: false,
            wheel: 0.0,
            reported: None,
            recorder,
            candidates: Candidates::default(),
            textures: std::collections::HashMap::new(),
            stale: Vec::new(),
            helper,
            popup: None,
            popup_number: 0,
            suggestion: None,
            suggested_for: None,
            recent: Vec::new(),
            sweep: None,
            sweep_waiting: false,
        }
    }

    /// Follows the newest prompt with the light the settings ask for: a new
    /// light for a new prompt, none when the setting is off.
    fn follow_prompt(&mut self, now: Instant) {
        let prompt = &self.settings.prompt;
        let newest = self.session.newest_prompt();
        if prompt.shine == unifyterm_config::prompt::Shine::Off || newest.is_none() {
            self.sweep = None;
            return;
        }
        if newest != self.sweep.map(|sweep| sweep.span)
            && let Some(span) = newest
        {
            self.sweep = shine::Sweep::new(span, prompt.shine, prompt.shine_every, now);
        }
    }

    /// Asks to be woken for the next pass of a light that comes again, while
    /// none is under way: no frame is drawn in between.
    fn wake_for_light(&mut self, cx: &mut Context<'_, Self>) {
        let now = Instant::now();
        let Some(sweep) = self.sweep else { return };
        if self.sweep_waiting || sweep.progress(now).is_some() {
            return;
        }
        let Some(wait) = sweep.until_next(now).filter(|wait| !wait.is_zero()) else { return };
        self.sweep_waiting = true;
        cx.spawn(async move |this, cx| {
            smol::Timer::after(wait).await;
            let _ = this.update(cx, |view, cx| {
                view.sweep_waiting = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// The title the program set; `None` when it set none or an empty one.
    pub(crate) fn title(&self) -> Option<&str> {
        self.title.as_deref().filter(|title| !title.trim().is_empty())
    }

    /// Takes settings that were read again: the font, the colors and the
    /// padding are drawn from them from the next frame on.
    pub(crate) fn use_settings(&mut self, settings: Arc<Settings>, cx: &mut Context<'_, Self>) {
        self.settings = settings;
        cx.notify();
    }

    /// Puts a line of UnifyTerm's own on the screen (a problem with the settings file).
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
                Notice::Finished(finished) => {
                    self.recent.insert(0, finished.text.clone());
                    self.recent.truncate(RECENT);
                    // Failed, or a shell that does not say (cmd.exe): the output tells whether it was not found.
                    if finished.exit.is_none_or(|exit| exit != 0) {
                        self.offer_fix(&finished, cx);
                    }
                    cx.emit(TerminalEvent::Finished(finished));
                }
            }
        }
        self.ask(cx);
        self.suggest(cx);
        cx.notify();
    }

    /// Opens a popup; an answer for the one before is no longer wanted.
    fn open(&mut self, popup: Popup) -> u64 {
        self.popup = Some(popup);
        self.popup_number += 1;
        self.popup_number
    }

    /// For a command that failed: the local fix for a name that was not found, else the
    /// fix from its program's own command list, else the model's.
    fn offer_fix(&mut self, finished: &crate::prompt::Finished, cx: &mut Context<'_, Self>) {
        let local = self
            .helper
            .local_fix(&finished.text, &finished.output, finished.exit, &self.recent[1..])
            .or_else(|| self.helper.list_fix(&finished.text, finished.exit));
        if let Some(fix) = local {
            self.open(Popup::Fix(fix));
            return;
        }
        if !self.helper.wants_fix(&finished.output, finished.exit) {
            return;
        }
        let pending =
            Pending::Fix { command: finished.text.clone(), exit: finished.exit, output: finished.output.clone() };
        self.send_to_model(pending, cx);
    }

    /// Asks the model, or first the user whether that may be done.
    fn send_to_model(&mut self, pending: Pending, cx: &mut Context<'_, Self>) {
        match self.helper.consent() {
            Consent::Refused => {}
            Consent::Unasked => {
                self.open(Popup::Consent(pending));
            }
            Consent::Given => self.run(pending, cx),
        }
        cx.notify();
    }

    /// Sends a question the user has agreed to; the answer opens a popup (or shows the suggestion).
    fn run(&mut self, pending: Pending, cx: &mut Context<'_, Self>) {
        let shell = self.session.shell().unwrap_or("a shell").to_owned();
        match pending {
            Pending::Suggest(typed) => {
                let cwd = self.session.cwd().unwrap_or_default().to_owned();
                let Some(answer) = self.helper.suggest(typed, cwd, shell, self.recent.clone()) else { return };
                cx.spawn(async move |this, cx| {
                    let answered = answer.recv().await;
                    if let Ok(Err(error)) = &answered {
                        log::warn!("suggestion: {error}");
                    }
                    if let Ok(Ok(Some(completion))) = answered {
                        let _ = this.update(cx, |view, cx| {
                            // Only while the line still begins as it did.
                            if view.session.input().is_some_and(|input| completion.starts_with(&input.text)) {
                                view.suggestion = Some(completion);
                                cx.notify();
                            }
                        });
                    }
                })
                .detach();
            }
            Pending::Fix { command, exit, output } => {
                let Some(answer) = self.helper.fix(command, exit, output, shell) else { return };
                let number = self.popup_number;
                cx.spawn(async move |this, cx| {
                    let answered = answer.recv().await;
                    if let Ok(Err(error)) = &answered {
                        log::warn!("fix: {error}");
                    }
                    if let Ok(Ok(Some(fix))) = answered {
                        let _ = this.update(cx, |view, cx| {
                            // Not over a popup the user opened since.
                            if view.popup.is_none() || view.popup_number == number {
                                view.open(Popup::Fix(fix));
                                cx.notify();
                            }
                        });
                    }
                })
                .detach();
            }
            Pending::Ask { words, line } => {
                let number = self.open(Popup::Waiting(format!("{SENTENCE} {words}")));
                let cwd = self.session.cwd().unwrap_or_default().to_owned();
                let Some(answer) = self.helper.command_for(words, cwd, shell, self.recent.clone()) else {
                    self.open(Popup::Note("The model is not in use.".into()));
                    return;
                };
                cx.spawn(async move |this, cx| {
                    let answered = answer.recv().await;
                    let _ = this.update(cx, |view, cx| {
                        if view.popup_number != number {
                            return; // Closed meanwhile: the answer is not wanted.
                        }
                        match answered {
                            Ok(Ok(Some(asked))) => view.asked(asked, &line, cx),
                            Ok(Ok(None)) => view.popup = Some(Popup::Note("The model has no command for that.".into())),
                            Ok(Err(error)) => view.popup = Some(Popup::Note(error.to_string())),
                            Err(_) => view.popup = Some(Popup::Note("No answer.".into())),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Pending::Explain(command) => {
                let number = self.open(Popup::Waiting(command.clone()));
                let Some(answer) = self.helper.explain(command.clone(), shell) else {
                    self.open(Popup::Note("The model is not in use.".into()));
                    return;
                };
                cx.spawn(async move |this, cx| {
                    let answered = answer.recv().await;
                    let _ = this.update(cx, |view, cx| {
                        if view.popup_number != number {
                            return; // Closed meanwhile.
                        }
                        view.popup = Some(match answered {
                            Ok(Ok(parts)) => Popup::Explained(command, parts, None),
                            Ok(Err(error)) => Popup::Note(error.to_string()),
                            Err(_) => Popup::Note("No answer.".into()),
                        });
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    /// After a pause in typing, asks the model for the rest of the line.
    fn suggest(&mut self, cx: &mut Context<'_, Self>) {
        let input = self.session.input().map(|input| input.text).filter(|text| !text.trim().is_empty());
        if self
            .suggestion
            .as_ref()
            .is_some_and(|completion| input.as_ref().is_none_or(|text| !completion.starts_with(text.as_str())))
        {
            self.suggestion = None;
        }
        let Some(typed) = input else { return };
        if !self.helper.offers(Feature::Suggest) || self.helper.consent() == Consent::Refused {
            return;
        }
        // A sentence for the model is not a command to complete.
        if typed.trim_start().starts_with(SENTENCE) && self.helper.offers(Feature::Ask) {
            return;
        }
        if self.suggested_for.as_deref() == Some(typed.as_str()) || self.candidates.visible().is_some() {
            return;
        }
        self.suggested_for = Some(typed.clone());
        cx.spawn(async move |this, cx| {
            smol::Timer::after(SUGGEST_PAUSE).await;
            let _ = this.update(cx, |view, cx| {
                let still = view.session.input().is_some_and(|input| input.text == typed);
                if still && view.popup.is_none() {
                    view.send_to_model(Pending::Suggest(typed), cx);
                }
            });
        })
        .detach();
    }

    /// The key for it (`keys.explain`, Ctrl-Shift-E): what each part of the command on the line (or the last one) means.
    fn explain(&mut self, cx: &mut Context<'_, Self>) {
        let typed = self.session.input().map(|input| input.text).filter(|text| !text.trim().is_empty());
        let Some(command) = typed.or_else(|| self.recent.first().cloned()) else {
            self.open(Popup::Note("Type a command first: Ctrl-Shift-E explains the one on the line.".into()));
            cx.notify();
            return;
        };
        if let Some(told) = self.helper.told(command.trim()) {
            // The program's own list tells it: at once, with no model, and nothing is sent.
            let parts =
                told.parts.into_iter().map(|(text, meaning)| unifyterm_assist::Part { text, meaning }).collect();
            let version = if told.version.is_empty() { String::new() } else { format!(" ({})", told.version) };
            let from = format!("From {}'s own command list{version}. Nothing was sent.", told.program);
            self.open(Popup::Explained(command.trim().to_owned(), parts, Some(from)));
        } else if !self.helper.has_model() {
            self.open(Popup::Note("No model is set: [ai] in the settings names one.".into()));
        } else if !self.helper.offers(Feature::Explain) {
            self.open(Popup::Note("Explanations are off: ai.explain is false in the settings.".into()));
        } else if self.helper.consent() == Consent::Refused {
            self.open(Popup::Note("Sending to the model was declined; it is off until UnifyTerm starts again.".into()));
        } else {
            self.send_to_model(Pending::Explain(command.trim().to_owned()), cx);
        }
        cx.notify();
    }

    /// A key while a popup is open: Enter and Esc are the popup's; any other key closes a
    /// popup that is only shown (not the question), and goes on to the shell. `true`: taken.
    fn popup_key(&mut self, key: &gpui::Keystroke, cx: &mut Context<'_, Self>) -> bool {
        let Some(popup) = self.popup.clone() else { return false };
        let held = key.modifiers;
        let plain = !held.control && !held.shift && !held.alt;
        match (key.key.as_str(), plain) {
            ("enter", true) => {
                // The popup is gone before what it stood for is done: a question sent now opens
                // its own popup, and its answer must not be taken for one meant for this popup.
                self.close_popup();
                match popup {
                    Popup::Consent(pending) => {
                        self.helper.agree();
                        self.run(pending, cx);
                    }
                    Popup::Fix(fix) => self.fill(&fix.command, cx),
                    // The command is on the input line: this Enter is the shell's, and runs it.
                    Popup::Asked(..) => {
                        cx.notify();
                        return false;
                    }
                    Popup::Waiting(_) | Popup::Explained(..) | Popup::Note(_) => {}
                }
            }
            ("escape", true) => {
                self.close_popup();
                if let Popup::Consent(_) = popup {
                    self.helper.refuse();
                }
            }
            _ => {
                if !matches!(popup, Popup::Consent(_)) {
                    self.popup = None;
                }
                cx.notify();
                return false;
            }
        }
        cx.notify();
        true
    }

    /// Closes the popup for good: an answer still on its way to it is no longer wanted.
    fn close_popup(&mut self) {
        self.popup = None;
        self.popup_number += 1;
    }

    /// Asks the history and the command lists about what is on the input line now, if that changed.
    fn ask(&mut self, cx: &mut Context<'_, Self>) {
        let history = &self.settings.config.history;
        if !history.candidates {
            return;
        }
        let input = self.session.input();
        let Some((number, text)) = self.candidates.question(input.as_ref().map(|input| input.text.as_str())) else {
            return;
        };
        // What the line's program says may come next: in memory, no waiting.
        let listed = self.helper.offers_for(&text);
        let Some(recorder) = self.recorder.clone() else {
            // The history is off: the command lists' words alone.
            self.candidates.answer(number, Vec::new(), listed);
            return;
        };
        let answer = recorder.search(&text, self.session.cwd().unwrap_or_default(), history.limit);
        cx.spawn(async move |this, cx| {
            if let Ok(rows) = answer.recv().await {
                // The tab may be closed by now: nobody to show it to.
                let _ = this.update(cx, |view, cx| {
                    view.candidates.answer(number, rows, listed);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Puts a command on the shell's input line in place of what is there,
    /// without running it. The keys that empty the line are the shell's
    /// own; for a shell that did not say what it is, nothing is done.
    fn fill(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        let Some(empty_line) = self.empties_the_line() else { return };
        // A line break would run it.
        if text.contains(['\n', '\r']) {
            return;
        }
        logged(self.session.write(empty_line));
        logged(self.session.paste(text));
        self.session.scroll_to_bottom();
        cx.notify();
    }

    /// The keys that empty this shell's input line; `None` for a shell that did not say what it is.
    fn empties_the_line(&self) -> Option<&'static [u8]> {
        match self.session.shell() {
            // PowerShell's line editor and cmd.exe: Escape empties the line.
            Some("pwsh" | "powershell" | "cmd") => Some(b"\x1b"),
            // Emacs keys: to the end, then everything before.
            Some("bash" | "zsh" | "fish") => Some(b"\x05\x15"),
            _ => None,
        }
    }

    /// The sentence on the input line, when Enter is to hand it to the model and not to the
    /// shell: the words after `#`, and the line as it is. `None` leaves Enter to the shell,
    /// which takes such a line as a comment: with no model set, nothing changes.
    fn sentence(&self) -> Option<(String, String)> {
        if !self.helper.offers(Feature::Ask) || self.helper.consent() == Consent::Refused {
            return None;
        }
        // The command is put on the line in the sentence's place: only where that can be done.
        self.empties_the_line()?;
        let line = self.session.input()?.text;
        let words = line.trim_start().strip_prefix(SENTENCE)?.trim();
        (!words.is_empty()).then(|| (words.to_owned(), line.clone()))
    }

    /// The model's command for a sentence: put on the input line in the sentence's place, if the
    /// line is still as it was, and run when the settings say so and it only reads.
    fn asked(&mut self, asked: unifyterm_assist::Asked, line: &str, cx: &mut Context<'_, Self>) {
        // A command or an option the program's own list does not have: the model may have made it up.
        let doubt = self.helper.doubt(&asked.command);
        if self.session.input().is_none_or(|input| input.text != line) {
            self.popup = Some(Popup::Asked(asked, NOT_PUT, doubt));
            return;
        }
        self.fill(&asked.command, cx);
        if !self.helper.runs_what_only_reads() {
            self.popup = Some(Popup::Asked(asked, ON_THE_LINE, doubt));
        } else if doubt.is_some() {
            self.popup = Some(Popup::Asked(asked, DOUBTED, doubt));
        } else if unifyterm_assist::only_reads(&asked.command, self.session.shell().unwrap_or_default()) {
            // As the Enter key does it: the command starts now, and its text is known.
            self.session.enter_with(&asked.command);
            self.candidates.question(None);
            self.suggestion = None;
            let cursor_keys = self.session.modes().cursor_keys;
            if let Some(enter) = keys::encode("enter", None, Mods::default(), cursor_keys) {
                self.send(&enter, cx);
            }
            // No box: it would lie over what the command prints. The command itself is on the screen.
            self.popup = None;
        } else {
            self.popup = Some(Popup::Asked(asked, NOT_RUN, None));
        }
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
        let snapshot = self.session.snapshot();
        let now = Instant::now();
        self.follow_prompt(now);
        let glow = self.sweep.map_or_else(shine::Glow::default, |sweep| shine::glow(&snapshot, &sweep, now));
        // While the light is on the prompt, or about to set out, every frame is drawn; between
        // passes, none (see `wake_for_light`).
        if self.sweep.is_some_and(|sweep| {
            sweep.progress(now).is_some() || sweep.until_next(now).is_some_and(|wait| wait <= LIGHT_SOON)
        }) {
            window.request_animation_frame();
        }
        let frame = frame::layout_lit(&snapshot, &self.settings.theme, focus, &glow);
        (self.origin, self.view_origin, self.cell) = (origin, bounds.origin, cell);
        if let Some(cursor) = &frame.cursor {
            self.cursor_cell = (cursor.line, cursor.column);
        }
        let scale = window.scale_factor();
        self.session.set_cell_pixels(device_pixels(cell.width, scale), device_pixels(cell.height, scale));
        let pictures = self.pictures(&frame);
        Prepared {
            frame,
            origin,
            cell,
            composing: self.composing.clone(),
            pictures,
            stale: std::mem::take(&mut self.stale),
        }
    }

    /// The images the frame shows, with textures made for the ones new or sent again;
    /// textures of images gone are set aside to free.
    fn pictures(&mut self, frame: &Frame) -> Vec<(frame::ImageArea, Arc<RenderImage>, (usize, usize))> {
        let mut pictures = Vec::new();
        let (cell_width, cell_height) = self.session.cell_pixels();
        for area in &frame.images {
            let Some(image) = self.session.image(area.id) else { continue };
            // The placement's cells (one cell for a picture placed by no command of ours).
            let cells = image.cells.unwrap_or((1, 1));
            let wide = |count: usize, pixels: u32| u32::try_from(count).unwrap_or(u32::MAX).saturating_mul(pixels);
            let fit = (wide(cells.0, cell_width), wide(cells.1, cell_height));
            let current = self
                .textures
                .get(&area.id)
                .filter(|(generation, made_for, _)| *generation == image.generation && *made_for == fit);
            let texture = if let Some((_, _, texture)) = current {
                Arc::clone(texture)
            } else {
                let Some(texture) = texture(image, fit) else { continue };
                if let Some((_, _, old)) = self.textures.insert(area.id, (image.generation, fit, Arc::clone(&texture)))
                {
                    self.stale.push(old);
                }
                texture
            };
            pictures.push((*area, texture, cells));
        }
        // Images the program deleted, or the memory limit removed.
        let gone: Vec<u32> = self.textures.keys().copied().filter(|id| self.session.image(*id).is_none()).collect();
        for id in gone {
            if let Some((_, _, texture)) = self.textures.remove(&id) {
                self.stale.push(texture);
            }
        }
        pictures
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
        let plain = !held.control && !held.shift && !held.alt;
        if self.popup_key(key, cx) {
            cx.stop_propagation();
            return;
        }
        let list_key = match key.key.as_str() {
            "n" if held.control && !held.shift && !held.alt => Some(candidates::Key::Next),
            "p" if held.control && !held.shift && !held.alt => Some(candidates::Key::Previous),
            "down" if plain => Some(candidates::Key::Next),
            "up" if plain => Some(candidates::Key::Previous),
            "tab" | "right" if plain => Some(candidates::Key::Take),
            "enter" if plain => Some(candidates::Key::Enter),
            "escape" if plain => Some(candidates::Key::Close),
            _ => None,
        };
        if let Some(list_key) = list_key {
            match self.candidates.key(list_key) {
                // Right with no row chosen takes the model's suggestion, when there is one.
                Outcome::Passed if list_key == candidates::Key::Take && key.key == "right" => {
                    if let Some(completion) = self.shown_suggestion() {
                        self.suggestion = None;
                        self.fill(&completion, cx);
                        cx.stop_propagation();
                        return;
                    }
                }
                Outcome::Passed => {}
                Outcome::Taken => {
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                Outcome::Fill(text) => {
                    self.fill(&text, cx);
                    cx.stop_propagation();
                    return;
                }
            }
        }
        if key.key == "enter"
            && plain
            && let Some((words, line)) = self.sentence()
        {
            // A sentence, not a command: it goes to the model, and the shell gets no Enter.
            self.candidates.question(None);
            self.suggestion = None;
            self.send_to_model(Pending::Ask { words, line }, cx);
            cx.stop_propagation();
            return;
        }
        if key.key == "enter" && plain {
            // What is on the input line now is the command the shell is about to run.
            self.session.enter();
            self.candidates.question(None);
            self.suggestion = None;
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

/// The view's input, with one difference from GPUI's own: on macOS a key held
/// down types again and again, as in other terminals, and does not open the
/// menu of accented letters (é è ê for `e`; for `k`, nothing at all).
struct TerminalInput(ElementInputHandler<TerminalView>);

impl InputHandler for TerminalInput {
    fn selected_text_range(&mut self, ignore: bool, window: &mut Window, cx: &mut App) -> Option<UTF16Selection> {
        self.0.selected_text_range(ignore, window, cx)
    }

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.0.marked_text_range(window, cx)
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.0.text_for_range(range, adjusted, window, cx)
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        self.0.replace_text_in_range(range, text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0.replace_and_mark_text_in_range(range, text, selected, window, cx);
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.0.unmark_text(window, cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        self.0.bounds_for_range(range, window, cx)
    }

    fn character_index_for_point(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<usize> {
        self.0.character_index_for_point(at, window, cx)
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.wake_for_light(cx);
        let (view, painted) = (cx.entity(), cx.entity());
        let (settings, focus) = (Arc::clone(&self.settings), self.focus.clone());
        let fonts = self.fonts();
        let font_size = self.font_size();
        div()
            .track_focus(&self.focus)
            .size_full()
            // With `theme.opacity` under 1, what is behind the window shows through this color alone.
            // Under a wallpaper the window has drawn the color already, with the picture over it.
            .when(self.settings.backdrop == Backdrop::Color, |view| {
                view.bg(gpui::Hsla { a: self.settings.theme.opacity, ..hsla(self.settings.theme.background) })
            })
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(|view, _: &Explain, _, cx| view.explain(cx)))
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
                        window.handle_input(&focus, TerminalInput(ElementInputHandler::new(bounds, painted)), cx);
                        Painter { prepared: &prepared, theme: &settings.theme, fonts: &fonts, font_size }
                            .paint(window, cx);
                    },
                )
                .size_full(),
            )
            .children(self.candidate_list())
            .children(self.suggestion_text())
            .children(self.popup_box())
    }
}

/// How many cells `text` takes: two for a wide character (CJK), none for a combining one.
fn width_of(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// `text` in lines of at most `room` cells, broken between words where it
/// can be, inside a word only when the word alone is wider.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let wanted = if line.is_empty() { width_of(word) } else { width_of(&line) + 1 + width_of(word) };
        if wanted <= room {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for c in word.chars() {
            if width_of(&line) + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) > room {
                lines.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    lines.push(line);
    lines
}

/// `text` cut into what comes before the part that matches `typed`, that
/// part, and what comes after; case aside. No match: all of it before.
fn split_match<'a>(text: &'a str, typed: &str) -> (&'a str, &'a str, &'a str) {
    let found = text.to_lowercase().find(&typed.to_lowercase()).filter(|_| text.len() == text.to_lowercase().len());
    match found {
        Some(at) if text.is_char_boundary(at) && text.is_char_boundary(at + typed.len()) => {
            (&text[..at], &text[at..at + typed.len()], &text[at + typed.len()..])
        }
        _ => (text, "", ""),
    }
}

impl TerminalView {
    /// The suggestion to show now: when the line still begins as it did,
    /// no list is open and the rest of the line is empty.
    fn shown_suggestion(&self) -> Option<String> {
        let completion = self.suggestion.as_ref()?;
        let input = self.session.input()?;
        let fits = completion.starts_with(&input.text) && completion.len() > input.text.len();
        (fits && self.candidates.visible().is_none() && self.session.blank_right_of_cursor())
            .then(|| completion.clone())
    }

    /// The rest of the suggested line, in grey, from the cursor.
    fn suggestion_text(&self) -> Option<impl IntoElement> {
        let completion = self.shown_suggestion()?;
        let input = self.session.input()?;
        let rest = completion[input.text.len()..].to_owned();
        let theme = &self.settings.theme;
        let at = self.origin - self.view_origin
            + point(self.cell.width * factor(input.cursor.1), self.cell.height * factor(input.cursor.0));
        Some(
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .font_family(SharedString::from(self.settings.config.font.family.clone()))
                .text_size(self.font_size())
                .line_height(self.cell.height)
                .whitespace_nowrap()
                .text_color(hsla(theme.foreground.toward(theme.background, 0.55)))
                .child(SharedString::from(rest)),
        )
    }

    /// The popup's lines: (text, whether it is a heading).
    fn popup_lines(&self) -> Option<Vec<(String, bool)>> {
        let lines = match self.popup.as_ref()? {
            Popup::Consent(_) => vec![
                (format!("Ask the model ({}) for help with commands?", self.helper.model()), true),
                (WHAT_IS_SENT.to_owned(), false),
                ("Enter: yes, from now on    Esc: no, not while UnifyTerm runs".to_owned(), false),
            ],
            Popup::Fix(fix) => {
                let mut lines = vec![(fix.command.clone(), true)];
                if !fix.reason.is_empty() {
                    lines.push((fix.reason.clone(), false));
                }
                lines.push(("Enter: put it on the line (not run)    Esc: close".to_owned(), false));
                lines
            }
            Popup::Waiting(command) => vec![(command.clone(), true), ("Asking the model…".to_owned(), false)],
            Popup::Explained(command, parts, from) => {
                let widest = parts.iter().map(|part| part.text.chars().count()).max().unwrap_or(0);
                let mut lines = vec![(command.clone(), true)];
                lines.extend(parts.iter().map(|part| {
                    let pad = widest.saturating_sub(part.text.chars().count());
                    (format!("{}{}  {}", part.text, " ".repeat(pad), part.meaning), false)
                }));
                lines.extend(from.iter().map(|from| (from.clone(), false)));
                lines.push(("Enter or Esc: close".to_owned(), false));
                lines
            }
            Popup::Asked(asked, became, doubt) => {
                let mut lines = vec![(asked.command.clone(), true)];
                if !asked.note.is_empty() {
                    lines.push((asked.note.clone(), false));
                }
                lines.extend(doubt.iter().map(|doubt| (doubt.clone(), false)));
                lines.push(((*became).to_owned(), false));
                lines
            }
            Popup::Note(text) => vec![(text.clone(), false), ("Enter or Esc: close".to_owned(), false)],
        };
        Some(lines)
    }

    /// The popup, under the input line (over it when there is no room below).
    fn popup_box(&self) -> Option<impl IntoElement> {
        let theme = &self.settings.theme;
        let (columns, screen_lines) = self.session.size();
        // Nothing is cut off: a long line goes on on the next one.
        let room = usize::from(columns).saturating_sub(4).max(16);
        let lines: Vec<(String, bool)> = self
            .popup_lines()?
            .into_iter()
            .flat_map(|(text, heading)| wrap(&text, room).into_iter().map(move |line| (line, heading)))
            .collect();
        let cursor_line = self.session.input().map_or_else(|| self.cursor_cell.0, |input| input.cursor.0);
        let count = lines.len();
        let below = cursor_line + 1;
        let first = if below + count <= usize::from(screen_lines) { below } else { cursor_line.saturating_sub(count) };
        let widest = lines.iter().map(|(text, _)| width_of(text)).max().unwrap_or(0) + 2;
        let width = widest.min(usize::from(columns).saturating_sub(2)).max(20);
        let at = self.origin - self.view_origin + point(self.cell.width, self.cell.height * factor(first));
        Some(self.over_the_text(at, self.cell.width * factor(width) + px(2. * OVER_TEXT_INSET)).children(
            lines.into_iter().map(|(text, heading)| {
                div()
                    .px(px(OVER_TEXT_INSET))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(hsla(if heading { theme.foreground } else { theme.muted() }))
                    .when(heading, |line| line.font_weight(FontWeight::SEMIBOLD))
                    .child(SharedString::from(text))
            }),
        ))
    }

    /// What lies over the text (the list of commands, a box): a panel with
    /// a line around it and a shadow under it, at `at` in the view, `width`
    /// wide, with the terminal's letters in the terminal's line height.
    ///
    /// It stays inside the text's own width: one that would go past the last
    /// column begins further left.
    fn over_the_text(&self, at: Point<Pixels>, width: Pixels) -> gpui::Div {
        let theme = &self.settings.theme;
        let first_column = (self.origin - self.view_origin).x;
        let past_last_column = first_column + self.cell.width * factor(usize::from(self.session.size().0));
        let width = if width > past_last_column - first_column { past_last_column - first_column } else { width };
        let left = if at.x + width > past_last_column { past_last_column - width } else { at.x };
        let left = if left < first_column { first_column } else { left };
        div()
            .absolute()
            .left(left)
            .top(at.y)
            .w(width)
            .bg(hsla(theme.panel()))
            .border_1()
            .border_color(hsla(theme.edge()))
            .rounded(px(theme.panel_rounding()))
            .overflow_hidden()
            .shadow_lg()
            .font_family(SharedString::from(self.settings.config.font.family.clone()))
            .text_size(self.font_size())
            .line_height(self.cell.height)
    }

    /// The list of commands from the history and of words from the command
    /// lists, under the input line (over it when there is no room below);
    /// `None` when it is closed.
    fn candidate_list(&self) -> Option<impl IntoElement> {
        let (rows, chosen, typed) = self.candidates.visible()?;
        let input = self.session.input()?;
        let theme = &self.settings.theme;
        let (columns, lines) = self.session.size();
        // The rows, and the heading above them.
        let count = rows.len() + 1;
        let below = input.cursor.0 + 1;
        let first_line = if below + count <= usize::from(lines) { below } else { input.cursor.0.saturating_sub(count) };
        let now = unifyterm_history::now_ms();
        // Beside each row: for a command from the history, how often and when it ran; for a
        // word of a command list, what the list says of it.
        let beside = |row: &candidates::Row| match row {
            candidates::Row::Ran(ran) => {
                let failed = ran.last_exit.is_some_and(|exit| exit != 0);
                let mark = if failed { "✗ " } else { "" };
                format!("{mark}×{} {}", ran.count, unifyterm_history::ago(ran.last_at_ms, now))
            }
            candidates::Row::Listed(offer) => offer.note.clone(),
        };
        let widest =
            rows.iter().map(|row| row.text().chars().count() + beside(row).chars().count() + 4).max().unwrap_or(0);
        let width = widest.min(usize::from(columns).saturating_sub(input.start_column)).max(10);
        // Placed in the view, which starts below the tab bar, not in the window.
        let text_origin = self.origin - self.view_origin;
        let at =
            text_origin + point(self.cell.width * factor(input.start_column), self.cell.height * factor(first_line));
        let (muted, accent) = (hsla(theme.muted()), hsla(theme.accent()));
        // What the rows are: commands that ran, or (with none of those) a program's own words.
        let heading =
            if matches!(rows.first(), Some(candidates::Row::Ran(_))) { "Command history" } else { "Commands" };
        let head = div()
            .flex()
            .flex_row()
            .justify_between()
            .px(px(OVER_TEXT_INSET))
            .whitespace_nowrap()
            .overflow_hidden()
            .border_b_1()
            .border_color(hsla(theme.rule()))
            .text_color(muted)
            .child(div().text_color(hsla(theme.foreground)).font_weight(FontWeight::SEMIBOLD).child(heading))
            .child(div().pl(px(OVER_TEXT_INSET)).child("↑ ↓ choose · Tab fill"));
        let list = self
            .over_the_text(at, self.cell.width * factor(width) + px(2. * OVER_TEXT_INSET))
            .child(head)
            .children(rows.iter().enumerate().map(|(index, row)| {
                let (before, matched, after) = split_match(row.text(), typed);
                let info = beside(row);
                let current = chosen == Some(index);
                // The row in use has the selection's color behind it and a short line at its left.
                let mark = div()
                    .absolute()
                    .left_0()
                    .top(px(CHOSEN_MARK_INSET))
                    .bottom(px(CHOSEN_MARK_INSET))
                    .w(px(2.))
                    .rounded(px(1.))
                    .bg(accent);
                div()
                    .relative()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px(px(OVER_TEXT_INSET))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(hsla(theme.foreground))
                    .when(current, |line| line.bg(hsla(theme.selection)).child(mark))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .child(SharedString::from(before.to_owned()))
                            .child(div().text_color(accent).child(SharedString::from(matched.to_owned())))
                            .child(SharedString::from(after.to_owned())),
                    )
                    .child(div().pl(px(OVER_TEXT_INSET)).text_color(muted).child(SharedString::from(info)))
            }));
        Some(list)
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

/// Whether the window draws `c` itself, and no font: box-drawing lines,
/// block elements, powerline's separators.
pub(crate) fn drawn_here(c: char) -> bool {
    boxdraw::draws(c) || powerline::draws(c)
}

/// Paints a powerline separator into a cell's bounds, as the polygons of its shape.
fn paint_shape(shape: powerline::Shape, color: Rgb, bounds: Bounds<Pixels>, scale: f32, window: &mut Window) {
    let (w, h) = (bounds.size.width / px(1.) * scale, bounds.size.height / px(1.) * scale);
    for polygon in powerline::polygons(shape, w, h) {
        let at = |[x, y]: powerline::Point| bounds.origin + point(px(x / scale), px(y / scale));
        let mut path = gpui::PathBuilder::fill();
        let mut corners = polygon.into_iter();
        let Some(first) = corners.next() else { continue };
        path.move_to(at(first));
        for corner in corners {
            path.line_to(at(corner));
        }
        path.close();
        match path.build() {
            Ok(path) => window.paint_path(path, rgb(color.hex())),
            Err(error) => log::debug!("powerline: a shape was not drawn: {error}"),
        }
    }
}

/// Paints a character the window draws itself ([`drawn_here`]) into a cell's bounds.
fn paint_drawn(c: char, color: Rgb, bounds: Bounds<Pixels>, scale: f32, window: &mut Window) {
    if let Some(shape) = powerline::shape(c) {
        paint_shape(shape, color, bounds, scale, window);
        return;
    }
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
        let Prepared { frame, origin, cell, composing, pictures, stale } = self.prepared;
        for texture in stale {
            logged(window.drop_image(Arc::clone(texture)));
        }
        let scale = window.scale_factor();
        let x = |column: usize| snap(origin.x + cell.width * factor(column), scale);
        let y = |line: usize| snap(origin.y + cell.height * factor(line), scale);
        let span = |line: usize, column: usize, cells: usize| {
            Bounds::from_corners(point(x(column), y(line)), point(x(column + cells), y(line + 1)))
        };

        for item in &frame.fills {
            window.paint_quad(fill(span(item.line, item.column, item.cells), hsla(item.color)));
        }
        for (area, texture, (columns, rows)) in pictures {
            // The whole picture where its top left cell is, cut to the cells of the screen that show it.
            let at = |line: i64, column: i64| {
                let to_f32 = |n: i64| f32::from(i16::try_from(n).unwrap_or(if n < 0 { i16::MIN } else { i16::MAX }));
                point(origin.x + cell.width * to_f32(column), origin.y + cell.height * to_f32(line))
            };
            let whole = Bounds::new(
                at(area.origin.0, area.origin.1),
                size(cell.width * factor(*columns), cell.height * factor(*rows)),
            );
            let shown = Bounds::from_corners(
                point(x(area.first.1), y(area.first.0)),
                point(x(area.last.1 + 1), y(area.last.0 + 1)),
            );
            window.with_content_mask(Some(ContentMask { bounds: shown }), |window| {
                logged(window.paint_image(whole, Corners::default(), Arc::clone(texture), 0, false));
            });
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
                if let Some(drawn) = over.chars().next().filter(|c| drawn_here(*c)) {
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
    fn long_lines_are_wrapped_between_words_and_nothing_is_lost() {
        assert_eq!(wrap("short", 20), ["short"]);
        assert_eq!(wrap("the command line you type", 12), ["the command", "line you", "type"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"], "a word wider than the room is cut inside");
        assert_eq!(wrap("中文中文中文", 5), ["中文", "中文", "中文"], "two cells a character");
        let text = "one two three four five six seven eight nine ten";
        let lines = wrap(text, 9);
        assert!(lines.iter().all(|line| width_of(line) <= 9), "{lines:?}");
        assert_eq!(lines.join(" "), text, "nothing lost");
        assert_eq!(wrap("", 10), [""]);
    }

    #[test]
    fn the_typed_part_is_found_case_aside() {
        assert_eq!(split_match("git status", "git"), ("", "git", " status"));
        assert_eq!(split_match("echo Commit", "comm"), ("echo ", "Comm", "it"));
        assert_eq!(split_match("ls", "zz"), ("ls", "", ""));
        assert_eq!(split_match("echo 中文 x", "文"), ("echo 中", "文", " x"));
        assert_eq!(split_match("İstanbul", "i"), ("İstanbul", "", ""), "where lower case changes the length: no part");
    }

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
