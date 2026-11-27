//! The form that adds a mailbox, changes one, or signs one in again: in place of the
//! conversation, with the list left where it is. The address says which servers and which
//! way of logging in are likely (an authorization code for QQ, a sign-in in the browser for
//! Outlook); the person may choose another. "Test" tries the mailbox the way its sync will
//! (connect, log in, find the sent folder, log in to SMTP), step by step, and only when all
//! went is anything kept: the settings file, and the password or sign-in in the keychain.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{Context, ElementInputHandler, FontWeight, Hsla, SharedString, Window, canvas, div, prelude::*, px};
use unifyterm_config::{MailAccount, MailAuth, MailConfig, MailboxSettings};
use unifyterm_mail::oauth::{self, DeviceCode, Tokens};
use unifyterm_mail::{Account, Auth, Credential, Plan, Step};

use super::{MailEvent, MailView, Read, Tone, Weight};

/// What the form is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Purpose {
    Add,
    /// The mailbox of this name: its address, servers, proxy.
    Change(String),
    /// The mailbox of this name: a new password, or a new sign-in.
    SignIn(String),
}

/// How the mailbox logs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    /// An authorization code or app password, at known servers.
    Code,
    /// A sign-in in the browser (`OAuth2`).
    Browser,
    /// Servers given by hand, with a password.
    Manual,
    /// A program that prints the password.
    Command,
}

impl Way {
    const ALL: [Self; 4] = [Self::Code, Self::Browser, Self::Manual, Self::Command];

    fn title(self) -> &'static str {
        match self {
            Self::Code => "Authorization code / app password",
            Self::Browser => "Sign in in the browser",
            Self::Manual => "Servers by hand",
            Self::Command => "A program gives the password",
        }
    }

    fn says(self) -> &'static str {
        match self {
            Self::Code => "QQ, 163, 126, Gmail, iCloud and others",
            Self::Browser => "Outlook, Hotmail, work mail at Microsoft",
            Self::Manual => "another domain, your own server",
            Self::Command => "pass, 1Password and the like",
        }
    }
}

/// How the servers are reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Direct,
    Proxy,
}

/// A text field of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Address,
    Secret,
    Imap,
    Smtp,
    Login,
    Command,
    Proxy,
    Name,
}

impl Field {
    const COUNT: usize = 8;

    const fn at(self) -> usize {
        match self {
            Self::Address => 0,
            Self::Secret => 1,
            Self::Imap => 2,
            Self::Smtp => 3,
            Self::Login => 4,
            Self::Command => 5,
            Self::Proxy => 6,
            Self::Name => 7,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Address => "Mail address",
            Self::Secret => "Authorization code or app password",
            Self::Imap => "IMAP server",
            Self::Smtp => "SMTP server",
            Self::Login => "Login name",
            Self::Command => "The program that prints the password",
            Self::Proxy => "Proxy",
            Self::Name => "Name (shown at the top left)",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Address => "me@qq.com, me@outlook.com, me@gmail.com…",
            Self::Secret => "not the web password",
            Self::Imap => "imap.example.com:993",
            Self::Smtp => "smtp.example.com:465 or :587",
            Self::Login => "empty: the address",
            Self::Command => "pass show mail/work",
            Self::Proxy => "socks5://127.0.0.1:7890 or http://127.0.0.1:7890",
            Self::Name => "automatic",
        }
    }
}

/// Where the form is.
#[derive(Debug, Clone)]
enum Stage {
    Filling,
    /// Waiting for the person to sign in, with the code once Microsoft gave it.
    SigningIn(Option<DeviceCode>),
    Testing,
}

/// What lets the mailbox in, for the test.
enum Given {
    Typed(String),
    /// What is kept already, or what the program prints.
    Kept,
    Token(Tokens),
}

type Steps = Arc<Mutex<Vec<(Step, Result<String, String>)>>>;

/// The form, filled in.
pub(super) struct Form {
    purpose: Purpose,
    way: Way,
    /// The way was chosen by hand: another address does not change it.
    way_chosen: bool,
    via: Via,
    text: [String; Field::COUNT],
    focused: Field,
    stage: Stage,
    tokens: Option<Tokens>,
    steps: Steps,
    said: Option<(String, Tone)>,
    /// Ends a sign-in still waiting when the form goes.
    cancel: Arc<AtomicBool>,
}

impl Drop for Form {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Form {
    fn new(purpose: Purpose, mailbox: Option<&MailboxSettings>) -> Self {
        let mut form = Self {
            purpose,
            way: Way::Code,
            way_chosen: false,
            via: Via::Direct,
            text: Default::default(),
            focused: Field::Address,
            stage: Stage::Filling,
            tokens: None,
            steps: Arc::new(Mutex::new(Vec::new())),
            said: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        if let Some(mailbox) = mailbox {
            form.text[Field::Address.at()].clone_from(&mailbox.address);
            form.text[Field::Imap.at()].clone_from(&mailbox.imap);
            form.text[Field::Smtp.at()].clone_from(&mailbox.smtp);
            form.text[Field::Login.at()].clone_from(&mailbox.login);
            form.text[Field::Command.at()] = mailbox.password_command.join(" ");
            form.text[Field::Proxy.at()].clone_from(&mailbox.proxy);
            if !mailbox.proxy.trim().is_empty() {
                form.via = Via::Proxy;
            }
            form.way = if !mailbox.password_command.is_empty() {
                Way::Command
            } else if !mailbox.imap.trim().is_empty() && mailbox.auth != MailAuth::Oauth2 {
                Way::Manual
            } else {
                form.recommended()
            };
            form.way_chosen = true;
            form.focused = form.fields().first().copied().unwrap_or(Field::Address);
        }
        form
    }

    pub(super) fn typed_mut(&mut self) -> &mut String {
        &mut self.text[self.focused.at()]
    }

    fn text(&self, field: Field) -> &str {
        self.text[field.at()].trim()
    }

    fn adding(&self) -> bool {
        self.purpose == Purpose::Add
    }

    fn signing_in(&self) -> bool {
        matches!(self.purpose, Purpose::SignIn(_))
    }

    /// The account the address alone gives: its servers and its way in, when the domain is known.
    fn found(&self) -> Option<Account> {
        let address = self.text(Field::Address);
        if !address.contains('@') {
            return None;
        }
        Account::new(address, "", "", "", "", MailAuth::Auto).ok()
    }

    /// The way the address makes likely.
    fn recommended(&self) -> Way {
        if !self.text(Field::Address).contains('@') {
            return Way::Code;
        }
        match self.found() {
            Some(account) if account.auth == Auth::OAuth2 => Way::Browser,
            Some(_) => Way::Code,
            None => Way::Manual,
        }
    }

    /// The way in use: the one chosen, else the likely one.
    fn way(&self) -> Way {
        if self.way_chosen { self.way } else { self.recommended() }
    }

    /// The fields shown, in order: what the purpose and the way need.
    fn fields(&self) -> Vec<Field> {
        let way = self.way();
        let mut fields = Vec::new();
        if !self.signing_in() {
            fields.push(Field::Address);
        }
        match way {
            Way::Code => fields.push(Field::Secret),
            Way::Browser => {}
            Way::Manual if self.signing_in() => fields.push(Field::Secret),
            Way::Manual => fields.extend([Field::Imap, Field::Smtp, Field::Login, Field::Secret]),
            Way::Command if self.signing_in() => {}
            Way::Command => fields.push(Field::Command),
        }
        if !self.signing_in() && self.via == Via::Proxy {
            fields.push(Field::Proxy);
        }
        if self.adding() {
            fields.push(Field::Name);
        }
        fields
    }

    /// The next field shown, or the one before.
    fn move_focus(&mut self, back: Direction) {
        let fields = self.fields();
        let at = fields.iter().position(|field| *field == self.focused).unwrap_or(0);
        let next = match back {
            Direction::Forward => (at + 1) % fields.len().max(1),
            Direction::Back => (at + fields.len().max(1) - 1) % fields.len().max(1),
        };
        if let Some(field) = fields.get(next) {
            self.focused = *field;
        }
    }

    /// The mailbox as the settings will have it.
    fn account(&self) -> MailAccount {
        let way = self.way();
        let by_hand = way == Way::Manual;
        MailAccount {
            address: self.text(Field::Address).to_lowercase(),
            imap: if by_hand { self.text(Field::Imap).to_owned() } else { String::new() },
            smtp: if by_hand { self.text(Field::Smtp).to_owned() } else { String::new() },
            login: if by_hand { self.text(Field::Login).to_owned() } else { String::new() },
            password_command: if way == Way::Command {
                self.text(Field::Command).split_whitespace().map(str::to_owned).collect()
            } else {
                Vec::new()
            },
            proxy: if self.via == Via::Proxy { self.text(Field::Proxy).to_owned() } else { String::new() },
            days: None,
            auth: if way == Way::Browser { MailAuth::Oauth2 } else { MailAuth::Auto },
        }
    }

    /// The mailbox's name: the one it has; for a new one, the one typed, else its domain's
    /// first word (`qq`), made different from the names there are.
    fn name(&self, settings: &MailConfig) -> String {
        if let Purpose::Change(name) | Purpose::SignIn(name) = &self.purpose {
            return name.clone();
        }
        let typed = self.text(Field::Name).to_lowercase();
        let base = if typed.is_empty() {
            let domain = self.text(Field::Address).rsplit_once('@').map_or("", |(_, domain)| domain).to_lowercase();
            let word: String = domain.split('.').next().unwrap_or_default().to_owned();
            if unifyterm_config::is_mailbox_name(&word) { word } else { "mail".to_owned() }
        } else {
            typed
        };
        let taken: Vec<String> = settings.mailboxes().into_iter().map(|mailbox| mailbox.name).collect();
        if !taken.contains(&base) {
            return base;
        }
        // Of len + 1 names at least one is not taken.
        (2..=taken.len() + 2).map(|n| format!("{base}-{n}")).find(|name| !taken.contains(name)).unwrap_or(base)
    }

    /// Where to get what the Secret field takes, for the address's domain.
    fn hint(&self) -> &'static str {
        let domain =
            self.text(Field::Address).rsplit_once('@').map_or(String::new(), |(_, domain)| domain.to_lowercase());
        match domain.as_str() {
            "qq.com" | "foxmail.com" | "vip.qq.com" => {
                "QQ Mail on the web: Settings → Account → POP3/IMAP/SMTP. Turn IMAP on; the authorization code it gives goes here, not the web password."
            }
            "163.com" | "126.com" | "yeah.net" => {
                "NetEase Mail on the web: Settings → POP3/SMTP/IMAP. Turn IMAP on; the authorization code goes here."
            }
            "gmail.com" | "googlemail.com" => {
                "Google Account: turn on 2-Step Verification, then Security → App passwords. In mainland China it needs a proxy."
            }
            "icloud.com" | "me.com" | "mac.com" => "account.apple.com: Sign-In and Security → App-Specific Passwords.",
            _ => "The password your provider gives mail programs: often an app password, not the web password.",
        }
    }

    /// The same, as a command in a shell.
    fn command_line(&self, name: &str) -> String {
        match &self.purpose {
            Purpose::SignIn(name) => format!("ut mail login --account {name}"),
            Purpose::Change(_) => String::new(),
            Purpose::Add => {
                let address = if self.text(Field::Address).is_empty() { "ADDRESS" } else { self.text(Field::Address) };
                let account = self.account();
                let mut line = format!("ut mail account add {name} {address}");
                for (flag, value) in [("--imap", &account.imap), ("--smtp", &account.smtp), ("--proxy", &account.proxy)]
                {
                    if !value.is_empty() {
                        let _ = write!(line, " {flag} {value}");
                    }
                }
                if account.auth == MailAuth::Oauth2 && self.recommended() != Way::Browser {
                    line.push_str(" --auth oauth2");
                }
                line
            }
        }
    }

    /// What lets the mailbox in for the test, or what is missing.
    fn given(&self) -> Result<Given, &'static str> {
        match self.way() {
            Way::Browser => self.tokens.clone().map(Given::Token).ok_or("sign in in the browser first"),
            Way::Command => Ok(Given::Kept),
            Way::Code | Way::Manual => match self.text(Field::Secret) {
                "" if self.adding() => Err("type the authorization code or app password"),
                "" => Ok(Given::Kept),
                typed => Ok(Given::Typed(typed.to_owned())),
            },
        }
    }
}

/// Which way Tab goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Forward,
    Back,
}

impl MailView {
    /// Opens the form, in place of the conversation.
    pub(crate) fn open_form(&mut self, purpose: Purpose, window: &mut Window, cx: &mut Context<'_, Self>) {
        let mailbox = match &purpose {
            Purpose::Add => None,
            Purpose::Change(name) | Purpose::SignIn(name) => {
                self.plans.iter().find(|(mailbox, _)| &mailbox.name == name).map(|(mailbox, _)| mailbox)
            }
        };
        self.form = Some(Form::new(purpose, mailbox));
        self.menu = None;
        self.composing.clear();
        window.focus(&self.focus);
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<'_, Self>) {
        self.form = None;
        self.composing.clear();
        if self.plans.is_empty() {
            cx.emit(MailEvent::Leave);
        }
        cx.notify();
    }

    /// A key in the form; whether it was the form's.
    pub(super) fn form_key(
        &mut self,
        keys: &gpui::Keystroke,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(form) = &mut self.form else { return false };
        match keys.key.as_str() {
            "escape" => self.close_form(cx),
            "enter" => self.submit(cx),
            "tab" if keys.modifiers.shift => form.move_focus(Direction::Back),
            "tab" => form.move_focus(Direction::Forward),
            "backspace" => {
                form.typed_mut().pop();
            }
            _ => return false,
        }
        true
    }

    /// Tries the mailbox in the background; when all went, keeps it.
    fn submit(&mut self, cx: &mut Context<'_, Self>) {
        let settings = self.settings.config.mail.clone();
        let path = self.settings_path.clone();
        let Some(form) = &mut self.form else { return };
        if !matches!(form.stage, Stage::Filling) {
            return;
        }
        let name = form.name(&settings);
        let prepared = prepare(form, &name, path.as_deref());
        let (file, plan) = match prepared {
            Ok(prepared) => prepared,
            Err(why) => {
                form.said = Some((why, Tone::Wrong));
                cx.notify();
                return;
            }
        };
        if plan.account.auth == Auth::OAuth2 && form.tokens.is_none() {
            self.sign_in(&plan, cx);
            return;
        }
        let given = match form.given() {
            Ok(given) => given,
            Err(why) => {
                form.said = Some((why.to_owned(), Tone::Wrong));
                cx.notify();
                return;
            }
        };
        form.stage = Stage::Testing;
        form.said = None;
        form.steps.lock().unwrap_or_else(PoisonError::into_inner).clear();
        let steps = Arc::clone(&form.steps);
        let save = if form.signing_in() { Save::Not } else { Save::File };
        let purpose = form.purpose.clone();
        let address = plan.account.address.clone();
        Self::tick(cx);
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { try_and_keep(&plan, given, &steps, file, save) }).await;
            let _ = this.update(cx, |this, cx| {
                this.tested(done, &purpose, address, &name, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// The test is over: the form goes when all went, else it says why and stays.
    fn tested(
        &mut self,
        done: Result<(), String>,
        purpose: &Purpose,
        address: String,
        name: &str,
        cx: &mut Context<'_, Self>,
    ) {
        match done {
            Ok(()) => {
                self.form = None;
                match purpose {
                    Purpose::Add => {
                        // The new mailbox, with nothing chosen in it yet: its mail is still coming.
                        self.shown = super::Shown::All;
                        self.show_mailbox(Some(address), cx);
                        self.note = Some((format!("added {name}: its mail is being read"), Tone::Fine));
                    }
                    Purpose::SignIn(_) => {
                        self.note = Some((format!("{name} is let in again"), Tone::Fine));
                        cx.emit(MailEvent::SignedIn);
                    }
                    Purpose::Change(_) => self.note = Some((format!("{name} is changed"), Tone::Fine)),
                }
                self.refresh(Read::Always);
            }
            Err(why) => {
                if let Some(form) = &mut self.form {
                    form.stage = Stage::Filling;
                    form.said = Some((why, Tone::Wrong));
                }
            }
        }
        cx.notify();
    }

    /// Asks Microsoft for a code, opens the browser at its page, and waits for the sign-in; then the test.
    fn sign_in(&mut self, plan: &Plan, cx: &mut Context<'_, Self>) {
        let Some(form) = &mut self.form else { return };
        form.stage = Stage::SigningIn(None);
        form.said = None;
        let cancel = Arc::clone(&form.cancel);
        let provider = plan.provider.clone();
        let proxy = plan.account.proxy.clone();
        Self::tick(cx);
        cx.spawn(async move |this, cx| {
            let (asking, through) = (provider.clone(), proxy.clone());
            let code = cx.background_spawn(async move { oauth::start(&asking, through.as_ref()) }).await;
            let code = match code {
                Ok(code) => code,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.signed_in(Err(format!("{error:#}")), cx));
                    return;
                }
            };
            let shown = code.clone();
            let _ = this.update(cx, |this, cx| {
                if let Some(form) = &mut this.form {
                    form.stage = Stage::SigningIn(Some(shown.clone()));
                    cx.open_url(&shown.verification_uri);
                }
                cx.notify();
            });
            let tokens = cx
                .background_spawn(async move {
                    oauth::finish(&provider, proxy.as_ref(), &code, &unifyterm_history::now_ms, &|| {
                        cancel.load(Ordering::Relaxed)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| this.signed_in(tokens.map_err(|refusal| refusal.to_string()), cx));
        })
        .detach();
        cx.notify();
    }

    fn signed_in(&mut self, tokens: Result<Tokens, String>, cx: &mut Context<'_, Self>) {
        let Some(form) = &mut self.form else { return };
        form.stage = Stage::Filling;
        match tokens {
            Ok(tokens) => {
                form.tokens = Some(tokens);
                self.submit(cx);
            }
            Err(why) => form.said = Some((format!("not signed in: {why}"), Tone::Wrong)),
        }
        cx.notify();
    }

    /// Draws the form again and again while it waits: the steps and the sign-in come from other threads.
    fn tick(cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let waiting = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        this.form.as_ref().is_some_and(|form| !matches!(form.stage, Stage::Filling))
                    })
                    .unwrap_or(false);
                if !waiting {
                    break;
                }
            }
        })
        .detach();
    }

    fn choose_way(&mut self, way: Way, cx: &mut Context<'_, Self>) {
        if let Some(form) = &mut self.form {
            form.way = way;
            form.way_chosen = true;
            if !form.fields().contains(&form.focused) {
                form.focused = Field::Address;
            }
        }
        cx.notify();
    }

    fn choose_via(&mut self, via: Via, cx: &mut Context<'_, Self>) {
        if let Some(form) = &mut self.form {
            form.via = via;
            if via == Via::Proxy {
                form.focused = Field::Proxy;
            } else if form.focused == Field::Proxy {
                form.focused = Field::Address;
            }
        }
        cx.notify();
    }

    fn focus_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(form) = &mut self.form {
            form.focused = field;
        }
        self.composing.clear();
        window.focus(&self.focus);
        cx.notify();
    }
}

/// The settings file with the mailbox in it (not saved yet), and the mailbox's plan.
fn prepare(form: &Form, name: &str, path: Option<&std::path::Path>) -> Result<(unifyterm_config::File, Plan), String> {
    if !form.signing_in() && !form.text(Field::Address).contains('@') {
        return Err("type the mail address".to_owned());
    }
    let path = path.ok_or("UnifyTerm has no settings file to keep the mailbox in")?;
    let mut file = unifyterm_config::File::open(path).map_err(|error| format!("{error:#}"))?;
    if !form.signing_in() {
        file.set_mailbox(name, &form.account()).map_err(|error| format!("{error:#}"))?;
    }
    let config = unifyterm_config::Config::parse(&file.text()).map_err(|error| format!("{error:#}"))?;
    let mailbox = config
        .mail
        .mailboxes()
        .into_iter()
        .find(|mailbox| mailbox.name == name)
        .ok_or_else(|| format!("there is no mailbox {name}"))?;
    let plan = unifyterm_mail::plan(&mailbox, &config.mail).map_err(|error| format!("{error:#}"))?;
    Ok((file, plan))
}

/// The test, on a thread of its own: each step told to `steps`; when all went, what lets the
/// mailbox in is kept, and the settings file saved.
fn try_and_keep(
    plan: &Plan,
    given: Given,
    steps: &Steps,
    mut file: unifyterm_config::File,
    save: Save,
) -> Result<(), String> {
    let (credential, tokens, keep) = match given {
        Given::Typed(password) => (Credential::Password(password), None, true),
        Given::Token(tokens) => (Credential::Token(tokens.access.clone()), Some(tokens), true),
        Given::Kept => (plan.credential().map_err(|failure| failure.to_string())?, None, false),
    };
    let mut told = |step, outcome| steps.lock().unwrap_or_else(PoisonError::into_inner).push((step, outcome));
    if !unifyterm_mail::check(&plan.account, &credential, &mut told) {
        // The step that failed, said at the foot: the list of steps may be out of view.
        let failed = steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .rev()
            .find_map(|(step, outcome)| outcome.as_ref().err().map(|why| format!("{}: {why}", step_name(*step))));
        return Err(match failed {
            Some(failed) => format!("{failed}. Nothing was kept."),
            None => "Nothing was kept.".to_owned(),
        });
    }
    if keep {
        unifyterm_mail::keep_credential(&plan.account, &credential, tokens.as_ref())
            .map_err(|error| format!("{error:#}"))?;
    }
    if save == Save::File {
        file.save().map_err(|error| format!("{error:#}"))?;
    }
    Ok(())
}

/// The narrowest a way of logging in is drawn before the four go one to a row.
const WAY_WIDTH: f32 = 230.;

/// Whether the settings file is saved after the test: not for a sign-in, which changes no setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Save {
    File,
    Not,
}

/// The words of a step, as the form lists it.
fn step_name(step: Step) -> &'static str {
    match step {
        Step::Connect => "connect to the IMAP server",
        Step::Login => "log in",
        Step::SentFolder => "find the sent folder",
        Step::Smtp => "log in to the SMTP server",
    }
}

impl MailView {
    /// The form, drawn.
    pub(super) fn form_panel(&self, cx: &mut Context<'_, Self>) -> gpui::Div {
        let Some(form) = &self.form else { return div().flex_1() };
        let colors = self.colors();
        let title = match &form.purpose {
            Purpose::Add => "Add a mailbox".to_owned(),
            Purpose::Change(name) => format!("Change {name}"),
            Purpose::SignIn(name) => format!("Sign {name} in again"),
        };
        let mut body = div().max_w(px(640.)).flex().flex_col().gap_3();
        if !form.signing_in() {
            body = body.child(self.field(form, Field::Address, cx)).child(self.found_line(form));
            body = body.child(self.ways(form, cx));
        }
        body = body.child(self.way_panel(form, cx));
        if !form.signing_in() {
            body = body.child(self.connection(form, cx));
        }
        if form.adding() {
            body = body.child(self.field(form, Field::Name, cx));
        }
        body = body.child(self.steps_box(form));
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(colors.rule)
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child(SharedString::from(title)))
                    .child(self.button(
                        "close-form",
                        "×",
                        Weight::Plain,
                        cx.listener(|this, _, _, cx| this.close_form(cx)),
                    )),
            )
            .child(div().id("form").flex_1().min_h_0().overflow_y_scroll().px_5().py_4().child(body))
            .child(self.form_foot(form, cx))
    }

    /// A text field: its label, what is typed (dots for a password), the caret when in use.
    fn field(&self, form: &Form, field: Field, cx: &mut Context<'_, Self>) -> gpui::AnyElement {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let focused = form.focused == field;
        let typed = &form.text[field.at()];
        let composing = if focused { self.composing.as_str() } else { "" };
        let shown = if field == Field::Secret { "•".repeat(typed.chars().count()) } else { typed.clone() };
        let empty = shown.is_empty() && composing.is_empty();
        let placeholder = if field == Field::Name {
            format!("automatic: {}", form.name(&self.settings.config.mail))
        } else {
            field.placeholder().to_owned()
        };
        let words: SharedString = if empty { placeholder.into() } else { format!("{shown}{composing}").into() };
        let (view, painted, focus) = (cx.entity(), cx.entity(), self.focus.clone());
        let mut input = div()
            .id(("field", field.at()))
            .relative()
            .flex()
            .items_center()
            .min_h(px(32.))
            .px_2()
            .rounded(px(theme.button_rounding()))
            .border_1()
            .border_color(if focused { colors.accent } else { colors.rule })
            .bg(colors.back)
            .text_size(px(13.))
            .text_color(if empty { colors.muted } else { colors.ink })
            .cursor_text()
            .on_click(cx.listener(move |this, _, window, cx| this.focus_field(field, window, cx)))
            .child(div().min_w_0().truncate().child(words))
            .when(focused, |input| input.child(div().w(px(1.5)).h(px(16.)).bg(colors.accent)));
        if focused {
            input = input.child(
                canvas(
                    move |bounds, _, cx| view.update(cx, |view, _| view.input_bounds = Some(bounds)),
                    move |bounds, (), window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, painted), cx);
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(px(12.)).text_color(colors.muted).child(field.label()))
            .child(input)
            .into_any_element()
    }

    /// What the address says: the servers of its domain, or that they are not known.
    fn found_line(&self, form: &Form) -> gpui::Div {
        let colors = self.colors();
        let line = div().text_size(px(12.)).text_color(colors.muted);
        if !form.text(Field::Address).contains('@') {
            return line.child("The address says which servers and which way in are likely.");
        }
        match form.found() {
            Some(account) => line.child(SharedString::from(format!(
                "Servers: {}:{} · {}:{}",
                account.imap.host, account.imap.port, account.smtp.host, account.smtp.port
            ))),
            None => {
                line.text_color(colors.waiting).child("The servers of this domain are not known: give them by hand.")
            }
        }
    }

    /// The four ways in, two by two: the one in use pressed, the likely one marked.
    fn ways(&self, form: &Form, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let (now, likely) = (form.way(), form.recommended());
        div().flex().flex_wrap().gap_2().children(Way::ALL.into_iter().enumerate().map(|(at, way)| {
            let on = way == now;
            div()
                .id(("way", at))
                // Two to a row when there is room, one when there is not: each grows from this width.
                .flex_grow()
                .flex_basis(px(WAY_WIDTH))
                .px_3()
                .py_2()
                .rounded(px(theme.panel_rounding()))
                .border_1()
                .border_color(if on { colors.accent } else { colors.rule })
                .bg(colors.back)
                .cursor_pointer()
                .hover(move |tile| tile.border_color(colors.edge))
                .on_click(cx.listener(move |this, _, _, cx| this.choose_way(way, cx)))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_2()
                        // Words in a row wrap only when given the width that is left: measured as drawn.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(way.title()),
                        )
                        .when(way == likely && form.text(Field::Address).contains('@'), |line| {
                            line.child(div().flex_none().text_size(px(10.5)).text_color(colors.fine).child("likely"))
                        }),
                )
                .child(div().text_size(px(11.5)).text_color(colors.muted).child(way.says()))
        }))
    }

    /// What the way in use needs.
    fn way_panel(&self, form: &Form, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let hint = |text: String| {
            div()
                .px_3()
                .py_2()
                .border_l_2()
                .border_color(colors.accent)
                .bg(colors.accent.opacity(0.07))
                .text_size(px(12.))
                .text_color(colors.muted)
                .child(SharedString::from(text))
        };
        let panel = div().flex().flex_col().gap_2();
        let secret_note = (!form.adding()).then(|| "Leave it empty to keep the one kept now.".to_owned());
        match form.way() {
            Way::Code => panel
                .child(hint(form.hint().to_owned()))
                .child(self.field(form, Field::Secret, cx))
                .children(secret_note.map(|note| div().text_size(px(11.5)).text_color(colors.muted).child(SharedString::from(note)))),
            Way::Browser => panel.child(hint(
                "Microsoft's page opens in the browser: sign in there and enter the code shown here. UnifyTerm keeps the token Microsoft gives (in the system's keychain), never your password.".to_owned(),
            )).child(self.sign_in_box(form)),
            Way::Manual if form.signing_in() => panel.child(self.field(form, Field::Secret, cx)),
            Way::Manual => panel
                .child(div().flex().gap_2().child(div().flex_1().child(self.field(form, Field::Imap, cx))).child(div().flex_1().child(self.field(form, Field::Smtp, cx))))
                .child(div().flex().gap_2().child(div().flex_1().child(self.field(form, Field::Login, cx))).child(div().flex_1().child(self.field(form, Field::Secret, cx)))),
            Way::Command if form.signing_in() => panel.child(hint("The program is run again to test the mailbox.".to_owned())),
            Way::Command => panel
                .child(self.field(form, Field::Command, cx))
                .child(hint("Run each time the mailbox logs in; its first line is the password. Nothing is kept.".to_owned())),
        }
    }

    /// The code to enter in the browser, and whether the sign-in is done.
    fn sign_in_box(&self, form: &Form) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let (big, small, color) = match (&form.stage, &form.tokens) {
            (_, Some(_)) => (String::new(), "✓ signed in".to_owned(), colors.fine),
            (Stage::SigningIn(Some(code)), _) => (
                code.user_code.clone(),
                format!("Enter it at {} · waiting for the sign-in…", code.verification_uri),
                colors.muted,
            ),
            (Stage::SigningIn(None), _) => (String::new(), "asking Microsoft for a code…".to_owned(), colors.muted),
            _ => (String::new(), "Press Sign in below: the code shows here.".to_owned(), colors.muted),
        };
        div()
            .flex()
            .items_center()
            .gap_4()
            .p_3()
            .rounded(px(theme.panel_rounding()))
            .border_1()
            .border_dashed()
            .border_color(colors.edge)
            .bg(colors.back)
            .when(!big.is_empty(), |line| {
                line.child(
                    div()
                        .flex_none()
                        .font_family(SharedString::from(self.settings.config.font.family.clone()))
                        .text_size(px(22.))
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(big)),
                )
            })
            .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(color).child(SharedString::from(small)))
    }

    /// Directly, or through a proxy.
    fn connection(&self, form: &Form, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let pill = |id: &'static str, label: &'static str, via: Via| {
            let on = form.via == via;
            div()
                .id(id)
                .px_3()
                .py(px(3.))
                .rounded_full()
                .border_1()
                .border_color(if on { colors.accent } else { colors.rule })
                .text_size(px(12.))
                .text_color(if on { colors.ink } else { colors.muted })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.choose_via(via, cx)))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_none().text_size(px(12.)).text_color(colors.muted).child("Connection"))
                    .child(pill("via-direct", "Directly", Via::Direct))
                    .child(pill("via-proxy", "Through a proxy", Via::Proxy)),
            )
            .when(form.via == Via::Proxy, |row| row.child(self.field(form, Field::Proxy, cx)))
    }

    /// The test's steps: done, failed, under way, or to come.
    fn steps_box(&self, form: &Form) -> gpui::Div {
        let colors = self.colors();
        let theme = &self.settings.theme;
        let steps = form.steps.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let testing = matches!(form.stage, Stage::Testing);
        if steps.is_empty() && !testing {
            return div();
        }
        let lines = Step::ALL.into_iter().enumerate().map(|(at, step)| {
            let (text, color): (String, Hsla) = match steps.iter().find(|(done, _)| *done == step) {
                Some((_, Ok(word))) => (format!("✓ {}: {word}", step_name(step)), colors.fine),
                Some((_, Err(why))) => (format!("✗ {}: {why}", step_name(step)), colors.wrong),
                None if testing && at == steps.len() => (format!("… {}", step_name(step)), colors.waiting),
                None => (format!("· {}", step_name(step)), colors.muted),
            };
            div().text_color(color).child(SharedString::from(text))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded(px(theme.panel_rounding()))
            .bg(colors.back)
            .font_family(SharedString::from(self.settings.config.font.family.clone()))
            .text_size(px(12.))
            .children(lines)
    }

    /// The command that does the same, what went wrong, Cancel, and the main button.
    fn form_foot(&self, form: &Form, cx: &mut Context<'_, Self>) -> gpui::Div {
        let colors = self.colors();
        let name = form.name(&self.settings.config.mail);
        let busy = !matches!(form.stage, Stage::Filling);
        let main = match (&form.purpose, form.way()) {
            (_, _) if busy => "Working…",
            (Purpose::SignIn(_), Way::Browser) if form.tokens.is_none() => "Sign in",
            (Purpose::SignIn(_), _) => "Test and keep",
            (_, Way::Browser) if form.tokens.is_none() => "Sign in and add",
            (Purpose::Add, _) => "Test and add",
            (Purpose::Change(_), _) => "Test and save",
        };
        let said = form.said.clone().map(|(text, tone)| {
            div()
                .text_size(px(12.))
                .text_color(if tone == Tone::Wrong { colors.wrong } else { colors.muted })
                .child(SharedString::from(text))
        });
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(colors.rule)
            .children(said)
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
                            .text_size(px(11.5))
                            .text_color(colors.muted)
                            .font_family(SharedString::from(self.settings.config.font.family.clone()))
                            .child(SharedString::from(form.command_line(&name))),
                    )
                    .child(self.button(
                        "cancel-form",
                        "Cancel",
                        Weight::Plain,
                        cx.listener(|this, _, _, cx| this.close_form(cx)),
                    ))
                    .child(self.button(
                        "submit-form",
                        main,
                        Weight::Strong,
                        cx.listener(|this, _, _, cx| this.submit(cx)),
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(address: &str) -> Form {
        let mut form = Form::new(Purpose::Add, None);
        form.text[Field::Address.at()] = address.to_owned();
        form
    }

    #[test]
    fn the_address_makes_a_way_likely_and_a_way_chosen_stays() {
        assert_eq!(typed("me@qq.com").way(), Way::Code);
        assert_eq!(typed("me@Outlook.com").way(), Way::Browser);
        assert_eq!(typed("me@mycompany.cn").way(), Way::Manual, "servers not known: by hand");
        assert_eq!(typed("me").way(), Way::Code, "no address yet");
        let mut chosen = typed("me@qq.com");
        chosen.way = Way::Command;
        chosen.way_chosen = true;
        chosen.text[Field::Address.at()] = "me@outlook.com".into();
        assert_eq!(chosen.way(), Way::Command, "chosen by hand: kept");
        assert_eq!(typed("me@qq.com").fields(), [Field::Address, Field::Secret, Field::Name]);
        assert_eq!(typed("me@outlook.com").fields(), [Field::Address, Field::Name]);
        let mut manual = typed("me@mycompany.cn");
        manual.via = Via::Proxy;
        assert_eq!(
            manual.fields(),
            [Field::Address, Field::Imap, Field::Smtp, Field::Login, Field::Secret, Field::Proxy, Field::Name]
        );
    }

    #[test]
    fn the_mailbox_written_has_only_what_its_way_uses() {
        let mut form = typed(" Me@QQ.com ");
        form.text[Field::Imap.at()] = "left.example.com:993".into();
        form.text[Field::Proxy.at()] = "socks5://127.0.0.1:7890".into();
        let account = form.account();
        assert_eq!((account.address.as_str(), account.imap.as_str(), account.proxy.as_str()), ("me@qq.com", "", ""));
        form.via = Via::Proxy;
        assert_eq!(form.account().proxy, "socks5://127.0.0.1:7890");
        assert_eq!(typed("me@outlook.com").account().auth, MailAuth::Oauth2);
        let mut command = typed("me@example.com");
        command.way = Way::Command;
        command.way_chosen = true;
        command.text[Field::Command.at()] = "pass show  mail/work".into();
        assert_eq!(command.account().password_command, ["pass", "show", "mail/work"]);
    }

    #[test]
    fn a_new_mailbox_is_named_after_its_domain_unless_named_and_never_twice() {
        let settings = unifyterm_config::Config::parse(
            "[mail]\naddress = \"me@qq.com\"\n[mail.accounts.qq-2]\naddress = \"b@qq.com\"",
        )
        .expect("settings")
        .mail;
        assert_eq!(typed("c@qq.com").name(&settings), "qq-3", "qq and qq-2 are taken");
        assert_eq!(typed("me@gmail.com").name(&settings), "gmail");
        let mut named = typed("me@gmail.com");
        named.text[Field::Name.at()] = "Work".into();
        assert_eq!(named.name(&settings), "work");
        assert_eq!(Form::new(Purpose::SignIn("qq".into()), None).name(&settings), "qq");
        assert_eq!(typed("me@gmail.com").command_line("gmail"), "ut mail account add gmail me@gmail.com");
    }

    #[test]
    fn what_lets_it_in_is_asked_for_before_a_test() {
        assert!(typed("me@qq.com").given().is_err(), "a new mailbox needs its code");
        let mut form = typed("me@qq.com");
        form.text[Field::Secret.at()] = " code ".into();
        assert!(matches!(form.given(), Ok(Given::Typed(code)) if code == "code"));
        assert!(typed("me@outlook.com").given().is_err(), "the sign-in first");
        let mailbox = MailboxSettings {
            name: "qq".into(),
            address: "me@qq.com".into(),
            imap: String::new(),
            smtp: String::new(),
            login: String::new(),
            password_command: Vec::new(),
            proxy: "socks5://127.0.0.1:1".into(),
            days: 30,
            auth: MailAuth::Auto,
        };
        let change = Form::new(Purpose::Change("qq".into()), Some(&mailbox));
        assert!(matches!(change.given(), Ok(Given::Kept)), "changed: the code kept is used");
        assert_eq!((change.via, change.way()), (Via::Proxy, Way::Code));
        let again = Form::new(Purpose::SignIn("qq".into()), Some(&mailbox));
        assert_eq!(again.fields(), [Field::Secret], "signing in again asks only for the code");
    }
}
