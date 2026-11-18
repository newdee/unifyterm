//! The agents keepane watches (Claude Code, Codex, pi, Cursor CLI in its
//! panes), for the sidebar: what `keepane list-agents -J` prints, read and
//! said in a line or two. The command is a setting (`agents.command`), so a
//! named keepane server, or another program that prints the same JSON, works too.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// How long the command may take before it is stopped.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(1);
/// How often the end of the command is looked for.
const LOOK: Duration = Duration::from_millis(10);
/// The same problem goes to the log at most this often.
const LOG_EVERY: Duration = Duration::from_secs(60);

/// One agent, as `keepane list-agents -J` gives it. Fields keepane adds
/// later are passed over; any of these left out take their empty value.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub(crate) struct Agent {
    /// keepane's pane number: `%426`.
    pub(crate) pane: String,
    /// Where the pane is: `session:window.pane`.
    pub(crate) target: String,
    /// Which agent: `claude`, `codex`, `pi`, `cursor`.
    #[serde(rename = "agent")]
    pub(crate) kind: String,
    pub(crate) model: String,
    /// In dollars.
    pub(crate) cost: f64,
    /// How full its context is, in percent, when known.
    pub(crate) context_pct: Option<u32>,
    /// Percent of one processor.
    pub(crate) cpu: f64,
    /// Bytes.
    pub(crate) mem: u64,
    pub(crate) turns: u64,
}

impl Agent {
    /// The card's first line: which agent, which model.
    pub(crate) fn title(&self) -> String {
        let agent = if self.kind.is_empty() { "agent" } else { &self.kind };
        if self.model.is_empty() { agent.to_owned() } else { format!("{agent} · {}", self.model) }
    }

    /// The card's second line: where it is.
    pub(crate) fn place(&self) -> String {
        match (self.target.is_empty(), self.pane.is_empty()) {
            (false, false) => format!("{} ({})", self.target, self.pane),
            (false, true) => self.target.clone(),
            _ => self.pane.clone(),
        }
    }

    /// The card's third line: cost, context, processor, memory, turns.
    pub(crate) fn figures(&self) -> String {
        let mut parts = vec![format!("${:.2}", self.cost)];
        if let Some(percent) = self.context_pct {
            parts.push(format!("context {percent}%"));
        }
        parts.push(format!("CPU {:.0}%", self.cpu));
        parts.push(format!("{} MiB", self.mem / (1024 * 1024)));
        parts.push(format!("{} turns", self.turns));
        parts.join(" · ")
    }
}

/// What the sidebar shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Agents {
    /// Not asked yet.
    Asking,
    Listed(Vec<Agent>),
    /// The program is not there: keepane is not installed, or not on `PATH`.
    NoProgram(String),
    /// It ran and said something is wrong: no keepane server, for one. The first line it printed.
    Failed(String),
    /// It printed something that is not the agents' list.
    Unreadable(String),
    /// It took longer than [`TIMEOUT`].
    TooSlow,
}

impl Agents {
    /// The line in place of the cards; `None` when there are cards.
    pub(crate) fn notice(&self) -> Option<String> {
        match self {
            Self::Asking => Some("Asking keepane…".into()),
            Self::Listed(agents) if agents.is_empty() => Some("No agents in keepane's panes.".into()),
            Self::Listed(_) => None,
            Self::NoProgram(program) => Some(format!("{program} was not found: is keepane installed?")),
            Self::Failed(said) => Some(said.clone()),
            Self::Unreadable(_) => Some("keepane's answer could not be read: see the log.".into()),
            Self::TooSlow => Some(format!("keepane did not answer within {} s.", TIMEOUT.as_secs())),
        }
    }

    /// What goes to the log; `None` when nothing is wrong.
    fn problem(&self) -> Option<String> {
        match self {
            Self::Asking | Self::Listed(_) => None,
            Self::Unreadable(error) => Some(format!("agents: keepane's answer is not the agents' list: {error}")),
            other => other.notice().map(|notice| format!("agents: {notice}")),
        }
    }
}

/// The agents in the command's output.
pub(crate) fn parse(output: &[u8]) -> Agents {
    match serde_json::from_slice(output) {
        Ok(agents) => Agents::Listed(agents),
        Err(error) => Agents::Unreadable(error.to_string()),
    }
}

/// How a program run by [`run`] went.
enum Ran {
    /// It ended within [`TIMEOUT`]: whether it succeeded, then its first line of complaint.
    Ended {
        output: Vec<u8>,
        complaint: Option<String>,
    },
    NotFound,
    DidNotStart(String),
    TooSlow,
}

/// Runs `command` (program, then arguments) with no window, for at most
/// [`TIMEOUT`]: then it is stopped. What it printed is read as it runs.
fn run(command: &[String]) -> Ran {
    let Some((program, args)) = command.split_first() else { return Ran::NotFound };
    let mut start = Command::new(program);
    start.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::shell::no_window(&mut start);
    let mut child = match start.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ran::NotFound,
        Err(error) => return Ran::DidNotStart(format!("{program} did not start: {error}")),
    };
    // Read while it runs: a full pipe would stop it before it ends.
    let (out, err) = (read_all(child.stdout.take()), read_all(child.stderr.take()));
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(LOOK),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // A program it started may still hold the pipes open: wait for them only until the same deadline.
    let left = || deadline.saturating_duration_since(Instant::now());
    let (Some(status), Ok(output), Ok(err)) = (status, out.recv_timeout(left()), err.recv_timeout(left())) else {
        return Ran::TooSlow;
    };
    let complaint = (!status.success()).then(|| {
        let said = String::from_utf8_lossy(if err.is_empty() { &output } else { &err }).into_owned();
        let line = said.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_owned);
        line.unwrap_or_else(|| format!("{program} failed ({status})"))
    });
    Ran::Ended { output, complaint }
}

/// Asks `command` (program, then arguments) for the agents.
pub(crate) fn ask(command: &[String]) -> Agents {
    match run(command) {
        Ran::Ended { output, complaint: None } => parse(&output),
        Ran::Ended { complaint: Some(said), .. } | Ran::DidNotStart(said) => Agents::Failed(said),
        Ran::NotFound => Agents::NoProgram(command.first().cloned().unwrap_or_default()),
        Ran::TooSlow => Agents::TooSlow,
    }
}
/// Reads all of a pipe on a thread of its own; the bytes come when the pipe closes.
fn read_all<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut bytes);
        }
        // Nobody may be waiting any more: then the bytes are not wanted.
        let _ = send.send(bytes);
    });
    receive
}

/// Writes a problem to the log, but the same one at most once a minute:
/// the list is asked for every `agents.poll_ms` (2 s unless set).
#[derive(Default)]
pub(crate) struct Quiet {
    last: Option<(String, Instant)>,
}

impl Quiet {
    pub(crate) fn log(&mut self, agents: &Agents) {
        let Some(problem) = agents.problem() else {
            self.last = None;
            return;
        };
        let now = Instant::now();
        if self.last.as_ref().is_some_and(|(said, at)| *said == problem && now.duration_since(*at) < LOG_EVERY) {
            return;
        }
        log::warn!("{problem}");
        self.last = Some((problem, now));
    }
}

/// The focus command for one pane: `{pane}` in each word becomes its number.
pub(crate) fn focus_command(template: &[String], pane: &str) -> Vec<String> {
    template.iter().map(|word| word.replace("{pane}", pane)).collect()
}

/// Runs the focus command, as [`ask`] runs its own (no window, at most
/// [`TIMEOUT`]); `Err` with what went wrong.
pub(crate) fn focus(command: &[String]) -> Result<(), String> {
    let program = command.first().map_or("agents.focus", String::as_str);
    match run(command) {
        Ran::Ended { complaint: None, .. } => Ok(()),
        Ran::Ended { complaint: Some(said), .. } | Ran::DidNotStart(said) => Err(said),
        Ran::NotFound => Err(format!("{program} was not found")),
        Ran::TooSlow => Err(format!("{program} did not end within {} s", TIMEOUT.as_secs())),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/agents.json");

    #[test]
    fn the_fixture_is_read() {
        let Agents::Listed(agents) = parse(FIXTURE.as_bytes()) else { panic!("not read") };
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].title(), "claude · claude-opus-5-5");
        assert_eq!(agents[0].place(), "work:1.0 (%12)");
        assert_eq!(agents[0].figures(), "$3.25 · context 41% · CPU 12% · 512 MiB · 37 turns");
        assert_eq!(agents[1].title(), "codex");
        assert_eq!(agents[1].figures(), "$0.00 · CPU 0% · 128 MiB · 0 turns", "no context share: left out");
    }

    #[test]
    fn unknown_fields_are_passed_over_and_missing_ones_are_empty() {
        let Agents::Listed(agents) = parse(br#"[{"pane":"%1","agent":"pi","new_field":{"x":1}}]"#) else { panic!() };
        assert_eq!(agents[0].place(), "%1");
        assert!(agents[0].cost.abs() < f64::EPSILON);
        assert_eq!(parse(b"[]"), Agents::Listed(Vec::new()));
        assert_eq!(Agents::Listed(Vec::new()).notice().as_deref(), Some("No agents in keepane's panes."));
        assert!(matches!(parse(b"keepane: no server"), Agents::Unreadable(_)));
        assert!(matches!(parse(br#"{"pane":"%1"}"#), Agents::Unreadable(_)), "an object, not a list");
    }

    #[cfg(windows)]
    const SHELL: [&str; 2] = ["cmd", "/c"];
    #[cfg(not(windows))]
    const SHELL: [&str; 2] = ["sh", "-c"];

    fn shell(script: &str) -> Vec<String> {
        vec![SHELL[0].into(), SHELL[1].into(), script.into()]
    }

    #[test]
    fn the_command_is_run_and_its_trouble_named() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.json");
        std::fs::write(&file, r#"[{"pane":"%3"}]"#).unwrap();
        // A program that prints a file, given its path as an argument (no shell's quoting in between).
        #[cfg(windows)]
        let print = vec!["findstr".to_owned(), ".".to_owned(), file.display().to_string()];
        #[cfg(not(windows))]
        let print = vec!["cat".to_owned(), file.display().to_string()];
        assert_eq!(ask(&print), Agents::Listed(vec![Agent { pane: "%3".into(), ..Agent::default() }]));
        assert_eq!(ask(&["unifyterm-no-such-program".into()]), Agents::NoProgram("unifyterm-no-such-program".into()));
        assert_eq!(
            ask(&shell("echo keepane: no server running 1>&2 && exit 1")),
            Agents::Failed("keepane: no server running".into())
        );
        #[cfg(windows)]
        let slow = shell("ping -n 4 127.0.0.1 >nul");
        #[cfg(not(windows))]
        let slow = shell("sleep 3");
        let started = Instant::now();
        assert_eq!(ask(&slow), Agents::TooSlow);
        assert!(
            started.elapsed() < TIMEOUT + Duration::from_millis(900),
            "stopped at the limit: {:?}",
            started.elapsed()
        );
        // It ends at once, but leaves a program behind that holds the pipe: as slow.
        #[cfg(windows)]
        let behind = shell("start /b ping -n 4 127.0.0.1 >nul & exit 0");
        #[cfg(not(windows))]
        let behind = shell("sleep 3 & exit 0");
        let started = Instant::now();
        assert_eq!(ask(&behind), Agents::TooSlow);
        assert!(
            started.elapsed() < TIMEOUT + Duration::from_millis(900),
            "stopped at the limit: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_same_problem_is_logged_once_a_minute() {
        let mut quiet = Quiet::default();
        quiet.log(&Agents::TooSlow);
        let first = quiet.last.clone().unwrap();
        quiet.log(&Agents::TooSlow);
        assert_eq!(quiet.last.as_ref().unwrap().1, first.1, "not logged again");
        quiet.log(&Agents::Listed(Vec::new()));
        assert!(quiet.last.is_none(), "fine again: the next problem is logged at once");
    }

    #[test]
    fn the_focus_command_names_the_pane() {
        let template = ["keepane".to_owned(), "focus-pane".to_owned(), "{pane}".to_owned()];
        assert_eq!(focus_command(&template, "%7"), ["keepane", "focus-pane", "%7"]);
        assert_eq!(focus(&["unifyterm-no-such-program".into()]), Err("unifyterm-no-such-program was not found".into()));
        assert!(focus(&[]).is_err());
        assert_eq!(focus(&shell("exit 0")), Ok(()));
        // A focus command that hangs is stopped like the list's.
        #[cfg(windows)]
        let slow = shell("ping -n 4 127.0.0.1 >nul");
        #[cfg(not(windows))]
        let slow = shell("sleep 3");
        let started = Instant::now();
        assert!(focus(&slow).is_err_and(|error| error.contains("did not end")));
        assert!(started.elapsed() < TIMEOUT + Duration::from_millis(900), "{:?}", started.elapsed());
    }
}
