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
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result};

pub use account::{Account, Auth, Security, Server};
pub use message::{Address, Attachment, Parsed};
pub use net::{Credential, Failure, NewMail};
pub use store::{Added, Contact, Draft, Filter, Incoming, Mail, Status, Store, Summary, Thread};
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

/// Who a mail goes to, when the person chose: in place of whom an answer would go to by itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recipients {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
}

/// Sends `text` as the answer to conversation `thread`, from the mailbox it is in (one of
/// `plans`): through SMTP, then kept here at once, the conversation's draft dropped, and a copy
/// put in the server's sent folder when the server did not keep one itself.
pub fn send_answer(plans: &[Plan], thread: i64, text: &str) -> Result<()> {
    send_answer_to(plans, thread, text, None)
}

/// [`send_answer`], to the people `chosen` names when it is given, instead of whom the
/// conversation says (its other person, and the others of its newest mail in Cc).
pub fn send_answer_to(plans: &[Plan], thread: i64, text: &str, chosen: Option<&Recipients>) -> Result<()> {
    let path = plans.first().map_or_else(default_path, |plan| Ok(plan.store.clone()))?;
    let mut store = Store::open_background(&path)?;
    let conversation = store.thread(thread)?.with_context(|| format!("there is no conversation {thread}"))?;
    let address = &conversation.summary.account;
    let plan = plans
        .iter()
        .find(|plan| &plan.account.address == address)
        .with_context(|| format!("the conversation is in the mailbox {address}, which the settings no longer name"))?;
    let mut answer = send::answer(&conversation, &plan.account.address, text)?;
    if let Some(chosen) = chosen {
        if chosen.to.is_empty() {
            anyhow::bail!("the answer goes to no one: name someone in To");
        }
        answer.to.clone_from(&chosen.to);
        answer.cc.clone_from(&chosen.cc);
    }
    deliver(plan, &mut store, &answer)?;
    store.drop_draft(thread)?;
    Ok(())
}

/// Sends a new mail, not an answer, from the mailbox of `plan`: kept here as a conversation of
/// its own, and in the server's sent folder.
pub fn send_new(plan: &Plan, recipients: &Recipients, subject: &str, text: &str) -> Result<()> {
    if recipients.to.is_empty() {
        anyhow::bail!("the mail goes to no one: name someone in To");
    }
    let answer = send::Answer {
        from: Address::new("", &plan.account.address),
        to: recipients.to.clone(),
        cc: recipients.cc.clone(),
        subject: subject.trim().to_owned(),
        in_reply_to: None,
        references: Vec::new(),
        text: text.trim_end().to_owned(),
    };
    let mut store = Store::open_background(&plan.store)?;
    deliver(plan, &mut store, &answer)
}

/// Writes, sends and keeps a mail: here at once, and in the server's sent folder when the server
/// did not put it there itself.
fn deliver(plan: &Plan, store: &mut Store, answer: &send::Answer) -> Result<()> {
    let now = unifyterm_history::now_ms();
    let id = send::new_message_id(&plan.account.address, now);
    let mail = send::write(answer, &id)?;
    let credential = plan.credential().map_err(|failure| anyhow::anyhow!("{failure}"))?;
    send::send(&plan.account, &credential, &mail)?;
    let raw = mail.formatted();
    let parsed = message::parse(&raw);
    store.add(&Incoming {
        account: &plan.account.address,
        folder: "",
        uidvalidity: 0,
        uid: None,
        mail: &parsed,
        seen: true,
        outgoing: true,
        received_ms: now,
    })?;
    if let Err(error) = net::keep_sent(&plan.account, &credential, &raw, &id) {
        log::warn!("mail: sent, but no copy put in the sent folder: {error:#}");
    }
    Ok(())
}

/// Tells apart the files being written at once by [`attachment_file`].
static PARTS: AtomicU64 = AtomicU64::new(0);

/// The `index`th attachment of the mail `mail` (a row of the store), as a file on this machine:
/// fetched from the server the first time (only names and sizes are kept with the mail), kept in
/// the data directory's `mail-files` after. A mail the server no longer has cannot give it.
pub fn attachment_file(plans: &[Plan], mail: i64, index: usize) -> Result<PathBuf> {
    let path = plans.first().map_or_else(default_path, |plan| Ok(plan.store.clone()))?;
    let store = Store::open_background(&path)?;
    let (account, folder, uidvalidity, uid) =
        store.place(mail)?.context("this mail is not on the server (yet): its files cannot be fetched")?;
    let folder_dir = path.parent().context("the mail's file has no folder")?.join("mail-files").join(mail.to_string());
    let plan = plans
        .iter()
        .find(|plan| plan.account.address == account)
        .with_context(|| format!("the mail is in the mailbox {account}, which the settings no longer name"))?;
    if let Some(kept) = std::fs::read_dir(&folder_dir).ok().and_then(|mut entries| {
        entries.find_map(|entry| {
            let entry = entry.ok()?;
            entry.file_name().to_string_lossy().starts_with(&format!("{index}-")).then(|| entry.path())
        })
    }) {
        return Ok(kept);
    }
    let credential = plan.credential().map_err(|failure| anyhow::anyhow!("{failure}"))?;
    let mut session = net::login(&plan.account, &credential).map_err(|failure| anyhow::anyhow!("{failure}"))?;
    let selected = session.select(&folder)?;
    if selected.uid_validity.unwrap_or_default() != uidvalidity {
        anyhow::bail!("the server numbered the folder anew: sync, then open the file again");
    }
    let fetched = session.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")?;
    let raw = fetched.iter().find_map(|fetch| fetch.body()).context("the server no longer has this mail")?.to_vec();
    session.logout().ok();
    let (name, bytes) = message::attachment(&raw, index).context("the mail has no such attachment")?;
    std::fs::create_dir_all(&folder_dir)?;
    let file = folder_dir.join(format!("{index}-{}", safe_name(&name)));
    // Written whole under another name, then renamed: a file cut short (the program stopped, the
    // disk full) is never taken for the file kept, and two fetches at once each write their own.
    let part =
        folder_dir.join(format!(".part-{index}-{}-{}", std::process::id(), PARTS.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&part, bytes).with_context(|| format!("write {}", part.display()))?;
    std::fs::rename(&part, &file).with_context(|| format!("rename {} to {}", part.display(), file.display()))?;
    Ok(file)
}

/// A file's name as an attachment gave it, safe to keep on any system: no folders, no characters
/// a system refuses, not empty.
/// A name a mail gave a file, safe as a file's name on every system: no folder in it, no sign a
/// system refuses, never empty. The name is the sender's to choose: it is never used as it is.
pub fn safe_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned();
    if safe.is_empty() { "attachment".to_owned() } else { safe }
}
