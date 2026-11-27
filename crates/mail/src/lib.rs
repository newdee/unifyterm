//! UnifyTerm's mail: the mailboxes the settings name, kept on this machine in `mail.db` and read
//! as conversations. The window syncs each (IMAP) while it is open and sends answers (SMTP);
//! `ut mail` and `ut mcp` read them, and write drafts. An agent may write a draft; only the
//! person sends.

pub mod account;
#[doc(hidden)]
pub mod fake;
pub mod hook;
pub mod message;
pub mod net;
pub mod oauth;
pub mod secret;
pub mod send;
pub mod store;
pub mod sync;

use std::path::PathBuf;

use anyhow::{Context as _, Result};

pub use account::{Account, Auth, Security, Server};
pub use message::{Address, Attachment, Parsed};
pub use net::{Credential, Failure, NewMail};
pub use store::{Added, Draft, Filter, Incoming, Mail, Status, Store, Summary, Thread};
pub use sync::{Lock, Plan, Stopper};
use unifyterm_config::{MailConfig, MailboxSettings};

/// The mailbox's file in UnifyTerm's data directory.
pub fn default_path() -> Result<PathBuf> {
    Ok(unifyterm_history::data_directory()?.join("mail.db"))
}

/// `mail.db`, open. The mails version 1 kept (one mailbox) are the mailbox's in `[mail]` from
/// now on: that is the mailbox version 1 read.
pub fn open(settings: &MailConfig) -> Result<Store> {
    let mut store = Store::open(&default_path()?)?;
    let first = settings.address.trim();
    if !first.is_empty() {
        store.adopt(&first.to_lowercase())?;
    }
    Ok(store)
}

/// The plan of one mailbox.
pub fn plan(mailbox: &MailboxSettings, settings: &MailConfig) -> Result<Plan> {
    let account =
        Account::new(&mailbox.address, &mailbox.imap, &mailbox.smtp, &mailbox.login, &mailbox.proxy, mailbox.auth)?;
    Ok(Plan {
        name: mailbox.name.clone(),
        account,
        password_command: mailbox.password_command.clone(),
        provider: oauth::Provider::microsoft(&settings.oauth_client_id),
        store: default_path()?,
        days: mailbox.days,
        wait: sync::WAIT,
        retry: sync::RETRY_FIRST,
    })
}

/// Every mailbox the settings name, with its plan or why it has none (servers not known, a
/// proxy that cannot be read): one that cannot be synced does not keep the others from it.
pub fn plans(settings: &MailConfig) -> Vec<(MailboxSettings, Result<Plan>)> {
    settings
        .mailboxes()
        .into_iter()
        .map(|mailbox| {
            let plan = plan(&mailbox, settings);
            (mailbox, plan)
        })
        .collect()
}

/// One step of trying a mailbox before it is kept ([`check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The IMAP server answers (through the proxy, when there is one).
    Connect,
    /// It lets the mailbox in.
    Login,
    /// Its sent folder is found (none is no failure: answers are then kept here only).
    SentFolder,
    /// The SMTP server lets the mailbox in: mail can be sent.
    Smtp,
}

impl Step {
    pub const ALL: [Self; 4] = [Self::Connect, Self::Login, Self::SentFolder, Self::Smtp];
}

/// Tries a mailbox the way its sync and its answers will use it, step by step: `told` hears of
/// each step done (with a word on it) or failed (why), and the first failure ends the check.
pub fn check(account: &Account, credential: &Credential, told: &mut dyn FnMut(Step, Result<String, String>)) -> bool {
    let client = match net::greeted(account, |_| {}) {
        Ok(client) => client,
        Err(error) => {
            told(Step::Connect, Err(format!("{error:#}")));
            return false;
        }
    };
    told(Step::Connect, Ok(format!("{}:{}", account.imap.host, account.imap.port)));
    let mut session = match net::log_in(client, account, credential) {
        Ok(session) => session,
        Err(failure) => {
            told(Step::Login, Err(failure.to_string()));
            return false;
        }
    };
    told(Step::Login, Ok(account.login.clone()));
    match net::sent_folder(&mut session) {
        Ok(Some(folder)) => told(Step::SentFolder, Ok(net::decode_folder(&folder))),
        Ok(None) => told(Step::SentFolder, Ok("none: answers are kept here only".to_owned())),
        Err(error) => {
            told(Step::SentFolder, Err(format!("{error:#}")));
            return false;
        }
    }
    session.logout().ok();
    match send::check(account, credential) {
        Ok(()) => told(Step::Smtp, Ok(format!("{}:{}", account.smtp.host, account.smtp.port))),
        Err(error) => {
            told(Step::Smtp, Err(format!("{error:#}")));
            return false;
        }
    }
    true
}

/// Keeps what lets the mailbox in: the password in the keychain, or the sign-in's tokens.
pub fn keep_credential(account: &Account, credential: &Credential, tokens: Option<&oauth::Tokens>) -> Result<()> {
    match (credential, tokens) {
        (Credential::Password(password), _) => secret::keep(&account.login, password),
        (Credential::Token(_), Some(tokens)) => oauth::remember(&account.login, tokens),
        (Credential::Token(_), None) => Ok(()),
    }
}

/// Takes away what lets the mailbox in: a kept password and a kept sign-in.
pub fn forget_credential(login: &str) -> Result<()> {
    secret::forget(login)?;
    oauth::forget(login)
}

/// Who wrote a draft by hand: the person, not an agent.
pub const BY_HAND: &str = "you";

/// Sends `text` as the answer to conversation `thread`, from the mailbox it is in (one of
/// `plans`): through SMTP, then kept here at once, the conversation's draft dropped, and a copy
/// put in the server's sent folder when the server did not keep one itself.
pub fn send_answer(plans: &[Plan], thread: i64, text: &str) -> Result<()> {
    let path = plans.first().map_or_else(default_path, |plan| Ok(plan.store.clone()))?;
    let mut store = Store::open_background(&path)?;
    let conversation = store.thread(thread)?.with_context(|| format!("there is no conversation {thread}"))?;
    let address = &conversation.summary.account;
    let plan = plans
        .iter()
        .find(|plan| &plan.account.address == address)
        .with_context(|| format!("the conversation is in the mailbox {address}, which the settings no longer name"))?;
    let answer = send::answer(&conversation, &plan.account.address, text)?;
    let now = unifyterm_history::now_ms();
    let id = send::new_message_id(&plan.account.address, now);
    let mail = send::write(&answer, &id)?;
    let credential = plan.credential().map_err(|failure| anyhow::anyhow!("{failure}"))?;
    send::send(&plan.account, &credential, &mail)?;
    let raw = mail.formatted();
    let parsed = message::parse(&raw);
    store.add(&Incoming {
        account: address,
        folder: "",
        uidvalidity: 0,
        uid: None,
        mail: &parsed,
        seen: true,
        outgoing: true,
        received_ms: now,
    })?;
    store.drop_draft(thread)?;
    if let Err(error) = net::keep_sent(&plan.account, &credential, &raw, &id) {
        log::warn!("mail: sent, but no copy put in the sent folder: {error:#}");
    }
    Ok(())
}
