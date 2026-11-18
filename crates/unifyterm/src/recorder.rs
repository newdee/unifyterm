//! The history file, kept by a thread of its own: the window hands it
//! commands to store and questions to answer, and never waits for it.

use std::path::Path;
use std::sync::mpsc;

use anyhow::{Context as _, Result};
use unifyterm_history::{Candidate, History, Query, Recorded, Run, now_ms};

use crate::prompt::Finished;

/// What the window asks of the thread.
enum Request {
    Record(Run),
    Search { text: String, cwd: String, limit: usize, reply: smol::channel::Sender<Vec<Candidate>> },
}

/// The handle the window keeps. Cloning it shares the one thread.
#[derive(Debug, Clone)]
pub(crate) struct Recorder {
    requests: mpsc::Sender<Request>,
}

impl Recorder {
    /// Opens the history file and starts the thread that keeps it. An
    /// error (a damaged file, no room) is the caller's to tell the user.
    pub(crate) fn start(path: &Path) -> Result<Self> {
        let mut history = History::open(path)?;
        let (requests, inbox) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("history".into())
            .spawn(move || {
                // Ends when every handle is gone: the window closed.
                while let Ok(request) = inbox.recv() {
                    match request {
                        Request::Record(run) => match history.record(&run) {
                            Ok(Recorded::Stored | Recorded::Refused(_)) => {}
                            Err(error) => log::error!("history: {error:#}"),
                        },
                        Request::Search { text, cwd, limit, reply } => {
                            let query = Query { text: &text, cwd: &cwd, now_ms: now_ms(), limit };
                            let found = history.search(&query).unwrap_or_else(|error| {
                                log::error!("history: {error:#}");
                                Vec::new()
                            });
                            // The asker may be gone (the tab closed): nobody to answer.
                            let _ = reply.try_send(found);
                        }
                    }
                }
            })
            .context("start the history thread")?;
        Ok(Self { requests })
    }

    /// Stores a command that ended, as its shell's hook told it.
    pub(crate) fn record(&self, finished: &Finished, shell: &str) {
        let duration = finished.duration_ms.unwrap_or(0);
        let run = Run {
            text: finished.text.clone(),
            at_ms: now_ms().saturating_sub(duration),
            cwd: finished.cwd.clone().unwrap_or_default(),
            exit: finished.exit,
            duration_ms: finished.duration_ms,
            shell: finished.shell.clone().unwrap_or_else(|| shell.to_owned()),
        };
        // The thread is gone only when the window is: nothing to keep then.
        let _ = self.requests.send(Request::Record(run));
    }

    /// Asks for the commands that match `text`; the answer comes on the channel returned.
    pub(crate) fn search(&self, text: &str, cwd: &str, limit: usize) -> smol::channel::Receiver<Vec<Candidate>> {
        let (reply, answer) = smol::channel::bounded(1);
        let request = Request::Search { text: text.to_owned(), cwd: cwd.to_owned(), limit, reply };
        let _ = self.requests.send(request);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_recorded_is_found_and_the_thread_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Recorder::start(&dir.path().join("history.db")).unwrap();
        let finished = |text: &str, exit| Finished {
            text: text.into(),
            cwd: Some("C:/work".into()),
            exit,
            duration_ms: Some(5),
            shell: Some("pwsh".into()),
            line: None,
            output: String::new(),
        };
        recorder.record(&finished("git status", Some(0)), "pwsh");
        recorder.record(&finished("git push", Some(1)), "pwsh");
        recorder.record(&finished("echo --password hunter2", Some(0)), "pwsh");
        let found = smol::block_on(recorder.search("git", "C:/work", 8).recv()).unwrap();
        let texts: Vec<&str> = found.iter().map(|candidate| candidate.text.as_str()).collect();
        assert_eq!(texts, ["git status", "git push"], "the failed push ranks below; the secret was not kept");
        let none = smol::block_on(recorder.search("password", "C:/work", 8).recv()).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn a_damaged_file_is_an_error_to_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        std::fs::write(&path, b"this is not a database file, it is plain text of some length").unwrap();
        assert!(Recorder::start(&path).is_err());
    }
}
