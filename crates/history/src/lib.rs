//! uni-term's command history: one SQLite file. The terminal writes a row
//! for each command a shell ran; the terminal's candidate list, `ut history`
//! and `ut mcp` read it.
//!
//! A command's text is stored once (`commands`), with how often it ran and
//! how often it failed; each run of it is a row of `runs` (when, where, how
//! it ended). Searching recalls rows with an index (a prefix of the text)
//! and with FTS5's trigram index (a part of it anywhere), then ranks them
//! in Rust: see [`score`].

mod secrets;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};

pub use secrets::{Refusal, refusal};

/// The environment variable that puts all of uni-term's data (command
/// links, start scripts, history, log) in another directory: tests use it,
/// so that they never touch the user's own.
pub const DATA_DIRECTORY_VARIABLE: &str = "UNI_TERM_DATA_DIR";

/// Where uni-term keeps its data: `UNI_TERM_DATA_DIR` when it is set, else
/// `uni-term` in the system's directory for local application data.
pub fn data_directory() -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os(DATA_DIRECTORY_VARIABLE).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(directory));
    }
    Ok(dirs::data_local_dir().context("this system has no data directory")?.join("uni-term"))
}

/// Now, in milliseconds since 1970.
#[must_use]
pub fn now_ms() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// How long ago `then_ms` was at `now_ms`, in a few characters: `now`, `5m`, `3h`, `2d`.
#[must_use]
pub fn ago(then_ms: i64, now_ms: i64) -> String {
    let minutes = now_ms.saturating_sub(then_ms).max(0) / 60_000;
    match minutes {
        0 => "now".into(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

/// The history file in [`data_directory`].
pub fn default_path() -> Result<PathBuf> {
    Ok(data_directory()?.join("history.db"))
}

/// One command a shell ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    /// When it started, in milliseconds since 1970.
    pub at_ms: i64,
    /// The directory it ran in, with `/` for separators.
    pub cwd: String,
    /// `None` when the shell does not say (cmd.exe).
    pub exit: Option<i32>,
    pub duration_ms: Option<i64>,
    /// `pwsh`, `bash`, `cmd`, ...
    pub shell: String,
}

/// What became of a run given to [`History::record`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Stored,
    /// Not stored, and why.
    Refused(Refusal),
}

/// What to search for.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    /// What the user has typed so far.
    pub text: &'a str,
    /// The directory the shell is in, with `/`.
    pub cwd: &'a str,
    /// Now, in milliseconds since 1970: given, so that a test can fix it.
    pub now_ms: i64,
    pub limit: usize,
}

/// A command found, with what is known of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub count: i64,
    pub fail_count: i64,
    pub last_at_ms: i64,
    /// How its last run ended; `None` when that was not known.
    pub last_exit: Option<i32>,
    /// What [`score`] gave it; 0 for a list not ranked by it.
    pub score: f64,
}

/// The history file, open.
#[derive(Debug)]
pub struct History {
    db: Connection,
}

/// The schema's version: written to `meta`, checked at each open.
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE commands (
    id         INTEGER PRIMARY KEY,
    text       TEXT NOT NULL UNIQUE,
    count      INTEGER NOT NULL,
    ok_count   INTEGER NOT NULL,
    fail_count INTEGER NOT NULL,
    last_at    INTEGER NOT NULL,
    last_exit  INTEGER,
    -- The first one and two characters: what the first keystrokes are compared with.
    head1      TEXT GENERATED ALWAYS AS (substr(text, 1, 1)) VIRTUAL,
    head2      TEXT GENERATED ALWAYS AS (substr(text, 1, 2)) VIRTUAL
);
CREATE TABLE runs (
    id          INTEGER PRIMARY KEY,
    command_id  INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
    at          INTEGER NOT NULL,
    cwd         TEXT NOT NULL,
    exit        INTEGER,
    duration_ms INTEGER,
    shell       TEXT NOT NULL
);
CREATE INDEX runs_by_command ON runs(command_id, at);
CREATE INDEX runs_by_cwd ON runs(cwd, command_id);
CREATE INDEX commands_by_time ON commands(last_at);
-- A prefix of the text, newest first, read from the index alone. One or two characters
-- match a large share of the commands: those are found by the first characters, already
-- in time order, instead of being sorted.
CREATE INDEX commands_by_text ON commands(text, last_at);
CREATE INDEX commands_by_head1 ON commands(head1, last_at);
CREATE INDEX commands_by_head2 ON commands(head2, last_at);
CREATE VIRTUAL TABLE commands_fts USING fts5(text, content='commands', content_rowid='id', tokenize='trigram');
CREATE TRIGGER commands_added AFTER INSERT ON commands BEGIN
    INSERT INTO commands_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER commands_removed AFTER DELETE ON commands BEGIN
    INSERT INTO commands_fts(commands_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The columns every list of candidates reads, from `commands c`.
const CANDIDATE_COLUMNS: &str = "c.id, c.text, c.count, c.fail_count, c.last_at, c.last_exit";

/// How many rows each way of recalling gives the ranking at most: the newest
/// of them. The list shows 8; frequent commands are also recent ones.
const RECALL: usize = 100;
/// How much of the file is read through memory mapping, in bytes: reads
/// then cost no system call. 100 000 commands take about 32 MiB.
const MAP_BYTES: i64 = 256 * 1024 * 1024;
/// Below this many characters a query is a prefix only: the trigram index needs three.
const TRIGRAM: usize = 3;
/// Milliseconds in a day.
const DAY_MS: f64 = 86_400_000.0;

/// What makes one candidate rank above another, as weights of [`score`].
const PREFIX_WEIGHT: f64 = 4.0;
const FREQUENCY_WEIGHT: f64 = 1.0;
const RECENCY_WEIGHT: f64 = 2.0;
/// Days after which recency has fallen to 1/e.
const RECENCY_DAYS: f64 = 7.0;
const HERE_WEIGHT: f64 = 1.5;
const FAILURE_WEIGHT: f64 = 2.0;

/// What is known of a recalled command, for ranking it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facts {
    /// It begins with what was typed.
    pub prefix: bool,
    pub count: i64,
    pub fail_count: i64,
    /// Days since it last ran.
    pub days: f64,
    /// It ran in the directory the shell is in.
    pub here: bool,
}

/// A candidate's rank: higher first.
///
/// `4 × prefix + ln(1 + count) + 2 × e^(−days/7) + 1.5 × here − 2 × failures/count`.
/// A command that begins with what was typed comes first; among those,
/// one used often, lately, here and without failing.
#[must_use]
pub fn score(facts: Facts) -> f64 {
    let flag = |yes: bool| if yes { 1.0 } else { 0.0 };
    let count = to_f64(facts.count.max(1));
    PREFIX_WEIGHT * flag(facts.prefix)
        + FREQUENCY_WEIGHT * count.ln_1p()
        + RECENCY_WEIGHT * (-facts.days.max(0.0) / RECENCY_DAYS).exp()
        + HERE_WEIGHT * flag(facts.here)
        - FAILURE_WEIGHT * to_f64(facts.fail_count.clamp(0, facts.count.max(1))) / count
}

/// A count as a float. Counts of commands stay far below 2^53, where this is exact.
#[expect(clippy::as_conversions, clippy::cast_precision_loss, reason = "counts are far below 2^53")]
fn to_f64(count: i64) -> f64 {
    count as f64
}

impl History {
    /// Opens the history file for reading and writing, making it (and its
    /// directory) when it is not there.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(directory) = path.parent().filter(|directory| !directory.as_os_str().is_empty()) {
            std::fs::create_dir_all(directory).with_context(|| format!("make {}", directory.display()))?;
        }
        let db = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        // Write-ahead logging: the terminal writes while `ut history` reads.
        db.pragma_update(None, "journal_mode", "WAL")?;
        // No flush to disk at each command: a crash of the program loses nothing, a power cut the last few.
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.pragma_update(None, "mmap_size", MAP_BYTES)?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        let history = Self { db };
        history.migrate()?;
        Ok(history)
    }

    /// Opens the history file only to read it; `None` when there is none.
    /// Nothing is made.
    pub fn open_read_only(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let db = Connection::open_with_flags(path, flags).with_context(|| format!("open {}", path.display()))?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        db.pragma_update(None, "mmap_size", MAP_BYTES)?;
        let history = Self { db };
        match history.version()? {
            Some(SCHEMA_VERSION) => Ok(Some(history)),
            Some(other) => {
                bail!("{} is of version {other}; this program reads version {SCHEMA_VERSION}", path.display())
            }
            None => bail!("{} is not a uni-term history file", path.display()),
        }
    }

    /// An empty history in memory, for tests.
    pub fn in_memory() -> Result<Self> {
        let history = Self { db: Connection::open_in_memory()? };
        history.db.pragma_update(None, "foreign_keys", true)?;
        history.migrate()?;
        Ok(history)
    }

    fn version(&self) -> Result<Option<i64>> {
        let has_meta: bool = self.db.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
            [],
            |row| row.get(0),
        )?;
        if !has_meta {
            return Ok(None);
        }
        Ok(self.db.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| row.get(0)).optional()?)
    }

    /// Makes the tables in a new file; checks the version of an old one.
    fn migrate(&self) -> Result<()> {
        match self.version()? {
            Some(SCHEMA_VERSION) => Ok(()),
            Some(other) => bail!("the history file is of version {other}; this program knows version {SCHEMA_VERSION}"),
            None => {
                self.db.execute_batch(&format!(
                    "BEGIN; {SCHEMA} INSERT INTO meta (key, value) VALUES ('schema', {SCHEMA_VERSION}); COMMIT;"
                ))?;
                Ok(())
            }
        }
    }

    /// Stores a run, unless its text is one not to keep (see [`refusal`]).
    pub fn record(&mut self, run: &Run) -> Result<Recorded> {
        if let Some(why) = refusal(&run.text) {
            return Ok(Recorded::Refused(why));
        }
        let (ok, failed) = match run.exit {
            Some(0) => (1, 0),
            Some(_) => (0, 1),
            None => (0, 0),
        };
        let transaction = self.db.transaction()?;
        let id: i64 = transaction.query_row(
            "INSERT INTO commands (text, count, ok_count, fail_count, last_at, last_exit) VALUES (?1, 1, ?2, ?3, ?4, ?5)
             ON CONFLICT (text) DO UPDATE SET count = count + 1, ok_count = ok_count + excluded.ok_count,
                 fail_count = fail_count + excluded.fail_count,
                 last_exit = CASE WHEN excluded.last_at >= last_at THEN excluded.last_exit ELSE last_exit END,
                 last_at = max(last_at, excluded.last_at)
             RETURNING id",
            params![run.text, ok, failed, run.at_ms, run.exit],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO runs (command_id, at, cwd, exit, duration_ms, shell) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, run.at_ms, run.cwd, run.exit, run.duration_ms, run.shell],
        )?;
        transaction.commit()?;
        Ok(Recorded::Stored)
    }

    /// The commands that match what was typed, best first: those that
    /// begin with it and (from three characters) those that have it
    /// anywhere, case aside. Nothing typed: the commands ranked without the prefix.
    pub fn search(&self, query: &Query<'_>) -> Result<Vec<Candidate>> {
        let mut recalled: Vec<(i64, Candidate)> = Vec::new();
        let mut add = |rows: Vec<(i64, Candidate)>| {
            for (id, candidate) in rows {
                if !recalled.iter().any(|(seen, _)| *seen == id) {
                    recalled.push((id, candidate));
                }
            }
        };
        let limit = i64::try_from(RECALL).unwrap_or(i64::MAX);
        if query.text.is_empty() {
            add(self.candidates(&format!("ORDER BY c.last_at DESC LIMIT {limit}"), params![])?);
        } else {
            // The newest commands that begin with the text.
            let prefixed = match query.text.chars().count() {
                1 => format!(
                    "SELECT id FROM commands INDEXED BY commands_by_head1 WHERE head1 = ?1 ORDER BY last_at DESC LIMIT {limit}"
                ),
                2 => format!(
                    "SELECT id FROM commands INDEXED BY commands_by_head2 WHERE head2 = ?1 ORDER BY last_at DESC LIMIT {limit}"
                ),
                // From the text up to the text followed by the last character there is: few rows by now.
                _ => format!(
                    "SELECT id FROM commands INDEXED BY commands_by_text WHERE text >= ?1 AND text < ?1 || char(1114111)
                     ORDER BY last_at DESC LIMIT {limit}"
                ),
            };
            let prefix_rows = self.candidates(&format!("WHERE c.id IN ({prefixed})"), params![query.text])?;
            // As many commands begin with the text as are taken: they fill the list, each 4 ahead
            // of any match elsewhere in the text. The search inside the text (the slow part) is left out.
            let full = prefix_rows.len() >= RECALL;
            add(prefix_rows);
            if !full && query.text.chars().count() >= TRIGRAM {
                // Anywhere in the text: the newest commands first, by the order the text index keeps (rowid).
                let phrase = format!("\"{}\"", query.text.replace('"', "\"\""));
                add(self.candidates(
                    &format!(
                        "WHERE c.id IN (SELECT rowid FROM commands_fts WHERE commands_fts MATCH ?1
                                        ORDER BY rowid DESC LIMIT {limit})"
                    ),
                    params![phrase],
                )?);
            }
        }
        let here = self.ran_here(query.cwd, recalled.iter().map(|(id, _)| *id))?;
        let mut ranked: Vec<Candidate> = recalled
            .into_iter()
            .map(|(id, mut candidate)| {
                candidate.score = score(Facts {
                    prefix: !query.text.is_empty() && candidate.text.starts_with(query.text),
                    count: candidate.count,
                    fail_count: candidate.fail_count,
                    days: to_f64(query.now_ms.saturating_sub(candidate.last_at_ms)) / DAY_MS,
                    here: here.contains(&id),
                });
                candidate
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.score.total_cmp(&a.score).then(b.last_at_ms.cmp(&a.last_at_ms)).then_with(|| a.text.cmp(&b.text))
        });
        ranked.truncate(query.limit);
        Ok(ranked)
    }

    /// The commands last run, newest first; with `cwd`, only those run there.
    pub fn recent(&self, limit: usize, cwd: Option<&str>) -> Result<Vec<Candidate>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = match cwd {
            Some(cwd) => self.candidates(
                &format!(
                    "WHERE c.id IN (SELECT command_id FROM runs WHERE cwd = ?1) ORDER BY c.last_at DESC, c.id DESC LIMIT {limit}"
                ),
                params![cwd],
            )?,
            None => self.candidates(&format!("ORDER BY c.last_at DESC, c.id DESC LIMIT {limit}"), params![])?,
        };
        Ok(rows.into_iter().map(|(_, candidate)| candidate).collect())
    }

    /// Removes a command and every run of it; how many commands that was (0 or 1).
    pub fn forget(&mut self, text: &str) -> Result<usize> {
        Ok(self.db.execute("DELETE FROM commands WHERE text = ?1", params![text])?)
    }

    /// How many commands and runs there are.
    pub fn size(&self) -> Result<(i64, i64)> {
        let commands = self.db.query_row("SELECT count(*) FROM commands", [], |row| row.get(0))?;
        let runs = self.db.query_row("SELECT count(*) FROM runs", [], |row| row.get(0))?;
        Ok((commands, runs))
    }

    fn candidates(&self, rest: &str, values: impl rusqlite::Params) -> Result<Vec<(i64, Candidate)>> {
        let mut statement = self.db.prepare_cached(&format!("SELECT {CANDIDATE_COLUMNS} FROM commands c {rest}"))?;
        let rows = statement.query_map(values, |row| {
            Ok((
                row.get(0)?,
                Candidate {
                    text: row.get(1)?,
                    count: row.get(2)?,
                    fail_count: row.get(3)?,
                    last_at_ms: row.get(4)?,
                    last_exit: row.get(5)?,
                    score: 0.0,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Which of these commands ran in `cwd`: one query, the ids given as a JSON array.
    fn ran_here(&self, cwd: &str, ids: impl Iterator<Item = i64>) -> Result<Vec<i64>> {
        let list = format!("[{}]", ids.map(|id| id.to_string()).collect::<Vec<_>>().join(","));
        let mut statement = self.db.prepare_cached(
            "SELECT DISTINCT command_id FROM runs WHERE cwd = ?1 AND command_id IN (SELECT value FROM json_each(?2))",
        )?;
        let here = statement.query_map(params![cwd, list], |row| row.get(0))?;
        Ok(here.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const HOUR: i64 = 3_600_000;

    fn run(text: &str, hours_ago: i64, cwd: &str, exit: Option<i32>) -> Run {
        Run {
            text: text.into(),
            at_ms: NOW - hours_ago * HOUR,
            cwd: cwd.into(),
            exit,
            duration_ms: Some(10),
            shell: "pwsh".into(),
        }
    }

    fn texts(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|candidate| candidate.text.as_str()).collect()
    }

    fn query(text: &str) -> Query<'_> {
        Query { text, cwd: "C:/work", now_ms: NOW, limit: 10 }
    }

    #[test]
    fn a_run_is_kept_and_found() {
        let mut history = History::in_memory().unwrap();
        assert_eq!(history.record(&run("git status", 1, "C:/work", Some(0))).unwrap(), Recorded::Stored);
        assert_eq!(history.record(&run("git status", 0, "C:/work", Some(1))).unwrap(), Recorded::Stored);
        let found = history.search(&query("git")).unwrap();
        assert_eq!(texts(&found), ["git status"]);
        assert_eq!((found[0].count, found[0].fail_count, found[0].last_exit), (2, 1, Some(1)));
        assert_eq!(found[0].last_at_ms, NOW);
        assert_eq!(history.size().unwrap(), (1, 2));
    }

    #[test]
    fn a_prefix_and_a_part_anywhere_are_both_found() {
        let mut history = History::in_memory().unwrap();
        for text in ["git commit -m wip", "cargo test", "echo commit", "Git Log"] {
            history.record(&run(text, 1, "C:/work", Some(0))).unwrap();
        }
        assert_eq!(texts(&history.search(&query("c")).unwrap()), ["cargo test"], "one character: a prefix only");
        assert_eq!(texts(&history.search(&query("ec")).unwrap()), ["echo commit"]);
        let found = history.search(&query("commit")).unwrap();
        assert_eq!(found.len(), 2, "three characters or more: anywhere");
        assert!(texts(&history.search(&query("git l")).unwrap()).contains(&"Git Log"), "case aside");
        assert!(history.search(&query("zzz")).unwrap().is_empty());
        assert!(history.search(&query("\"; DROP")).unwrap().is_empty(), "quotes are text, not query syntax");
        assert_eq!(history.search(&query("")).unwrap().len(), 4, "nothing typed: all of them, ranked");
    }

    #[test]
    fn the_search_inside_the_text_is_left_out_when_the_prefix_fills_the_list() {
        let mut history = History::in_memory().unwrap();
        history.record(&run("echo git", 0, "C:/work", Some(0))).unwrap();
        assert_eq!(texts(&history.search(&query("git")).unwrap()), ["echo git"], "few with the prefix: found inside");
        for number in 0..RECALL {
            history.record(&run(&format!("git log -{number}"), 1, "C:/work", Some(0))).unwrap();
        }
        let found = history.search(&Query { limit: RECALL + 5, ..query("git") }).unwrap();
        assert_eq!(found.len(), RECALL);
        assert!(found.iter().all(|candidate| candidate.text.starts_with("git")), "a full prefix: none from inside");
    }

    #[test]
    fn the_ranking_is_the_one_written_down() {
        let mut history = History::in_memory().unwrap();
        // Six commands with "test" in them, made to rank in one order.
        history.record(&run("cargo test --release", 24 * 30, "C:/other", Some(0))).unwrap(); // old, elsewhere
        history.record(&run("test-it now", 1, "C:/work", Some(0))).unwrap(); // the prefix
        for _ in 0..5 {
            history.record(&run("npm test", 2, "C:/work", Some(0))).unwrap(); // often, here
        }
        for _ in 0..5 {
            history.record(&run("make test", 2, "C:/work", Some(2))).unwrap(); // as often, always failing
        }
        history.record(&run("go test ./...", 1, "C:/other", Some(0))).unwrap(); // lately, elsewhere
        history.record(&run("pytest -x", 1, "C:/work", Some(0))).unwrap(); // lately, here
        let found = history.search(&query("test")).unwrap();
        // By the formula: 8.18, 5.27, 4.18, 3.27, 2.68, 0.72. Five runs here, failing, still
        // outrank one run elsewhere: frequency and place outweigh the failures.
        assert_eq!(
            texts(&found),
            ["test-it now", "npm test", "pytest -x", "make test", "go test ./...", "cargo test --release"],
            "{found:#?}"
        );
        let scores: Vec<f64> = found.iter().map(|candidate| (candidate.score * 100.0).round() / 100.0).collect();
        assert_eq!(scores, [8.18, 5.27, 4.18, 3.27, 2.68, 0.72]);
        assert!(found[1].score > found[4].score, "a failing command ranks below the same one succeeding");
    }

    #[test]
    fn the_score_has_its_parts() {
        let base = Facts { prefix: false, count: 1, fail_count: 0, days: 0.0, here: false };
        let at = |facts: Facts| score(facts);
        assert!((at(base) - (2.0_f64.ln() + 2.0)).abs() < 1e-9);
        assert!((at(Facts { prefix: true, ..base }) - at(base) - 4.0).abs() < 1e-9);
        assert!((at(Facts { here: true, ..base }) - at(base) - 1.5).abs() < 1e-9);
        assert!((at(Facts { fail_count: 1, ..base }) - at(base) + 2.0).abs() < 1e-9);
        assert!((at(Facts { days: 7.0, ..base }) - (2.0_f64.ln() + 2.0 / std::f64::consts::E)).abs() < 1e-9);
        assert!(at(Facts { days: -3.0, ..base }) <= at(base) + 1e-9, "a clock that went back counts as now");
    }

    #[test]
    fn recent_is_newest_first_and_by_directory() {
        let mut history = History::in_memory().unwrap();
        history.record(&run("one", 3, "C:/a", Some(0))).unwrap();
        history.record(&run("two", 2, "C:/b", Some(0))).unwrap();
        history.record(&run("three", 1, "C:/a", None)).unwrap();
        assert_eq!(texts(&history.recent(10, None).unwrap()), ["three", "two", "one"]);
        assert_eq!(texts(&history.recent(10, Some("C:/a")).unwrap()), ["three", "one"]);
        assert_eq!(texts(&history.recent(1, None).unwrap()), ["three"]);
        assert_eq!(history.recent(10, None).unwrap()[0].last_exit, None);
    }

    #[test]
    fn a_secret_is_not_kept() {
        let mut history = History::in_memory().unwrap();
        let refused = history.record(&run("mysql --password hunter2", 0, "C:/work", Some(0))).unwrap();
        assert_eq!(refused, Recorded::Refused(Refusal::Secret));
        assert_eq!(
            history.record(&run(" ls", 0, "C:/work", Some(0))).unwrap(),
            Recorded::Refused(Refusal::LeadingSpace)
        );
        assert_eq!(history.size().unwrap(), (0, 0));
    }

    #[test]
    fn forgetting_removes_every_trace() {
        let mut history = History::in_memory().unwrap();
        history.record(&run("rm -rf build", 1, "C:/work", Some(0))).unwrap();
        history.record(&run("rm -rf build", 0, "C:/work", Some(0))).unwrap();
        history.record(&run("ls", 0, "C:/work", Some(0))).unwrap();
        assert_eq!(history.forget("rm -rf build").unwrap(), 1);
        assert_eq!(history.forget("rm -rf build").unwrap(), 0);
        assert_eq!(history.size().unwrap(), (1, 1), "its runs went with it");
        assert!(history.search(&query("build")).unwrap().is_empty(), "and the text index forgot it too");
    }

    #[test]
    fn a_file_is_made_reopened_and_read_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deeper/history.db");
        assert!(History::open_read_only(&path).unwrap().is_none(), "no file: nothing, and nothing made");
        assert!(!path.exists());
        History::open(&path).unwrap().record(&run("ls", 0, "C:/work", Some(0))).unwrap();
        let again = History::open(&path).unwrap();
        assert_eq!(again.size().unwrap(), (1, 1));
        let reader = History::open_read_only(&path).unwrap().unwrap();
        assert_eq!(texts(&reader.recent(5, None).unwrap()), ["ls"]);
        std::fs::write(dir.path().join("other.db"), b"not a database at all, just text").unwrap();
        assert!(History::open(&dir.path().join("other.db")).is_err(), "a damaged file is an error, not a crash");
    }

    #[test]
    fn a_time_ago_is_a_few_characters() {
        let now = 1_000_000_000_000;
        assert_eq!(ago(now, now), "now");
        assert_eq!(ago(now + 5_000, now), "now", "a clock that went back");
        assert_eq!(ago(now - 59_999, now), "now");
        assert_eq!(ago(now - 5 * 60_000, now), "5m");
        assert_eq!(ago(now - 3 * 3_600_000, now), "3h");
        assert_eq!(ago(now - 49 * 3_600_000, now), "2d");
        assert!(now_ms() > 1_700_000_000_000, "after 2023");
    }

    #[test]
    fn the_data_directory_can_be_moved() {
        // Read only: the variable is not changed here, tests run side by side.
        let directory = data_directory().unwrap();
        match std::env::var_os(DATA_DIRECTORY_VARIABLE).filter(|value| !value.is_empty()) {
            Some(moved) => assert_eq!(directory, PathBuf::from(moved)),
            None => assert!(directory.ends_with("uni-term")),
        }
        assert_eq!(default_path().unwrap(), directory.join("history.db"));
    }
}
