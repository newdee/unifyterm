//! The mail, in the window: `Ctrl-Shift-M` shows it in place of the
//! terminal, and again shows the terminal. The conversations are on the
//! left; on the right, the chosen one as bubbles (theirs left, yours right),
//! the draft waiting in it (an agent's, through `ut mcp`), and a line to
//! write an answer on. The colors are the theme's.
//!
//! The view reads `mail.db`; the sync that fills it runs on a thread of its
//! own (see [`start_sync`]). Sending runs in the background, and only on the
//! person's own press: of Send, or of Ctrl-Enter.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, Pixels, Point, ScrollHandle, SharedString, UTF16Selection, Window, canvas, div, prelude::*, px,
};
use unifyterm_mail::{Filter, Plan, Status, Stopper, Store, Summary, Thread, message};

use crate::terminal::{Backdrop, Settings};
use crate::theme::{Rgb, Theme};
use crate::{seen_through, solid};

/// The list of conversations: its width.
const LIST_WIDTH: f32 = 300.;
/// The widest a bubble grows.
const BUBBLE_WIDTH: f32 = 560.;
/// Words wider than this many cells make a bubble as wide as it grows: text that wraps is
/// measured at the width it has.
const WRAPS_AT: usize = 56;
/// What a bubble takes from its width around its words: its padding and border.
const BUBBLE_INSIDE: f32 = 2. * 12. + 2.;
/// The most conversations the list shows.
const LIST_MOST: usize = 200;
/// The colors a person's circle may have: red, green, yellow, blue, magenta, cyan of the theme.
const CIRCLES: [usize; 6] = [1, 2, 3, 4, 5, 6];
/// How long the sync waits before it asks again for the lock another holds.
const LOCK_AGAIN: Duration = Duration::from_secs(30);

/// What the view tells the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MailEvent {
    /// Esc: the terminal again.
    Leave,
}

/// The mail view.
pub(crate) struct MailView {
    pub(crate) focus: FocusHandle,
    settings: Arc<Settings>,
    /// The mailbox the settings describe; `Err` says why there is none.
    plan: Result<Plan, String>,
    store: Option<Store>,
    /// The file's version when it was last read: another connection's write changes it.
    version: i64,
    threads: Vec<Summary>,
    chosen: Option<i64>,
    thread: Option<Thread>,
    /// The mails whose quoted part is shown.
    quotes_open: HashSet<i64>,
    /// The answer being written, and what an input method is still composing.
    compose: String,
    composing: String,
    status: Option<Status>,
    /// What the last press did: sent, or why not.
    note: Option<(String, Tone)>,
    sending: bool,
    stream: ScrollHandle,
    /// Where the answer line is, for the input method's window.
    input_bounds: Option<Bounds<Pixels>>,
}

/// Whether a note is good news.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Fine,
    Wrong,
}

impl EventEmitter<MailEvent> for MailView {}

/// The plan of the settings' `[mail]`, or why there is none, in words for the view.
pub(crate) fn plan_of(settings: &unifyterm_config::MailConfig) -> Result<Plan, String> {
    match unifyterm_mail::plan(settings) {
        Ok(Some(plan)) => Ok(plan),
        Ok(None) => Err(String::new()),
        Err(error) => Err(format!("[mail] in the settings: {error:#}")),
    }
}

impl MailView {
    pub(crate) fn new(settings: Arc<Settings>, cx: &mut Context<'_, Self>) -> Self {
        let plan = plan_of(&settings.config.mail);
        let mut view = Self {
            focus: cx.focus_handle(),
            settings,
            plan,
            store: None,
            version: -1,
            threads: Vec::new(),
            chosen: None,
            thread: None,
            quotes_open: HashSet::new(),
            compose: String::new(),
            composing: String::new(),
            status: None,
            note: None,
            sending: false,
            stream: ScrollHandle::new(),
            input_bounds: None,
        };
        view.refresh(Read::Always);
        view
    }

    /// New settings: the colors, and the mailbox when it changed.
    pub(crate) fn use_settings(&mut self, settings: Arc<Settings>, cx: &mut Context<'_, Self>) {
        if settings.config.mail != self.settings.config.mail {
            self.plan = plan_of(&settings.config.mail);
            self.store = None;
            self.chosen = None;
            self.thread = None;
            self.version = -1;
        }
        self.settings = settings;
        self.refresh(Read::Always);
        cx.notify();
    }

    /// Reads the file again: always, or when another connection wrote to it.
    pub(crate) fn refresh(&mut self, read: Read) {
        let Ok(plan) = &self.plan else { return };
        if self.store.is_none() {
            if !plan.store.exists() {
                return;
            }
            match Store::open(&plan.store) {
                Ok(store) => self.store = Some(store),
                Err(error) => {
                    self.note = Some((format!("{error:#}"), Tone::Wrong));
                    return;
                }
            }
        }
        let Some(store) = &self.store else { return };
        let version = store.data_version().unwrap_or(-1);
        if read == Read::IfChanged && version == self.version {
            return;
        }
        self.version = version;
        self.threads = store.threads(LIST_MOST, Filter::Inbox).unwrap_or_default();
        self.status = store.status().ok().flatten();
        if let Some(id) = self.chosen {
            self.thread = store.thread(id).ok().flatten();
        }
    }

    /// The unread count and the newest mail from someone else, for the window's row of tabs.
    pub(crate) fn counts(&mut self) -> Option<(usize, i64)> {
        self.refresh(Read::IfChanged);
        let store = self.store.as_ref()?;
        Some((store.unread().ok()?, store.newest_incoming().ok()?))
    }

    /// The conversation of a mail's row, as the window's note of new mail names it.
    pub(crate) fn newest(&self) -> Option<Summary> {
        self.threads.iter().max_by_key(|summary| summary.last_at_ms).cloned()
    }

    /// Shows a conversation: its mails are read now.
    pub(crate) fn choose(&mut self, id: i64, cx: &mut Context<'_, Self>) {
        let Some(store) = &self.store else { return };
        if let Err(error) = store.mark_seen(id) {
            log::warn!("mail: could not mark conversation {id} read: {error:#}");
        }
        if self.chosen != Some(id) {
            self.compose.clear();
            self.quotes_open.clear();
        }
        self.chosen = Some(id);
        self.thread = store.thread(id).ok().flatten();
        self.threads = store.threads(LIST_MOST, Filter::Inbox).unwrap_or_default();
        self.note = None;
        self.stream.scroll_to_bottom();
        cx.notify();
    }

    /// When the view is shown with nothing chosen: the newest conversation.
    pub(crate) fn shown(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh(Read::Always);
        if self.chosen.is_none()
            && let Some(first) = self.threads.first().map(|summary| summary.id)
        {
            self.choose(first, cx);
        }
        self.stream.scroll_to_bottom();
    }

    fn step(&mut self, by: isize, cx: &mut Context<'_, Self>) {
        let at = self.chosen.and_then(|id| self.threads.iter().position(|summary| summary.id == id)).unwrap_or(0);
        let next = at.checked_add_signed(by).filter(|next| *next < self.threads.len());
        if let Some(id) = next.and_then(|next| self.threads.get(next)).map(|summary| summary.id) {
            self.choose(id, cx);
        }
    }

    /// Sends `text` as the answer to the chosen conversation, in the background.
    fn send(&mut self, text: String, cx: &mut Context<'_, Self>) {
        let (Ok(plan), Some(id)) = (self.plan.clone(), self.chosen) else { return };
        if text.trim().is_empty() || self.sending {
            return;
        }
        self.sending = true;
        self.note = Some(("sending…".into(), Tone::Fine));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let sent = cx.background_spawn(async move { unifyterm_mail::send_answer(&plan, id, &text) }).await;
            let _ = this.update(cx, |this, cx| {
                this.sending = false;
                match sent {
                    Ok(()) => {
                        this.compose.clear();
                        this.note = Some(("sent".into(), Tone::Fine));
                    }
                    // The words stay where they were: nothing is lost, and nothing is sent twice by itself.
                    Err(error) => this.note = Some((format!("not sent: {error:#}"), Tone::Wrong)),
                }
                this.refresh(Read::Always);
                this.stream.scroll_to_bottom();
                cx.notify();
            });
        })
        .detach();
    }

    fn send_draft(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(draft) = self.thread.as_ref().and_then(|thread| thread.draft.clone()) {
            self.send(draft.text, cx);
        }
    }

    /// The draft goes onto the answer line, to change before it is sent.
    fn edit_draft(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(store), Some(id)) = (&self.store, self.chosen) else { return };
        if let Some(draft) = self.thread.as_ref().and_then(|thread| thread.draft.clone()) {
            self.compose = draft.text;
            if let Err(error) = store.drop_draft(id) {
                log::warn!("mail: {error:#}");
            }
            self.refresh(Read::Always);
            cx.notify();
        }
    }

    fn drop_draft(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(store), Some(id)) = (&self.store, self.chosen) else { return };
        if let Err(error) = store.drop_draft(id) {
            log::warn!("mail: {error:#}");
        }
        self.refresh(Read::Always);
        cx.notify();
    }

    fn archive(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(store), Some(id)) = (&self.store, self.chosen) else { return };
        if let Err(error) = store.archive(id) {
            log::warn!("mail: {error:#}");
        }
        self.chosen = None;
        self.thread = None;
        self.refresh(Read::Always);
        if let Some(next) = self.threads.first().map(|summary| summary.id) {
            self.choose(next, cx);
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let keys = &event.keystroke;
        let held = keys.modifiers.control || keys.modifiers.platform;
        match keys.key.as_str() {
            "enter" if held => {
                let text = self.compose.clone();
                self.send(text, cx);
            }
            "enter" if !keys.modifiers.alt => self.compose.push('\n'),
            "backspace" => {
                self.compose.pop();
            }
            "escape" => cx.emit(MailEvent::Leave),
            "up" if self.compose.is_empty() => self.step(-1, cx),
            "down" if self.compose.is_empty() => self.step(1, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// Whether [`MailView::refresh`] reads when nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Always,
    IfChanged,
}

impl EntityInputHandler for MailView {
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
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        (!self.composing.is_empty()).then(|| 0..self.composing.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.composing.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.composing.clear();
        self.compose.push_str(text);
        cx.notify();
    }

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

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        self.input_bounds
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

/// A time as the view shows it: the time of day for today, else the day too.
fn when(ms: i64) -> String {
    use chrono::{Datelike as _, Local, TimeZone as _};
    let Some(at) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let now = Local::now();
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else if at.year() == now.year() {
        at.format("%m-%d %H:%M").to_string()
    } else {
        at.format("%Y-%m-%d").to_string()
    }
}

/// A person's circle color: the same person, the same color.
fn circle(theme: &Theme, who: &str) -> Rgb {
    let sum = who.bytes().fold(0usize, |sum, byte| sum.wrapping_mul(31).wrapping_add(usize::from(byte)));
    theme.ansi[CIRCLES[sum % CIRCLES.len()]]
}

/// The first letter of a name, for its circle.
fn initial(who: &str) -> SharedString {
    who.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default().into()
}

impl MailView {
    fn colors(&self) -> Colors {
        let theme = &self.settings.theme;
        Colors {
            ink: solid(theme.foreground),
            muted: solid(theme.muted()),
            rule: solid(theme.rule()),
            edge: solid(theme.edge()),
            panel: solid(theme.panel()),
            well: solid(theme.well()),
            accent: solid(theme.accent()),
            back: solid(theme.background),
            blue: solid(theme.background.toward(theme.ansi[4], 0.22)),
            blue_edge: solid(theme.rule().toward(theme.ansi[4], 0.45)),
            draft: solid(theme.ansi[5]),
            draft_back: solid(theme.background.toward(theme.ansi[5], 0.1)),
            wrong: solid(theme.ansi[1]),
            waiting: solid(theme.ansi[3]),
        }
    }

    /// The list of conversations, with the account at its head.
    fn list(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = &self.settings.theme;
        let colors = self.colors();
        let (address, status) = match &self.plan {
            Ok(plan) => (plan.account.address.clone(), self.status.clone()),
            Err(_) => ("Mail".to_owned(), None),
        };
        let status_line = status.map(|status| {
            let text = if status.failed { status.text } else { format!("{} {}", status.text, when(status.at_ms)) };
            div()
                .text_size(px(11.))
                .text_color(if status.failed { colors.wrong } else { colors.muted })
                .child(SharedString::from(text))
        });
        let rows: Vec<_> = self.threads.iter().map(|summary| self.row(summary, cx)).collect();
        div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(seen_through(theme.bar(), theme.opacity))
            .border_r_1()
            .border_color(colors.rule)
            .child(
                div()
                    .flex_none()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .border_b_1()
                    .border_color(colors.rule)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(address)))
                    .children(status_line),
            )
            .child(
                div()
                    .id("threads")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows),
            )
    }

    /// One conversation in the list: who, when, the subject, and a draft or an answer waiting.
    fn row(&self, summary: &Summary, cx: &mut Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let theme = &self.settings.theme;
        let colors = self.colors();
        let id = summary.id;
        let chosen = self.chosen == Some(id);
        let tag = if let Some(by) = &summary.draft_by {
            Some((format!("✎ draft by {by}"), colors.draft))
        } else if summary.waiting && summary.unread == 0 {
            Some(("↩ waiting for you".to_owned(), colors.waiting))
        } else {
            None
        };
        div()
            .id(("thread", usize::try_from(id).unwrap_or_default()))
            .flex()
            .gap_3()
            .px_2()
            .py_2()
            .rounded(px(theme.rounding))
            .when(chosen, |row| row.bg(solid(theme.selection)))
            .when(!chosen, |row| row.hover(move |row| row.bg(colors.well)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                this.choose(id, cx);
            }))
            .child(self.circle_of(&summary.with, 32.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(if summary.unread > 0 {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::SEMIBOLD
                                    })
                                    .child(SharedString::from(summary.with.clone())),
                            )
                            .when(summary.unread > 0, |line| {
                                line.child(div().size(px(7.)).rounded_full().bg(colors.accent))
                            })
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(colors.muted)
                                    .child(SharedString::from(when(summary.last_at_ms))),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(SharedString::from(summary.subject.clone())),
                    )
                    .children(tag.map(|(text, color)| {
                        div().text_size(px(11.)).text_color(color).child(SharedString::from(text))
                    })),
            )
    }

    fn circle_of(&self, who: &str, size: f32) -> gpui::Div {
        let color = circle(&self.settings.theme, who);
        div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(solid(self.settings.theme.background.toward(color, 0.28)))
            .text_color(solid(color))
            .text_size(px(size * 0.42))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial(who))
    }

    /// The chosen conversation: its head, its mails as bubbles, its draft, the answer line.
    fn conversation(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let Some(thread) = &self.thread else {
            let text = if self.threads.is_empty() { "No mail yet." } else { "Choose a conversation on the left." };
            return div().flex_1().flex().items_center().justify_center().text_color(colors.muted).child(text);
        };
        let people: Vec<String> = {
            let mut seen: Vec<String> = Vec::new();
            for mail in &thread.mails {
                for address in std::iter::once(&mail.from).chain(&mail.to).chain(&mail.cc) {
                    let shown = address.shown().to_owned();
                    if !seen.contains(&shown)
                        && Ok(address.addr.as_str()) != self.plan.as_ref().map(|plan| plan.account.address.as_str())
                    {
                        seen.push(shown);
                    }
                }
            }
            seen
        };
        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(colors.rule)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .child(SharedString::from(thread.summary.subject.clone())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(SharedString::from(people.join(" · "))),
                    ),
            )
            .child(self.button("archive", "Archive", Weight::Plain, cx.listener(|this, _, _, cx| this.archive(cx))));
        let mut items: Vec<gpui::AnyElement> = thread.mails.iter().map(|mail| self.bubble(mail, cx)).collect();
        if let Some(draft) = &thread.draft {
            items.push(self.draft_box(draft, cx));
        }
        let stream = div()
            .id("stream")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.stream)
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .gap_3()
            .children(items);
        div().flex_1().min_w_0().h_full().flex().flex_col().child(head).child(stream).child(self.answer_line(cx))
    }

    /// One mail as a bubble: theirs on the left, yours on the right.
    fn bubble(&self, mail: &unifyterm_mail::Mail, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let (own, quoted) = message::fold_quote(&mail.text);
        let open = self.quotes_open.contains(&mail.id);
        let id = mail.id;
        let words = if open { mail.text.clone() } else { own };
        let wraps = words.lines().any(|line| unicode_width::UnicodeWidthStr::width(line) > WRAPS_AT);
        let mut bubble = div()
            .max_w(px(BUBBLE_WIDTH))
            .when(wraps, |bubble| bubble.w(px(BUBBLE_WIDTH)))
            .px_3()
            .py_2()
            .rounded(px(theme.panel_rounding() + 4.))
            .border_1()
            .border_color(if mail.outgoing { colors.blue_edge } else { colors.rule })
            .bg(if mail.outgoing { colors.blue } else { colors.panel })
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(colors.muted)
                    .when(mail.outgoing, gpui::Styled::justify_end)
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(colors.ink).child(SharedString::from(
                        if mail.outgoing { "You".to_owned() } else { mail.from.shown().to_owned() },
                    )))
                    .child(SharedString::from(when(mail.at_ms))),
            )
            .child(
                div()
                    .text_size(px(13.5))
                    // Words that wrap are given the width they wrap at: measured and drawn alike.
                    .when(wraps, |text| text.w(px(BUBBLE_WIDTH - BUBBLE_INSIDE)))
                    .child(SharedString::from(words)),
            );
        if quoted > 0 {
            let label =
                if open { "hide the quoted mail".to_owned() } else { format!("show the {quoted} quoted lines") };
            bubble = bubble.child(
                div()
                    .id(("quote", usize::try_from(id).unwrap_or_default()))
                    .text_size(px(11.5))
                    .text_color(colors.muted)
                    .underline()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.quotes_open.remove(&id) {
                            this.quotes_open.insert(id);
                        }
                        cx.notify();
                    }))
                    .child(SharedString::from(label)),
            );
        }
        if !mail.attachments.is_empty() {
            bubble = bubble.child(div().flex().flex_wrap().gap_1().children(mail.attachments.iter().map(|file| {
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(theme.button_rounding()))
                    .border_1()
                    .border_color(colors.rule)
                    .bg(colors.back)
                    .text_size(px(11.5))
                    .child(SharedString::from(format!("📎 {} · {} KB", file.name, file.bytes.div_ceil(1024))))
            })));
        }
        let who = if mail.outgoing { "you" } else { mail.from.shown() };
        div()
            .flex()
            .gap_2()
            .when(mail.outgoing, gpui::Styled::flex_row_reverse)
            .child(self.circle_of(who, 28.))
            .child(bubble)
            .into_any_element()
    }

    /// The draft waiting in the conversation: a dashed box, with Drop, Edit and Send.
    fn draft_box(&self, draft: &unifyterm_mail::Draft, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let colors = self.colors();
        let theme = &self.settings.theme;
        div()
            .flex()
            .justify_end()
            .child(
                div()
                    // As wide as it grows: its words wrap, and the buttons go under them.
                    .w(px(BUBBLE_WIDTH))
                    .px_3()
                    .py_2()
                    .rounded(px(theme.panel_rounding() + 4.))
                    .border_2()
                    .border_dashed()
                    .border_color(colors.draft)
                    .bg(colors.draft_back)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_3()
                            .text_size(px(11.5))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(colors.draft)
                                    .child(SharedString::from(format!("✎ draft by {}", draft.author))),
                            )
                            .child(div().text_color(colors.muted).child("not sent")),
                    )
                    .child(div().text_size(px(13.5)).child(SharedString::from(draft.text.clone())))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(self.button(
                                "drop-draft",
                                "Drop",
                                Weight::Plain,
                                cx.listener(|this, _, _, cx| this.drop_draft(cx)),
                            ))
                            .child(self.button(
                                "edit-draft",
                                "Edit",
                                Weight::Plain,
                                cx.listener(|this, _, _, cx| this.edit_draft(cx)),
                            ))
                            .child(self.button(
                                "send-draft",
                                "Send",
                                Weight::Strong,
                                cx.listener(|this, _, _, cx| this.send_draft(cx)),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The line an answer is written on: what is typed, the cursor, and Send.
    fn answer_line(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let empty = self.compose.is_empty() && self.composing.is_empty();
        let shown: SharedString = if empty {
            "Write an answer…  Ctrl+Enter sends".into()
        } else {
            format!("{}{}", self.compose, self.composing).into()
        };
        let (view, painted) = (cx.entity(), cx.entity());
        let focus = self.focus.clone();
        let note = self.note.clone().map(|(text, tone)| {
            div()
                .px_1()
                .text_size(px(12.))
                .text_color(if tone == Tone::Wrong { colors.wrong } else { colors.muted })
                .child(SharedString::from(text))
        });
        div().flex_none().mx_5().mb_3().flex().flex_col().gap_1().children(note).child(
            div()
                .flex()
                .items_end()
                .gap_2()
                .p_2()
                .rounded(px(theme.panel_rounding() + 2.))
                .border_1()
                .border_color(colors.edge)
                .bg(colors.panel)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(40.))
                        .max_h(px(160.))
                        .px_1()
                        .text_size(px(13.5))
                        .text_color(if empty { colors.muted } else { colors.ink })
                        .child(shown)
                        .child(
                            canvas(
                                move |bounds, _, cx| view.update(cx, |view, _| view.input_bounds = Some(bounds)),
                                move |bounds, (), window, cx| {
                                    window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx);
                                },
                            )
                            .absolute()
                            .size_full(),
                        ),
                )
                .child(self.button(
                    "send",
                    if self.sending { "Sending…" } else { "Send" },
                    Weight::Strong,
                    cx.listener(|this, _, _, cx| {
                        let text = this.compose.clone();
                        this.send(text, cx);
                    }),
                )),
        )
    }

    /// What the view says when no mailbox is set, or the settings name one that cannot be.
    fn setup(&self) -> gpui::Div {
        let colors = self.colors();
        let wrong = self.plan.as_ref().err().filter(|why| !why.is_empty()).cloned();
        let lines = [
            "No mailbox is set. In a tab:",
            "  ut setting set mail.address me@example.com",
            "  ut mail login",
            "QQ, 163, 126, Gmail, iCloud and others need nothing more (an app password or",
            "authorization code, not the web password). Another domain: mail.imap, mail.smtp.",
            "A mailbox abroad: mail.proxy = \"socks5://127.0.0.1:7890\".",
        ];
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(colors.muted)
            .children(wrong.map(|why| div().text_color(colors.wrong).child(SharedString::from(why))))
            .children(lines.iter().map(|line| {
                let command = line.starts_with("  ");
                div()
                    .when(command, |line| {
                        line.font_family(SharedString::from(self.settings.config.font.family.clone()))
                            .text_color(colors.ink)
                    })
                    .child(*line)
            }))
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        weight: Weight,
        click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let strong = weight == Weight::Strong;
        let colors = self.colors();
        let theme = &self.settings.theme;
        div()
            .id(id)
            .flex_none()
            .px_3()
            .py_1()
            .rounded(px(theme.button_rounding()))
            .border_1()
            .border_color(if strong { colors.accent } else { colors.rule })
            .bg(if strong { colors.accent } else { colors.panel })
            .text_color(if strong { colors.back } else { colors.ink })
            .text_size(px(12.))
            .font_weight(if strong { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
            .cursor_pointer()
            .hover(move |button| button.border_color(colors.edge))
            .on_click(click)
            .child(label)
    }
}

/// A button that is the main thing to press, or one of the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    Strong,
    Plain,
}

/// The view's colors, from the theme.
#[derive(Clone, Copy)]
struct Colors {
    ink: Hsla,
    muted: Hsla,
    rule: Hsla,
    edge: Hsla,
    panel: Hsla,
    well: Hsla,
    accent: Hsla,
    back: Hsla,
    blue: Hsla,
    blue_edge: Hsla,
    draft: Hsla,
    draft_back: Hsla,
    wrong: Hsla,
    waiting: Hsla,
}

impl Render for MailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let colors = self.colors();
        let body = if self.plan.is_err() {
            div().flex_1().flex().child(self.setup())
        } else {
            div().flex_1().min_w_0().flex().child(self.list(cx)).child(self.conversation(cx))
        };
        let theme = &self.settings.theme;
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            // As the terminal: the theme's color behind it, or, under a wallpaper, what the window drew.
            .when(self.settings.backdrop == Backdrop::Color, |view| {
                view.bg(seen_through(theme.background, theme.opacity))
            })
            .text_color(colors.ink)
            .on_key_down(cx.listener(Self::key_down))
            .child(body)
    }
}

/// Starts the mail sync on a thread of its own: it waits for the machine's sync lock (another
/// window may hold it), then syncs until stopped; `on_new` is run for each new mail from someone else.
pub(crate) fn start_sync(plan: Plan, on_new: Vec<String>) -> Arc<Stopper> {
    let stopper = Arc::new(Stopper::default());
    let stop = Arc::clone(&stopper);
    let started = std::thread::Builder::new().name("mail sync".into()).spawn(move || {
        let Some(directory) = plan.store.parent().map(std::path::Path::to_path_buf) else { return };
        while !stop.stopped() {
            match unifyterm_mail::Lock::take(&directory) {
                Ok(Some(_lock)) => {
                    unifyterm_mail::sync::run(&plan, &stop, |new| {
                        for mail in &new {
                            if let Err(error) = unifyterm_mail::hook::run(&on_new, mail) {
                                log::warn!("{error:#}");
                            }
                        }
                    });
                    return;
                }
                Ok(None) => {}
                Err(error) => log::warn!("mail: {error:#}"),
            }
            let mut waited = Duration::ZERO;
            while waited < LOCK_AGAIN && !stop.stopped() {
                std::thread::sleep(Duration::from_millis(250));
                waited += Duration::from_millis(250);
            }
        }
    });
    if let Err(error) = started {
        log::warn!("mail: the sync could not start: {error}");
    }
    stopper
}
