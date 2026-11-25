//! Where UnifyTerm says what went wrong while it runs: stderr, and a file.
//!
//! A release build on Windows is a window program with no stderr anyone
//! sees, so the file is the only place such a message can be read:
//! `unifyterm.log` in the user's local data directory
//! (`%LOCALAPPDATA%\unifyterm`, `~/Library/Application Support/unifyterm`,
//! `~/.local/share/unifyterm`).

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, Log, Metadata, Record};

/// What is written when nothing says otherwise: warnings and errors,
/// UnifyTerm's own and its libraries'.
const LEVEL: Level = Level::Warn;
/// The variable that asks for more: `info` or `debug`, for looking into a
/// problem (which settings were taken, what the system was told).
const LEVEL_VARIABLE: &str = "UNIFYTERM_LOG";

/// The level `asked` names; the usual one for anything else.
fn level_of(asked: Option<&str>) -> Level {
    match asked.map(str::trim) {
        Some("info") => Level::Info,
        Some("debug") => Level::Debug,
        _ => LEVEL,
    }
}
/// A log file larger than this is started over when UnifyTerm starts, so it
/// cannot grow without end.
const MAX_BYTES: u64 = 1024 * 1024;

struct Logger {
    /// `None` when the file could not be opened: stderr alone is written.
    file: Option<Mutex<File>>,
    /// Messages less grave than this are dropped.
    level: Level,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Seconds since 1970: enough to order messages and match them to a time.
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let line = format!("{seconds} {} {}: {}\n", record.level(), record.target(), record.args());
        // A message that cannot be written has nowhere else to go.
        let _ = std::io::stderr().write_all(line.as_bytes());
        if let Some(file) = &self.file {
            let _ = file.lock().expect("lock poisoned").write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

/// The log file's path; `None` on a system with no data directory.
pub(crate) fn path() -> Option<PathBuf> {
    unifyterm_history::data_directory().ok().map(|dir| dir.join("unifyterm.log"))
}

/// Opens the log file for appending, starting it over when it has grown large.
fn open(path: &PathBuf) -> Option<File> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    let large = std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_BYTES);
    OpenOptions::new().create(true).append(!large).write(true).truncate(large).open(path).ok()
}

/// Makes `log::error!` and `log::warn!` write to stderr and the log file.
/// Called once, before the window opens.
pub(crate) fn init() {
    let file = path().as_ref().and_then(open).map(Mutex::new);
    let level = level_of(std::env::var(LEVEL_VARIABLE).ok().as_deref());
    // Set twice only by a second call, which changes nothing.
    if log::set_boxed_logger(Box::new(Logger { file, level })).is_ok() {
        log::set_max_level(level.to_level_filter());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_is_warnings_unless_the_variable_asks_for_more() {
        assert_eq!(level_of(None), Level::Warn);
        assert_eq!(level_of(Some("info")), Level::Info);
        assert_eq!(level_of(Some(" debug ")), Level::Debug);
        for other in ["", "INFO", "trace", "1"] {
            assert_eq!(level_of(Some(other)), Level::Warn, "{other}");
        }
    }

    #[test]
    fn a_large_log_is_started_over_and_a_small_one_kept() {
        let dir = std::env::temp_dir().join(format!("unifyterm-log-test-{}", std::process::id()));
        let file = dir.join("nested").join("unifyterm.log");
        drop(open(&file).expect("the directory is made on the way"));
        std::fs::write(&file, "earlier line\n").unwrap();
        open(&file).unwrap().write_all(b"new line\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "earlier line\nnew line\n", "appended");

        let big = usize::try_from(MAX_BYTES).unwrap() + 1;
        std::fs::write(&file, vec![b'x'; big]).unwrap();
        open(&file).unwrap().write_all(b"fresh\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fresh\n", "started over");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_warnings_and_errors_are_written() {
        let logger = Logger { file: None, level: LEVEL };
        let level = |level| logger.enabled(&Metadata::builder().level(level).build());
        assert!(level(Level::Error) && level(Level::Warn));
        assert!(!level(Level::Info) && !level(Level::Debug));
        // Asked for more: what was told is written too, what is only for finding a fault is not.
        let more = Logger { file: None, level: Level::Info };
        let level = |level| more.enabled(&Metadata::builder().level(level).build());
        assert!(level(Level::Warn) && level(Level::Info) && !level(Level::Debug));
    }
}
