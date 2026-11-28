//! The mail by keys, as keepane's dashboard: four zones ([1] the mailboxes and filters, [2] the
//! conversations, [3] the conversation, [4] the mail being written), taken with 1 to 4, Tab, h
//! and l; each with keys of its own. NORMAL takes keys as commands; INSERT, only where text is
//! typed (the mail being written, the search); VISUAL selects words of a mail to copy or quote.
//! The line at the foot says the mode, the zone and its keys; `?` shows them all.

use std::path::PathBuf;

use gpui::{ClipboardItem, Context, FontWeight, Keystroke, SharedString, Window, div, img, prelude::*, px};
use unifyterm_mail::{Draft, Plan};

use super::compose::Part;
use super::{MailView, Read, Shown, Tone};

/// The zones, in the order 1 to 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Zone {
    Mailboxes,
    List,
    Conversation,
    Compose,
}

impl Zone {
    const ALL: [Self; 4] = [Self::Mailboxes, Self::List, Self::Conversation, Self::Compose];

    fn number(self) -> usize {
        Self::ALL.iter().position(|zone| *zone == self).unwrap_or(0) + 1
    }

    fn name(self) -> &'static str {
        match self {
            Self::Mailboxes => "Mailboxes",
            Self::List => "Conversations",
            Self::Conversation => "Conversation",
            Self::Compose => "Writing",
        }
    }

    fn turned(self, by: isize) -> Self {
        let at = self.number() - 1;
        Self::ALL[(at.cast_signed() + by).rem_euclid(4).cast_unsigned()]
    }
}

/// How keys are taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Normal,
    Insert,
    Visual,
}

/// Which way Tab goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Forward,
    Back,
}

/// Whom an answer goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Everyone {
    One,
    All,
}

/// Words of a mail selected in VISUAL: character positions in its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Visual {
    pub(super) mail: usize,
    pub(super) anchor: usize,
    pub(super) caret: usize,
}

impl Visual {
    /// The selection as a byte range of `text`.
    pub(super) fn bytes(self, text: &str) -> std::ops::Range<usize> {
        let (low, high) = (self.anchor.min(self.caret), self.anchor.max(self.caret));
        let at = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte);
        at(low)..at(high + 1)
    }
}

/// What is over the mail.
#[derive(Debug, Clone)]
pub(super) enum Overlay {
    Help,
    Files { cursor: usize },
    Preview { name: String, path: PathBuf, cursor: usize },
    Confirm { text: String, then: Pending },
}

/// What a confirmation does on `y`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pending {
    SendCompose,
    SendDraft,
    Remove(String),
}

/// What `u` undoes.
#[derive(Debug, Clone)]
pub(super) enum Undo {
    Archived(i64),
    Dropped(Draft),
}

/// What is done with an attachment once it is on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    Open,
    Save,
    Preview,
}

/// An overlay's share of the window, and the widest it is drawn: the keys, and the rest.
const CARD_SHARE: f32 = 0.92;
const HELP_WIDEST: f32 = 1100.;
const CARD_WIDEST: f32 = 720.;
/// A group of keys in the help: its width, a key's column and the gap after it.
const HELP_GROUP: f32 = 320.;
const HELP_KEY: f32 = 130.;
const HELP_GAP: f32 = 12.;

/// An overlay's width in a window `window` wide.
fn card_width(window: f32, widest: f32) -> f32 {
    (window * CARD_SHARE).min(widest).max(0.)
}

/// How many files of the same name Downloads may have before saving gives up.
const SAME_NAMES: usize = 1000;
/// How many mails or conversations Ctrl+d and Ctrl+u move by.
const PAGE: usize = 5;
/// The longest a text attachment's preview reads.
const PREVIEW_CHARS: usize = 6000;

/// A printable key without Ctrl, Cmd or Alt: what it types (`J`, `?`, `>`).
fn typed(keys: &Keystroke) -> Option<char> {
    let held = keys.modifiers.control || keys.modifiers.platform || keys.modifiers.alt;
    let text = keys.key_char.as_deref().filter(|_| !held)?;
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

impl MailView {
    /// A key in the mail view; whether it was the view's.
    pub(super) fn take_key(&mut self, keys: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.overlay.is_some() {
            return self.overlay_key(keys, cx);
        }
        match self.mode {
            Mode::Insert => self.insert_key(keys, window, cx),
            Mode::Visual => self.visual_key(keys, cx),
            Mode::Normal => self.normal_key(keys, window, cx),
        }
    }

    fn normal_key(&mut self, keys: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let ctrl = keys.modifiers.control || keys.modifiers.platform;
        if ctrl && (keys.key == "d" || keys.key == "u") {
            self.page(if keys.key == "d" { Direction::Forward } else { Direction::Back }, cx);
            return true;
        }
        match (keys.key.as_str(), typed(keys)) {
            ("tab", _) => self.zone = self.zone.turned(if keys.modifiers.shift { -1 } else { 1 }),
            // Esc takes the search away first; with none, as q, back to the terminal.
            ("escape", _) if self.searching.is_some() => {
                self.searching = None;
                self.refresh(Read::Always);
            }
            ("escape", _) | (_, Some('q')) => cx.emit(super::MailEvent::Leave),
            (_, Some(digit @ '1'..='4')) => {
                self.zone = Zone::ALL[usize::from(u8::try_from(digit).unwrap_or(b'1') - b'1')];
            }
            (_, Some('h')) => self.zone = if self.zone == Zone::Mailboxes { Zone::Mailboxes } else { Zone::List },
            (_, Some('l')) => {
                if matches!(self.zone, Zone::Mailboxes | Zone::List) {
                    self.zone = Zone::Conversation;
                }
            }
            (_, Some('?')) => self.overlay = Some(Overlay::Help),
            (_, Some('/')) => {
                self.searching = Some(String::new());
                self.mode = Mode::Insert;
                self.zone = Zone::List;
            }
            (_, Some('c')) => self.start_new(),
            (_, Some(f @ ('f' | 'F'))) => self.turn_filter(if f == 'f' { 1 } else { -1 }, cx),
            _ => return self.zone_key(keys, window, cx),
        }
        true
    }

    fn zone_key(&mut self, keys: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        match self.zone {
            Zone::Mailboxes => self.mailboxes_key(keys, window, cx),
            Zone::List => self.list_key(keys, cx),
            Zone::Conversation => self.conversation_key(keys, cx),
            Zone::Compose => {
                if keys.key == "enter" || typed(keys) == Some('i') {
                    if self.compose.to.is_empty() && self.compose.kind == super::compose::Kind::Reply {
                        self.start_reply(Everyone::One);
                    }
                    self.mode = Mode::Insert;
                    return true;
                }
                false
            }
        }
    }

    /// j k g G and the arrows: the new position in a list of `count`.
    fn moved(keys: &Keystroke, at: usize, count: usize) -> Option<usize> {
        let last = count.saturating_sub(1);
        match (keys.key.as_str(), typed(keys)) {
            ("down", _) | (_, Some('j')) => Some((at + 1).min(last)),
            ("up", _) | (_, Some('k')) => Some(at.saturating_sub(1)),
            ("home", _) | (_, Some('g')) => Some(0),
            ("end", _) | (_, Some('G')) => Some(last),
            _ => None,
        }
    }

    fn mailboxes_key(&mut self, keys: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let count = self.plans.len() + 1;
        if let Some(at) = Self::moved(keys, self.box_at(), count) {
            self.box_cursor = at;
            return true;
        }
        // 0 is All; the others are the mailboxes in the settings' order.
        let chosen = self.box_at().checked_sub(1).and_then(|at| self.plans.get(at)).map(|(mailbox, _)| mailbox.clone());
        match (keys.key.as_str(), typed(keys)) {
            ("enter", _) => {
                self.show_mailbox(chosen.map(|mailbox| mailbox.address), cx);
                self.zone = Zone::List;
            }
            (_, Some('a')) => self.open_form(super::Purpose::Add, window, cx),
            (_, Some('s')) => {
                if let Some(mailbox) = chosen {
                    self.open_form(super::Purpose::SignIn(mailbox.name), window, cx);
                }
            }
            (_, Some('m')) => {
                if let Some(mailbox) = chosen {
                    self.open_form(super::Purpose::Change(mailbox.name), window, cx);
                }
            }
            (_, Some('x')) => {
                if let Some(mailbox) = chosen {
                    self.overlay = Some(Overlay::Confirm {
                        text: format!(
                            "Remove {}? Its settings, its kept password and the mail kept here go. The server keeps its mail.",
                            mailbox.name
                        ),
                        then: Pending::Remove(mailbox.name),
                    });
                }
            }
            _ => return false,
        }
        true
    }

    fn list_key(&mut self, keys: &Keystroke, cx: &mut Context<'_, Self>) -> bool {
        let listed: Vec<i64> = self.listed().iter().map(|summary| summary.id).collect();
        let at = self.chosen.and_then(|id| listed.iter().position(|one| *one == id)).unwrap_or(0);
        if let Some(next) = Self::moved(keys, at, listed.len()) {
            if let Some(id) = listed.get(next) {
                self.choose(*id, cx);
            }
            return true;
        }
        match (keys.key.as_str(), typed(keys)) {
            ("enter", _) => {
                if let Some(id) = self.chosen.or_else(|| listed.first().copied()) {
                    self.choose(id, cx);
                    self.zone = Zone::Conversation;
                    self.mail_cursor = self.thread.as_ref().map_or(0, |thread| thread.mails.len().saturating_sub(1));
                }
            }
            (_, Some('e')) => {
                if let Some(id) = self.chosen {
                    self.archive(cx);
                    self.undo = Some(Undo::Archived(id));
                    self.note = Some(("archived · u brings it back".into(), Tone::Fine));
                }
            }
            (_, Some('u')) => self.undo(cx),
            (_, Some('U')) => {
                if let (Some(store), Some(id)) = (&self.store, self.chosen) {
                    if let Err(error) = store.mark_unread(id) {
                        self.note = Some((format!("{error:#}"), Tone::Wrong));
                    }
                    self.refresh(Read::Always);
                    self.note = Some(("unread again: the server is told at the next sync".into(), Tone::Fine));
                }
            }
            (_, Some('r')) => self.start_reply(Everyone::One),
            _ => return false,
        }
        true
    }

    fn conversation_key(&mut self, keys: &Keystroke, cx: &mut Context<'_, Self>) -> bool {
        let Some(thread) = self.thread.clone() else { return false };
        if let Some(at) = Self::moved(keys, self.mail_cursor, thread.mails.len()) {
            self.mail_cursor = at;
            return true;
        }
        let Some(mail) = thread.mails.get(self.mail_cursor).cloned() else { return false };
        match typed(keys) {
            Some('o') => {
                if !self.quotes_open.remove(&mail.id) {
                    self.quotes_open.insert(mail.id);
                }
            }
            Some('y') => self.copy(&mail.text, cx),
            Some('v') => {
                self.visual = Some(Visual { mail: self.mail_cursor, anchor: 0, caret: 0 });
                self.mode = Mode::Visual;
            }
            Some('V') => {
                let line = mail.text.lines().next().map_or(0, |line| line.chars().count());
                self.visual = Some(Visual { mail: self.mail_cursor, anchor: 0, caret: line.saturating_sub(1) });
                self.mode = Mode::Visual;
            }
            Some('r') => self.start_reply(Everyone::One),
            Some('R') => self.start_reply(Everyone::All),
            Some('>') => self.quote(&mail.text),
            Some('t') => {
                if mail.attachments.is_empty() {
                    self.note = Some(("this mail has no files".into(), Tone::Fine));
                } else {
                    self.overlay = Some(Overlay::Files { cursor: 0 });
                }
            }
            Some('S') if thread.draft.is_some() => self.ask_send_draft(),
            Some('E') if thread.draft.is_some() => self.edit_draft(cx),
            Some('D') => {
                if let Some(draft) = thread.draft {
                    self.drop_draft(cx);
                    self.undo = Some(Undo::Dropped(draft));
                    self.note = Some(("draft dropped · u brings it back".into(), Tone::Fine));
                }
            }
            Some('u') => self.undo(cx),
            _ => return false,
        }
        true
    }

    fn insert_key(&mut self, keys: &Keystroke, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.typing_search() {
            let search = self.searching.get_or_insert_default();
            match keys.key.as_str() {
                "escape" => {
                    self.searching = None;
                    self.mode = Mode::Normal;
                }
                "enter" => self.mode = Mode::Normal,
                "backspace" => {
                    search.pop();
                }
                _ => return false,
            }
            self.refresh(Read::Always);
            return true;
        }
        let ctrl = keys.modifiers.control || keys.modifiers.platform;
        let part = self.compose.part;
        match keys.key.as_str() {
            "escape" => {
                self.mode = Mode::Normal;
                self.composing.clear();
            }
            "enter" if ctrl => self.ask_send(),
            "enter" if matches!(part, Part::To | Part::Cc) => {
                if !self.compose.take(&self.contacts) {
                    self.compose.next_part(Direction::Forward);
                }
            }
            "enter" if part == Part::Subject => self.compose.next_part(Direction::Forward),
            "enter" => self.compose.body.push('\n'),
            "tab" => {
                if matches!(part, Part::To | Part::Cc) && !self.compose.typed.trim().is_empty() {
                    self.compose.take(&self.contacts);
                }
                self.compose.next_part(if keys.modifiers.shift { Direction::Back } else { Direction::Forward });
            }
            "down" if matches!(part, Part::To | Part::Cc) => {
                let count = self.compose.suggestions(&self.contacts).len();
                self.compose.suggestion = (self.compose.suggestion + 1).min(count.saturating_sub(1));
            }
            "up" if matches!(part, Part::To | Part::Cc) => {
                self.compose.suggestion = self.compose.suggestion.saturating_sub(1);
            }
            "backspace" => self.compose.backspace(),
            _ => return false,
        }
        let _ = window;
        let _ = cx;
        true
    }

    fn visual_key(&mut self, keys: &Keystroke, cx: &mut Context<'_, Self>) -> bool {
        let Some(mut visual) = self.visual else {
            self.mode = Mode::Normal;
            return true;
        };
        let Some(text) =
            self.thread.as_ref().and_then(|thread| thread.mails.get(visual.mail)).map(|mail| mail.text.clone())
        else {
            return false;
        };
        let chars: Vec<char> = text.chars().collect();
        let last = chars.len().saturating_sub(1);
        match (keys.key.as_str(), typed(keys)) {
            ("escape", _) => {
                self.visual = None;
                self.mode = Mode::Normal;
                return true;
            }
            ("left", _) | (_, Some('h')) => visual.caret = visual.caret.saturating_sub(1),
            ("right", _) | (_, Some('l')) => visual.caret = (visual.caret + 1).min(last),
            (_, Some('w')) => visual.caret = word_step(&chars, visual.caret, Direction::Forward),
            (_, Some('b')) => visual.caret = word_step(&chars, visual.caret, Direction::Back),
            ("down", _) | (_, Some('j')) => visual.caret = line_step(&chars, visual.caret, Direction::Forward),
            ("up", _) | (_, Some('k')) => visual.caret = line_step(&chars, visual.caret, Direction::Back),
            (_, Some(what @ ('y' | '>'))) => {
                let selected = text[visual.bytes(&text)].to_owned();
                self.visual = None;
                self.mode = Mode::Normal;
                if what == 'y' {
                    self.copy(&selected, cx);
                } else {
                    self.quote(&selected);
                }
                return true;
            }
            _ => return false,
        }
        self.visual = Some(visual);
        true
    }

    fn overlay_key(&mut self, keys: &Keystroke, cx: &mut Context<'_, Self>) -> bool {
        let Some(overlay) = self.overlay.clone() else { return false };
        match overlay {
            Overlay::Confirm { then, .. } => {
                self.overlay = None;
                if typed(keys) == Some('y') {
                    match then {
                        Pending::SendCompose => self.send_compose(cx),
                        Pending::SendDraft => self.send_draft(cx),
                        Pending::Remove(name) => self.remove_mailbox(&name, cx),
                    }
                } else {
                    self.note = Some(("not done".into(), Tone::Fine));
                }
            }
            Overlay::Help => {
                if keys.key == "escape" || typed(keys) == Some('?') {
                    self.overlay = None;
                }
            }
            Overlay::Preview { cursor, .. } => {
                if keys.key == "escape" {
                    self.overlay = Some(Overlay::Files { cursor });
                }
            }
            Overlay::Files { cursor } => {
                let count = self.current_mail().map_or(0, |mail| mail.attachments.len());
                if let Some(at) = Self::moved(keys, cursor, count) {
                    self.overlay = Some(Overlay::Files { cursor: at });
                } else {
                    match (keys.key.as_str(), typed(keys)) {
                        ("escape", _) => self.overlay = None,
                        ("enter", _) => self.attachment(cursor, Use::Open, cx),
                        (_, Some('s')) => self.attachment(cursor, Use::Save, cx),
                        (_, Some('p')) => self.attachment(cursor, Use::Preview, cx),
                        _ => return false,
                    }
                }
            }
        }
        true
    }

    /// The mailbox the keys of [1] are on: 0 is All, which is not there with one mailbox; never
    /// past the last (one may have been removed).
    pub(super) fn box_at(&self) -> usize {
        let first = usize::from(self.plans.len() == 1);
        self.box_cursor.clamp(first, self.plans.len().max(first))
    }

    /// Asks before an agent's draft is sent, as before anything is: by S and by its Send alike.
    pub(super) fn ask_send_draft(&mut self) {
        let Some(thread) = &self.thread else { return };
        let Some(draft) = &thread.draft else { return };
        let text = format!("Send {}'s draft to {}?", draft.author, thread.summary.with);
        self.overlay = Some(Overlay::Confirm { text, then: Pending::SendDraft });
    }

    /// Whether typing goes into the search: `/` was pressed, and Enter or Esc not yet.
    pub(super) fn typing_search(&self) -> bool {
        self.mode == Mode::Insert && self.zone == Zone::List && self.searching.is_some()
    }

    fn current_mail(&self) -> Option<&unifyterm_mail::Mail> {
        self.thread.as_ref().and_then(|thread| thread.mails.get(self.mail_cursor))
    }

    /// Ctrl+d and Ctrl+u: several mails, or several conversations, at once.
    fn page(&mut self, way: Direction, cx: &mut Context<'_, Self>) {
        let step = |at: usize, count: usize| match way {
            Direction::Forward => (at + PAGE).min(count.saturating_sub(1)),
            Direction::Back => at.saturating_sub(PAGE),
        };
        match self.zone {
            Zone::Conversation => {
                let count = self.thread.as_ref().map_or(0, |thread| thread.mails.len());
                self.mail_cursor = step(self.mail_cursor, count);
            }
            Zone::List => {
                let listed: Vec<i64> = self.listed().iter().map(|summary| summary.id).collect();
                let at = self.chosen.and_then(|id| listed.iter().position(|one| *one == id)).unwrap_or(0);
                if let Some(id) = listed.get(step(at, listed.len())) {
                    self.choose(*id, cx);
                }
            }
            Zone::Mailboxes | Zone::Compose => {}
        }
    }

    fn turn_filter(&mut self, by: isize, cx: &mut Context<'_, Self>) {
        let at = Shown::ALL.iter().position(|shown| *shown == self.shown).unwrap_or(0);
        let next = Shown::ALL[(at.cast_signed() + by).rem_euclid(4).cast_unsigned()];
        self.show(next, cx);
        self.note = Some((format!("showing: {}", next.label()), Tone::Fine));
    }

    fn copy(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        self.note = Some((format!("copied {} characters", text.chars().count()), Tone::Fine));
    }

    fn undo(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(undo), Some(store)) = (self.undo.take(), &self.store) else { return };
        let done = match undo {
            Undo::Archived(id) => store.unarchive(id),
            Undo::Dropped(draft) => store.save_draft(draft.thread, &draft.author, &draft.text, draft.at_ms),
        };
        if let Err(error) = done {
            self.note = Some((format!("{error:#}"), Tone::Wrong));
        } else {
            self.note = Some(("undone".into(), Tone::Fine));
        }
        self.refresh(Read::Always);
        cx.notify();
    }

    /// The attachment `index` of the mail at the cursor: fetched (or found kept) in the
    /// background, then opened, saved to Downloads, or shown.
    fn attachment(&mut self, index: usize, then: Use, cx: &mut Context<'_, Self>) {
        let Some(mail) = self.current_mail() else { return };
        let (id, name) = (mail.id, mail.attachments.get(index).map(|file| file.name.clone()).unwrap_or_default());
        let plans: Vec<Plan> = self.plans.iter().filter_map(|(_, plan)| plan.as_ref().ok().cloned()).collect();
        self.note = Some((format!("fetching {name}…"), Tone::Fine));
        cx.spawn(async move |this, cx| {
            let file = cx.background_spawn(async move { unifyterm_mail::attachment_file(&plans, id, index) }).await;
            let _ = this.update(cx, |this, cx| {
                match (file, then) {
                    (Err(error), _) => this.note = Some((format!("{error:#}"), Tone::Wrong)),
                    (Ok(path), Use::Open) => {
                        cx.open_with_system(&path);
                        this.note = Some((format!("opened {name}"), Tone::Fine));
                    }
                    (Ok(path), Use::Save) => this.note = Some(save_to_downloads(&path, &name)),
                    (Ok(path), Use::Preview) => {
                        this.note = None;
                        this.overlay = Some(Overlay::Preview { name, path, cursor: index });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The line at the foot: the mode, where the keys go, and the keys there.
    pub(super) fn foot_line(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let mode = if self.form.is_some() { Mode::Insert } else { self.mode };
        let (word, back) = match mode {
            Mode::Normal => ("NORMAL", crate::solid(theme.ansi[4])),
            Mode::Insert => ("INSERT", crate::solid(theme.ansi[2])),
            Mode::Visual => ("VISUAL", crate::solid(theme.ansi[3])),
        };
        let place = match (&self.overlay, &self.searching, self.form.is_some()) {
            (Some(Overlay::Help), _, _) => "all keys".to_owned(),
            (Some(Overlay::Files { .. }), _, _) => "files".to_owned(),
            (Some(Overlay::Preview { name, .. }), _, _) => name.clone(),
            (Some(Overlay::Confirm { .. }), _, _) => "confirm".to_owned(),
            (None, _, true) => "add a mailbox".to_owned(),
            (None, Some(_), false) if self.typing_search() => "search".to_owned(),
            (None, _, false) => format!("[{}] {}", self.zone.number(), self.zone.name()),
        };
        let hints: Vec<(&str, &str)> = self.hints();
        // The search, while it is typed and while it keeps the list to what it found; typed here.
        let typing = self.typing_search();
        let search = self.searching.as_ref().filter(|text| typing || !text.is_empty()).map(|text| {
            let composing = if typing { self.composing.as_str() } else { "" };
            let shown = SharedString::from(format!("/ {text}{composing}"));
            let field = div().relative().min_w(px(120.)).text_color(colors.ink);
            if !typing {
                return field.child(shown);
            }
            let (view, painted, focus) = (cx.entity(), cx.entity(), self.focus.clone());
            field.child(super::compose::with_caret(shown, Some(colors.accent))).child(
                gpui::canvas(
                    move |bounds, _, cx| view.update(cx, |view, _| view.input_bounds = Some(bounds)),
                    move |bounds, (), window, cx| {
                        window.handle_input(&focus, gpui::ElementInputHandler::new(bounds, painted), cx);
                    },
                )
                .absolute()
                .size_full(),
            )
        });
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py(px(3.))
            .border_t_1()
            .border_color(colors.rule)
            .bg(crate::seen_through(theme.bar(), theme.opacity))
            .text_size(px(12.))
            .child(
                div()
                    .px_2()
                    .rounded(px(3.))
                    .bg(back)
                    .text_color(colors.back)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(11.))
                    .child(word),
            )
            .child(div().text_color(colors.muted).child(SharedString::from(place)))
            .children(search)
            .child(div().flex().gap_3().overflow_hidden().children(hints.into_iter().map(|(key, what)| {
                div()
                    .flex()
                    .gap_1()
                    .child(div().text_color(crate::solid(theme.ansi[6])).font_weight(FontWeight::SEMIBOLD).child(key))
                    .child(div().text_color(colors.muted).child(what))
            })))
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.overlay.is_some() {
            return match self.overlay {
                Some(Overlay::Files { .. }) => {
                    vec![("j/k", "choose"), ("Enter", "open"), ("s", "save"), ("p", "preview"), ("Esc", "close")]
                }
                Some(Overlay::Confirm { .. }) => vec![("y", "yes"), ("any other", "no")],
                _ => vec![("Esc", "close")],
            };
        }
        if self.form.is_some() {
            return vec![("Tab", "next field"), ("Enter", "test"), ("Esc", "close")];
        }
        if self.typing_search() {
            return vec![("Enter", "done"), ("Esc", "clear")];
        }
        match (self.mode, self.zone) {
            (Mode::Insert, _) => {
                vec![("Tab", "next part"), ("↑/↓", "choose a person"), ("Ctrl+Enter", "send"), ("Esc", "NORMAL")]
            }
            (Mode::Visual, _) => vec![("h/l w/b j/k", "select"), ("y", "copy"), (">", "quote"), ("Esc", "out")],
            (Mode::Normal, Zone::Mailboxes) => {
                vec![
                    ("j/k", "choose"),
                    ("Enter", "show"),
                    ("f", "filter"),
                    ("a", "add"),
                    ("s", "sign in"),
                    ("x", "remove"),
                    ("?", "keys"),
                ]
            }
            (Mode::Normal, Zone::List) => vec![
                ("j/k", "move"),
                ("Enter", "open"),
                ("e", "archive"),
                ("U", "unread"),
                ("r", "answer"),
                ("c", "new"),
                ("/", "search"),
                ("f", "filter"),
                ("?", "keys"),
            ],
            (Mode::Normal, Zone::Conversation) => vec![
                ("j/k", "mail"),
                ("v", "select"),
                ("y", "copy"),
                (">", "quote"),
                ("r/R", "answer"),
                ("t", "files"),
                ("S/E/D", "draft"),
                ("?", "keys"),
            ],
            (Mode::Normal, Zone::Compose) => vec![("i", "write"), ("c", "new mail"), ("f", "filter"), ("?", "keys")],
        }
    }

    /// What is over the mail, when something is.
    pub(super) fn overlay_card(&self) -> Option<gpui::AnyElement> {
        let overlay = self.overlay.as_ref()?;
        let colors = self.colors();
        let theme = &self.settings.theme;
        // A definite width (GPUI measures words that wrap on one line otherwise): the window's share, up to a
        // width that reads well; the keys take more.
        let widest = if matches!(overlay, Overlay::Help) { HELP_WIDEST } else { CARD_WIDEST };
        let card = div()
            .id("overlay-card")
            .w(px(card_width(self.widths.window, widest)))
            .max_h(gpui::relative(0.88))
            .overflow_y_scroll()
            .p_4()
            .rounded(px(theme.panel_rounding() + 2.))
            .border_1()
            .border_color(colors.edge)
            .bg(colors.panel)
            .flex()
            .flex_col()
            .gap_2();
        let card = match overlay {
            Overlay::Help => card.child(div().font_weight(FontWeight::SEMIBOLD).child("All keys")).child(self.help()),
            Overlay::Confirm { text, .. } => card
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Sure?"))
                .child(div().child(SharedString::from(text.clone())))
                .child(div().text_size(px(12.)).text_color(colors.muted).child("y: yes · any other key: no")),
            Overlay::Files { cursor } => {
                let files = self.current_mail().map(|mail| mail.attachments.clone()).unwrap_or_default();
                card.child(div().font_weight(FontWeight::SEMIBOLD).child("Files"))
                    .children(files.into_iter().enumerate().map(|(at, file)| {
                        div()
                            .flex()
                            .justify_between()
                            .gap_6()
                            .px_2()
                            .py(px(3.))
                            .rounded(px(theme.button_rounding()))
                            .when(at == *cursor, |row| row.bg(crate::solid(theme.selection)))
                            .child(SharedString::from(file.name))
                            .child(div().text_color(colors.muted).child(SharedString::from(format!(
                                "{} KB",
                                file.bytes.div_ceil(1024)
                            ))))
                    }))
                    .child(div().text_size(px(12.)).text_color(colors.muted).child(
                        "Enter opens with the system's program · s saves to Downloads · p shows it here. Fetched from the server the first time, kept after.",
                    ))
            }
            Overlay::Preview { name, path, .. } => card
                .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(name.clone())))
                .child(preview(path, colors.ink)),
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.45))
                .child(card)
                .into_any_element(),
        )
    }

    fn help(&self) -> gpui::Div {
        let colors = self.colors();
        let cyan = crate::solid(self.settings.theme.ansi[6]);
        let groups: [(&str, &[(&str, &str)]); 7] = [
            (
                "Everywhere (NORMAL)",
                &[
                    ("1 2 3 4, Tab, h l", "zones"),
                    ("j k, g G", "move"),
                    ("f F", "filter"),
                    ("/", "search"),
                    ("c", "new mail"),
                    ("q, Esc", "the terminal"),
                    ("?", "these keys"),
                ],
            ),
            (
                "[1] Mailboxes",
                &[("Enter", "show it"), ("a", "add"), ("s", "sign in again"), ("m", "change"), ("x", "remove")],
            ),
            (
                "[2] Conversations",
                &[("Enter", "open"), ("e", "archive (u undoes)"), ("U", "unread again"), ("r", "answer")],
            ),
            (
                "[3] Conversation",
                &[
                    ("j k", "mail by mail"),
                    ("o", "quoted mail"),
                    ("y", "copy the mail"),
                    ("v V", "select words, lines"),
                    ("r R", "answer, answer all"),
                    (">", "quote into the answer"),
                    ("t", "files"),
                    ("S E D", "draft: send, edit, drop"),
                ],
            ),
            ("VISUAL", &[("h l, w b, j k", "select"), ("y", "copy"), (">", "quote"), ("Esc", "out")]),
            (
                "[4] Writing (INSERT)",
                &[
                    ("i", "write"),
                    ("Tab", "To, Cc, words"),
                    ("typing in To, Cc", "the people listed"),
                    ("↑ ↓, Enter", "choose one"),
                    ("Backspace", "the last person, when nothing is typed"),
                    ("Ctrl+Enter", "send (asks first)"),
                    ("Esc", "NORMAL, the words kept"),
                ],
            ),
            ("Files", &[("j k", "choose"), ("Enter", "open"), ("s", "save to Downloads"), ("p", "preview")]),
        ];
        div().flex().flex_wrap().gap_x_8().gap_y_3().text_size(px(12.5)).children(groups.into_iter().map(
            |(title, keys)| {
                div()
                    .flex()
                    .flex_col()
                    .w(px(HELP_GROUP))
                    .child(div().text_color(colors.accent).font_weight(FontWeight::SEMIBOLD).child(title))
                    .children(keys.iter().map(|(key, what)| {
                        div()
                            .flex()
                            .gap(px(HELP_GAP))
                            .child(div().w(px(HELP_KEY)).flex_none().text_color(cyan).child(*key))
                            .child(div().w(px(HELP_GROUP - HELP_KEY - HELP_GAP)).text_color(colors.ink).child(*what))
                    }))
            },
        ))
    }
}

/// A file shown in the window: a picture as it is, text as text, anything else said.
fn preview(path: &std::path::Path, ink: gpui::Hsla) -> gpui::AnyElement {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") {
        return img(path.to_path_buf()).max_w(px(760.)).max_h(px(460.)).into_any_element();
    }
    match std::fs::read(path) {
        Ok(bytes) if !bytes.contains(&0) => {
            let text: String = String::from_utf8_lossy(&bytes).chars().take(PREVIEW_CHARS).collect();
            div()
                .id("preview-text")
                .max_h(px(460.))
                .overflow_y_scroll()
                .text_size(px(12.5))
                .text_color(ink)
                .child(SharedString::from(text))
                .into_any_element()
        }
        Ok(_) => {
            div().child("This file cannot be shown here: Enter opens it with the system's program.").into_any_element()
        }
        Err(error) => div().child(SharedString::from(format!("{error}"))).into_any_element(),
    }
}

/// Copies a fetched attachment into the Downloads folder, under a name not taken there.
fn save_to_downloads(path: &std::path::Path, name: &str) -> (String, Tone) {
    let Some(folder) = dirs::download_dir() else {
        return ("this system names no Downloads folder".into(), Tone::Wrong);
    };
    match free_name(&folder, name).map(|target| std::fs::copy(path, &target).map(|_| target)) {
        Some(Ok(target)) => (format!("saved to {}", target.display()), Tone::Fine),
        Some(Err(error)) => (format!("not saved: {error}"), Tone::Wrong),
        None => ("not saved: too many files of that name".into(), Tone::Wrong),
    }
}

/// A file in `folder` for the attachment `name` that is not there yet: the name made safe (the
/// sender chose it: it may name another folder), then "name (1).ext" and on.
fn free_name(folder: &std::path::Path, name: &str) -> Option<PathBuf> {
    let name = unifyterm_mail::safe_name(name);
    let (stem, extension) =
        name.rsplit_once('.').map_or((name.as_str(), String::new()), |(stem, ext)| (stem, format!(".{ext}")));
    (0..SAME_NAMES)
        .map(|n| folder.join(if n == 0 { name.clone() } else { format!("{stem} ({n}){extension}") }))
        .find(|candidate| !candidate.exists())
}

/// The start of the next word (or the one before), as `w` and `b` move.
fn word_step(chars: &[char], at: usize, way: Direction) -> usize {
    let word = |c: char| {
        c.is_alphanumeric() && !('\u{3000}'..='\u{303f}').contains(&c) && !('\u{ff00}'..='\u{ffef}').contains(&c)
    };
    let last = chars.len().saturating_sub(1);
    let mut i = at;
    let start = chars.get(at).copied().is_some_and(word);
    loop {
        let next = match way {
            Direction::Forward if i < last => i + 1,
            Direction::Back if i > 0 => i - 1,
            _ => return i,
        };
        i = next;
        if chars.get(i).copied().is_some_and(word) != start {
            return i;
        }
    }
}

/// The same column a line down (or up).
fn line_step(chars: &[char], at: usize, way: Direction) -> usize {
    let starts: Vec<usize> =
        std::iter::once(0).chain(chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(i, _)| i + 1)).collect();
    let line = starts.iter().rposition(|start| *start <= at).unwrap_or(0);
    let column = at - starts[line];
    let next = match way {
        Direction::Forward if line + 1 < starts.len() => line + 1,
        Direction::Back if line > 0 => line - 1,
        _ => return at,
    };
    let end = starts.get(next + 1).map_or(chars.len(), |start| start - 1);
    (starts[next] + column).min(end.saturating_sub(1).max(starts[next]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overlay_is_as_wide_as_the_window_allows_up_to_what_reads_well() {
        assert!((card_width(2560., CARD_WIDEST) - CARD_WIDEST).abs() < 0.01);
        assert!((card_width(2560., HELP_WIDEST) - HELP_WIDEST).abs() < 0.01, "the keys take more");
        assert!((card_width(600., CARD_WIDEST) - 600. * CARD_SHARE).abs() < 0.01, "a narrow window: its share");
        assert!(card_width(0., CARD_WIDEST) >= 0.);
        const { assert!(HELP_GROUP - HELP_KEY - HELP_GAP > 0., "room for what a key does") };
    }

    #[test]
    fn a_file_saved_stays_in_downloads_under_a_name_not_taken() {
        let folder = std::env::temp_dir().join(format!("unifyterm-free-name-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a temp folder");
        for evil in ["../out.txt", "/etc/out.txt", "C:\\out.txt", "..\\..\\out.txt"] {
            let target = free_name(&folder, evil).expect("a name");
            assert_eq!(target.parent(), Some(folder.as_path()), "{evil} stays in the folder");
        }
        let first = free_name(&folder, "notes.txt").expect("a name");
        assert_eq!(first, folder.join("notes.txt"));
        std::fs::write(&first, b"x").expect("written");
        assert_eq!(
            free_name(&folder, "notes.txt"),
            Some(folder.join("notes (1).txt")),
            "a name taken is not written over"
        );
        assert_eq!(free_name(&folder, ""), Some(folder.join("attachment")), "no name: still a file");
        std::fs::remove_dir_all(&folder).expect("cleaned");
    }

    #[test]
    fn zones_turn_round_and_a_selection_is_bytes_of_its_text() {
        assert_eq!(Zone::Compose.turned(1), Zone::Mailboxes);
        assert_eq!(Zone::Mailboxes.turned(-1), Zone::Compose);
        assert_eq!(Zone::List.number(), 2);
        let text = "周四 10 点";
        let visual = Visual { mail: 0, anchor: 3, caret: 0 };
        assert_eq!(&text[visual.bytes(text)], "周四 1");
        let chars: Vec<char> = "可以，周四\nok then".chars().collect();
        assert_eq!(line_step(&chars, 1, Direction::Forward), 7, "a line down, the same column");
        assert_eq!(line_step(&chars, 7, Direction::Back), 1);
        let words: Vec<char> = "hello big world".chars().collect();
        assert_eq!(word_step(&words, 0, Direction::Forward), 5);
        assert_eq!(word_step(&words, 6, Direction::Back), 5);
    }
}
