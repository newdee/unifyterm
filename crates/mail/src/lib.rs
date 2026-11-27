//! UnifyTerm's mail: one mailbox, kept on this machine in `mail.db` and read
//! as conversations. The window syncs it (IMAP) while it is open and sends
//! answers (SMTP); `ut mail` and `ut mcp` read it, and write drafts. An agent
//! may write a draft; only the person sends.

pub mod account;
#[doc(hidden)]
pub mod fake;
pub mod hook;
pub mod message;
pub mod net;
pub mod secret;
pub mod send;
pub mod store;
pub mod sync;

use std::path::PathBuf;

use anyhow::{Context as _, Result};

pub use account::{Account, Security, Server};
pub use message::{Address, Attachment, Parsed};
pub use net::NewMail;
pub use store::{Added, Draft, Filter, Incoming, Mail, Status, Store, Summary, Thread};
pub use sync::{Lock, Plan, Stopper};

/// The mailbox's file in UnifyTerm's data directory.
pub fn default_path() -> Result<PathBuf> {
    Ok(unifyterm_history::data_directory()?.join("mail.db"))
}

/// The plan the settings' `[mail]` table describes; `None` when no address is set (no mail).
pub fn plan(settings: &unifyterm_config::MailConfig) -> Result<Option<Plan>> {
    if settings.address.trim().is_empty() {
        return Ok(None);
    }
    let account = Account::new(&settings.address, &settings.imap, &settings.smtp, &settings.login, &settings.proxy)?;
    Ok(Some(Plan {
        account,
        password_command: settings.password_command.clone(),
        store: default_path()?,
        days: settings.days,
        wait: sync::WAIT,
        retry: sync::RETRY_FIRST,
    }))
}

/// Who wrote a draft by hand: the person, not an agent.
pub const BY_HAND: &str = "you";

/// Sends `text` as the answer to conversation `thread`: through SMTP, then kept here at once,
/// the conversation's draft dropped, and a copy put in the server's sent folder when the
/// server did not keep one itself.
pub fn send_answer(plan: &Plan, thread: i64, text: &str) -> Result<()> {
    let mut store = Store::open(&plan.store)?;
    let conversation = store.thread(thread)?.with_context(|| format!("there is no conversation {thread}"))?;
    let answer = send::answer(&conversation, &plan.account.address, text)?;
    let now = unifyterm_history::now_ms();
    let id = send::new_message_id(&plan.account.address, now);
    let mail = send::write(&answer, &id)?;
    let password = secret::password(&plan.account.login, &plan.password_command)?;
    send::send(&plan.account, &password, &mail)?;
    let raw = mail.formatted();
    let parsed = message::parse(&raw);
    store.add(&Incoming {
        folder: "",
        uidvalidity: 0,
        uid: None,
        mail: &parsed,
        seen: true,
        outgoing: true,
        received_ms: now,
    })?;
    store.drop_draft(thread)?;
    if let Err(error) = net::keep_sent(&plan.account, &password, &raw, &id) {
        log::warn!("mail: sent, but no copy put in the sent folder: {error:#}");
    }
    Ok(())
}
