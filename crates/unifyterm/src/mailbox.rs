//! The mail, in the window: `Ctrl-Shift-M` shows it in place of the
//! terminal, and again shows the terminal. The mailboxes are named at the top
//! left (All mixes them); under them the conversations, with All, Unread,
//! Waiting (for you) and Sent (waiting for them); on the right, the chosen one
//! as bubbles (theirs left, yours right), the draft waiting in it (an agent's,
//! through `ut mcp`), and a line to write an answer on. A mailbox is added in
//! the same place (`+`, or `Ctrl-Alt-Shift-M`): see [`form`]. The colors are the theme's.
//!
//! The view reads `mail.db`; the syncs that fill it run on threads of their
//! own (see [`start_syncs`]). Sending runs in the background, and only on the
//! person's own press: of Send, or of Ctrl-Enter.

mod compose;
mod form;
mod keys;

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, EntityInputHandler, EventEmitter, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton,
    Pixels, Point, ScrollHandle, SharedString, UTF16Selection, Window, anchored, deferred, div, prelude::*, px,
};
use unifyterm_config::{MailConfig, MailboxSettings};
use unifyterm_mail::{Filter, Plan, Status, Stopper, Store, Summary, Thread, message};

use form::Form;
pub(crate) use form::Purpose;

use crate::terminal::{Backdrop, Settings};
use crate::theme::{Rgb, Theme};
use crate::{seen_through, solid};

/// The list of conversations: this share of the window, within these widths.
const LIST_SHARE: f32 = 0.26;
const LIST_NARROWEST: f32 = 240.;
const LIST_WIDEST: f32 = 340.;
/// A bubble grows to this share of the room beside a person's circle, and no wider than
/// `BUBBLE_WIDEST` (lines longer than that are hard to read); never narrower than `BUBBLE_NARROWEST`
/// unless the room itself is.
const BUBBLE_SHARE: f32 = 0.8;
const BUBBLE_WIDEST: f32 = 880.;
const BUBBLE_NARROWEST: f32 = 200.;
/// The widest the form that adds a mailbox is drawn.
const FORM_WIDEST: f32 = 640.;
/// The conversation's padding on each side, and a circle with the gap after it.
const STREAM_PADDING: f32 = 20.;
const CIRCLE_ROOM: f32 = 28. + 8.;
/// What a bubble takes from its width around its words: its padding and border.
const BUBBLE_INSIDE: f32 = 2. * 12. + 2.;
/// A guess of a half-width character's width in a bubble's 13.5 px text, on the wide side of
/// every system's fonts: a line guessed to wrap is given the bubble's width, which is always safe;
/// one guessed not to, and wrapping after all, would be measured short.
const CELL_GUESS: f32 = 8.;
/// The most people To and Cc list from.
const CONTACTS_MOST: usize = 500;
/// The most conversations the list shows.
const LIST_MOST: usize = 200;
/// The colors a person's circle, or a mailbox's dot, may have: red, green, yellow, blue, magenta, cyan of the theme.
const CIRCLES: [usize; 6] = [1, 2, 3, 4, 5, 6];
/// The colors of the mailboxes' dots, in order: cyan, red, green, yellow, magenta, blue.
const DOTS: [usize; 6] = [6, 1, 2, 3, 5, 4];
/// How long a sync waits before it asks again for the lock another holds.
const LOCK_AGAIN: Duration = Duration::from_secs(30);

/// What the view tells the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MailEvent {
    /// Esc: the terminal again.
    Leave,
    /// A mailbox signed in again: its sync starts again with what lets it in now.
    SignedIn,
}

/// Which conversations the list shows, of those not archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    All,
    Unread,
    /// The newest mail is someone else's: it waits for an answer.
    Waiting,
    /// The newest mail is the person's: sent, waiting for theirs.
    Sent,
}

impl Shown {
    const ALL: [Self; 4] = [Self::All, Self::Unread, Self::Waiting, Self::Sent];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Unread => "Unread",
            Self::Waiting => "To answer",
            Self::Sent => "Sent",
        }
    }

    fn keeps(self, summary: &Summary) -> bool {
        match self {
            Self::All => true,
            Self::Unread => summary.unread > 0,
            Self::Waiting => summary.waiting,
            Self::Sent => !summary.waiting,
        }
    }
}

/// The menu of a mailbox, opened with the right button on its name.
#[derive(Debug, Clone)]
struct Menu {
    name: String,
    at: Point<Pixels>,
    /// Remove was pressed once: the menu asks to be sure.
    sure: bool,
}

/// The mail view.
pub(crate) struct MailView {
    pub(crate) focus: FocusHandle,
    settings: Arc<Settings>,
    /// The settings file: where an added mailbox is written.
    settings_path: Option<PathBuf>,
    /// The mailboxes the settings name, each with its plan or why it has none.
    plans: Vec<(MailboxSettings, Result<Plan, String>)>,
    store: Option<Store>,
    /// The file's version when it was last read: another connection's write changes it.
    version: i64,
    /// The conversations not archived, of the mailbox shown (or all), newest first.
    threads: Vec<Summary>,
    /// The mailbox shown, by address; `None`: all of them.
    showing: Option<String>,
    shown: Shown,
    chosen: Option<i64>,
    thread: Option<Thread>,
    /// The mails whose quoted part is shown.
    quotes_open: HashSet<i64>,
    /// The mail being written, and what an input method is still composing.
    compose: compose::Compose,
    composing: String,
    /// Where keys go, and how they are taken.
    zone: keys::Zone,
    mode: keys::Mode,
    /// The mailbox the keys of [1] are on: 0 is All, then the settings' order.
    box_cursor: usize,
    /// The mail of the conversation the keys of [3] are on.
    mail_cursor: usize,
    visual: Option<keys::Visual>,
    overlay: Option<keys::Overlay>,
    undo: Option<keys::Undo>,
    /// What the search has, while there is one.
    searching: Option<String>,
    /// The people the mailboxes have written with, for To and Cc.
    contacts: Vec<unifyterm_mail::Contact>,
    /// What each mailbox's sync last said, by address.
    statuses: Vec<(String, Status)>,
    /// What the last press did: sent, or why not.
    note: Option<(String, Tone)>,
    sending: bool,
    stream: ScrollHandle,
    /// The list of conversations, scrolled to keep the one chosen by keys in sight.
    list_scroll: ScrollHandle,
    /// What is over the mail, scrolled to keep the file chosen by keys in sight.
    overlay_scroll: ScrollHandle,
    /// Where the line being typed on is, for the input method's window.
    input_bounds: Option<Bounds<Pixels>>,
    /// The form that adds a mailbox, or changes one: in place of the conversation.
    form: Option<Form>,
    menu: Option<Menu>,
    /// The widths of the window as last drawn.
    widths: Widths,
}

/// The widths that follow the window: the list, and the widest a bubble grows. Text that wraps
/// is given a definite width (GPUI measures text in a row on one line otherwise): this one.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Widths {
    window: f32,
    list: f32,
    bubble: f32,
}

impl Widths {
    fn of(window: f32) -> Self {
        let list = (window * LIST_SHARE).clamp(LIST_NARROWEST, LIST_WIDEST);
        let room = (window - list - 2. * STREAM_PADDING - CIRCLE_ROOM).max(BUBBLE_INSIDE * 2.);
        let bubble = (room * BUBBLE_SHARE).clamp(room.min(BUBBLE_NARROWEST), BUBBLE_WIDEST);
        Self { window, list, bubble }
    }

    /// The form's width: the room beside the list (`beside`: its width, 0 when it is not
    /// there), less the padding, no wider than reads well.
    fn form(self, beside: f32) -> f32 {
        (self.window - beside - 2. * STREAM_PADDING).clamp(0., FORM_WIDEST)
    }

    /// Whether `text` wraps in a bubble this wide: by the guess, which errs on the side of wrapping.
    fn wraps(self, text: &str) -> bool {
        let inside = self.bubble - BUBBLE_INSIDE;
        text.lines().any(|line| {
            let cells = unicode_width::UnicodeWidthStr::width(line);
            f32::from(u16::try_from(cells).unwrap_or(u16::MAX)) * CELL_GUESS > inside
        })
    }
}

/// Whether a note is good news.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Fine,
    Wrong,
}

impl EventEmitter<MailEvent> for MailView {}

/// The mailboxes of the settings, each with its plan or why it has none, in words for the view.
fn plans_of(settings: &MailConfig) -> Vec<(MailboxSettings, Result<Plan, String>)> {
    unifyterm_mail::plans(settings)
        .into_iter()
        .map(|(mailbox, plan)| (mailbox, plan.map_err(|error| format!("{error:#}"))))
        .collect()
}

impl MailView {
    pub(crate) fn new(settings: Arc<Settings>, settings_path: Option<PathBuf>, cx: &mut Context<'_, Self>) -> Self {
        let plans = plans_of(&settings.config.mail);
        let mut view = Self {
            focus: cx.focus_handle(),
            settings,
            settings_path,
            plans,
            store: None,
            version: -1,
            threads: Vec::new(),
            showing: None,
            shown: Shown::All,
            chosen: None,
            thread: None,
            quotes_open: HashSet::new(),
            compose: compose::Compose::default(),
            composing: String::new(),
            zone: keys::Zone::List,
            mode: keys::Mode::Normal,
            box_cursor: 0,
            mail_cursor: 0,
            visual: None,
            overlay: None,
            undo: None,
            searching: None,
            contacts: Vec::new(),
            statuses: Vec::new(),
            note: None,
            sending: false,
            stream: ScrollHandle::new(),
            list_scroll: ScrollHandle::new(),
            overlay_scroll: ScrollHandle::new(),
            input_bounds: None,
            form: None,
            menu: None,
            widths: Widths::of(LIST_WIDEST / LIST_SHARE),
        };
        view.refresh(Read::Always);
        view
    }

    /// New settings: the colors, and the mailboxes when they changed.
    pub(crate) fn use_settings(&mut self, settings: Arc<Settings>, cx: &mut Context<'_, Self>) {
        if settings.config.mail != self.settings.config.mail {
            self.plans = plans_of(&settings.config.mail);
            self.store = None;
            self.version = -1;
            // A mailbox shown that is gone: all of them again.
            if self
                .showing
                .as_ref()
                .is_some_and(|address| !self.plans.iter().any(|(mailbox, _)| &mailbox.address == address))
            {
                self.showing = None;
                self.chosen = None;
                self.thread = None;
            }
        }
        self.settings = settings;
        self.refresh(Read::Always);
        cx.notify();
    }

    /// The addresses of the mailboxes, in the settings' order.
    fn addresses(&self) -> Vec<String> {
        self.plans.iter().map(|(mailbox, _)| mailbox.address.clone()).collect()
    }

    /// The name the settings give the mailbox at `address`, or the address.
    fn name_of(&self, address: &str) -> String {
        self.plans
            .iter()
            .find(|(mailbox, _)| mailbox.address == address)
            .map_or_else(|| address.to_owned(), |(mailbox, _)| mailbox.name.clone())
    }

    /// The edge of a zone: marked when the keys go there.
    fn zone_edge(&self, zone: keys::Zone) -> Hsla {
        if self.zone == zone && self.form.is_none() && self.overlay.is_none() {
            self.colors().accent.opacity(0.5)
        } else {
            gpui::transparent_black()
        }
    }

    /// The color of the mailbox at `address`'s dot.
    fn dot(&self, address: &str) -> Hsla {
        let at = self.plans.iter().position(|(mailbox, _)| mailbox.address == address).unwrap_or(0);
        solid(self.settings.theme.ansi[DOTS[at % DOTS.len()]])
    }

    /// Reads the file again: always, or when another connection wrote to it.
    pub(crate) fn refresh(&mut self, read: Read) {
        if self.plans.is_empty() {
            return;
        }
        if self.store.is_none() {
            if !unifyterm_mail::default_path().is_ok_and(|path| path.exists()) {
                return;
            }
            match unifyterm_mail::open(&self.settings.config.mail) {
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
        self.threads = self.read_threads(store);
        self.contacts = store.contacts(&self.addresses(), CONTACTS_MOST).unwrap_or_default();
        self.statuses = self
            .addresses()
            .into_iter()
            .filter_map(|address| store.status(&address).ok().flatten().map(|status| (address, status)))
            .collect();
        if let Some(id) = self.chosen {
            self.thread = store.thread(id).ok().flatten();
        }
    }

    /// The unread count and the newest mail from someone else, for the window's row of tabs.
    pub(crate) fn counts(&mut self) -> Option<(usize, i64)> {
        self.refresh(Read::IfChanged);
        let store = self.store.as_ref()?;
        Some((store.unread(None).ok()?, store.newest_incoming().ok()?))
    }

    /// The conversation of a mail's row, as the window's note of new mail names it.
    pub(crate) fn newest(&self) -> Option<Summary> {
        self.threads.iter().max_by_key(|summary| summary.last_at_ms).cloned()
    }

    /// The conversations of the mailbox shown, or those the search finds: one reading for the list
    /// whether it is read again or a conversation is chosen.
    fn read_threads(&self, store: &Store) -> Vec<Summary> {
        match self.searching.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => store.search(text, LIST_MOST, self.showing.as_deref()).unwrap_or_default(),
            None => store.threads(LIST_MOST, Filter::Inbox, self.showing.as_deref()).unwrap_or_default(),
        }
    }

    /// The conversations the list shows: those of the mailbox shown that the filter keeps, and
    /// the one shown (opening a conversation reads it: under Unread it stays until another is chosen).
    fn listed(&self) -> Vec<&Summary> {
        self.threads.iter().filter(|summary| self.shown.keeps(summary) || self.chosen == Some(summary.id)).collect()
    }

    /// Shows a conversation: its mails are read now.
    pub(crate) fn choose(&mut self, id: i64, cx: &mut Context<'_, Self>) {
        let Some(store) = &self.store else { return };
        if let Err(error) = store.mark_seen(id) {
            log::warn!("mail: could not mark conversation {id} read: {error:#}");
        }
        if self.chosen != Some(id) {
            self.quotes_open.clear();
            self.visual = None;
            // An answer belongs to its conversation: another shown, it goes (as before); a new mail stays.
            if self.compose.kind == compose::Kind::Reply {
                self.compose = compose::Compose::default();
                if self.zone == keys::Zone::Compose {
                    self.mode = keys::Mode::Normal;
                }
            }
        }
        self.form = None;
        self.chosen = Some(id);
        self.thread = store.thread(id).ok().flatten();
        self.mail_cursor = self.thread.as_ref().map_or(0, |thread| thread.mails.len().saturating_sub(1));
        self.threads = self.read_threads(store);
        // Chosen by keys too (j, k, Ctrl+d): the list follows, as the mouse's wheel would.
        if let Some(at) = self.listed().iter().position(|summary| summary.id == id) {
            self.list_scroll.scroll_to_item(at);
        }
        self.note = None;
        self.stream.scroll_to_bottom();
        cx.notify();
    }

    /// When the view is shown with nothing chosen: the newest conversation; with no mailbox, the form.
    pub(crate) fn shown(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.refresh(Read::Always);
        if self.plans.is_empty() && self.form.is_none() {
            self.open_form(Purpose::Add, window, cx);
            return;
        }
        if self.chosen.is_none()
            && self.form.is_none()
            && let Some(first) = self.listed().first().map(|summary| summary.id)
        {
            self.choose(first, cx);
        }
        self.stream.scroll_to_bottom();
    }

    /// Shows the mailbox at `address`, or all of them.
    fn show_mailbox(&mut self, address: Option<String>, cx: &mut Context<'_, Self>) {
        self.showing = address;
        self.chosen = None;
        self.thread = None;
        self.refresh(Read::Always);
        if self.form.is_none()
            && let Some(first) = self.listed().first().map(|summary| summary.id)
        {
            self.choose(first, cx);
        }
        cx.notify();
    }

    fn show(&mut self, shown: Shown, cx: &mut Context<'_, Self>) {
        self.shown = shown;
        if self.chosen.is_some_and(|id| !self.listed().iter().any(|summary| summary.id == id)) {
            if let Some(first) = self.listed().first().map(|summary| summary.id) {
                self.choose(first, cx);
            } else {
                // Nothing here: no conversation is shown that the list does not have.
                self.chosen = None;
                self.thread = None;
            }
        }
        cx.notify();
    }

    /// Sends `text` as the answer to the chosen conversation, from its mailbox, in the background.
    fn send(&mut self, text: String, cx: &mut Context<'_, Self>) {
        let Some(id) = self.chosen else { return };
        if text.trim().is_empty() || self.sending {
            return;
        }
        let plans: Vec<Plan> = self.plans.iter().filter_map(|(_, plan)| plan.as_ref().ok().cloned()).collect();
        self.sending = true;
        self.note = Some(("sending…".into(), Tone::Fine));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let sent = cx.background_spawn(async move { unifyterm_mail::send_answer(&plans, id, &text) }).await;
            let _ = this.update(cx, |this, cx| {
                this.sending = false;
                match sent {
                    Ok(()) => this.note = Some(("sent".into(), Tone::Fine)),
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

    /// The draft becomes the answer being written (to the conversation's other person), to change
    /// before it is sent: by E and by Edit alike.
    fn edit_draft(&mut self, cx: &mut Context<'_, Self>) {
        let Some(id) = self.chosen else { return };
        if let Some(draft) = self.thread.as_ref().and_then(|thread| thread.draft.clone()) {
            self.start_reply(keys::Everyone::One);
            self.compose.body = draft.text;
            let Some(store) = &self.store else { return };
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
        if let Some(next) = self.listed().first().map(|summary| summary.id) {
            self.choose(next, cx);
        }
        cx.notify();
    }

    /// The text being typed on: the form's field in use, the search, or the mail being written.
    fn typed_mut(&mut self) -> &mut String {
        let search = self.typing_search();
        match (&mut self.form, search) {
            (Some(form), _) => form.typed_mut(),
            (None, true) => self.searching.get_or_insert_default(),
            (None, false) => self.compose.text_mut(),
        }
    }

    /// Whether typed text is taken now: only where text is typed.
    fn takes_text(&self) -> bool {
        self.form.is_some() || (self.mode == keys::Mode::Insert && self.overlay.is_none())
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let keys = &event.keystroke;
        let held = keys.modifiers.control || keys.modifiers.platform;
        if keys.key == "v" && held && !keys.modifiers.alt && self.takes_text() {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let text = if self.form.is_some()
                    || self.mode == keys::Mode::Insert && self.compose.part != compose::Part::Body
                {
                    text.trim().to_owned()
                } else {
                    text
                };
                self.typed_mut().push_str(&text);
            }
        } else if keys.key == "escape" && self.menu.is_some() {
            self.menu = None;
        } else if self.form.is_some() {
            if !self.form_key(keys, window, cx) {
                return;
            }
        } else if !self.take_key(keys, window, cx) {
            // Not the view's: typed where text is typed, and the window's keys go on to the window.
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The menu of the mailbox `name`, at the pointer.
    fn open_menu(&mut self, name: String, at: Point<Pixels>, cx: &mut Context<'_, Self>) {
        self.menu = Some(Menu { name, at, sure: false });
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
        if self.takes_text() {
            self.typed_mut().push_str(text);
            if self.searching.is_some() {
                self.refresh(Read::Always);
            }
        }
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
            fine: solid(theme.ansi[2]),
        }
    }

    /// The list: the mailboxes, what their syncs say, the filters, and the conversations.
    fn list(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = &self.settings.theme;
        let colors = self.colors();
        let rows: Vec<_> = self.listed().into_iter().map(|summary| self.row(summary, cx)).collect();
        div()
            .w(px(self.widths.list))
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
                    .border_1()
                    .border_color(self.zone_edge(keys::Zone::Mailboxes))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.zone = keys::Zone::Mailboxes;
                            cx.notify();
                        }),
                    )
                    .child(self.mailboxes(cx))
                    .child(self.status_line())
                    .child(self.filters(cx)),
            )
            .child(
                div()
                    .id("threads")
                    .track_scroll(&self.list_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .border_1()
                    .border_color(self.zone_edge(keys::Zone::List))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.zone = keys::Zone::List;
                            cx.notify();
                        }),
                    )
                    .children(rows),
            )
    }

    /// One mailbox's name, or All, as a pill: pressed when it is the one shown.
    fn pill(
        &self,
        id: usize,
        label: String,
        unread: usize,
        dot: Option<Hsla>,
        press: Press,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = self.colors();
        let on = press == Press::On;
        // Where the keys of [1] are: Enter shows that mailbox.
        let cursor = self.zone == keys::Zone::Mailboxes && id == self.box_at() && self.form.is_none();
        div()
            .id(("mailbox", id))
            .flex()
            .items_center()
            .gap(px(5.))
            .px_2()
            .py(px(2.))
            .rounded_full()
            .border_1()
            .border_color(if cursor {
                colors.accent
            } else if on {
                colors.rule
            } else {
                gpui::transparent_black()
            })
            .when(cursor, gpui::Styled::border_dashed)
            .when(on, |pill| pill.bg(colors.well))
            .text_size(px(12.))
            .text_color(if on { colors.ink } else { colors.muted })
            .cursor_pointer()
            .hover(move |pill| pill.text_color(colors.ink))
            .children(dot.map(|dot| div().size(px(8.)).rounded_full().bg(dot)))
            .child(SharedString::from(label))
            .when(unread > 0, |pill| {
                pill.child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.accent)
                        .child(SharedString::from(unread.to_string())),
                )
            })
    }

    /// The row of mailboxes: All, each by name (the right button: its menu), and `+`.
    fn mailboxes(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let unread_of = |address: Option<&str>| -> usize {
            self.store.as_ref().and_then(|store| store.unread(address).ok()).unwrap_or(0)
        };
        let mut row = div().flex_none().flex().flex_wrap().items_center().gap_1().px_2().pt_2().pb(px(6.));
        if self.plans.len() > 1 {
            row = row.child(
                self.pill(
                    0,
                    "All".into(),
                    unread_of(None),
                    None,
                    if self.showing.is_none() { Press::On } else { Press::Off },
                )
                .on_click(cx.listener(|this, _, _, cx| this.show_mailbox(None, cx))),
            );
        }
        for (at, (mailbox, _)) in self.plans.iter().enumerate() {
            let address = mailbox.address.clone();
            let name = mailbox.name.clone();
            let on = self.showing.as_deref() == Some(address.as_str()) || self.plans.len() == 1;
            let on = if on { Press::On } else { Press::Off };
            let pill = self
                .pill(at + 1, name.clone(), unread_of(Some(&address)), Some(self.dot(&address)), on)
                .on_click(cx.listener(move |this, _, _, cx| this.show_mailbox(Some(address.clone()), cx)))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        this.open_menu(name.clone(), event.position, cx);
                    }),
                );
            row = row.child(pill);
        }
        row.child(
            div()
                .id("add-mailbox")
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_dashed()
                .border_color(colors.edge)
                .text_color(colors.muted)
                .cursor_pointer()
                .hover(move |add| add.border_color(colors.accent).text_color(colors.accent))
                .on_click(cx.listener(|this, _, window, cx| this.open_form(Purpose::Add, window, cx)))
                .child("+"),
        )
    }

    /// What the syncs of the mailboxes shown last said.
    fn status_line(&self) -> gpui::Div {
        let colors = self.colors();
        // One mailbox to a line: a long failure wraps within the list instead of running past it.
        let mut line = div().flex_none().flex().flex_col().px_4().pb(px(6.)).text_size(px(11.));
        for (mailbox, plan) in &self.plans {
            if self.showing.as_ref().is_some_and(|address| address != &mailbox.address) {
                continue;
            }
            let (text, failed) = match (plan, self.statuses.iter().find(|(address, _)| address == &mailbox.address)) {
                (Err(why), _) => (why.clone(), true),
                (Ok(_), Some((_, status))) if status.failed => (status.text.clone(), true),
                (Ok(_), Some((_, status))) => (format!("{} {}", status.text, when(status.at_ms)), false),
                (Ok(_), None) => ("waiting to sync".to_owned(), false),
            };
            let shown = if self.plans.len() > 1 { format!("{}: {text}", mailbox.name) } else { text };
            line = line.child(
                div().text_color(if failed { colors.wrong } else { colors.muted }).child(SharedString::from(shown)),
            );
        }
        line.border_b_1().border_color(colors.rule)
    }

    /// All, Unread, To answer, Sent: each with how many it has.
    fn filters(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        div().flex_none().flex().gap(px(2.)).px_2().py(px(5.)).border_b_1().border_color(colors.rule).children(
            Shown::ALL.into_iter().enumerate().map(|(at, shown)| {
                let on = self.shown == shown;
                let count = self.threads.iter().filter(|summary| shown.keeps(summary)).count();
                div()
                    .id(("shown", at))
                    .flex_1()
                    .flex()
                    .justify_center()
                    .gap(px(3.))
                    .py(px(3.))
                    .rounded(px(theme.button_rounding()))
                    .when(on, |pill| pill.bg(colors.well))
                    .text_size(px(11.5))
                    .text_color(if on { colors.ink } else { colors.muted })
                    .when(on, |pill| pill.font_weight(FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.show(shown, cx)))
                    .child(shown.label())
                    .when(shown != Shown::All, |pill| {
                        pill.child(
                            div()
                                .text_size(px(10.))
                                .text_color(colors.accent)
                                .child(SharedString::from(count.to_string())),
                        )
                    })
            }),
        )
    }

    /// One conversation in the list: who, when, the subject, its mailbox (when all show), and a
    /// draft or an answer waiting.
    fn row(&self, summary: &Summary, cx: &mut Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let theme = &self.settings.theme;
        let colors = self.colors();
        let id = summary.id;
        let chosen = self.chosen == Some(id) && self.form.is_none();
        let tag = if let Some(by) = &summary.draft_by {
            Some((format!("✎ draft by {by}"), colors.draft))
        } else if summary.waiting && summary.unread == 0 {
            Some(("↩ waiting for you".to_owned(), colors.waiting))
        } else if !summary.waiting {
            Some(("↗ sent, waiting for theirs".to_owned(), colors.muted))
        } else {
            None
        };
        let from = (self.showing.is_none() && self.plans.len() > 1).then(|| {
            let dot = self.dot(&summary.account);
            div()
                .flex_none()
                .px(px(5.))
                .rounded(px(3.))
                .text_size(px(10.5))
                .text_color(dot)
                .bg(dot.opacity(0.16))
                .child(SharedString::from(self.name_of(&summary.account)))
        });
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
                    .child(self.row_head(summary))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .children(from)
                            .child(div().min_w_0().truncate().child(SharedString::from(summary.subject.clone()))),
                    )
                    .children(tag.map(|(text, color)| {
                        div().text_size(px(11.)).text_color(color).child(SharedString::from(text))
                    })),
            )
    }

    /// A row's first line: who, a dot when unread, when.
    fn row_head(&self, summary: &Summary) -> gpui::Div {
        let colors = self.colors();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(if summary.unread > 0 { FontWeight::BOLD } else { FontWeight::SEMIBOLD })
                    .child(SharedString::from(summary.with.clone())),
            )
            .when(summary.unread > 0, |line| line.child(div().size(px(7.)).rounded_full().bg(colors.accent)))
            .child(
                div().text_size(px(11.)).text_color(colors.muted).child(SharedString::from(when(summary.last_at_ms))),
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
                    if !seen.contains(&shown) && address.addr != thread.summary.account {
                        seen.push(shown);
                    }
                }
            }
            seen
        };
        let mailbox = (self.plans.len() > 1).then(|| format!("  ·  in {}", self.name_of(&thread.summary.account)));
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
                        div().truncate().text_size(px(12.)).text_color(colors.muted).child(SharedString::from(
                            format!("{}{}", people.join(" · "), mailbox.unwrap_or_default()),
                        )),
                    ),
            )
            .child(self.button("archive", "Archive", Weight::Plain, cx.listener(|this, _, _, cx| this.archive(cx))));
        let mut items: Vec<gpui::AnyElement> =
            thread.mails.iter().enumerate().map(|(at, mail)| self.bubble(mail, at, cx)).collect();
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
            .border_1()
            .border_color(if self.zone == keys::Zone::Conversation {
                colors.accent.opacity(0.5)
            } else {
                gpui::transparent_black()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.zone = keys::Zone::Conversation;
                    cx.notify();
                }),
            )
            .children(items);
        let compose = div()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.zone != keys::Zone::Compose {
                        this.zone = keys::Zone::Compose;
                        if this.compose.to.is_empty() && this.compose.kind == compose::Kind::Reply {
                            this.start_reply(keys::Everyone::One);
                        }
                        this.mode = keys::Mode::Insert;
                        cx.notify();
                    }
                }),
            )
            .child(self.compose_area(cx));
        div().flex_1().min_w_0().h_full().flex().flex_col().child(head).child(stream).child(compose)
    }

    /// One mail as a bubble: theirs on the left, yours on the right.
    fn bubble(&self, mail: &unifyterm_mail::Mail, at: usize, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let (own, quoted) = message::fold_quote(&mail.text);
        let open = self.quotes_open.contains(&mail.id);
        let id = mail.id;
        let words = if open { mail.text.clone() } else { own };
        let widest = self.widths.bubble;
        let wraps = self.widths.wraps(&words);
        let mut bubble = div()
            .max_w(px(widest))
            .when(wraps, |bubble| bubble.w(px(widest)))
            .px_3()
            .py_2()
            .rounded(px(theme.panel_rounding() + 4.))
            .border_1()
            .border_color(if self.zone == keys::Zone::Conversation && at == self.mail_cursor {
                colors.accent
            } else if mail.outgoing {
                colors.blue_edge
            } else {
                colors.rule
            })
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
                    .when(wraps, |text| text.w(px(widest - BUBBLE_INSIDE)))
                    .child(self.words(&words, at)),
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
                    .w(px(self.widths.bubble))
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
                                cx.listener(|this, _, _, cx| {
                                    this.ask_send_draft();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// A mail's words: the part selected in VISUAL, when it is this mail, marked.
    fn words(&self, words: &str, at: usize) -> gpui::AnyElement {
        let text = SharedString::from(words.to_owned());
        match self.visual.filter(|visual| self.mode == keys::Mode::Visual && visual.mail == at) {
            Some(visual) => {
                let marked = gpui::HighlightStyle {
                    background_color: Some(crate::solid(self.settings.theme.ansi[3]).opacity(0.45)),
                    ..gpui::HighlightStyle::default()
                };
                gpui::StyledText::new(text).with_highlights(vec![(visual.bytes(words), marked)]).into_any_element()
            }
            None => text.into_any_element(),
        }
    }
    /// The menu of a mailbox: sign in again, change it, remove it (asked twice).
    fn menu(&self, cx: &mut Context<'_, Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.clone()?;
        let colors = self.colors();
        let theme = &self.settings.theme;
        let item = |id: &'static str, label: String, color: Hsla| {
            div()
                .id(id)
                .px_3()
                .py(px(5.))
                .rounded(px(theme.button_rounding()))
                .text_size(px(12.5))
                .text_color(color)
                .cursor_pointer()
                .hover(move |item| item.bg(colors.well))
                .child(SharedString::from(label))
        };
        let name = menu.name.clone();
        let (again, change, remove) = (name.clone(), name.clone(), name.clone());
        let mut list = div()
            .min_w(px(180.))
            .p_1()
            .rounded(px(theme.panel_rounding()))
            .border_1()
            .border_color(colors.edge)
            .bg(colors.panel)
            .shadow_md()
            .flex()
            .flex_col();
        if menu.sure {
            list = list
                .child(div().px_3().py_1().text_size(px(12.)).text_color(colors.muted).max_w(px(260.)).child(
                    SharedString::from(format!(
                        "Remove {name}? Its settings, its kept password and the mail kept here go. The server keeps its mail."
                    )),
                ))
                .child(item("menu-remove-sure", format!("Remove {name}"), colors.wrong).on_click(cx.listener(
                    move |this, _, _, cx| this.remove_mailbox(&remove, cx),
                )))
                .child(item("menu-keep", "Keep it".into(), colors.ink).on_click(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                })));
        } else {
            list = list
                .child(item("menu-sign-in", "Sign in again".into(), colors.ink).on_click(
                    cx.listener(move |this, _, window, cx| this.open_form(Purpose::SignIn(again.clone()), window, cx)),
                ))
                .child(item("menu-change", "Change…".into(), colors.ink).on_click(
                    cx.listener(move |this, _, window, cx| this.open_form(Purpose::Change(change.clone()), window, cx)),
                ))
                .child(item("menu-remove", "Remove…".into(), colors.wrong).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(menu) = &mut this.menu {
                        menu.sure = true;
                    }
                    cx.notify();
                })));
        }
        Some(
            deferred(
                anchored().position(menu.at).child(
                    div()
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .child(list),
                ),
            )
            .into_any_element(),
        )
    }

    /// Takes the mailbox `name` out, in the background: its settings, its kept password, its mail here.
    fn remove_mailbox(&mut self, name: &str, cx: &mut Context<'_, Self>) {
        self.menu = None;
        let Some(path) = self.settings_path.clone() else {
            self.note = Some(("no settings file: nothing to remove from".into(), Tone::Wrong));
            return;
        };
        let Some(mailbox) =
            self.plans.iter().find(|(mailbox, _)| mailbox.name == name).map(|(mailbox, _)| mailbox.clone())
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let removed = cx
                .background_spawn(async move {
                    let mut file = unifyterm_config::File::open(&path)?;
                    file.remove_mailbox(&mailbox.name)?;
                    file.save()?;
                    let login = if mailbox.login.trim().is_empty() { &mailbox.address } else { mailbox.login.trim() };
                    unifyterm_mail::forget_credential(login)?;
                    let store = unifyterm_mail::default_path()?;
                    if store.exists() {
                        Store::open(&store)?.forget_account(&mailbox.address)?;
                    }
                    anyhow::Ok(())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = removed {
                    this.note = Some((format!("not removed: {error:#}"), Tone::Wrong));
                }
                this.refresh(Read::Always);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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

/// Whether a pill is pressed: the mailbox it names is the one shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    On,
    Off,
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
    fine: Hsla,
}

impl Render for MailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // The widths follow the window, each time it is drawn: resized, full screen, another display.
        self.widths = Widths::of(f32::from(window.viewport_size().width));
        let colors = self.colors();
        let right = if self.form.is_some() {
            self.form_panel(cx)
        } else if self.plans.is_empty() {
            div().flex_1()
        } else {
            self.conversation(cx)
        };
        let body =
            div().flex_1().min_w_0().flex().when(!self.plans.is_empty(), |body| body.child(self.list(cx))).child(right);
        let menu = self.menu(cx);
        let overlay = self.overlay_card();
        let status = (!self.plans.is_empty() || self.form.is_some()).then(|| self.foot_line(cx));
        let theme = &self.settings.theme;
        div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            // As the terminal: the theme's color behind it, or, under a wallpaper, what the window drew.
            .when(self.settings.backdrop == Backdrop::Color, |view| {
                view.bg(seen_through(theme.background, theme.opacity))
            })
            .text_color(colors.ink)
            .on_key_down(cx.listener(Self::key_down))
            .child(body.min_h_0())
            .children(status)
            .children(overlay)
            .children(menu)
    }
}

/// Starts a sync for each mailbox the settings name that can be synced.
pub(crate) fn start_syncs(settings: &MailConfig) -> Vec<Arc<Stopper>> {
    unifyterm_mail::plans(settings)
        .into_iter()
        .filter_map(|(mailbox, plan)| match plan {
            Ok(plan) => Some(start_sync(plan, settings.on_new.clone())),
            Err(error) => {
                log::warn!("mail ({}): {error:#}", mailbox.name);
                None
            }
        })
        .collect()
}

/// Starts a mailbox's sync on a thread of its own: it waits for the machine's lock of that
/// mailbox (another window may hold it), then syncs until stopped; `on_new` is run for each new
/// mail from someone else.
pub(crate) fn start_sync(plan: Plan, on_new: Vec<String>) -> Arc<Stopper> {
    let stopper = Arc::new(Stopper::default());
    let stop = Arc::clone(&stopper);
    let started = std::thread::Builder::new().name(format!("mail sync {}", plan.name)).spawn(move || {
        let Some(directory) = plan.store.parent().map(std::path::Path::to_path_buf) else { return };
        while !stop.stopped() {
            match unifyterm_mail::Lock::take(&directory, &plan.account.address) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_widths_follow_the_window_within_their_bounds() {
        let full = Widths::of(1920.);
        assert!((full.list - LIST_WIDEST).abs() < 0.5, "a wide window: the widest list, {}", full.list);
        assert!((full.bubble - BUBBLE_WIDEST).abs() < 0.5, "and bubbles as wide as is readable, {}", full.bubble);
        let usual = Widths::of(1100.);
        assert!(usual.list > LIST_NARROWEST && usual.list < LIST_WIDEST);
        assert!(usual.bubble > 560., "more than the old fixed width when there is room: {}", usual.bubble);
        let narrow = Widths::of(640.);
        assert!((narrow.list - LIST_NARROWEST).abs() < 0.5);
        let room = 640. - narrow.list - 2. * STREAM_PADDING - CIRCLE_ROOM;
        assert!(narrow.bubble <= room, "a bubble never wider than its room: {} of {room}", narrow.bubble);
        let tiny = Widths::of(300.);
        assert!(tiny.bubble >= BUBBLE_INSIDE, "something to draw in: {}", tiny.bubble);
        // The form beside the list, or alone: full screen it stops at the widest that reads well.
        assert!((full.form(full.list) - FORM_WIDEST).abs() < 0.5);
        let small = Widths::of(800.);
        let beside = small.form(small.list);
        assert!((beside - (800. - small.list - 2. * STREAM_PADDING)).abs() < 0.5, "the room beside the list: {beside}");
        assert!(small.form(0.) > beside, "no list: more room");
        assert!((usual.form(usual.list) - FORM_WIDEST).abs() < 0.5, "no wider than reads well");
        assert!(tiny.form(tiny.list) >= 0., "never less than nothing");
    }

    #[test]
    fn a_line_is_guessed_to_wrap_by_the_bubbles_width_and_errs_on_wrapping() {
        let usual = Widths::of(1100.);
        assert!(!usual.wraps("好，周四 10 点。"), "a short line fits");
        assert!(usual.wraps(&"可以接受，公告里写一句就行。".repeat(6)), "a long one wraps");
        let narrow = Widths::of(640.);
        let line = "0.8 的发布原来定在周五。市场那边希望周四上午就能发公告。";
        assert!(narrow.wraps(line) && !Widths::of(1920.).wraps(line), "the same line wraps when the window is narrow");
        assert_eq!(usual.wraps("short\nshort"), usual.wraps("short"), "each line on its own");
    }
}
