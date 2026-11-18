//! `ut history`: the command history UnifyTerm keeps, from the command line.
//!
//!     ut history recent [-n N] [--cwd DIR] [--long]
//!     ut history search TEXT [-n N] [--cwd DIR] [--long]
//!     ut history forget TEXT
//!
//! The file is UnifyTerm's (`history.db` in its data directory, or in
//! `UNIFYTERM_DATA_DIR`). Reading opens it read-only and makes nothing when
//! it is not there.

use std::ffi::OsString;
use std::io::{self, Write as _};

use unifyterm_history::{Candidate, History, Query, ago, now_ms};

const USAGE: &str = "usage: ut history recent [-n N] [--cwd DIR] [--long]
       ut history search TEXT [-n N] [--cwd DIR] [--long]
       ut history forget TEXT

  recent   the commands last run, newest first
  search   the commands that begin with TEXT or have it in them, best first
  forget   remove a command and every run of it
  -n N     at most N commands (default 20)
  --cwd    only commands run in DIR (recent), or rank those first (search)
  --long   also how often it ran, how it last ended, and when";

const USAGE_ERROR: i32 = 2;
const TROUBLE: i32 = 1;
const DEFAULT_COUNT: usize = 20;

/// What was asked.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    Recent,
    Search(String),
    Forget(String),
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    ask: Ask,
    count: usize,
    cwd: Option<String>,
    long: Detail,
}

/// How much to print of each command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detail {
    Text,
    Long,
}

fn parse(args: &[OsString]) -> Result<Request, String> {
    let words: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let mut words = words.into_iter();
    let what = words.next().ok_or("say what: recent, search or forget")?;
    let (mut text, mut count, mut cwd, mut long) = (None, DEFAULT_COUNT, None, Detail::Text);
    while let Some(word) = words.next() {
        match word.as_str() {
            "-n" => {
                let value = words.next().ok_or("-n needs a number")?;
                count = value.parse().map_err(|_| format!("-n {value}: not a number"))?;
            }
            "--cwd" => cwd = Some(words.next().ok_or("--cwd needs a directory")?.replace('\\', "/")),
            "--long" | "-l" => long = Detail::Long,
            other if other.starts_with('-') && other.len() > 1 => return Err(format!("unknown option '{other}'")),
            _ if text.is_none() => text = Some(word),
            _ => return Err(format!("unexpected argument '{word}'")),
        }
    }
    let ask = match (what.as_str(), text) {
        ("recent", None) => Ask::Recent,
        ("recent", Some(extra)) => return Err(format!("recent takes no text: '{extra}'")),
        ("search", Some(text)) => Ask::Search(text),
        ("forget", Some(text)) => Ask::Forget(text),
        ("search" | "forget", None) => return Err(format!("{what} needs the command's text")),
        (other, _) => return Err(format!("unknown: '{other}'; recent, search or forget")),
    };
    Ok(Request { ask, count, cwd, long })
}

fn print(found: &[Candidate], long: Detail, out: &mut impl io::Write) -> io::Result<()> {
    let now = now_ms();
    for candidate in found {
        match long {
            Detail::Text => writeln!(out, "{}", candidate.text)?,
            Detail::Long => {
                let exit = candidate.last_exit.map_or_else(|| "-".to_owned(), |exit| exit.to_string());
                let when = ago(candidate.last_at_ms, now);
                writeln!(out, "{:>5} {exit:>4} {when:>4}  {}", candidate.count, candidate.text)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let request = match parse(args) {
        Ok(request) => request,
        Err(problem) => {
            let _ = writeln!(io::stderr(), "ut history: {problem}\n{USAGE}");
            return USAGE_ERROR;
        }
    };
    match run(&request, &mut io::stdout().lock()) {
        Ok(code) => code,
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut history: {error:#}");
            TROUBLE
        }
    }
}

fn run(request: &Request, out: &mut impl io::Write) -> anyhow::Result<i32> {
    let path = unifyterm_history::default_path()?;
    if let Ask::Forget(text) = &request.ask {
        if !path.is_file() {
            return Ok(TROUBLE);
        }
        let removed = History::open(&path)?.forget(text)?;
        // Like grep: 1 when there was nothing to remove.
        return Ok(if removed == 0 { TROUBLE } else { 0 });
    }
    let Some(history) = History::open_read_only(&path)? else { return Ok(0) };
    let found = match &request.ask {
        Ask::Recent => history.recent(request.count, request.cwd.as_deref())?,
        Ask::Search(text) => {
            let cwd = request.cwd.clone().unwrap_or_default();
            history.search(&Query { text, cwd: &cwd, now_ms: now_ms(), limit: request.count })?
        }
        Ask::Forget(_) => Vec::new(),
    };
    print(&found, request.long, out)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_arguments_are_read() {
        assert_eq!(
            parse(&args(&["recent"])).unwrap(),
            Request { ask: Ask::Recent, count: 20, cwd: None, long: Detail::Text }
        );
        assert_eq!(
            parse(&args(&["search", "git", "-n", "5", "--cwd", "C:\\work", "--long"])).unwrap(),
            Request { ask: Ask::Search("git".into()), count: 5, cwd: Some("C:/work".into()), long: Detail::Long }
        );
        assert_eq!(parse(&args(&["forget", "rm -rf x"])).unwrap().ask, Ask::Forget("rm -rf x".into()));
        for wrong in [
            &[][..],
            &["search"],
            &["recent", "x"],
            &["recent", "-n"],
            &["recent", "-n", "many"],
            &["recent", "--bad"],
            &["remember", "x"],
            &["search", "a", "b"],
        ] {
            assert!(parse(&args(wrong)).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn long_lines_say_count_exit_and_when() {
        let found = vec![
            Candidate {
                text: "git push".into(),
                count: 3,
                fail_count: 1,
                last_at_ms: now_ms(),
                last_exit: Some(1),
                score: 0.0,
            },
            Candidate {
                text: "dir".into(),
                count: 1,
                fail_count: 0,
                last_at_ms: now_ms(),
                last_exit: None,
                score: 0.0,
            },
        ];
        let mut out = Vec::new();
        print(&found, Detail::Long, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "    3    1  now  git push\n    1    -  now  dir\n");
        let mut out = Vec::new();
        print(&found, Detail::Text, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "git push\ndir\n");
    }
}
