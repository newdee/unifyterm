//! The mailbox on this machine: one SQLite file, `mail.db`. The sync writes
//! what the server has; the window, `ut mail` and `ut mcp` read it, and
//! write drafts and what was read. Each process opens its own connection:
//! the file is in WAL mode, and a writer waits for another.
//!
//! Mails are kept as conversations (threads). A mail joins the conversation
//! of any mail it names in `In-Reply-To` or `References`, or that names it;
//! a mail that names none, but whose subject says it answers ("Re:"), joins
//! the newest conversation with the same plain subject and a person in common.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::Serialize;

use crate::message::{self, Address, Attachment, Parsed};

const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = "
CREATE TABLE threads (
    id       INTEGER PRIMARY KEY,
    subject  TEXT NOT NULL,
    last_at  INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE messages (
    id          INTEGER PRIMARY KEY,
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
    UNIQUE (folder, uidvalidity, uid)
);
CREATE INDEX messages_by_thread ON messages(thread_id, at);
CREATE INDEX messages_by_id ON messages(message_id);
-- Every message id seen, its own or one a mail names, and the conversation it is in.
CREATE TABLE ids (message_id TEXT PRIMARY KEY, thread_id INTEGER NOT NULL);
CREATE TABLE drafts (
    thread_id INTEGER PRIMARY KEY,
    author    TEXT NOT NULL,
    body      TEXT NOT NULL,
    at        INTEGER NOT NULL
);
CREATE TABLE folders (name TEXT PRIMARY KEY, uidvalidity INTEGER NOT NULL, last_uid INTEGER NOT NULL);
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// How long ago the newest mail of a conversation may be for a mail with the
/// same subject to join it when nothing else ties them: 60 days.
const SUBJECT_REACH_MS: i64 = 60 * 86_400_000;
/// How many times the journal is asked to change while another connection holds the file (50 ms apart).
const BUSY_TRIES: u32 = 100;
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

/// A mail the server knows is not seen, while it is seen here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Untold {
    pub id: i64,
    pub folder: String,
    pub uid: u32,
}

/// What the sync last said, for the window to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub at_ms: i64,
    pub failed: bool,
}

impl Store {
    /// The file at `path`, made if it is not there.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
        }
        let db = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        // Changing the journal does not wait for another connection by itself (SQLite gives no busy
        // handler to it): tried again for a while.
        let mut tries = 0;
        loop {
            match db.pragma_update(None, "journal_mode", "WAL") {
                Ok(()) => break,
                Err(rusqlite::Error::SqliteFailure(failure, _))
                    if failure.code == rusqlite::ErrorCode::DatabaseBusy && tries < BUSY_TRIES =>
                {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(error) => return Err(error).context("make mail.db a WAL file"),
            }
        }
        Self::ready(db)
    }

    /// One in memory, for the tests.
    pub fn in_memory() -> Result<Self> {
        Self::ready(Connection::open_in_memory()?)
    }

    /// Makes the tables of a new file, or checks an old one's version. Two processes may open a
    /// new file at once (the window's sync, `ut mcp`): the look and the making are one write
    /// transaction, so that the second waits and then finds the tables made.
    fn ready(mut db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", true)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let has_meta: bool =
            tx.query_row("SELECT count(*) > 0 FROM sqlite_master WHERE name = 'meta'", [], |row| row.get(0))?;
        if has_meta {
            let version: Option<String> =
                tx.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| row.get(0)).optional()?;
            if version.as_deref() != Some(SCHEMA_VERSION) {
                bail!("mail.db is of another version ({version:?}): move it away and the mail is read again");
            }
        } else {
            tx.execute_batch(SCHEMA)?;
            tx.execute("INSERT INTO meta (key, value) VALUES ('schema', ?1)", [SCHEMA_VERSION])?;
        }
        tx.commit()?;
        Ok(Self { db })
    }

    /// Changes when another connection has written: what a window looks at to know it should read again.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.db.query_row("PRAGMA data_version", [], |row| row.get(0))?)
    }

    /// Keeps a mail, in its conversation.
    pub fn add(&mut self, incoming: &Incoming<'_>) -> Result<Added> {
        let mail = incoming.mail;
        let tx = self.db.transaction()?;
        if let Some(id) = &mail.message_id {
            let known: Option<(i64, String, Option<u32>)> = tx
                .query_row("SELECT id, folder, uid FROM messages WHERE message_id = ?1", [id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
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
            thread =
                tx.query_row("SELECT thread_id FROM ids WHERE message_id = ?1", [id], |row| row.get(0)).optional()?;
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
            thread = Self::by_subject(&tx, &plain, mail, at)?;
        }
        let thread = if let Some(thread) = thread {
            thread
        } else {
            tx.execute("INSERT INTO threads (subject, last_at) VALUES (?1, ?2)", params![plain, at])?;
            tx.last_insert_rowid()
        };
        let from = mail.from.clone().unwrap_or_else(|| Address::new("", ""));
        let refs: Vec<&str> = mail.in_reply_to.iter().chain(&mail.references).map(String::as_str).collect();
        tx.execute(
            "INSERT INTO messages (thread_id, folder, uidvalidity, uid, message_id, from_name, from_addr, reply_to,
                 to_json, cc_json, subject, at, body, files_json, refs, outgoing, seen, seen_told)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
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
            ],
        )?;
        let message = tx.last_insert_rowid();
        for id in named {
            tx.execute("INSERT OR IGNORE INTO ids (message_id, thread_id) VALUES (?1, ?2)", params![id, thread])?;
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
    fn by_subject(tx: &rusqlite::Transaction<'_>, plain: &str, mail: &Parsed, at: i64) -> Result<Option<i64>> {
        if plain.is_empty() {
            return Ok(None);
        }
        let people: Vec<&str> =
            mail.from.iter().chain(&mail.to).chain(&mail.cc).map(|address| address.addr.as_str()).collect();
        let mut find = tx.prepare(
            "SELECT t.id, group_concat(m.from_addr || ' ' || m.to_json || ' ' || m.cc_json, ' ')
             FROM threads t JOIN messages m ON m.thread_id = t.id
             WHERE t.subject = ?1 AND t.last_at >= ?2
             GROUP BY t.id ORDER BY t.last_at DESC",
        )?;
        let found = find
            .query_map(params![plain, at - SUBJECT_REACH_MS], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .find(|(_, seen)| people.iter().any(|person| !person.is_empty() && seen.contains(person)))
            .map(|(id, _)| id);
        Ok(found)
    }

    /// The conversations, newest first: at most `limit`; archived ones too only when asked.
    pub fn threads(&self, limit: usize, filter: Filter) -> Result<Vec<Summary>> {
        let ids: Vec<i64> = {
            let mut list = self.db.prepare(
                "SELECT t.id FROM threads t
                 WHERE (?1 OR t.archived = 0)
                   AND (NOT ?2 OR EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.seen = 0))
                 ORDER BY t.last_at DESC LIMIT ?3",
            )?;
            let all = matches!(filter, Filter::All);
            let unread = matches!(filter, Filter::Unread);
            list.query_map(params![all, unread, i64::try_from(limit).unwrap_or(i64::MAX)], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        ids.into_iter().map(|id| self.summary(id)).collect()
    }

    /// The conversations with a mail whose subject, words or sender has `text`, newest first.
    pub fn search(&self, text: &str, limit: usize) -> Result<Vec<Summary>> {
        let pattern = format!("%{}%", text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let ids: Vec<i64> = {
            let mut list = self.db.prepare(
                "SELECT t.id FROM threads t WHERE EXISTS (
                     SELECT 1 FROM messages m WHERE m.thread_id = t.id AND (
                         m.subject LIKE ?1 ESCAPE '\\' OR m.body LIKE ?1 ESCAPE '\\'
                         OR m.from_name LIKE ?1 ESCAPE '\\' OR m.from_addr LIKE ?1 ESCAPE '\\'))
                 ORDER BY t.last_at DESC LIMIT ?2",
            )?;
            list.query_map(params![pattern, i64::try_from(limit).unwrap_or(i64::MAX)], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        ids.into_iter().map(|id| self.summary(id)).collect()
    }

    fn summary(&self, id: i64) -> Result<Summary> {
        let (subject, last_at, archived): (String, i64, bool) = self
            .db
            .query_row("SELECT subject, last_at, archived FROM threads WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
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

    /// How many mails from others are not read, in conversations not archived.
    pub fn unread(&self) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT count(*) FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.seen = 0 AND t.archived = 0",
            [],
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

    /// Mails read here that the server has not been told of.
    pub fn untold(&self) -> Result<Vec<Untold>> {
        let mut list = self.db.prepare(
            "SELECT id, folder, uid FROM messages WHERE seen = 1 AND seen_told = 0 AND uid IS NOT NULL ORDER BY folder",
        )?;
        let rows = list.query_map([], |row| Ok(Untold { id: row.get(0)?, folder: row.get(1)?, uid: row.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The server has been told these are read.
    pub fn told(&self, ids: &[i64]) -> Result<()> {
        for id in ids {
            self.db.execute("UPDATE messages SET seen_told = 1 WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// What the server says of these mails of `folder`: the ones in `seen` are read, the rest of
    /// `checked` are not. A mail read here that the server has not been told of yet stays read.
    pub fn server_seen(&self, folder: &str, uidvalidity: u32, checked: &[u32], seen: &[u32]) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        for uid in checked {
            let on = seen.contains(uid);
            tx.execute(
                "UPDATE messages SET seen = ?4, seen_told = ?4
                 WHERE folder = ?1 AND uidvalidity = ?2 AND uid = ?3 AND outgoing = 0 AND NOT (seen = 1 AND seen_told = 0)",
                params![folder, uidvalidity, uid, on],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The uids of `folder`'s mails kept here.
    pub fn uids(&self, folder: &str, uidvalidity: u32) -> Result<Vec<u32>> {
        let mut list = self.db.prepare(
            "SELECT uid FROM messages WHERE folder = ?1 AND uidvalidity = ?2 AND uid IS NOT NULL ORDER BY uid",
        )?;
        let rows = list.query_map(params![folder, uidvalidity], |row| row.get(0))?;
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
    pub fn folder(&self, name: &str) -> Result<Option<(u32, u32)>> {
        Ok(self
            .db
            .query_row("SELECT uidvalidity, last_uid FROM folders WHERE name = ?1", [name], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?)
    }

    pub fn set_folder(&self, name: &str, uidvalidity: u32, last_uid: u32) -> Result<()> {
        self.db.execute(
            "INSERT INTO folders (name, uidvalidity, last_uid) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO UPDATE SET uidvalidity = excluded.uidvalidity, last_uid = excluded.last_uid",
            params![name, uidvalidity, last_uid],
        )?;
        Ok(())
    }

    /// The server numbered the folder anew (its uid validity changed): what was read of it goes.
    pub fn forget_folder(&self, name: &str) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM messages WHERE folder = ?1", [name])?;
        tx.execute("DELETE FROM folders WHERE name = ?1", [name])?;
        tx.execute("DELETE FROM drafts WHERE thread_id NOT IN (SELECT thread_id FROM messages)", [])?;
        tx.execute("DELETE FROM ids WHERE thread_id NOT IN (SELECT thread_id FROM messages)", [])?;
        tx.execute("DELETE FROM threads WHERE id NOT IN (SELECT thread_id FROM messages)", [])?;
        tx.commit()?;
        Ok(())
    }

    /// What the sync says now.
    pub fn set_status(&self, text: &str, health: Health, at_ms: i64) -> Result<()> {
        let value = serde_json::json!({ "text": text, "failed": health == Health::Failed, "at": at_ms }).to_string();
        self.db.execute(
            "INSERT INTO meta (key, value) VALUES ('status', ?1) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [value],
        )?;
        Ok(())
    }

    pub fn status(&self) -> Result<Option<Status>> {
        let value: Option<String> =
            self.db.query_row("SELECT value FROM meta WHERE key = 'status'", [], |row| row.get(0)).optional()?;
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
            .add(&Incoming { folder, uidvalidity: 7, uid: Some(uid), mail, seen: false, outgoing, received_ms: 0 })
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
        let list = store.threads(10, Filter::Inbox).expect("list");
        assert_eq!(list.iter().map(|summary| summary.id).collect::<Vec<_>>(), [first, other], "newest first");
        assert_eq!(store.unread().expect("unread"), 3);
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
        assert_eq!(store.uids("Sent", 7).expect("uids"), [9], "the row is now the server's copy");
        assert_eq!(add(&mut store, "Sent", 10, &sent), Added::Known, "and is not kept twice");
    }

    #[test]
    fn read_and_told_and_what_the_server_says() {
        let mut store = Store::in_memory().expect("store");
        let thread = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "x", &[], 1000)));
        add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "Re: x", &["a1"], 2000));
        assert_eq!(store.mark_seen(thread).expect("seen"), 2);
        let untold = store.untold().expect("untold");
        assert_eq!(untold.iter().map(|mail| mail.uid).collect::<Vec<_>>(), [1, 2]);
        store.server_seen("INBOX", 7, &[1, 2], &[]).expect("server");
        assert_eq!(store.unread().expect("unread"), 0, "read here and not told yet: stays read");
        store.told(&untold.iter().map(|mail| mail.id).collect::<Vec<_>>()).expect("told");
        store.server_seen("INBOX", 7, &[1, 2], &[2]).expect("server");
        assert_eq!(store.unread().expect("unread"), 1, "told, then the server marked 1 unread again");
    }

    #[test]
    fn drafts_archive_search_and_a_folder_numbered_anew() {
        let mut store = Store::in_memory().expect("store");
        let thread = thread_of(add(&mut store, "INBOX", 1, &mail("a1", "lin@example.com", "发布计划", &[], 1000)));
        store.save_draft(thread, "claude", "可以", 5).expect("draft");
        store.save_draft(thread, "you", "可以，周四", 6).expect("again");
        let draft = store.draft(thread).expect("draft").expect("there");
        assert_eq!((draft.author.as_str(), draft.text.as_str()), ("you", "可以，周四"), "one draft a conversation");
        assert_eq!(store.threads(5, Filter::Inbox).expect("list")[0].draft_by.as_deref(), Some("you"));
        assert!(store.save_draft(999, "x", "y", 0).is_err(), "no such conversation");
        assert_eq!(store.search("计划", 5).expect("search").len(), 1);
        assert_eq!(store.search("lin@", 5).expect("search").len(), 1, "by sender");
        assert!(store.search("100%", 5).expect("search").is_empty(), "% is a character, not a pattern");
        store.archive(thread).expect("archive");
        assert!(store.threads(5, Filter::Inbox).expect("list").is_empty());
        assert_eq!(store.threads(5, Filter::All).expect("list").len(), 1);
        add(&mut store, "INBOX", 2, &mail("a2", "lin@example.com", "Re: 发布计划", &["a1"], 2000));
        assert_eq!(store.threads(5, Filter::Inbox).expect("list").len(), 1, "a new mail brings it back");
        store.set_folder("INBOX", 7, 2).expect("folder");
        store.forget_folder("INBOX").expect("forget");
        assert!(
            store.threads(5, Filter::All).expect("list").is_empty() && store.folder("INBOX").expect("folder").is_none()
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
    fn a_file_of_another_version_is_said_not_overwritten() {
        let folder = tempfile::tempdir().expect("folder");
        let path = folder.path().join("mail.db");
        let store = Store::open(&path).expect("made");
        store.set_status("synced", Health::Fine, 42).expect("status");
        drop(store);
        let again = Store::open(&path).expect("opened again");
        assert_eq!(again.status().expect("status"), Some(Status { text: "synced".into(), at_ms: 42, failed: false }));
        again.db.execute("UPDATE meta SET value = '99' WHERE key = 'schema'", []).expect("older");
        drop(again);
        assert!(format!("{:#}", Store::open(&path).expect_err("refused")).contains("another version"));
    }
}
