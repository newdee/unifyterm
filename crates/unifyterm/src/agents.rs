//! The agents keepane watches (Claude Code, Codex, pi, Cursor CLI in its
//! panes), for the sidebar: what `keepane list-agents -J` prints, read and
//! said in a line or two. The command is a setting (`agents.command`), so a
//! named keepane server, or another program that prints the same JSON, works too.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use unifyterm_programs::or_empty;

/// How long the command may take before it is stopped.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(1);
/// How often the end of the command is looked for.
const LOOK: Duration = Duration::from_millis(10);
/// The same problem goes to the log at most this often.
const LOG_EVERY: Duration = Duration::from_secs(60);
/// Of the entries that could not be read, the log line says why for the first this many.
const UNREAD_NAMED: usize = 3;

/// One agent, as `keepane list-agents -J` gives it. Fields keepane adds
/// later are passed over; any of these left out, or given as `null`, take
/// their empty value. keepane gives `null` for a figure it does not know:
/// the cost of an agent it has no prices for, the context's share before
/// the first reply.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub(crate) struct Agent {
    /// keepane's pane number: `%426`.
    #[serde(deserialize_with = "or_empty")]
    pub(crate) pane: String,
    /// Where the pane is: `session:window.pane`.
    #[serde(deserialize_with = "or_empty")]
    pub(crate) target: String,
    /// Which agent: `claude`, `codex`, `pi`, `cursor`.
    #[serde(rename = "agent", deserialize_with = "or_empty")]
    pub(crate) kind: String,
    #[serde(deserialize_with = "or_empty")]
    pub(crate) model: String,
    /// In dollars, when known.
    pub(crate) cost: Option<f64>,
    /// Some of its tokens are of a model keepane has no price for: the
    /// real cost is more than [`Self::cost`].
    #[serde(deserialize_with = "or_empty")]
    pub(crate) cost_partial: bool,
    /// How full its context is, in percent, when known.
    pub(crate) context_pct: Option<u32>,
    /// Percent of one processor.
    #[serde(deserialize_with = "or_empty")]
    pub(crate) cpu: f64,
    /// Bytes.
    #[serde(deserialize_with = "or_empty")]
    pub(crate) mem: u64,
    #[serde(deserialize_with = "or_empty")]
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

    /// The card's third line: cost, context, processor, memory, turns. A
    /// cost or a context's share that is not known is left out; a cost that
    /// is only part of it has a `+`, as keepane writes it (`$2.10+`).
    pub(crate) fn figures(&self) -> String {
        let mut parts = Vec::new();
        if let Some(cost) = self.cost {
            let more = if self.cost_partial { "+" } else { "" };
            parts.push(format!("${cost:.2}{more}"));
        }
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
    /// The list was read: its agents, and for each entry that is not an
    /// agent's, which one and why. Such an entry does not take the others
    /// with it.
    Listed { agents: Vec<Agent>, unread: Vec<String> },
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
    /// A list with nothing wrong in it.
    #[cfg(test)]
    fn listed(agents: Vec<Agent>) -> Self {
        Self::Listed { agents, unread: Vec::new() }
    }

    /// The line above the cards, or in their place; `None` when the cards say it all.
    pub(crate) fn notice(&self) -> Option<String> {
        match self {
            Self::Asking => Some("Asking keepane…".into()),
            Self::Listed { agents, unread } if !unread.is_empty() => Some(format!(
                "{} of keepane's {} agents could not be read: see the log.",
                unread.len(),
                agents.len() + unread.len()
            )),
            Self::Listed { agents, .. } if agents.is_empty() => Some("No agents in keepane's panes.".into()),
            Self::Listed { .. } => None,
            Self::NoProgram(program) => Some(format!("{program} was not found: is keepane installed?")),
            Self::Failed(said) => Some(said.clone()),
            Self::Unreadable(_) => Some("keepane's answer could not be read: see the log.".into()),
            Self::TooSlow => Some(format!("keepane did not answer within {} s.", TIMEOUT.as_secs())),
        }
    }

    /// What goes to the log; `None` when nothing is wrong.
    fn problem(&self) -> Option<String> {
        match self {
            Self::Asking => None,
            Self::Listed { unread, .. } if unread.is_empty() => None,
            Self::Listed { agents, unread } => Some(format!(
                "agents: {} of the {} entries in keepane's answer are not an agent's: {}",
                unread.len(),
                agents.len() + unread.len(),
                unread.iter().take(UNREAD_NAMED).map(String::as_str).collect::<Vec<_>>().join("; ")
            )),
            Self::Unreadable(error) => Some(format!("agents: keepane's answer is not the agents' list: {error}")),
            other => other.notice().map(|notice| format!("agents: {notice}")),
        }
    }
}

/// The agents in the command's output. The output is a list; each of its
/// entries is read on its own, so that one that cannot be read (a figure of
/// another type than today's) is named and the others are still shown.
pub(crate) fn parse(output: &[u8]) -> Agents {
    let entries: Vec<serde_json::Value> = match serde_json::from_slice(output) {
        Ok(entries) => entries,
        Err(error) => return Agents::Unreadable(error.to_string()),
    };
    let (mut agents, mut unread) = (Vec::with_capacity(entries.len()), Vec::new());
    for (index, entry) in entries.into_iter().enumerate() {
        match serde_json::from_value(entry) {
            Ok(agent) => agents.push(agent),
            Err(error) => unread.push(format!("entry {}: {error}", index + 1)),
        }
    }
    Agents::Listed { agents, unread }
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
///
/// A program that is not in UnifyTerm's own `PATH` is looked for the way the
/// user's shell finds it (see [`where_the_shell_finds`]), and run from there.
fn run(command: &[String]) -> Ran {
    let ran = run_as_named(command);
    let (Ran::NotFound, Some((program, args))) = (&ran, command.split_first()) else { return ran };
    let Some(found) = where_the_shell_finds(program) else { return ran };
    let mut named: Vec<String> = vec![found];
    named.extend_from_slice(args);
    run_as_named(&named)
}

/// Where the user's login shell finds `program`; `None` when it does not,
/// or when `program` is not a plain name. Asked once for each program.
///
/// UnifyTerm is opened from the Dock or a launcher: its own `PATH` is then
/// the system's short one, without Homebrew's or the user's directories.
/// Those come from the shell's profile files, so the shell is asked, as a
/// login shell: `$SHELL -l -c 'command -v keepane'`.
#[cfg(unix)]
fn where_the_shell_finds(program: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};
    static FOUND: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
    let mut found = FOUND.lock().unwrap_or_else(PoisonError::into_inner);
    let known = found.get_or_insert_with(HashMap::new);
    if let Some(place) = known.get(program) {
        return place.clone();
    }
    let place = std::env::var("SHELL").ok().and_then(|shell| ask_login_shell(&shell, program));
    known.insert(program.to_owned(), place.clone());
    place
}

/// Windows has one `PATH` for programs however they are started.
#[cfg(not(unix))]
const fn where_the_shell_finds(_program: &str) -> Option<String> {
    None
}

/// What `shell`, started as a login shell, answers to `command -v program`:
/// the program's full path. bash, zsh and fish all take these arguments.
#[cfg(unix)]
fn ask_login_shell(shell: &str, program: &str) -> Option<String> {
    // The name goes into a shell's command line: only a plain one does.
    let plain = !program.is_empty() && program.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if !plain {
        return None;
    }
    let asked = [shell.to_owned(), "-l".to_owned(), "-c".to_owned(), format!("command -v {program}")];
    let Ran::Ended { output, complaint: None } = run_as_named(&asked) else { return None };
    // A profile file may print lines of its own: the path is the last line that is one.
    String::from_utf8_lossy(&output).lines().map(str::trim).rfind(|line| line.starts_with('/')).map(str::to_owned)
}

/// [`run`], with the program exactly as it is named.
fn run_as_named(command: &[String]) -> Ran {
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

    /// A home whose profile file puts a directory with `fake-agent` in `PATH`: a login shell finds it there.
    #[cfg(unix)]
    #[test]
    fn a_program_is_found_the_way_the_login_shell_finds_it() {
        use std::os::unix::fs::PermissionsExt as _;
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("my bin");
        std::fs::create_dir(&bin).unwrap();
        let program = bin.join("fake-agent");
        std::fs::write(&program, "#!/bin/sh\necho '[]'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A shell of our own making, so that the test does not read or change anybody's files:
        // it does what a login shell does with a profile, then runs what it was asked.
        let shell = home.path().join("login-shell");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = -l ] || exit 9\necho 'a line a profile prints'\nPATH=\"{}:$PATH\"\nshift 2\neval \"$1\"\n",
            bin.display()
        );
        std::fs::write(&shell, script).unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shell = shell.to_string_lossy().into_owned();
        // A script just written cannot be run while a process that another test thread is
        // starting still holds it open for writing (Linux: "Text file busy"): ask again a
        // few times. The program's own use runs the user's shell, written long before.
        let found = (0..20).find_map(|_| {
            ask_login_shell(&shell, "fake-agent").or_else(|| {
                std::thread::sleep(std::time::Duration::from_millis(25));
                None
            })
        });
        assert_eq!(found, Some(program.to_string_lossy().into_owned()));
        assert_eq!(ask_login_shell(&shell, "no-such-agent-xyz"), None, "not there");
        assert_eq!(ask_login_shell(&shell, "fake-agent; rm -rf x"), None, "only a plain name is asked for");
        assert_eq!(ask_login_shell(&shell, ""), None);
        assert_eq!(ask_login_shell("/no/such/shell", "fake-agent"), None, "no shell to ask");
    }
    const FIXTURE: &str = include_str!("../tests/fixtures/agents.json");

    /// The agents of a list in which every entry was read.
    fn all_read(output: &[u8]) -> Vec<Agent> {
        match parse(output) {
            Agents::Listed { agents, unread } if unread.is_empty() => agents,
            other => panic!("not all read: {other:?}"),
        }
    }

    #[test]
    fn the_fixture_is_read() {
        let agents = all_read(FIXTURE.as_bytes());
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].title(), "claude · claude-opus-5-5");
        assert_eq!(agents[0].place(), "work:1.0 (%12)");
        assert_eq!(agents[0].figures(), "$3.25 · context 41% · CPU 12% · 512 MiB · 37 turns");
        assert_eq!(agents[1].title(), "codex");
        assert_eq!(agents[1].figures(), "$0.00 · CPU 0% · 128 MiB · 0 turns", "no context share: left out");
    }

    #[test]
    fn unknown_fields_are_passed_over_and_missing_ones_are_empty() {
        let agents = all_read(br#"[{"pane":"%1","agent":"pi","new_field":{"x":1}}]"#);
        assert_eq!(agents[0].place(), "%1");
        assert_eq!(agents[0].cost, None, "no cost given: not known, not $0.00");
        assert_eq!(agents[0].figures(), "CPU 0% · 0 MiB · 0 turns");
        assert_eq!(parse(b"[]"), Agents::listed(Vec::new()));
        assert_eq!(Agents::listed(Vec::new()).notice().as_deref(), Some("No agents in keepane's panes."));
        assert_eq!(Agents::listed(vec![Agent::default()]).notice(), None, "cards: no line");
        assert!(matches!(parse(b"keepane: no server"), Agents::Unreadable(_)));
        assert!(matches!(parse(br#"{"pane":"%1"}"#), Agents::Unreadable(_)), "an object, not a list");
        assert!(matches!(parse(b"null"), Agents::Unreadable(_)), "no list at all");
    }

    /// keepane gives `null` for a cost it does not know (an agent it has no
    /// prices for, `agent-cost off`): the card has no cost, and the list is read.
    #[test]
    fn a_figure_given_as_null_is_not_known_and_the_list_is_still_read() {
        let unknown = FIXTURE.replacen("\"cost\":3.25", "\"cost\":null", 1);
        assert_ne!(unknown, FIXTURE, "the fixture has that cost");
        let agents = all_read(unknown.as_bytes());
        assert_eq!(agents.len(), 2, "both cards, as before");
        assert_eq!(agents[0].figures(), "context 41% · CPU 12% · 512 MiB · 37 turns", "no cost: left out");
        assert_eq!(agents[1].figures(), "$0.00 · CPU 0% · 128 MiB · 0 turns", "a cost of nothing is a cost");
        // Every field as null: an agent all the same, with its empty values.
        let nulls = br#"[{"pane":null,"target":null,"agent":null,"model":null,"cost":null,"cost_partial":null,
            "context_pct":null,"cpu":null,"mem":null,"turns":null}]"#;
        assert_eq!(all_read(nulls), [Agent::default()]);
        assert_eq!(Agent::default().title(), "agent");
    }

    /// keepane's `cost_partial`: some tokens had no price, so the cost is less than the real
    /// one. The card says so with a `+`, as keepane does; with no cost known there is nothing to mark.
    #[test]
    fn a_cost_that_is_only_part_of_it_has_a_plus() {
        let partial = FIXTURE.replacen("\"cost_partial\":false", "\"cost_partial\":true", 1);
        assert_ne!(partial, FIXTURE, "the fixture has that field");
        let agents = all_read(partial.as_bytes());
        assert_eq!(agents[0].figures(), "$3.25+ · context 41% · CPU 12% · 512 MiB · 37 turns");
        assert_eq!(agents[1].figures(), "$0.00 · CPU 0% · 128 MiB · 0 turns", "the other card: no +");
        let cases: [(&[u8], &str); 4] = [
            (br#"[{"cost":0.0,"cost_partial":true}]"#, "$0.00+ · "),
            (br#"[{"cost":null,"cost_partial":true}]"#, "CPU 0%"),
            (br#"[{"cost":2.1,"cost_partial":null}]"#, "$2.10 · "),
            (br#"[{"cost":2.1}]"#, "$2.10 · "),
        ];
        for (list, starts) in cases {
            let figures = all_read(list)[0].figures();
            assert!(figures.starts_with(starts), "{}: {figures}", String::from_utf8_lossy(list));
        }
        assert!(
            matches!(parse(br#"[{"cost":1.0,"cost_partial":"yes"}]"#), Agents::Listed { unread, .. } if unread.len() == 1)
        );
    }

    /// An entry of another shape than an agent's: named, and the others are shown.
    #[test]
    fn an_entry_that_cannot_be_read_does_not_take_the_others() {
        let list = br#"[{"pane":"%1","cost":1.5}, {"pane":"%2","mem":"lots"}, 7, {"pane":"%4","cost":null}]"#;
        let Agents::Listed { agents, unread } = parse(list) else { panic!("not a list") };
        let panes: Vec<&str> = agents.iter().map(|agent| agent.pane.as_str()).collect();
        assert_eq!(panes, ["%1", "%4"], "in the list's order");
        assert_eq!(unread.len(), 2);
        assert!(unread[0].starts_with("entry 2: ") && unread[0].contains("expected u64"), "{}", unread[0]);
        assert!(unread[1].starts_with("entry 3: "), "{}", unread[1]);
        let read = Agents::Listed { agents, unread };
        assert_eq!(read.notice().as_deref(), Some("2 of keepane's 4 agents could not be read: see the log."));
        let problem = read.problem().expect("it goes to the log");
        assert!(problem.starts_with("agents: 2 of the 4 entries in keepane's answer are not an agent's: entry 2: "));
        // None could be read: the line says so, not "no agents".
        let none = parse(b"[1, 2]");
        assert_eq!(none.notice().as_deref(), Some("2 of keepane's 2 agents could not be read: see the log."));
        // Many: the log line names the first three.
        let many = parse(b"[1, 2, 3, 4, 5]").problem().expect("it goes to the log");
        assert!(many.contains("entry 3: ") && !many.contains("entry 4: "), "{many}");
        assert_eq!(Agents::listed(vec![Agent::default()]).problem(), None, "nothing wrong: nothing logged");
    }

    #[cfg(windows)]
    const SHELL: [&str; 2] = ["cmd", "/c"];
    #[cfg(not(windows))]
    const SHELL: [&str; 2] = ["sh", "-c"];

    fn shell(script: &str) -> Vec<String> {
        vec![SHELL[0].into(), SHELL[1].into(), script.into()]
    }

    /// How many times a test asks a program that a busy machine may start too slowly.
    const TRIES: usize = 3;

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
        // A busy machine (a CI runner) may take more than the second to start a program: only that
        // is asked again, and any other answer is checked as it is.
        let listed =
            (0..TRIES).map(|_| ask(&print)).find(|agents| *agents != Agents::TooSlow).unwrap_or(Agents::TooSlow);
        assert_eq!(listed, Agents::listed(vec![Agent { pane: "%3".into(), ..Agent::default() }]));
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
        quiet.log(&Agents::listed(Vec::new()));
        assert!(quiet.last.is_none(), "fine again: the next problem is logged at once");
        // An entry that cannot be read is a problem like the others: said, then not again within the minute.
        let unread = Agents::Listed { agents: vec![Agent::default()], unread: vec!["entry 2: no agent".into()] };
        quiet.log(&unread);
        let first = quiet.last.clone().expect("it was logged");
        quiet.log(&unread);
        assert_eq!(quiet.last.as_ref().map(|(_, at)| *at), Some(first.1), "not logged again");
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
