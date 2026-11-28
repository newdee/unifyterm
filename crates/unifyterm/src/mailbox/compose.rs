//! The mail being written, at the foot of the mail: an answer to the conversation shown, or a
//! new mail. To and Cc are chips; typing a name or an address in them lists the people the
//! mailboxes have written with. A new mail has a subject. Nothing is sent until the person says
//! yes to what the confirmation shows.

use gpui::{Context, ElementInputHandler, FontWeight, Hsla, SharedString, canvas, div, prelude::*, px};
use unifyterm_mail::{Address, Contact, Plan, Recipients};

use super::keys::{Mode, Overlay, Pending, Zone};
use super::{MailView, Read, Tone};

/// How many people the list under To or Cc shows.
const SUGGESTIONS: usize = 5;

/// What is being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Reply,
    New,
}

/// A part of the mail being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    To,
    Cc,
    Subject,
    Body,
}

/// The mail being written.
#[derive(Debug, Clone)]
pub(super) struct Compose {
    pub(super) kind: Kind,
    /// The conversation an answer answers.
    pub(super) thread: Option<i64>,
    pub(super) to: Vec<Address>,
    pub(super) cc: Vec<Address>,
    pub(super) subject: String,
    pub(super) body: String,
    pub(super) part: Part,
    /// A name or an address being typed in To or Cc.
    pub(super) typed: String,
    /// The person chosen in the list under it.
    pub(super) suggestion: usize,
}

impl Default for Compose {
    fn default() -> Self {
        Self {
            kind: Kind::Reply,
            thread: None,
            to: Vec::new(),
            cc: Vec::new(),
            subject: String::new(),
            body: String::new(),
            part: Part::Body,
            typed: String::new(),
            suggestion: 0,
        }
    }
}

impl Compose {
    /// The parts in order: a new mail has a subject, an answer takes its conversation's.
    pub(super) fn parts(&self) -> &'static [Part] {
        match self.kind {
            // Nothing started: only the words, with how to start.
            Kind::Reply if self.thread.is_none() => &[Part::Body],
            Kind::Reply => &[Part::To, Part::Cc, Part::Body],
            Kind::New => &[Part::To, Part::Cc, Part::Subject, Part::Body],
        }
    }

    pub(super) fn next_part(&mut self, back: super::keys::Direction) {
        let parts = self.parts();
        let at = parts.iter().position(|part| *part == self.part).unwrap_or(0);
        let next = match back {
            super::keys::Direction::Forward => (at + 1) % parts.len(),
            super::keys::Direction::Back => (at + parts.len() - 1) % parts.len(),
        };
        self.part = parts[next];
        self.suggestion = 0;
    }

    /// The text typing goes into now.
    pub(super) fn text_mut(&mut self) -> &mut String {
        match self.part {
            Part::To | Part::Cc => &mut self.typed,
            Part::Subject => &mut self.subject,
            Part::Body => &mut self.body,
        }
    }

    fn chips_mut(&mut self) -> Option<&mut Vec<Address>> {
        match self.part {
            Part::To => Some(&mut self.to),
            Part::Cc => Some(&mut self.cc),
            Part::Subject | Part::Body => None,
        }
    }

    /// The people whose name or address has what is typed, not chosen yet: most written with first.
    pub(super) fn suggestions<'a>(&self, contacts: &'a [Contact]) -> Vec<&'a Contact> {
        let typed = self.typed.trim().to_lowercase();
        if typed.is_empty() || !matches!(self.part, Part::To | Part::Cc) {
            return Vec::new();
        }
        contacts
            .iter()
            .filter(|contact| !self.to.iter().chain(&self.cc).any(|chosen| chosen.addr == contact.addr))
            .filter(|contact| contact.name.to_lowercase().contains(&typed) || contact.addr.contains(&typed))
            .take(SUGGESTIONS)
            .collect()
    }

    /// The person chosen in the list, or the address typed when it is one, becomes a chip.
    pub(super) fn take(&mut self, contacts: &[Contact]) -> bool {
        let chosen =
            self.suggestions(contacts).get(self.suggestion).map(|contact| Address::new(&contact.name, &contact.addr));
        let typed = self.typed.trim().to_owned();
        let address = chosen.or_else(|| typed.contains('@').then(|| Address::new("", &typed)));
        let Some(address) = address else { return false };
        if let Some(chips) = self.chips_mut() {
            chips.push(address);
        }
        self.typed.clear();
        self.suggestion = 0;
        true
    }

    /// Backspace: a character, or in To and Cc with nothing typed, the last person.
    pub(super) fn backspace(&mut self) {
        if matches!(self.part, Part::To | Part::Cc) && self.typed.is_empty() {
            if let Some(chips) = self.chips_mut() {
                chips.pop();
            }
        } else {
            self.text_mut().pop();
        }
        self.suggestion = 0;
    }

    /// Whether this is the mail `other` was: the same kind, conversation, people and words.
    pub(super) fn same_mail(&self, other: &Self) -> bool {
        (self.kind, self.thread, &self.to, &self.cc, &self.subject, &self.body)
            == (other.kind, other.thread, &other.to, &other.cc, &other.subject, &other.body)
    }

    pub(super) fn recipients(&self) -> Recipients {
        Recipients { to: self.to.clone(), cc: self.cc.clone() }
    }
}

impl MailView {
    /// Answers the conversation shown: to its other person, and with `everyone` the others of its
    /// newest mail in Cc. Words written for this conversation before stay.
    pub(super) fn start_reply(&mut self, everyone: super::keys::Everyone) {
        let Some(thread) = &self.thread else { return };
        let me = thread.summary.account.clone();
        let Ok(answer) = unifyterm_mail::send::answer(thread, &me, "") else { return };
        let same = self.compose.kind == Kind::Reply && self.compose.thread == Some(thread.summary.id);
        let body = if same { std::mem::take(&mut self.compose.body) } else { String::new() };
        self.compose = Compose {
            kind: Kind::Reply,
            thread: Some(thread.summary.id),
            to: answer.to,
            cc: if everyone == super::keys::Everyone::All { answer.cc } else { Vec::new() },
            body,
            ..Compose::default()
        };
        self.zone = Zone::Compose;
        self.mode = Mode::Insert;
    }

    /// A new mail, from the mailbox shown (or the first).
    pub(super) fn start_new(&mut self) {
        self.compose = Compose { kind: Kind::New, part: Part::To, ..Compose::default() };
        self.zone = Zone::Compose;
        self.mode = Mode::Insert;
    }

    /// `text` quoted (each line after "> ") at the head of the answer, which starts when there is none.
    pub(super) fn quote(&mut self, text: &str) {
        if self.compose.kind != Kind::Reply || self.compose.thread != self.thread.as_ref().map(|t| t.summary.id) {
            self.start_reply(super::keys::Everyone::One);
        }
        let quoted = text.lines().fold(String::new(), |mut quoted, line| {
            quoted.push_str("> ");
            quoted.push_str(line);
            quoted.push('\n');
            quoted
        });
        self.compose.body = format!("{quoted}\n{}", self.compose.body);
        self.compose.part = Part::Body;
        self.zone = Zone::Compose;
        self.mode = Mode::Insert;
        self.note = Some(("quoted into the answer".into(), Tone::Fine));
    }

    /// Asks before sending what is written: to whom it goes is shown.
    pub(super) fn ask_send(&mut self) {
        let c = &self.compose;
        if c.to.is_empty() {
            self.note = Some(("name someone in To first".into(), Tone::Wrong));
            return;
        }
        if c.body.trim().is_empty() {
            self.note = Some(("there are no words to send".into(), Tone::Wrong));
            return;
        }
        let names = |list: &[Address]| list.iter().map(|a| a.shown().to_owned()).collect::<Vec<_>>().join(", ");
        let cc = if c.cc.is_empty() { String::new() } else { format!(", Cc {}", names(&c.cc)) };
        self.overlay =
            Some(Overlay::Confirm { text: format!("Send to {}{cc}?", names(&c.to)), then: Pending::SendCompose });
    }

    /// Sends what is written, in the background: an answer from its conversation's mailbox, a
    /// new mail from the mailbox shown.
    pub(super) fn send_compose(&mut self, cx: &mut Context<'_, Self>) {
        let plans: Vec<Plan> = self.plans.iter().filter_map(|(_, plan)| plan.as_ref().ok().cloned()).collect();
        let c = self.compose.clone();
        let from = self
            .showing
            .as_ref()
            .and_then(|address| plans.iter().find(|plan| &plan.account.address == address))
            .or_else(|| plans.first())
            .cloned();
        if self.sending {
            return;
        }
        self.sending = true;
        self.note = Some(("sending…".into(), Tone::Fine));
        // What was sent, to clear once it went: not what was started while it went.
        let went = c.clone();
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_spawn(async move {
                    match (c.kind, c.thread, from) {
                        (Kind::Reply, Some(thread), _) => {
                            unifyterm_mail::send_answer_to(&plans, thread, &c.body, Some(&c.recipients()))
                        }
                        (Kind::New, _, Some(plan)) => {
                            unifyterm_mail::send_new(&plan, &c.recipients(), &c.subject, &c.body)
                        }
                        _ => Err(anyhow::anyhow!("no mailbox to send from")),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.sending = false;
                match sent {
                    Ok(()) => {
                        if this.compose.same_mail(&went) {
                            this.compose = Compose::default();
                            if this.zone == Zone::Compose {
                                this.mode = Mode::Normal;
                            }
                        }
                        this.note = Some(("sent".into(), Tone::Fine));
                    }
                    // What was written stays: nothing is lost, and nothing is sent twice by itself.
                    Err(error) => this.note = Some((format!("not sent: {error:#}"), Tone::Wrong)),
                }
                this.refresh(Read::Always);
                this.stream.scroll_to_bottom();
                cx.notify();
            });
        })
        .detach();
    }

    /// The mail being written: its parts, the list under To or Cc, what the last press did.
    pub(super) fn compose_area(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let c = &self.compose;
        let note = self.note.clone().map(|(text, tone)| {
            div()
                .px_1()
                .text_size(px(12.))
                .text_color(if tone == Tone::Wrong { colors.wrong } else { colors.muted })
                .child(SharedString::from(text))
        });
        let mut area = div().flex_none().mx_5().mb_3().flex().flex_col().gap_1().children(note);
        for part in c.parts() {
            area = area.child(self.part_row(*part, cx));
        }
        // Started: Send, for the mouse, as Ctrl+Enter; it asks first.
        if c.thread.is_some() || c.kind == Kind::New {
            let send = self.button(
                "send",
                if self.sending { "Sending…" } else { "Send" },
                super::Weight::Strong,
                cx.listener(|this, _, _, cx| {
                    this.ask_send();
                    cx.notify();
                }),
            );
            area = area.child(div().flex().justify_end().child(send));
        }
        area
    }

    /// One part of the mail being written.
    fn part_row(&self, part: Part, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let c = &self.compose;
        let active = self.mode == Mode::Insert
            && self.zone == Zone::Compose
            && self.form.is_none()
            && !self.typing_search()
            && c.part == part;
        let (label, chips, text): (&str, &[Address], &str) = match part {
            Part::To => ("To", &c.to, if c.part == Part::To { &c.typed } else { "" }),
            Part::Cc => ("Cc", &c.cc, if c.part == Part::Cc { &c.typed } else { "" }),
            Part::Subject => ("Subject", &[], &c.subject),
            Part::Body => ("", &[], &c.body),
        };
        let composing = if active { self.composing.as_str() } else { "" };
        let started = self.zone == Zone::Compose || c.thread.is_some() || c.kind == Kind::New;
        let hint = match (part, started) {
            (Part::Body, true) if !active => "i writes · Ctrl+Enter sends",
            (Part::Body, false) if !active => "r answers · c writes a new mail",
            _ => "",
        };
        let shown: SharedString =
            if text.is_empty() && composing.is_empty() { hint.into() } else { format!("{text}{composing}").into() };
        let body = part == Part::Body;
        let mut row = div()
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_2()
            .py(px(4.))
            .when(body, |row| row.min_h(px(52.)).max_h(px(180.)).items_start())
            .rounded(px(theme.panel_rounding() + 2.))
            .border_1()
            .border_color(if active { colors.accent } else { colors.edge })
            .bg(colors.panel)
            .text_size(px(13.5))
            .when(!label.is_empty(), |row| {
                row.child(div().w(px(56.)).flex_none().text_size(px(11.5)).text_color(colors.muted).child(label))
            })
            .children(chips.iter().map(|address| {
                div()
                    .px_2()
                    .rounded_full()
                    .border_1()
                    .border_color(colors.rule)
                    .bg(colors.well)
                    .text_size(px(12.))
                    .child(SharedString::from(address.shown().to_owned()))
            }))
            .child(
                div()
                    .when(body, gpui::Styled::w_full)
                    .text_color(if text.is_empty() && composing.is_empty() { colors.muted } else { colors.ink })
                    .child(with_caret(shown, active.then_some(colors.accent))),
            );
        if active {
            let (view, painted, focus) = (cx.entity(), cx.entity(), self.focus.clone());
            row = row.child(
                canvas(
                    move |bounds, _, cx| view.update(cx, |view, _| view.input_bounds = Some(bounds)),
                    move |bounds, (), window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx);
                    },
                )
                .absolute()
                .size_full(),
            );
            if matches!(part, Part::To | Part::Cc) {
                row = row.children(self.suggestion_list(colors.ink, colors.muted));
            }
        }
        row.into_any_element()
    }

    /// The people that match what is typed in To or Cc, under it.
    fn suggestion_list(&self, ink: Hsla, muted: Hsla) -> Option<gpui::Div> {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let found = self.compose.suggestions(&self.contacts);
        if found.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .bottom_full()
                .left(px(64.))
                .min_w(px(320.))
                .mb_1()
                .p_1()
                .rounded(px(theme.panel_rounding()))
                .border_1()
                .border_color(colors.edge)
                .bg(colors.panel)
                .flex()
                .flex_col()
                .children(found.into_iter().enumerate().map(|(at, contact)| {
                    let name = if contact.name.is_empty() { contact.addr.clone() } else { contact.name.clone() };
                    div()
                        .flex()
                        .justify_between()
                        .gap_4()
                        .px_2()
                        .py(px(3.))
                        .rounded(px(theme.button_rounding()))
                        .when(at == self.compose.suggestion, |row| row.bg(solid_selection(self)))
                        .text_size(px(12.5))
                        .text_color(ink)
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(name)))
                        .child(
                            div()
                                .text_color(muted)
                                .text_size(px(11.5))
                                .child(SharedString::from(format!("{} · {} mails", contact.addr, contact.count))),
                        )
                })),
        )
    }
}

fn solid_selection(view: &MailView) -> Hsla {
    crate::solid(view.settings.theme.selection)
}

/// Text with the caret after it, in the line of the text (a caret apart would wrap to a line of
/// its own after text that fills its row).
pub(super) fn with_caret(text: SharedString, caret: Option<Hsla>) -> gpui::AnyElement {
    let Some(color) = caret else {
        return text.into_any_element();
    };
    let shown = format!("{text}\u{258f}");
    let caret = text.len()..shown.len();
    let marked = gpui::HighlightStyle { color: Some(color), ..gpui::HighlightStyle::default() };
    gpui::StyledText::new(shown).with_highlights(vec![(caret, marked)]).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::keys::Direction;

    fn person(name: &str, addr: &str, count: usize) -> Contact {
        Contact { name: name.into(), addr: addr.into(), count, last_at_ms: 0 }
    }

    #[test]
    fn people_are_listed_chosen_and_taken_back() {
        let contacts = [person("林晓", "lin@example.com", 6), person("Alex Chen", "alex@example.com", 9)];
        let mut c = Compose { kind: Kind::New, part: Part::To, ..Compose::default() };
        assert_eq!(c.parts(), &[Part::To, Part::Cc, Part::Subject, Part::Body]);
        assert!(c.suggestions(&contacts).is_empty(), "nothing typed, nobody listed");
        c.typed = "LIN".into();
        assert_eq!(c.suggestions(&contacts).len(), 1, "names match whatever the case");
        assert!(c.take(&contacts));
        assert_eq!(c.to.first().map(|a| a.addr.as_str()), Some("lin@example.com"));
        c.typed = "lin".into();
        assert!(c.suggestions(&contacts).is_empty(), "someone chosen is not listed again");
        assert!(!c.take(&contacts), "neither a person listed nor an address");
        c.typed = "new@example.net".into();
        assert!(c.take(&contacts), "an address typed is taken as it is");
        assert_eq!(c.to.len(), 2);
        c.backspace();
        assert_eq!(c.to.len(), 1, "Backspace with nothing typed takes the last person away");
        c.next_part(Direction::Back);
        assert_eq!(c.part, Part::Body, "Shift+Tab from To goes round to the words");
        c.backspace();
        assert_eq!(c.to.len(), 1, "Backspace in the words takes no person");
        let went = c.clone();
        assert!(c.same_mail(&went));
        let started = Compose { kind: Kind::New, part: Part::To, ..Compose::default() };
        assert!(!started.same_mail(&went), "a mail started while one went is not the one that went");
        let answer = Compose::default();
        assert_eq!(answer.parts(), &[Part::Body], "nothing started: only the words");
        let answer = Compose { thread: Some(3), ..Compose::default() };
        assert_eq!(answer.parts(), &[Part::To, Part::Cc, Part::Body], "an answer has its conversation's subject");
    }
}
