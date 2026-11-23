//! A model's answers from a program on this machine: an agent's command
//! line (Claude Code, Codex, pi) or any program the settings name. The
//! question is written to the program's standard input, or put in place of
//! `{prompt}` in an argument; the answer is what the program prints.
//!
//! The program runs in a directory of its own with nothing in it, so that
//! it reads no project's files or instructions. It gets a fixed time; then
//! it is stopped, with every program it started.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crate::{Error, Provider, Request};

/// The least time a program gets to answer, whatever the question: an
/// agent's command line takes 10 to 40 seconds to start and answer.
const LEAST_WAIT: Duration = Duration::from_secs(90);
/// How often a running program is looked at.
const LOOK: Duration = Duration::from_millis(50);
/// What the question takes the place of in an argument.
const PROMPT: &str = "{prompt}";
/// The most of what a failed program said that is kept for the message.
const SAID: usize = 300;
/// How long the user's login shell gets to say its `PATH`.
#[cfg(unix)]
const SHELL_WAIT: Duration = Duration::from_secs(5);

/// A program that answers questions.
#[derive(Debug, Clone)]
pub struct Command {
    /// The program, then its arguments.
    line: Vec<String>,
    /// Where it runs.
    directory: PathBuf,
    wait: Duration,
}

impl Command {
    /// `line`: the program, then its arguments. `directory`: where it runs
    /// (made if it is not there); an empty one, so that the program finds nothing to read.
    #[must_use]
    pub const fn new(line: Vec<String>, directory: PathBuf) -> Self {
        Self { line, directory, wait: LEAST_WAIT }
    }

    /// The same, with another least time to answer.
    #[must_use]
    pub const fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// The program's name, as the settings give it.
    #[must_use]
    pub fn name(&self) -> &str {
        self.line.first().map_or("", String::as_str)
    }
}

/// The whole question as one text: instructions, the answer's shape, the question.
fn prompt(request: &Request) -> String {
    format!("{}\nThe JSON schema of the answer: {}\n\n{}", request.system, request.schema, request.user)
}

/// The arguments with the question in them, and what goes to the program's
/// standard input: the question, unless an argument has `{prompt}`, which it
/// then takes the place of.
fn with_question(arguments: &[String], question: String) -> (Vec<String>, Option<String>) {
    if arguments.iter().any(|argument| argument.contains(PROMPT)) {
        (arguments.iter().map(|argument| argument.replace(PROMPT, &question)).collect(), None)
    } else {
        (arguments.to_vec(), Some(question))
    }
}

impl Provider for Command {
    fn complete(&self, request: &Request) -> Result<String, Error> {
        let name = self.name();
        let Some(place) = place(name) else { return Err(Error::NoProgram(name.to_owned())) };
        if let Err(error) = std::fs::create_dir_all(&self.directory) {
            return Err(Error::Failed(format!("make {}: {error}", self.directory.display())));
        }
        let (arguments, input) = with_question(&self.line[1..], prompt(request));
        let mut process = Process::new(&place.program);
        process.args(arguments).current_dir(&self.directory);
        if let Some(path) = &place.path {
            process.env("PATH", path);
        }
        let ran = run(process, input.as_deref(), request.timeout.max(self.wait), name)?;
        if !ran.status.success() {
            let said = String::from_utf8_lossy(if ran.err.is_empty() { &ran.out } else { &ran.err }).into_owned();
            let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(SAID).collect();
            return Err(Error::Failed(format!("{name} failed ({}): {said}", ran.status)));
        }
        let answer = String::from_utf8_lossy(&ran.out).trim().to_owned();
        if answer.is_empty() { Err(Error::BadAnswer(format!("{name} printed nothing"))) } else { Ok(answer) }
    }
}

/// Where a program is, and the `PATH` it is to run with when that is not this process's own.
struct Place {
    program: PathBuf,
    path: Option<OsString>,
}

/// Where `program` is: as named when the name is a path, else in this
/// process's `PATH`, else in the `PATH` of the user's login shell.
///
/// UnifyTerm opened from the Dock or a launcher has the system's short
/// `PATH`, without Homebrew's or the user's directories: an agent's command
/// line is not in it, and neither is the `node` it runs with. So the program
/// then runs with the login shell's `PATH`.
fn place(program: &str) -> Option<Place> {
    if program.trim().is_empty() {
        return None;
    }
    if program.contains(['/', '\\']) {
        return Path::new(program).is_file().then(|| Place { program: program.into(), path: None });
    }
    if let Some(found) = find_in(program, &std::env::var_os("PATH").unwrap_or_default()) {
        return Some(Place { program: found, path: None });
    }
    let path = login_path()?;
    find_in(program, &path).map(|found| Place { program: found, path: Some(path) })
}

/// The first file called `program` in the directories of `path`. On
/// Windows a program is named without its extension: the extensions of
/// `PATHEXT` are tried in order, so `pi` finds `pi.cmd`.
fn find_in(program: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        let known = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        // The name as it is first: it may have its extension already.
        std::iter::once(String::new()).chain(known.split(';').map(str::to_owned)).collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(path)
        .filter(|directory| !directory.as_os_str().is_empty())
        .flat_map(|directory| extensions.iter().map(move |extension| directory.join(format!("{program}{extension}"))))
        .find(|file| file.is_file())
}

/// The `PATH` the user's login shell has, asked once: `$SHELL -l -c 'printenv PATH'`.
#[cfg(unix)]
fn login_path() -> Option<OsString> {
    static PATH: std::sync::OnceLock<Option<OsString>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var("SHELL").ok()?;
        let mut process = Process::new(&shell);
        process.args(["-l", "-c", "printenv PATH"]);
        let ran = run(process, None, SHELL_WAIT, &shell).ok()?;
        // A profile file may print lines of its own: the path is the last line that is one.
        let said = String::from_utf8_lossy(&ran.out).into_owned();
        said.lines().map(str::trim).rfind(|line| line.starts_with('/')).map(OsString::from)
    })
    .clone()
}

/// Windows has one `PATH` for programs however they are started.
#[cfg(not(unix))]
const fn login_path() -> Option<OsString> {
    None
}

/// A program that ended: how, and what it printed.
struct Ran {
    status: ExitStatus,
    out: Vec<u8>,
    err: Vec<u8>,
}

/// Runs `process` with no window, `input` on its standard input, for at
/// most `wait`: then it is stopped, with what it started. `name`: what the
/// program is called in an error.
fn run(mut process: Process, input: Option<&str>, wait: Duration, name: &str) -> Result<Ran, Error> {
    process
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apart(&mut process);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(Error::NoProgram(name.to_owned())),
        Err(error) => return Err(Error::Failed(format!("{name} did not start: {error}"))),
    };
    // Written and read while it runs: a full pipe would stop it before it ends.
    if let (Some(mut pipe), Some(text)) = (child.stdin.take(), input.map(str::to_owned)) {
        std::thread::spawn(move || {
            // A program that does not read its input closes the pipe: that is its business.
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let (out, err) = (read_all(child.stdout.take()), read_all(child.stderr.take()));
    let deadline = Instant::now() + wait;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(LOOK),
            Ok(None) | Err(_) => {
                stop(&mut child);
                return Err(Error::Failed(format!("{name} gave no answer in {} s: it was stopped", wait.as_secs())));
            }
        }
    };
    // A program it started may still hold the pipes open: they are not waited for longer than a moment.
    let moment = deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1));
    let (Ok(out), Ok(err)) = (out.recv_timeout(moment), err.recv_timeout(moment)) else {
        stop(&mut child);
        return Err(Error::Failed(format!("{name} ended, but what it started holds its output open")));
    };
    Ok(Ran { status, out, err })
}

/// Reads all of a pipe on a thread of its own; the bytes come when the pipe closes.
fn read_all<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> Receiver<Vec<u8>> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        // Nobody may be waiting any more: then the bytes are not wanted.
        let _ = send.send(bytes);
    });
    receive
}

/// Sets a process up to be stopped with everything it starts, and to show no window.
fn apart(process: &mut Process) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`: a console program started by a window program gets no console window.
        const NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // A process group of its own: the group is what gets stopped.
        process.process_group(0);
    }
}

/// Stops a program and every program it started.
fn stop(child: &mut Child) {
    let id = child.id().to_string();
    // Windows: `taskkill /T` ends the tree below the program. Unix: the program leads a
    // process group (see `apart`); a signal to minus its number goes to the whole group.
    let mut killer = if cfg!(windows) {
        let mut killer = Process::new("taskkill");
        killer.args(["/PID", &id, "/T", "/F"]);
        killer
    } else {
        let mut killer = Process::new("kill");
        killer.args(["-KILL", "--", &format!("-{id}")]);
        killer
    };
    killer.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        killer.creation_flags(0x0800_0000);
    }
    let _ = killer.status();
    // Whatever the above did, the program itself is stopped and waited for: no zombie is left.
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Effort;

    fn request() -> Request {
        Request {
            system: "be brief".into(),
            user: "hello there".into(),
            schema: json!({"type": "object"}),
            max_tokens: 100,
            effort: Effort::Low,
            timeout: Duration::from_secs(1),
        }
    }

    /// A shell that runs `script`, as a program and its arguments.
    fn shell(script: &str) -> Vec<String> {
        let (program, flag) = if cfg!(windows) { ("cmd", "/c") } else { ("sh", "-c") };
        vec![program.into(), flag.into(), script.into()]
    }

    fn in_a_directory(line: Vec<String>) -> (tempfile::TempDir, Command) {
        let directory = tempfile::tempdir().unwrap();
        let command = Command::new(line, directory.path().join("empty")).with_wait(Duration::from_secs(20));
        (directory, command)
    }

    #[test]
    fn the_question_goes_in_and_what_is_printed_comes_back() {
        // A program that prints what it read: the question was on its standard input.
        let copy = if cfg!(windows) { "findstr x*" } else { "cat" };
        let (_directory, command) = in_a_directory(shell(copy));
        let answer = command.complete(&request()).unwrap();
        assert_eq!(
            answer.replace("\r\n", "\n"),
            "be brief\nThe JSON schema of the answer: {\"type\":\"object\"}\n\nhello there",
            "instructions, the shape, the question"
        );
    }

    #[test]
    fn the_question_takes_the_place_of_prompt_in_an_argument() {
        let words = |words: &[&str]| words.iter().map(|&word| word.to_owned()).collect::<Vec<_>>();
        assert_eq!(
            with_question(&words(&["--quiet", "ask={prompt}!"]), "what?".into()),
            (words(&["--quiet", "ask=what?!"]), None),
            "in the argument: nothing is written to the program"
        );
        assert_eq!(
            with_question(&words(&["--quiet"]), "what?".into()),
            (words(&["--quiet"]), Some("what?".into())),
            "no {{prompt}}: the question is written to the program"
        );
        assert_eq!(with_question(&[], "what?".into()), (Vec::new(), Some("what?".into())));
    }

    /// The question in an argument reaches the program as that argument, line breaks and all.
    #[cfg(unix)]
    #[test]
    fn an_argument_with_the_question_reaches_the_program() {
        let line =
            ["sh", "-c", "printf %s \"$1\"; cat", "sh", "{prompt}"].iter().map(|&word| word.to_owned()).collect();
        let (_directory, command) = in_a_directory(line);
        let answer = command.complete(&request()).unwrap();
        assert!(answer.starts_with("be brief\n") && answer.ends_with("\n\nhello there"), "{answer}");
        assert_eq!(answer.matches("hello there").count(), 1, "once: nothing was on its input");
    }

    #[test]
    fn it_runs_in_its_own_directory_which_is_made() {
        let here = if cfg!(windows) { "cd" } else { "pwd" };
        let (directory, command) = in_a_directory(shell(here));
        let said = command.complete(&request()).unwrap();
        let wanted = directory.path().join("empty");
        assert!(wanted.is_dir(), "made");
        assert_eq!(
            std::fs::canonicalize(said.trim()).unwrap(),
            std::fs::canonicalize(&wanted).unwrap(),
            "the program is there"
        );
        assert_eq!(std::fs::read_dir(&wanted).unwrap().count(), 0, "with nothing in it");
    }

    #[test]
    fn what_goes_wrong_is_told_apart() {
        let (_directory, missing) = in_a_directory(vec!["unifyterm-no-such-program-4c1e".into()]);
        assert_eq!(missing.complete(&request()), Err(Error::NoProgram("unifyterm-no-such-program-4c1e".into())));
        let (_directory, nothing) = in_a_directory(Vec::new());
        assert_eq!(nothing.complete(&request()), Err(Error::NoProgram(String::new())));
        let (_directory, failing) = in_a_directory(shell("echo not logged in 1>&2 && exit 3"));
        let Err(Error::Failed(said)) = failing.complete(&request()) else { panic!("an exit code that is not 0 fails") };
        assert!(said.contains("not logged in") && said.contains('3'), "{said}");
        let quiet = if cfg!(windows) { "exit 0" } else { "cat > /dev/null" };
        let (_directory, silent) = in_a_directory(shell(quiet));
        assert!(matches!(silent.complete(&request()), Err(Error::BadAnswer(_))));
    }

    #[test]
    fn a_program_that_takes_too_long_is_stopped_with_what_it_started() {
        // The shell starts a second one, which writes a file after 3 seconds. Stopped after 1 second
        // with the first shell only, the second would still write it.
        let script = if cfg!(windows) {
            "cmd /c ping -n 4 127.0.0.1 ^>nul ^& echo late^>late.txt"
        } else {
            "sh -c 'sleep 3; echo late > late.txt'; :"
        };
        let directory = tempfile::tempdir().unwrap();
        let command = Command::new(shell(script), directory.path().to_owned()).with_wait(Duration::from_secs(1));
        let started = Instant::now();
        let Err(Error::Failed(said)) = command.complete(&request()) else { panic!("too slow is a failure") };
        assert!(said.contains("no answer in 1 s"), "{said}");
        assert!(started.elapsed() < Duration::from_secs(3), "stopped at its time: {:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(4));
        assert!(!directory.path().join("late.txt").exists(), "what it started was stopped too");
    }

    #[test]
    fn a_program_is_found_by_its_name_in_a_path() {
        let directory = tempfile::tempdir().unwrap();
        let file = if cfg!(windows) { "agent.cmd" } else { "agent" };
        std::fs::write(directory.path().join(file), "").unwrap();
        let path = std::env::join_paths([Path::new("no-such-directory"), directory.path()]).unwrap();
        // Windows does not tell `agent.cmd` from `agent.CMD`: the extension is as PATHEXT writes it.
        let found = find_in("agent", &path).map(|found| found.to_string_lossy().to_lowercase());
        let wanted = directory.path().join(file).to_string_lossy().to_lowercase();
        assert_eq!(found, Some(wanted), "without its extension on Windows");
        assert_eq!(find_in("other", &path), None);
        assert_eq!(find_in("agent", std::ffi::OsStr::new("")), None, "an empty PATH names no directory");
    }
}
