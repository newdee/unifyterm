//! The mailboxes on this machine: one SQLite file, `mail.db`, for all of them;
//! each row says whose it is (the mailbox's address). The sync writes
//! what the server has; the window, `ut mail` and `ut mcp` read it, and
//! write drafts and what was read. Each process opens its own connection:
//! the file is in WAL mode, and a writer waits for another.
//!
//! Mails are kept as conversations (threads). A mail joins the conversation
//! of any mail it names in `In-Reply-To` or `References`, or that names it;
//! a mail that names none, but whose subject says it answers ("Re:"), joins
//! the newest conversation with the same plain subject and a person in common.
//! A conversation is one mailbox's: the same mail in two mailboxes is kept in each.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::Serialize;

use crate::message::{self, Address, Attachment, Parsed};

const SCHEMA_VERSION: &str = "2";

/// The messages table, apart: a file of version 1 is moved into it.
const MESSAGES: &str = "
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY,
    -- The mailbox's address; '' for one kept by version 1 that no mailbox has taken yet.
    account     TEXT NOT NULL,
    thread_id   INTEGER NOT NULL REFERENCES threads(id),
    -- Where the server keeps it; '' and no uid for one sent from here whose copy has not come back.
    folder      TEXT NOT NULL,
    uidvalidity INTEGER NOT NULL,
    uid         INTEGER,
    message_id  TEXT,
    from_name   TEXT NOT NULL,
    from_addr   TEXT NOT NULL,
    reply_to    TEXT,
    to_json     TEXT NOT NULL,
    cc_json     TEXT NOT NULL,
    subject     TEXT NOT NULL,
    at          INTEGER NOT NULL,
    body        TEXT NOT NULL,
    files_json  TEXT NOT NULL,
    -- In-Reply-To and References, blank between: what an answer to it names.
    refs        TEXT NOT NULL,
    outgoing    INTEGER NOT NULL,
    seen        INTEGER NOT NULL,
    -- The server has been told it is seen (or told us so).
    seen_told   INTEGER NOT NULL,
    UNIQUE (account, folder, uidvalidity, uid)
);
CREATE INDEX messages_by_thread ON messages(thread_id, at);
CREATE INDEX messages_by_id ON messages(account, message_id);
";

/// The rest of version 2.
const SCHEMA: &str = "
CREATE TABLE threads (
    id       INTEGER PRIMARY KEY,
    subject  TEXT NOT NULL,
    last_at  INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    account  TEXT NOT NULL DEFAULT ''
);
CREATE INDEX threads_by_account ON threads(account, last_at);
-- Every message id seen in a mailbox, its own or one a mail names, and the conversation it is in.
CREATE TABLE ids (
    account    TEXT NOT NULL,
    message_id TEXT NOT NULL,
    thread_id  INTEGER NOT NULL,
    PRIMARY KEY (account, message_id)
);
CREATE TABLE drafts (
    thread_id INTEGER PRIMARY KEY,
    author    TEXT NOT NULL,
    body      TEXT NOT NULL,
    at        INTEGER NOT NULL
);
CREATE TABLE folders (
    account     TEXT NOT NULL,
    name        TEXT NOT NULL,
    uidvalidity INTEGER NOT NULL,
    last_uid    INTEGER NOT NULL,
    PRIMARY KEY (account, name)
);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// Version 1 (one mailbox) to 2: its rows are nobody's ('') until [`Store::adopt`] gives them a mailbox.
const FROM_ONE: &str = "
ALTER TABLE threads ADD COLUMN account TEXT NOT NULL DEFAULT '';
CREATE INDEX threads_by_account ON threads(account, last_at);
ALTER TABLE messages RENAME TO messages_one;
DROP INDEX messages_by_thread;
DROP INDEX messages_by_id;
{messages}
INSERT INTO messages (id, account, thread_id, folder, uidvalidity, uid, message_id, from_name, from_addr, reply_to,
        to_json, cc_json, subject, at, body, files_json, refs, outgoing, seen, seen_told)
    SELECT id, '', thread_id, folder, uidvalidity, uid, message_id, from_name, from_addr, reply_to,
        to_json, cc_json, subject, at, body, files_json, refs, outgoing, seen, seen_told FROM messages_one;
DROP TABLE messages_one;
ALTER TABLE ids RENAME TO ids_one;
CREATE TABLE ids (account TEXT NOT NULL, message_id TEXT NOT NULL, thread_id INTEGER NOT NULL, PRIMARY KEY (account, message_id));
INSERT INTO ids SELECT '', message_id, thread_id FROM ids_one;
DROP TABLE ids_one;
ALTER TABLE folders RENAME TO folders_one;
CREATE TABLE folders (account TEXT NOT NULL, name TEXT NOT NULL, uidvalidity INTEGER NOT NULL, last_uid INTEGER NOT NULL, PRIMARY KEY (account, name));
INSERT INTO folders SELECT '', name, uidvalidity, last_uid FROM folders_one;
DROP TABLE folders_one;
UPDATE meta SET key = 'status:' WHERE key = 'status';
UPDATE meta SET value = '2' WHERE key = 'schema';
";

/// How long ago the newest mail of a conversation may be for a mail with the
/// same subject to join it when nothing else ties them: 60 days.
const SUBJECT_REACH_MS: i64 = 60 * 86_400_000;
/// How long a write in the window waits for another writer: a window must not stop longer.
const WINDOW_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long a write on a thread of its own (a sync, a send) waits for the other writers: the
/// first syncs of several big mailboxes may hold the file in turn for a while.
pub const BACKGROUND_WAIT: std::time::Duration = std::time::Duration::from_mins(1);
/// `seen_told` of a mail made unread here: the server still has it `\Seen`, until it is told.
/// (0: read here, the server not told; 1: the server agrees.)
const UNSEEN_UNTOLD: i64 = 2;
/// How many characters of a mail's first line a list shows.
const SNIPPET_CHARS: usize = 80;

/// The mailbox, open.
#[derive(Debug)]
pub struct Store {
    db: Connection,
}

/// A mail from the server, or sent from here, to keep.
#[derive(Debug, Clone, Copy)]
pub struct Incoming<'a> {
    /// The mailbox's address.
    pub account: &'a str,
    /// The server's folder; `""` for one sent from here.
    pub folder: &'a str,
    pub uidvalidity: u32,
    pub uid: Option<u32>,
    pub mail: &'a Parsed,
    pub seen: bool,
    /// Written by the mailbox's owner.
    pub outgoing: bool,
    /// When it came, for a mail that does not say when it was written.
    pub received_ms: i64,
}

/// What became of a mail given to [`Store::add`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New {
        message: i64,
        thread: i64,
    },
    /// It was there already (the same message id).
    Known,
}

/// A conversation, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub id: i64,
    /// The mailbox's address.
    pub account: String,
    pub subject: String,
    /// Who it is with: the other person of its newest mail.
    pub with: String,
    pub last_at_ms: i64,
    pub messages: usize,
    pub unread: usize,
    /// Its newest mail is from someone else: it waits for an answer.
    pub waiting: bool,
    /// Who wrote the draft waiting in it, when there is one.
    pub draft_by: Option<String>,
    pub snippet: String,
    pub archived: bool,
}

/// One mail of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mail {
    pub id: i64,
    pub message_id: Option<String>,
    pub from: Address,
    pub reply_to: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    pub at_ms: i64,
    pub text: String,
    pub attachments: Vec<Attachment>,
    /// In-Reply-To and References.
    pub refs: Vec<String>,
    pub outgoing: bool,
    pub seen: bool,
}

/// An answer not sent yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Draft {
    pub thread: i64,
    /// `you`, or the agent that wrote it.
    pub author: String,
    pub text: String,
    pub at_ms: i64,
}

/// A conversation with its mails, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Thread {
    pub summary: Summary,
    pub mails: Vec<Mail>,
    pub draft: Option<Draft>,
}

/// A mail whose read state here the server has not been told of yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untold {
    pub id: i64,
    pub folder: String,
    pub uid: u32,
    /// Read here (the server is to mark it `\Seen`), or made unread here (to take `\Seen` away).
    pub seen: bool,
}

/// Someone the mailboxes have written with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contact {
    /// The name a mail gave, when one did.
    pub name: String,
    pub addr: String,
    /// How many mails they sent or were sent.
    pub count: usize,
    pub last_at_ms: i64,
}

/// What the sync last said, for the window to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub at_ms: i64,
    pub failed: bool,
}

impl Store {
    /// The file at `path`, made if it is not there; its writes wait [`WINDOW_WAIT`] for another writer.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_waiting(path, WINDOW_WAIT)
    }

    /// The file at `path`, for a thread of its own (a sync, a send): its writes wait [`BACKGROUND_WAIT`].
    pub fn open_background(path: &Path) -> Result<Self> {
        Self::open_waiting(path, BACKGROUND_WAIT)
    }

    fn open_waiting(path: &Path, wait: std::time::Duration) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
        }
        let db = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        db.busy_timeout(wait)?;
        let journal: String = db.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !journal.eq_ignore_ascii_case("wal") {
            // Changing the journal does not wait for another connection by itself (SQLite gives no
            // busy handler to it): tried again, 50 ms apart, for as long as a write would wait.
            let most = u32::try_from(wait.as_millis() / 50).unwrap_or(u32::MAX).max(1);
            let mut tries = 0;
            loop {
                match db.pragma_update(None, "journal_mode", "WAL") {
                    Ok(()) => break,
                    Err(rusqlite::Error::SqliteFailure(failure, _))
                        if failure.code == rusqlite::ErrorCode::DatabaseBusy && tries < most =>
                    {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(error) => return Err(error).context("make mail.db a WAL file"),
                }
            }
        }
        Self::ready(db)
    }

    /// One in memory, for the tests.
    pub fn in_memory() -> Result<Self> {
        Self::ready(Connection::open_in_memory()?)
    }

    /// Makes the tables of a new file, or checks an old one's version. A file of this version is
    /// only read: an open takes no write lock. Two processes may open a new file at once (the
    /// window's sync, `ut mcp`): the making looks again inside one write transaction, so that the
    /// second waits and then finds the tables made.
    fn ready(mut db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", true)?;
        let current: Option<String> = db
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| row.get(0))
            .optional()
            // No meta table yet (a new file), or anything else: the write transaction below looks again and says.
            .ok()
            .flatten();
        if current.as_deref() == Some(SCHEMA_VERSION) {
            return Ok(Self { db });
        }
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let has_meta: bool =
            tx.query_row("SELECT count(*) > 0 FROM sqlite_master WHERE name = 'meta'", [], |row| row.get(0))?;
        if has_meta {
            let version: Option<String> =
                tx.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| row.get(0)).optional()?;
            match version.as_deref() {
                Some(SCHEMA_VERSION) => {}
                Some("1") => tx.execute_batch(&FROM_ONE.replace("{messages}", MESSAGES))?,
                _ => bail!("mail.db is of another version ({version:?}): move it away and the mail is read again"),
            }
        } else {
            tx.execute_batch(SCHEMA)?;
            tx.execute_batch(MESSAGES)?;
            tx.execute("INSERT INTO meta (key, value) VALUES ('schema', ?1)", [SCHEMA_VERSION])?;
        }
        tx.commit()?;
        Ok(Self { db })
    }

    /// A transaction that writes: it takes the file's write lock at once (IMMEDIATE), so that it
    /// waits for another writer (the syncs of other mailboxes, a window, `ut`) instead of failing
    /// when it first reads and then writes after the other wrote.
    fn writing(&mut self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?)
    }

    /// Changes when another connection has written: what a window looks at to know it should read again.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.db.query_row("PRAGMA data_version", [], |row| row.get(0))?)
    }

    /// Gives the rows version 1 kept, which are nobody's, to the mailbox `account`: the one that
    /// was version 1's, in `[mail]`. A row it has already (read again since) is not kept twice.
    pub fn adopt(&mut self, account: &str) -> Result<()> {
        // Asked at each open: a look that writes nothing when there is nothing to take, as almost always.
        let any: bool = self.db.query_row(
            "SELECT EXISTS (SELECT 1 FROM threads WHERE account = '') OR EXISTS (SELECT 1 FROM ids WHERE account = '')
                 OR EXISTS (SELECT 1 FROM folders WHERE account = '') OR EXISTS (SELECT 1 FROM meta WHERE key = 'status:')",
            [],
            |row| row.get(0),
        )?;
        if !any {
            return Ok(());
        }
        let tx = self.writing()?;
        let nobodys: bool =
            tx.query_row("SELECT EXISTS (SELECT 1 FROM threads WHERE account = '')", [], |row| row.get(0))?;
        if nobodys {
            tx.execute("UPDATE threads SET account = ?1 WHERE account = ''", [account])?;
            tx.execute("UPDATE OR IGNORE messages SET account = ?1 WHERE account = ''", [account])?;
            tx.execute("DELETE FROM messages WHERE account = ''", [])?;
            tx.execute("UPDATE OR IGNORE ids SET account = ?1 WHERE account = ''", [account])?;
            tx.execute("UPDATE OR IGNORE folders SET account = ?1 WHERE account = ''", [account])?;
            Self::tidy(&tx)?;
        }
        tx.execute("DELETE FROM ids WHERE account = ''", [])?;
        tx.execute("DELETE FROM folders WHERE account = ''", [])?;
        tx.execute("UPDATE OR IGNORE meta SET key = 'status:' || ?1 WHERE key = 'status:'", [account])?;
        tx.execute("DELETE FROM meta WHERE key = 'status:'", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Everything kept of the mailbox `account` goes: its mails, conversations, drafts and status.
    /// The server keeps its mail; this is what a removed mailbox leaves here.
    pub fn forget_account(&mut self, account: &str) -> Result<()> {
        let tx = self.writing()?;
        tx.execute("DELETE FROM messages WHERE account = ?1", [account])?;
        tx.execute("DELETE FROM ids WHERE account = ?1", [account])?;
        tx.execute("DELETE FROM folders WHERE account = ?1", [account])?;
        tx.execute("DELETE FROM meta WHERE key = 'status:' || ?1", [account])?;
        Self::tidy(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Conversations left without mails go, with their drafts and the ids that led to them.
    fn tidy(tx: &rusqlite::Transaction<'_>) -> Result<()> {
        tx.execute("DELETE FROM drafts WHERE thread_id NOT IN (SELECT thread_id FROM messages)", [])?;
        tx.execute("DELETE FROM ids WHERE thread_id NOT IN (SELECT thread_id FROM messages)", [])?;
        tx.execute("DELETE FROM threads WHERE id NOT IN (SELECT thread_id FROM messages)", [])?;
        Ok(())
    }

    /// Keeps a mail, in its conversation.
    pub fn add(&mut self, incoming: &Incoming<'_>) -> Result<Added> {
        let mail = incoming.mail;
        let account = incoming.account;
        let tx = self.writing()?;
        if let Some(id) = &mail.message_id {
            let known: Option<(i64, String, Option<u32>)> = tx
                .query_row(
                    "SELECT id, folder, uid FROM messages WHERE account = ?1 AND message_id = ?2",
                    params![account, id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((row, folder, uid)) = known {
                // One sent from here, now seen in the server's folder: it is that copy from now on.
                if folder.is_empty() && uid.is_none() && incoming.uid.is_some() {
                    tx.execute(
                        "UPDATE messages SET folder = ?1, uidvalidity = ?2, uid = ?3 WHERE id = ?4",
                        params![incoming.folder, incoming.uidvalidity, incoming.uid, row],
                    )?;
                    tx.commit()?;
                }
                return Ok(Added::Known);
            }
        }
        let at = mail.date_ms.unwrap_or(incoming.received_ms);
        let named: Vec<&String> = mail.message_id.iter().chain(&mail.in_reply_to).chain(&mail.references).collect();
        let mut thread: Option<i64> = None;
        for id in &named {
            thread = tx
                .query_row(
                    "SELECT thread_id FROM ids WHERE account = ?1 AND message_id = ?2",
                    params![account, id],
                    |row| row.get(0),
                )
                .optional()?;
            if thread.is_some() {
                break;
            }
        }
        let plain = message::plain_subject(&mail.subject);
        if thread.is_none()
            && mail.in_reply_to.is_empty()
            && mail.references.is_empty()
            && message::is_answer(&mail.subject)
        {
            thread = Self::by_subject(&tx, account, &plain, mail, at)?;
        }
        let thread = if let Some(thread) = thread {
            thread
        } else {
            tx.execute(
                "INSERT INTO threads (subject, last_at, account) VALUES (?1, ?2, ?3)",
                params![plain, at, account],
            )?;
            tx.last_insert_rowid()
        };
        let from = mail.from.clone().unwrap_or_else(|| Address::new("", ""));
        let refs: Vec<&str> = mail.in_reply_to.iter().chain(&mail.references).map(String::as_str).collect();
        tx.execute(
            "INSERT INTO messages (thread_id, folder, uidvalidity, uid, message_id, from_name, from_addr, reply_to,
                 to_json, cc_json, subject, at, body, files_json, refs, outgoing, seen, seen_told, account)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17, ?18)",
            params![
                thread,
                incoming.folder,
                incoming.uidvalidity,
                incoming.uid,
                mail.message_id,
                from.name,
                from.addr,
                mail.reply_to.as_ref().map(Address::full),
                serde_json::to_string(&mail.to)?,
                serde_json::to_string(&mail.cc)?,
                mail.subject,
                at,
                mail.text,
                serde_json::to_string(&mail.attachments)?,
                refs.join(" "),
                incoming.outgoing,
                incoming.seen || incoming.outgoing,
                account,
            ],
        )?;
        let message = tx.last_insert_rowid();
        for id in named {
            tx.execute(
                "INSERT OR IGNORE INTO ids (account, message_id, thread_id) VALUES (?1, ?2, ?3)",
                params![account, id, thread],
            )?;
        }
        // A new mail from someone else brings an archived conversation back.
        tx.execute(
            "UPDATE threads SET last_at = max(last_at, ?2), archived = CASE WHEN ?3 THEN archived ELSE 0 END WHERE id = ?1",
            params![thread, at, incoming.outgoing],
        )?;
        tx.commit()?;
        Ok(Added::New { message, thread })
    }

    /// The newest conversation with this plain subject, recent enough, with a person in common.
    fn by_subject(
        tx: &rusqlite::Transaction<'_>,
        account: &str,
        plain: &str,
        mail: &Parsed,
        at: i64,
    ) -> Result<Option<i64>> {
        if plain.is_empty() {
            return Ok(None);
        }
        let people: Vec<&str> =
            mail.from.iter().chain(&mail.to).chain(&mail.cc).map(|address| address.addr.as_str()).collect();
        let mut find = tx.prepare(
            "SELECT t.id, group_concat(m.from_addr || ' ' || m.to_json || ' ' || m.cc_json, ' ')
             FROM threads t JOIN messages m ON m.thread_id = t.id
             WHERE t.subject = ?1 AND t.last_at >= ?2 AND t.account = ?3
             GROUP BY t.id ORDER BY t.last_at DESC",
        )?;
        let found = find
            .query_map(params![plain, at - SUBJECT_REACH_MS, account], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .find(|(_, seen)| people.iter().any(|person| !person.is_empty() && seen.contains(person)))
            .map(|(id, _)| id);
        Ok(found)
    }

    /// The conversations, newest first: at most `limit`; archived ones too only when asked;
    /// of the mailbox `account`, or of all when `None`.
    pub fn threads(&self, limit: usize, filter: Filter, account: Option<&str>) -> Result<Vec<Summary>> {
        let ids: Vec<i64> = {
            let mut list = self.db.prepare(
                "SELECT t.id FROM threads t
                 WHERE (?1 OR t.archived = 0)
                   AND (NOT ?2 OR EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.seen = 0))
                   AND (?4 IS NULL OR t.account = ?4)
                   AND (?5 IS NULL OR ?5 = (SELECT m.outgoing FROM messages m WHERE m.thread_id = t.id
                                            ORDER BY m.at DESC, m.id DESC LIMIT 1))
                 ORDER BY t.last_at DESC LIMIT ?3",
            )?;
            let all = matches!(filter, Filter::All);
            let unread = matches!(filter, Filter::Unread);
            // Whose the newest mail must be: someone else's (waiting for an answer), or mine.
            let newest_outgoing = match filter {
                Filter::Waiting => Some(false),
                Filter::Mine => Some(true),
                Filter::Inbox | Filter::Unread | Filter::All => None,
            };
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            list.query_map(params![all, unread, limit, account, newest_outgoing], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        ids.into_iter().map(|id| self.summary(id)).collect()
    }

    /// The conversations with a mail whose subject, words or sender has `text`, newest first.
    pub fn search(&self, text: &str, limit: usize, account: Option<&str>) -> Result<Vec<Summary>> {
        let pattern = format!("%{}%", text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let ids: Vec<i64> = {
            let mut list = self.db.prepare(
                "SELECT t.id FROM threads t WHERE EXISTS (
                     SELECT 1 FROM messages m WHERE m.thread_id = t.id AND (
                         m.subject LIKE ?1 ESCAPE '\\' OR m.body LIKE ?1 ESCAPE '\\'
                         OR m.from_name LIKE ?1 ESCAPE '\\' OR m.from_addr LIKE ?1 ESCAPE '\\'))
                   AND (?3 IS NULL OR t.account = ?3)
                 ORDER BY t.last_at DESC LIMIT ?2",
            )?;
            list.query_map(params![pattern, i64::try_from(limit).unwrap_or(i64::MAX), account], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        ids.into_iter().map(|id| self.summary(id)).collect()
    }

    fn summary(&self, id: i64) -> Result<Summary> {
        let (subject, last_at, archived, account): (String, i64, bool, String) = self
            .db
            .query_row("SELECT subject, last_at, archived, account FROM threads WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .with_context(|| format!("no conversation {id}"))?;
        let (messages, unread): (usize, usize) = self.db.query_row(
            "SELECT count(*), coalesce(sum(seen = 0), 0) FROM messages WHERE thread_id = ?1",
            [id],
            |row| Ok((row.get::<_, i64>(0)?.try_into().unwrap_or(0), row.get::<_, i64>(1)?.try_into().unwrap_or(0))),
        )?;
        let newest = self.mails(id, Some(1))?.into_iter().next();
        let with = self.with(id)?;
        let draft_by: Option<String> =
            self.db.query_row("SELECT author FROM drafts WHERE thread_id = ?1", [id], |row| row.get(0)).optional()?;
        Ok(Summary {
            id,
            account,
            subject: if subject.is_empty() { "(no subject)".to_owned() } else { subject },
            with,
            last_at_ms: last_at,
            messages,
            unread,
            waiting: newest.as_ref().is_some_and(|mail| !mail.outgoing),
            draft_by,
            snippet: newest.map(|mail| message::snippet(&mail.text, SNIPPET_CHARS)).unwrap_or_default(),
            archived,
        })
    }

    /// The other person of a conversation: who wrote its newest mail from outside,
    /// else whom its newest mail went to.
    fn with(&self, id: i64) -> Result<String> {
        let from: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT from_name, from_addr FROM messages WHERE thread_id = ?1 AND outgoing = 0 ORDER BY at DESC LIMIT 1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((name, addr)) = from {
            return Ok(Address::new(&name, &addr).shown().to_owned());
        }
        let to: Option<String> = self
            .db
            .query_row("SELECT to_json FROM messages WHERE thread_id = ?1 ORDER BY at DESC LIMIT 1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let to: Vec<Address> = to.map(|json| serde_json::from_str(&json)).transpose()?.unwrap_or_default();
        Ok(to.first().map(|address| address.shown().to_owned()).unwrap_or_default())
    }

    /// A conversation with its mails and its draft; `None` when there is no such one.
    pub fn thread(&self, id: i64) -> Result<Option<Thread>> {
        let there: bool =
            self.db.query_row("SELECT count(*) > 0 FROM threads WHERE id = ?1", [id], |row| row.get(0))?;
        if !there {
            return Ok(None);
        }
        let mut mails = self.mails(id, None)?;
        mails.reverse();
        Ok(Some(Thread { summary: self.summary(id)?, mails, draft: self.draft(id)? }))
    }

    /// A conversation's mails, newest first; at most `limit`.
    fn mails(&self, thread: i64, limit: Option<usize>) -> Result<Vec<Mail>> {
        let limit = limit.map_or(-1, |limit| i64::try_from(limit).unwrap_or(i64::MAX));
        let mut list = self.db.prepare(
            "SELECT id, message_id, from_name, from_addr, reply_to, to_json, cc_json, subject, at, body, files_json,
                    refs, outgoing, seen
             FROM messages WHERE thread_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
        )?;
        let rows = list.query_map(params![thread, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, bool>(12)?,
                row.get::<_, bool>(13)?,
            ))
        })?;
        let mut mails = Vec::new();
        for row in rows {
            let (id, message_id, name, addr, reply_to, to, cc, subject, at, text, files, refs, outgoing, seen) = row?;
            mails.push(Mail {
                id,
                message_id,
                from: Address::new(&name, &addr),
                reply_to: reply_to.map(|full| parse_full(&full)),
                to: serde_json::from_str(&to)?,
                cc: serde_json::from_str(&cc)?,
                subject,
                at_ms: at,
                text,
                attachments: serde_json::from_str(&files)?,
                refs: refs.split_whitespace().map(str::to_owned).collect(),
                outgoing,
                seen,
            });
        }
        Ok(mails)
    }

    /// How many mails from others are not read, in conversations not archived; of the mailbox
    /// `account`, or of all when `None`.
    pub fn unread(&self, account: Option<&str>) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT count(*) FROM messages m JOIN threads t ON t.id = m.thread_id
             WHERE m.seen = 0 AND t.archived = 0 AND (?1 IS NULL OR t.account = ?1)",
            [account],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// The newest mail's row from someone else, or 0: a window compares it to the last it saw.
    pub fn newest_incoming(&self) -> Result<i64> {
        Ok(self.db.query_row("SELECT coalesce(max(id), 0) FROM messages WHERE outgoing = 0", [], |row| row.get(0))?)
    }

    /// The conversation's mails are read.
    pub fn mark_seen(&self, thread: i64) -> Result<usize> {
        Ok(self.db.execute("UPDATE messages SET seen = 1 WHERE thread_id = ?1 AND seen = 0", [thread])?)
    }

    /// Mails of the mailbox `account` read here, or made unread here, that the server has not been
    /// told of.
    pub fn untold(&self, account: &str) -> Result<Vec<Untold>> {
        let mut list = self.db.prepare(
            "SELECT id, folder, uid, seen FROM messages
             WHERE account = ?1 AND uid IS NOT NULL
               AND ((seen = 1 AND seen_told = 0) OR seen_told = ?2)
             ORDER BY folder",
        )?;
        let rows = list.query_map(params![account, UNSEEN_UNTOLD], |row| {
            Ok(Untold { id: row.get(0)?, folder: row.get(1)?, uid: row.get(2)?, seen: row.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The conversation's newest mail from someone else is unread again, here and (at the next
    /// sync) on the server.
    pub fn mark_unread(&self, thread: i64) -> Result<()> {
        self.db.execute(
            "UPDATE messages SET seen = 0, seen_told = ?2 WHERE id = (
                 SELECT id FROM messages WHERE thread_id = ?1 AND outgoing = 0 ORDER BY at DESC, id DESC LIMIT 1)",
            params![thread, UNSEEN_UNTOLD],
        )?;
        Ok(())
    }

    /// Brings an archived conversation back to the inbox.
    pub fn unarchive(&self, thread: i64) -> Result<()> {
        self.db.execute("UPDATE threads SET archived = 0 WHERE id = ?1", [thread])?;
        Ok(())
    }

    /// Where the server keeps a mail: its mailbox, folder, uid validity and uid; `None` for one
    /// sent from here whose copy has not come back.
    pub fn place(&self, mail: i64) -> Result<Option<(String, String, u32, u32)>> {
        Ok(self
            .db
            .query_row(
                "SELECT account, folder, uidvalidity, uid FROM messages WHERE id = ?1 AND uid IS NOT NULL",
                [mail],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?)
    }

    /// The people the mailboxes have written with, most often and most lately first: from the
    /// senders and the receivers of every mail kept, less the mailboxes' own addresses.
    pub fn contacts(&self, own: &[String], limit: usize) -> Result<Vec<Contact>> {
        let mut found: std::collections::HashMap<String, Contact> = std::collections::HashMap::new();
        let mut note = |address: &Address, at: i64| {
            let addr = address.addr.trim().to_lowercase();
            if addr.is_empty() || own.contains(&addr) {
                return;
            }
            let entry = found.entry(addr.clone()).or_insert_with(|| Contact {
                name: String::new(),
                addr,
                count: 0,
                last_at_ms: at,
            });
            entry.count += 1;
            entry.last_at_ms = entry.last_at_ms.max(at);
            if entry.name.is_empty() && !address.name.trim().is_empty() {
                address.name.trim().clone_into(&mut entry.name);
            }
        };
        let mut rows = self.db.prepare("SELECT from_name, from_addr, to_json, cc_json, at FROM messages")?;
        let mut cursor = rows.query([])?;
        while let Some(row) = cursor.next()? {
            let at: i64 = row.get(4)?;
            note(&Address::new(&row.get::<_, String>(0)?, &row.get::<_, String>(1)?), at);
            for column in [2, 3] {
                let list: Vec<Address> = serde_json::from_str(&row.get::<_, String>(column)?)?;
                for address in &list {
                    note(address, at);
                }
            }
        }
        let mut contacts: Vec<Contact> = found.into_values().collect();
        contacts.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at_ms.cmp(&a.last_at_ms)).then(a.addr.cmp(&b.addr)));
        contacts.truncate(limit);
        Ok(contacts)
    }

    /// The server has been told these are read, or unread.
    pub fn told(&self, ids: &[i64]) -> Result<()> {
        for id in ids {
            self.db.execute("UPDATE messages SET seen_told = 1 WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// What the server says of these mails of `folder`: the ones in `seen` are read, the rest of
    /// `checked` are not. A mail read here that the server has not been told of yet stays read.
    pub fn server_seen(
        &self,
        account: &str,
        folder: &str,
        uidvalidity: u32,
        checked: &[u32],
        seen: &[u32],
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.db, rusqlite::TransactionBehavior::Immediate)?;
        for uid in checked {
            let on = seen.contains(uid);
            tx.execute(
                "UPDATE messages SET seen = ?4, seen_told = ?4
                 WHERE folder = ?1 AND uidvalidity = ?2 AND uid = ?3 AND outgoing = 0 AND NOT (seen = 1 AND seen_told = 0)
                   AND seen_told != ?6 AND account = ?5",
                params![folder, uidvalidity, uid, on, account, UNSEEN_UNTOLD],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The uids of `folder`'s mails kept here.
    pub fn uids(&self, account: &str, folder: &str, uidvalidity: u32) -> Result<Vec<u32>> {
        let mut list = self.db.prepare(
            "SELECT uid FROM messages
             WHERE account = ?3 AND folder = ?1 AND uidvalidity = ?2 AND uid IS NOT NULL ORDER BY uid",
        )?;
        let rows = list.query_map(params![folder, uidvalidity, account], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Puts a conversation away: it leaves the inbox until a new mail comes in it.
    pub fn archive(&self, thread: i64) -> Result<()> {
        self.db.execute("UPDATE threads SET archived = 1 WHERE id = ?1", [thread])?;
        Ok(())
    }

    /// Keeps `text` as the answer waiting in `thread`, in place of any before it.
    pub fn save_draft(&self, thread: i64, author: &str, text: &str, at_ms: i64) -> Result<()> {
        let there: bool =
            self.db.query_row("SELECT count(*) > 0 FROM threads WHERE id = ?1", [thread], |row| row.get(0))?;
        if !there {
            bail!("there is no conversation {thread}");
        }
        self.db.execute(
            "INSERT INTO drafts (thread_id, author, body, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (thread_id) DO UPDATE SET author = excluded.author, body = excluded.body, at = excluded.at",
            params![thread, author, text, at_ms],
        )?;
        Ok(())
    }

    pub fn draft(&self, thread: i64) -> Result<Option<Draft>> {
        Ok(self
            .db
            .query_row("SELECT author, body, at FROM drafts WHERE thread_id = ?1", [thread], |row| {
                Ok(Draft { thread, author: row.get(0)?, text: row.get(1)?, at_ms: row.get(2)? })
            })
            .optional()?)
    }

    pub fn drop_draft(&self, thread: i64) -> Result<()> {
        self.db.execute("DELETE FROM drafts WHERE thread_id = ?1", [thread])?;
        Ok(())
    }

    /// What is known of a server folder: its uid validity and the highest uid read.
    pub fn folder(&self, account: &str, name: &str) -> Result<Option<(u32, u32)>> {
        Ok(self
            .db
            .query_row(
                "SELECT uidvalidity, last_uid FROM folders WHERE account = ?1 AND name = ?2",
                params![account, name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn set_folder(&self, account: &str, name: &str, uidvalidity: u32, last_uid: u32) -> Result<()> {
        self.db.execute(
            "INSERT INTO folders (account, name, uidvalidity, last_uid) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account, name) DO UPDATE SET uidvalidity = excluded.uidvalidity, last_uid = excluded.last_uid",
            params![account, name, uidvalidity, last_uid],
        )?;
        Ok(())
    }

    /// The server numbered the folder anew (its uid validity changed): what was read of it goes.
    pub fn forget_folder(&mut self, account: &str, name: &str) -> Result<()> {
        let tx = self.writing()?;
        tx.execute("DELETE FROM messages WHERE account = ?1 AND folder = ?2", params![account, name])?;
        tx.execute("DELETE FROM folders WHERE account = ?1 AND name = ?2", params![account, name])?;
        Self::tidy(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// What the sync of the mailbox `account` says now.
    pub fn set_status(&self, account: &str, text: &str, health: Health, at_ms: i64) -> Result<()> {
        let value = serde_json::json!({ "text": text, "failed": health == Health::Failed, "at": at_ms }).to_string();
        self.db.execute(
            "INSERT INTO meta (key, value) VALUES ('status:' || ?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![account, value],
        )?;
        Ok(())
    }

    pub fn status(&self, account: &str) -> Result<Option<Status>> {
        let value: Option<String> = self
            .db
            .query_row("SELECT value FROM meta WHERE key = 'status:' || ?1", [account], |row| row.get(0))
            .optional()?;
        let Some(value) = value else { return Ok(None) };
        let json: serde_json::Value = serde_json::from_str(&value)?;
        Ok(Some(Status {
            text: json["text"].as_str().unwrap_or_default().to_owned(),
            at_ms: json["at"].as_i64().unwrap_or_default(),
            failed: json["failed"].as_bool().unwrap_or_default(),
        }))
    }
}

/// Whether the sync works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Fine,
    Failed,
}

/// Which conversations a list has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Those not archived.
    Inbox,
    /// Those not archived with a mail not read.
    Unread,
    /// Those not archived whose newest mail is from someone else: they wait for an answer.
    Waiting,
    /// Those not archived whose newest mail is mine: sent, waiting for theirs.
    Mine,
    /// All of them.
    All,
}

/// `Name <addr>` or `addr`, as [`Address::full`] writes it.
fn parse_full(full: &str) -> Address {
    match full.rsplit_once('<') {
        Some((name, addr)) => Address::new(name, addr.trim_end_matches('>')),
        None => Address::new("", full),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "me@example.com";

    fn mail(id: &str, from: &str, subject: &str, answers: &[&str], at: i64) -> Parsed {
        Parsed {
            message_id: (!id.is_empty()).then(|| id.to_owned()),
            in_reply_to: answers.last().map(|id| vec![(*id).to_owned()]).unwrap_or_default(),
            references: answers.iter().map(|id| (*id).to_owned()).collect(),
            from: Some(Address::new("", from)),
            to: vec![Address::new("", if from == "me@example.com" { "lin@example.com" } else { "me@example.com" })],
            subject: subject.into(),
            date_ms: Some(at),
            text: format!("words of {id}"),
            ..Parsed::default()
        }
    }

    fn add(store: &mut Store, folder: &str, uid: u32, mail: &Parsed) -> Added {
        let outgoing = mail.from.as_ref().is_some_and(|from| from.addr == "me@example.com");
        store
            .add(&Incoming {
                account: ME,
                folder,
                uidvalidity: 7,
                uid: Some(uid),
                mail,
                seen: false,
                outgoing,
                received_ms: 0,
            })
            .expect("added")
    }

    fn thread_of(added: Added) -> i64 {
        match added {
            Added::New { thread, .. } => thread,
            Added::Known => panic!("expected a new mail"),
        }
    }

    #[test]
    fn answers_join_the_conversation_they_answer_from_either_folder() {
        let mut store = Store::in_memory().expect("store");
        let first = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "发布", &[], 1000)));
        let mine = thread_of(add(&mut store, "Sent", 1, &mail("b1", "me@example.com", "Re: 发布", &["a1"], 2000)));
        let again =
            thread_of(add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "RE: 发布", &["a1", "b1"], 3000)));
        assert_eq!((mine, again), (first, first), "one conversation, inbox and sent together");
        let other = thread_of(add(&mut store, "INBOX", 3, &mail("c1", "ops@example.com", "下载页 502", &[], 2500)));
        assert_ne!(other, first);
        let thread = store.thread(first).expect("read").expect("there");
        assert_eq!(
            thread.mails.iter().map(|mail| mail.outgoing).collect::<Vec<_>>(),
            [false, true, false],
            "oldest first"
        );
        assert_eq!((thread.summary.subject.as_str(), thread.summary.with.as_str()), ("发布", "lin@example.com"));
        assert!(thread.summary.waiting, "its newest mail is from someone else");
        assert_eq!(thread.summary.unread, 2, "mine is read");
        let list = store.threads(10, Filter::Inbox, None).expect("list");
        assert_eq!(list.iter().map(|summary| summary.id).collect::<Vec<_>>(), [first, other], "newest first");
        let started = thread_of(add(&mut store, "Sent", 2, &mail("d1", "me@example.com", "报价", &[], 4000)));
        let ids = |filter: Filter| -> Vec<i64> {
            store.threads(10, filter, None).expect("list").iter().map(|summary| summary.id).collect()
        };
        assert_eq!(ids(Filter::Mine), [started], "the newest mail mine: sent, waiting for theirs");
        assert_eq!(ids(Filter::Waiting), [first, other], "the newest mail theirs");
        assert_eq!(store.unread(None).expect("unread"), 3);
    }

    #[test]
    fn a_mail_that_came_before_the_one_it_answers_still_finds_it() {
        let mut store = Store::in_memory().expect("store");
        let answer = thread_of(add(&mut store, "INBOX", 1, &mail("a2", "lin@example.com", "Re: x", &["a1"], 2000)));
        let original = thread_of(add(&mut store, "Sent", 1, &mail("a1", "me@example.com", "x", &[], 1000)));
        assert_eq!(original, answer, "a1 was named by a2: its id is known before it comes");
    }

    #[test]
    fn subjects_tie_answers_that_name_nothing_only_with_a_person_in_common() {
        let mut store = Store::in_memory().expect("store");
        let first = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "周报", &[], 1000)));
        let answer = thread_of(add(&mut store, "Sent", 1, &mail("", "me@example.com", "回复：周报", &[], 2000)));
        assert_eq!(answer, first, "Re: and a person in common");
        let mut stranger = mail("z1", "x@other.example", "Re: 周报", &[], 3000);
        stranger.to = vec![Address::new("", "y@other.example")];
        assert_ne!(thread_of(add(&mut store, "INBOX", 2, &stranger)), first, "no person in common");
        let late = thread_of(add(
            &mut store,
            "INBOX",
            3,
            &mail("", "lin@example.com", "Re: 周报", &[], 1000 + SUBJECT_REACH_MS + 5000),
        ));
        assert_ne!(late, first, "too long after");
        let plain = thread_of(add(&mut store, "INBOX", 4, &mail("q", "lin@example.com", "周报", &[], 2500)));
        assert_ne!(plain, first, "the same subject without Re: is a new conversation");
    }

    #[test]
    fn the_same_mail_twice_is_kept_once_and_a_sent_one_takes_its_server_copy() {
        let mut store = Store::in_memory().expect("store");
        let sent = mail("s1", "me@example.com", "Hello", &[], 1000);
        let here = store
            .add(&Incoming {
                account: ME,
                folder: "",
                uidvalidity: 0,
                uid: None,
                mail: &sent,
                seen: true,
                outgoing: true,
                received_ms: 0,
            })
            .expect("sent from here");
        assert!(matches!(here, Added::New { .. }));
        assert_eq!(add(&mut store, "Sent", 9, &sent), Added::Known, "the server's copy");
        assert_eq!(store.uids(ME, "Sent", 7).expect("uids"), [9], "the row is now the server's copy");
        assert_eq!(add(&mut store, "Sent", 10, &sent), Added::Known, "and is not kept twice");
    }

    #[test]
    fn read_and_told_and_what_the_server_says() {
        let mut store = Store::in_memory().expect("store");
        let thread = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "x", &[], 1000)));
        add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "Re: x", &["a1"], 2000));
        assert_eq!(store.mark_seen(thread).expect("seen"), 2);
        let untold = store.untold(ME).expect("untold");
        assert_eq!(untold.iter().map(|mail| mail.uid).collect::<Vec<_>>(), [1, 2]);
        store.server_seen(ME, "INBOX", 7, &[1, 2], &[]).expect("server");
        assert_eq!(store.unread(None).expect("unread"), 0, "read here and not told yet: stays read");
        store.told(&untold.iter().map(|mail| mail.id).collect::<Vec<_>>()).expect("told");
        store.server_seen(ME, "INBOX", 7, &[1, 2], &[2]).expect("server");
        assert_eq!(store.unread(None).expect("unread"), 1, "told, then the server marked 1 unread again");
    }

    #[test]
    fn drafts_archive_search_and_a_folder_numbered_anew() {
        let mut store = Store::in_memory().expect("store");
        let thread = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "发布计划", &[], 1000)));
        store.save_draft(thread, "claude", "可以", 5).expect("draft");
        store.save_draft(thread, "you", "可以，周四", 6).expect("again");
        let draft = store.draft(thread).expect("draft").expect("there");
        assert_eq!((draft.author.as_str(), draft.text.as_str()), ("you", "可以，周四"), "one draft a conversation");
        assert_eq!(store.threads(5, Filter::Inbox, None).expect("list")[0].draft_by.as_deref(), Some("you"));
        assert!(store.save_draft(999, "x", "y", 0).is_err(), "no such conversation");
        assert_eq!(store.search("计划", 5, None).expect("search").len(), 1);
        assert_eq!(store.search("lin@", 5, None).expect("search").len(), 1, "by sender");
        assert!(store.search("100%", 5, None).expect("search").is_empty(), "% is a character, not a pattern");
        store.archive(thread).expect("archive");
        assert!(store.threads(5, Filter::Inbox, None).expect("list").is_empty());
        assert_eq!(store.threads(5, Filter::All, None).expect("list").len(), 1);
        add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "Re: 发布计划", &["a1"], 2000));
        assert_eq!(store.threads(5, Filter::Inbox, None).expect("list").len(), 1, "a new mail brings it back");
        store.set_folder(ME, "INBOX", 7, 2).expect("folder");
        store.forget_folder(ME, "INBOX").expect("forget");
        assert!(
            store.threads(5, Filter::All, None).expect("list").is_empty()
                && store.folder(ME, "INBOX").expect("folder").is_none()
        );
        assert!(store.draft(thread).expect("draft").is_none(), "its draft went with it");
    }

    #[test]
    fn a_new_file_opened_by_many_at_once_is_made_once() {
        for _ in 0..10 {
            let folder = tempfile::tempdir().expect("folder");
            let path = folder.path().join("mail.db");
            let openers: Vec<_> = (0..6)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || Store::open(&path).map(|_| ()).map_err(|error| format!("{error:#}")))
                })
                .collect();
            for opener in openers {
                assert_eq!(opener.join().expect("thread"), Ok(()), "every opener finds the tables made");
            }
        }
    }

    #[test]
    fn unread_again_is_told_and_the_server_does_not_undo_it() {
        let mut store = Store::in_memory().expect("store");
        let thread = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "x", &[], 1000)));
        store.mark_seen(thread).expect("seen");
        store.told(&store.untold(ME).expect("untold").iter().map(|mail| mail.id).collect::<Vec<_>>()).expect("told");
        store.server_seen(ME, "INBOX", 7, &[1], &[1]).expect("the server: read");
        assert_eq!(store.unread(None).expect("unread"), 0);
        store.mark_unread(thread).expect("unread again");
        assert_eq!(store.unread(None).expect("unread"), 1);
        let untold = store.untold(ME).expect("untold");
        assert_eq!((untold.len(), untold[0].seen), (1, false), "the server is to take \\Seen away");
        store.server_seen(ME, "INBOX", 7, &[1], &[1]).expect("the server still says read");
        assert_eq!(store.unread(None).expect("unread"), 1, "not undone before the server is told");
        store.told(&[untold[0].id]).expect("told");
        assert!(store.untold(ME).expect("untold").is_empty());
        store.archive(thread).expect("archive");
        assert!(store.threads(5, Filter::Inbox, None).expect("list").is_empty());
        store.unarchive(thread).expect("back");
        assert_eq!(store.threads(5, Filter::Inbox, None).expect("list").len(), 1);
        let message = store.thread(thread).expect("read").expect("there").mails[0].id;
        assert_eq!(store.place(message).expect("place"), Some((ME.to_owned(), "INBOX".to_owned(), 7, 1)));
    }

    #[test]
    fn contacts_come_from_every_mail_less_the_mailboxes_own() {
        let mut store = Store::in_memory().expect("store");
        let mut first = mail("a1", "lin@example.com", "x", &[], 1000);
        first.from = Some(Address::new("林晓", "Lin@Example.com"));
        first.cc = vec![Address::new("Alex", "alex@example.com")];
        add(&mut store, "INBOX", 1, &first);
        add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "y", &[], 3000));
        add(&mut store, "INBOX", 3, &mail("a3", "ops@example.com", "z", &[], 2000));
        let contacts = store.contacts(&[ME.to_owned()], 10).expect("contacts");
        let addrs: Vec<&str> = contacts.iter().map(|contact| contact.addr.as_str()).collect();
        assert_eq!(addrs, ["lin@example.com", "ops@example.com", "alex@example.com"], "most often first, then latest");
        assert_eq!((contacts[0].name.as_str(), contacts[0].count, contacts[0].last_at_ms), ("林晓", 2, 3000));
        assert!(!addrs.contains(&ME), "the mailbox's own address is no contact");
        assert_eq!(store.contacts(&[ME.to_owned()], 1).expect("contacts").len(), 1);
    }

    #[test]
    fn many_writers_at_once_wait_for_each_other() {
        // The syncs of several mailboxes write to one file at once: a write that first reads must
        // wait for the others, not fail with "database is locked".
        let folder = tempfile::tempdir().expect("folder");
        let path = folder.path().join("mail.db");
        Store::open(&path).expect("made");
        let writers: Vec<_> = (0..8)
            .map(|writer| {
                let path = path.clone();
                std::thread::spawn(move || -> Result<(), String> {
                    let account = format!("me{writer}@example.com");
                    // The writers are syncs: they wait as a sync does.
                    let mut store = Store::open_background(&path).map_err(|error| format!("{error:#}"))?;
                    for n in 0..300u32 {
                        let id = format!("w{writer}-{n}");
                        let one = mail(&id, "lin@example.com", "x", &[], i64::from(n));
                        store
                            .add(&Incoming {
                                account: &account,
                                folder: "INBOX",
                                uidvalidity: 7,
                                uid: Some(n + 1),
                                mail: &one,
                                seen: false,
                                outgoing: false,
                                received_ms: 0,
                            })
                            .map_err(|error| format!("add: {error:#}"))?;
                        store
                            .server_seen(&account, "INBOX", 7, &[n + 1], &[])
                            .map_err(|error| format!("seen: {error:#}"))?;
                    }
                    Ok(())
                })
            })
            .collect();
        for writer in writers {
            assert_eq!(writer.join().expect("thread"), Ok(()));
        }
        assert_eq!(Store::open(&path).expect("open").unread(None).expect("unread"), 2400);
    }

    #[test]
    fn a_file_of_another_version_is_said_not_overwritten() {
        let folder = tempfile::tempdir().expect("folder");
        let path = folder.path().join("mail.db");
        let store = Store::open(&path).expect("made");
        store.set_status(ME, "synced", Health::Fine, 42).expect("status");
        drop(store);
        let again = Store::open(&path).expect("opened again");
        assert_eq!(again.status(ME).expect("status"), Some(Status { text: "synced".into(), at_ms: 42, failed: false }));
        again.db.execute("UPDATE meta SET value = '99' WHERE key = 'schema'", []).expect("older");
        drop(again);
        assert!(format!("{:#}", Store::open(&path).expect_err("refused")).contains("another version"));
    }

    #[test]
    fn mailboxes_keep_their_own_conversations() {
        const OTHER: &str = "me@other.example";
        let mut store = Store::in_memory().expect("store");
        let ours = mail("x1", "lin@example.com", "发布", &[], 1000);
        let mine = thread_of(add(&mut store, "INBOX", 1, &ours));
        let theirs = thread_of(
            store
                .add(&Incoming {
                    account: OTHER,
                    folder: "INBOX",
                    uidvalidity: 7,
                    uid: Some(1),
                    mail: &ours,
                    seen: false,
                    outgoing: false,
                    received_ms: 0,
                })
                .expect("added"),
        );
        assert_ne!(mine, theirs, "the same mail in two mailboxes: one conversation in each");
        assert_eq!(store.threads(9, Filter::Inbox, None).expect("all").len(), 2);
        let other = store.threads(9, Filter::Inbox, Some(OTHER)).expect("one");
        assert_eq!((other.len(), other[0].account.as_str()), (1, OTHER));
        assert_eq!(store.search("发布", 9, Some(ME)).expect("search").len(), 1);
        assert_eq!((store.unread(None).expect("all"), store.unread(Some(OTHER)).expect("one")), (2, 1));
        store.set_status(OTHER, "synced", Health::Fine, 5).expect("status");
        assert!(store.status(ME).expect("status").is_none(), "each mailbox its own status");
        store.set_folder(OTHER, "INBOX", 7, 1).expect("folder");
        store.forget_account(OTHER).expect("forget");
        assert_eq!(store.threads(9, Filter::All, None).expect("all").len(), 1, "only the other one's mail went");
        assert!(
            store.status(OTHER).expect("status").is_none() && store.folder(OTHER, "INBOX").expect("folder").is_none()
        );
        assert_eq!(store.uids(ME, "INBOX", 7).expect("uids"), [1]);
    }

    #[test]
    fn a_file_of_version_one_is_moved_and_its_rows_taken_by_its_mailbox() {
        let folder = tempfile::tempdir().expect("folder");
        let path = folder.path().join("mail.db");
        let one = Connection::open(&path).expect("open");
        one.execute_batch(
            "CREATE TABLE threads (id INTEGER PRIMARY KEY, subject TEXT NOT NULL, last_at INTEGER NOT NULL,
                 archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, thread_id INTEGER NOT NULL REFERENCES threads(id),
                 folder TEXT NOT NULL, uidvalidity INTEGER NOT NULL, uid INTEGER, message_id TEXT,
                 from_name TEXT NOT NULL, from_addr TEXT NOT NULL, reply_to TEXT, to_json TEXT NOT NULL,
                 cc_json TEXT NOT NULL, subject TEXT NOT NULL, at INTEGER NOT NULL, body TEXT NOT NULL,
                 files_json TEXT NOT NULL, refs TEXT NOT NULL, outgoing INTEGER NOT NULL, seen INTEGER NOT NULL,
                 seen_told INTEGER NOT NULL, UNIQUE (folder, uidvalidity, uid));
             CREATE INDEX messages_by_thread ON messages(thread_id, at);
             CREATE INDEX messages_by_id ON messages(message_id);
             CREATE TABLE ids (message_id TEXT PRIMARY KEY, thread_id INTEGER NOT NULL);
             CREATE TABLE drafts (thread_id INTEGER PRIMARY KEY, author TEXT NOT NULL, body TEXT NOT NULL,
                 at INTEGER NOT NULL);
             CREATE TABLE folders (name TEXT PRIMARY KEY, uidvalidity INTEGER NOT NULL, last_uid INTEGER NOT NULL);
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES ('schema', '1'), ('status', '{\"text\":\"synced\",\"failed\":false,\"at\":3}');
             INSERT INTO threads VALUES (1, '发布', 1000, 0);
             INSERT INTO messages VALUES (1, 1, 'INBOX', 7, 1, 'a1', '', 'lin@example.com', NULL, '[]', '[]',
                 '发布', 1000, 'words', '[]', '', 0, 0, 0);
             INSERT INTO ids VALUES ('a1', 1);
             INSERT INTO drafts VALUES (1, 'claude', '可以', 4);
             INSERT INTO folders VALUES ('INBOX', 7, 1);",
        )
        .expect("a file of version 1");
        drop(one);
        let mut store = Store::open(&path).expect("moved to version 2");
        let nobodys = store.threads(9, Filter::Inbox, None).expect("list");
        assert_eq!((nobodys.len(), nobodys[0].account.as_str()), (1, ""), "kept, nobody's yet");
        store.adopt(ME).expect("adopt");
        let list = store.threads(9, Filter::Inbox, Some(ME)).expect("list");
        assert_eq!(
            (list.len(), list[0].draft_by.as_deref()),
            (1, Some("claude")),
            "the mail and its draft are mine now"
        );
        assert_eq!(store.folder(ME, "INBOX").expect("folder"), Some((7, 1)), "not read again");
        assert_eq!(store.status(ME).expect("status").map(|status| status.at_ms), Some(3));
        let again = mail("a2", "lin@example.com", "Re: 发布", &["a1"], 2000);
        assert_eq!(thread_of(add(&mut store, "INBOX", 2, &again)), list[0].id, "an answer finds it by its id");
        assert_eq!(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "发布", &[], 1000)), Added::Known);
        store.adopt(ME).expect("a second time does nothing");
        assert_eq!(store.threads(9, Filter::All, None).expect("list").len(), 1);
        drop(store);
        Store::open(&path).expect("version 2 opens as it is");
    }
}
