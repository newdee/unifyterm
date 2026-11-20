//! What a tab's shell is started with: UnifyTerm's commands first in its
//! `PATH`, for PowerShell a start script that takes its aliases of the
//! same names out of their way, and a hook that marks the prompt and each
//! command for the history (PowerShell, bash, zsh, fish, cmd.exe).
//!
//! The commands are `ut`, the program beside `unifyterm`. A directory of
//! links to it, one a command, is made (by `ut --links`) in the user's data
//! directory; a shell with that directory first in `PATH` finds `cat`,
//! `ls` and the rest there on every system.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use portable_pty::CommandBuilder;

use unifyterm_config::shell::{directory_name, powershell_start};

use crate::config::CommandsConfig;
use crate::session;

/// The environment variable that tells programs where UnifyTerm's commands
/// are. The PowerShell start script reads it (`start.ps1`).
const COMMANDS_VARIABLE: &str = "UNIFYTERM_COMMANDS";
/// Tells bash's hook to read the profile files a login bash reads (see [`ShellStart::Login`]).
const LOGIN_VARIABLE: &str = "UNIFYTERM_LOGIN";
/// The `ZDOTDIR` zsh had before UnifyTerm's hook took its place: the hook reads the user's files from it.
const ZDOTDIR_VARIABLE: &str = "UNIFYTERM_ZDOTDIR";

/// UnifyTerm's commands, ready for a shell to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Commands {
    /// The directory of links, to go first in `PATH`.
    directory: PathBuf,
    /// The script PowerShell runs at its start.
    powershell_start: PathBuf,
    /// The names of the commands in use.
    names: Vec<String>,
}

/// Where UnifyTerm keeps the command links and the start scripts (and the history, and the log).
fn data_directory() -> Result<PathBuf> {
    unifyterm_history::data_directory()
}

/// `ut`, which is beside `unifyterm`.
fn ut_program() -> Result<PathBuf> {
    let own = std::env::current_exe().context("UnifyTerm cannot find its own file")?;
    let ut = own.with_file_name(if cfg!(windows) { "ut.exe" } else { "ut" });
    if !ut.is_file() {
        bail!("{} is not there", ut.display());
    }
    Ok(ut)
}

/// Starts `command` with no console window: on Windows a console program
/// started by a window program would otherwise get one. Elsewhere nothing changes.
pub(crate) fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`.
        const NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Runs `ut` with `args` and gives what it printed. No console window opens for it.
fn run_ut(ut: &Path, args: &[OsString]) -> Result<String> {
    let mut command = Command::new(ut);
    command.args(args);
    no_window(&mut command);
    let output = command.output().with_context(|| format!("start {}", ut.display()))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The names in `system` that are commands of `ut`'s, and those that are not.
fn known_and_unknown<'a>(system: &'a [String], commands: &[&str]) -> (Vec<&'a str>, Vec<&'a str>) {
    system.iter().map(String::as_str).partition(|name| commands.contains(name))
}

impl Commands {
    /// The names of UnifyTerm's commands in use (`cat`, `ls`, ...).
    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// The directory of links, first in each shell's `PATH`.
    pub(crate) fn directory(&self) -> PathBuf {
        self.directory.clone()
    }

    /// `ut`, which answers to these names.
    pub(crate) fn program() -> Result<PathBuf> {
        ut_program()
    }

    /// Makes UnifyTerm's commands ready as the settings say. `None` when the
    /// settings turn them off. The second value is what the user should be
    /// told: a name in `commands.system` that is no command, or why the
    /// commands could not be made ready (the terminal then runs without them).
    pub(crate) fn prepare(config: &CommandsConfig) -> (Option<Self>, Vec<String>) {
        if !config.builtin {
            return (None, Vec::new());
        }
        match Self::link(&config.system) {
            Ok((commands, unknown)) => {
                let told = unknown
                    .iter()
                    .map(|name| format!("commands.system names '{name}', which is not a command of UnifyTerm's"));
                (Some(commands), told.collect())
            }
            Err(error) => (None, vec![format!("UnifyTerm's commands are not in use: {error:#}")]),
        }
    }

    /// Links the commands, less the `system` ones; the names in `system` that are no command.
    fn link(system: &[String]) -> Result<(Self, Vec<String>)> {
        let ut = ut_program()?;
        let listed = run_ut(&ut, &["--list".into()])?;
        let all: Vec<&str> = listed.lines().collect();
        let (skipped, unknown) = known_and_unknown(system, &all);
        let data = data_directory()?;
        let directory = data.join(directory_name(&skipped));
        let mut args: Vec<OsString> = vec!["--links".into(), directory.clone().into()];
        if !skipped.is_empty() {
            args.extend(["--skip".into(), skipped.join(",").into()]);
        }
        run_ut(&ut, &args)?;

        let in_use: Vec<&str> = all.iter().copied().filter(|name| !skipped.contains(name)).collect();
        let powershell_start = data.join("shell").join(format!("{}.ps1", directory_name(&skipped)));
        let shell_directory = powershell_start.parent().context("the start script has a directory")?;
        std::fs::create_dir_all(shell_directory).with_context(|| format!("make {}", shell_directory.display()))?;
        std::fs::write(&powershell_start, self::powershell_start(&in_use))
            .with_context(|| format!("write {}", powershell_start.display()))?;
        let unknown = unknown.into_iter().map(str::to_owned).collect();
        let names = in_use.iter().map(|name| (*name).to_owned()).collect();
        Ok((Self { directory, powershell_start, names }, unknown))
    }
}

/// Which shell a program is, by its file's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellKind {
    PowerShell,
    Bash,
    Zsh,
    Fish,
    Cmd,
    Other,
}

/// The shell a program's path names (`pwsh.exe`, `/bin/zsh`, ...).
pub(crate) fn shell_kind(program: &str) -> ShellKind {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    match file.strip_suffix(".exe").unwrap_or(&file) {
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "bash" => ShellKind::Bash,
        "zsh" => ShellKind::Zsh,
        "fish" => ShellKind::Fish,
        "cmd" => ShellKind::Cmd,
        _ => ShellKind::Other,
    }
}

/// The hooks' files, written to UnifyTerm's data directory at each start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hooks {
    powershell: PathBuf,
    bash: PathBuf,
    /// The directory zsh is pointed to (`ZDOTDIR`).
    zsh: PathBuf,
    fish: PathBuf,
}

const POWERSHELL_HOOK: &str = include_str!("hooks/hook.ps1");
const BASH_HOOK: &str = include_str!("hooks/bashrc");
const ZSH_HOOK: &str = include_str!("hooks/zshrc");
const FISH_HOOK: &str = include_str!("hooks/hook.fish");
/// zsh's start files before `.zshrc`: each reads the user's own, from where it is.
const ZSH_ENV: &str = include_str!("hooks/zshenv");
const ZSH_PROFILE: &str = include_str!("hooks/zprofile");
/// cmd.exe's prompt with the marks around `own` (the user's `PROMPT`, `$P$G`
/// when there is none): which shell, the directory, where the prompt starts
/// and where it ends. `$E` is the escape character, `$P` the directory.
fn cmd_prompt(own: Option<&str>) -> String {
    let own = own.filter(|own| !own.is_empty()).unwrap_or("$P$G");
    format!("$E]633;P;Shell=cmd$E\\$E]9;9;$P$E\\$E]133;A$E\\{own}$E]133;B$E\\")
}

impl Hooks {
    /// Writes the hooks' files. An error means no hooks: the terminal works on without a history.
    pub(crate) fn prepare() -> Result<Self> {
        let directory = data_directory()?.join("shell").join("hooks");
        let zsh = directory.join("zsh");
        std::fs::create_dir_all(&zsh).with_context(|| format!("make {}", zsh.display()))?;
        let hooks = Self {
            powershell: directory.join("hook.ps1"),
            bash: directory.join("bashrc"),
            zsh,
            fish: directory.join("hook.fish"),
        };
        let files = [
            (hooks.powershell.clone(), POWERSHELL_HOOK),
            (hooks.bash.clone(), BASH_HOOK),
            (hooks.zsh.join(".zshenv"), ZSH_ENV),
            (hooks.zsh.join(".zprofile"), ZSH_PROFILE),
            (hooks.zsh.join(".zshrc"), ZSH_HOOK),
            (hooks.fish.clone(), FISH_HOOK),
        ];
        for (path, text) in files {
            // Line ends as the shells read them everywhere: bash and zsh refuse \r.
            std::fs::write(&path, text.replace("\r\n", "\n")).with_context(|| format!("write {}", path.display()))?;
        }
        Ok(hooks)
    }
}

/// `PATH` with `first` before everything it already has. An empty entry is
/// left out: in `PATH` it means the current directory, which must not get
/// there by accident (an unset `PATH` would otherwise leave one).
fn path_with(first: &Path, path: Option<OsString>) -> Result<OsString> {
    let rest = path.unwrap_or_default();
    let existing = std::env::split_paths(&rest).filter(|entry| !entry.as_os_str().is_empty());
    let all = std::iter::once(first.to_path_buf()).chain(existing);
    std::env::join_paths(all).with_context(|| format!("{} cannot be put in PATH", first.display()))
}

/// The arguments that make PowerShell run `scripts` at its start, in order, and stay open.
fn powershell_arguments(scripts: &[&Path]) -> [String; 3] {
    // In single quotes PowerShell takes everything as it is, but a quote, which is written twice.
    let run: Vec<String> =
        scripts.iter().map(|script| format!(". '{}'", script.to_string_lossy().replace('\'', "''"))).collect();
    ["-NoExit".into(), "-Command".into(), run.join("; ")]
}

/// How a tab's shell is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellStart {
    /// As a login shell: it reads the profile files (`/etc/zprofile`,
    /// `~/.zprofile`, `~/.bash_profile`, ...), where `PATH` gets Homebrew's and
    /// the user's own directories. Every terminal on macOS does this: a program
    /// started from the Finder has the system's short `PATH` and nothing else,
    /// so without it `brew`, and whatever a `.zshrc` expects to find, is missing.
    Login,
    /// As a shell inside a session that has read the profile files already:
    /// a Linux desktop, Windows.
    Plain,
}

impl ShellStart {
    /// What terminals do on this system.
    pub(crate) const fn here() -> Self {
        if cfg!(target_os = "macos") { Self::Login } else { Self::Plain }
    }
}

/// The program a tab runs (the default shell when `program` is `None`),
/// with UnifyTerm's commands in its way when `commands` has them, and the
/// history's hook when `hooks` has it.
///
/// A shell gets start scripts, hooks and its login argument only when the
/// settings give it no arguments of its own: arguments the user wrote are
/// the user's.
pub(crate) fn command(
    program: Option<&str>,
    args: &[String],
    commands: Option<&Commands>,
    hooks: Option<&Hooks>,
    start: ShellStart,
) -> Result<CommandBuilder> {
    let mut command = session::shell_command(program, args);
    // What a UnifyTerm this one was started from set for its own tab is not this tab's
    // (`commands.builtin` off here, a bash that is no login one): each is set below only where it applies.
    for name in [COMMANDS_VARIABLE, LOGIN_VARIABLE, ZDOTDIR_VARIABLE] {
        command.env_remove(name);
    }
    if let Some(commands) = commands {
        command.env("PATH", path_with(&commands.directory, std::env::var_os("PATH"))?);
        command.env(COMMANDS_VARIABLE, &commands.directory);
    }
    if !args.is_empty() {
        return Ok(command);
    }
    let kind = command.get_argv().first().map_or(ShellKind::Other, |program| shell_kind(&program.to_string_lossy()));
    let login = start == ShellStart::Login;
    match (kind, hooks) {
        (ShellKind::PowerShell, _) => {
            if login {
                // PowerShell takes it as its first argument only.
                command.arg("-Login");
            }
            // UnifyTerm's commands first: the hook then wraps the prompt they set.
            let scripts: Vec<&Path> =
                [commands.map(|c| c.powershell_start.as_path()), hooks.map(|h| h.powershell.as_path())]
                    .into_iter()
                    .flatten()
                    .collect();
            if !scripts.is_empty() {
                command.args(powershell_arguments(&scripts));
            }
        }
        (ShellKind::Bash, Some(hooks)) => {
            // A login bash reads no --rcfile: the hook reads the profile files in its place.
            if login {
                command.env(LOGIN_VARIABLE, "1");
            }
            command.args([OsString::from("--rcfile"), hooks.bash.clone().into()]);
        }
        (ShellKind::Bash, None) if login => command.arg("-l"),
        (ShellKind::Zsh, hooks) => {
            if login {
                command.arg("-l");
            }
            if let Some(hooks) = hooks {
                let own = std::env::var_os("ZDOTDIR").or_else(|| dirs::home_dir().map(PathBuf::into_os_string));
                command.env(ZDOTDIR_VARIABLE, own.unwrap_or_default());
                command.env("ZDOTDIR", &hooks.zsh);
            }
        }
        (ShellKind::Fish, hooks) => {
            if login {
                command.arg("-l");
            }
            if let Some(hooks) = hooks {
                let source = format!("source '{}'", hooks.fish.to_string_lossy().replace('\'', "\\'"));
                command.args(["--init-command".to_owned(), source]);
            }
        }
        (ShellKind::Cmd, Some(_)) => {
            let own = std::env::var("PROMPT").ok();
            command.env("PROMPT", cmd_prompt(own.as_deref()));
        }
        _ => {}
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unifyterm_config::shell::POWERSHELL_START;

    fn ready() -> Commands {
        Commands {
            directory: PathBuf::from("/data/unifyterm/bin"),
            powershell_start: PathBuf::from("/data/unifyterm/shell/start.ps1"),
            names: vec!["cat".into(), "ls".into()],
        }
    }

    fn argv(command: &CommandBuilder) -> Vec<String> {
        command.get_argv().iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_shell_is_known_by_its_files_name() {
        for program in
            ["pwsh", "pwsh.exe", r"C:\Program Files\PowerShell\7\pwsh.exe", "PowerShell.EXE", "/usr/bin/pwsh"]
        {
            assert_eq!(shell_kind(program), ShellKind::PowerShell, "{program}");
        }
        let others = [
            ("cmd.exe", ShellKind::Cmd),
            (r"C:\Program Files\Git\bin\bash.exe", ShellKind::Bash),
            ("/bin/zsh", ShellKind::Zsh),
            ("/usr/local/bin/fish", ShellKind::Fish),
            ("pwsh-preview", ShellKind::Other),
            ("nu", ShellKind::Other),
        ];
        for (program, kind) in others {
            assert_eq!(shell_kind(program), kind, "{program}");
        }
    }

    fn hooked() -> Hooks {
        Hooks {
            powershell: PathBuf::from("/data/unifyterm/shell/hooks/hook.ps1"),
            bash: PathBuf::from("/data/unifyterm/shell/hooks/bashrc"),
            zsh: PathBuf::from("/data/unifyterm/shell/hooks/zsh"),
            fish: PathBuf::from("/data/unifyterm/shell/hooks/hook.fish"),
        }
    }

    #[test]
    fn each_shell_gets_its_hook_and_only_without_arguments_of_the_users() {
        let (commands, hooks) = (ready(), hooked());
        let powershell = command(Some("pwsh.exe"), &[], Some(&commands), Some(&hooks), ShellStart::Plain).unwrap();
        assert_eq!(
            argv(&powershell)[1..],
            ["-NoExit", "-Command", ". '/data/unifyterm/shell/start.ps1'; . '/data/unifyterm/shell/hooks/hook.ps1'"],
            "the commands' script first: the hook wraps the prompt it sets"
        );
        let only_hook = command(Some("pwsh.exe"), &[], None, Some(&hooks), ShellStart::Plain).unwrap();
        assert_eq!(argv(&only_hook)[3], ". '/data/unifyterm/shell/hooks/hook.ps1'");
        assert_eq!(
            argv(&command(Some("bash"), &[], None, Some(&hooks), ShellStart::Plain).unwrap()),
            ["bash", "--rcfile", "/data/unifyterm/shell/hooks/bashrc"]
        );
        let zsh = command(Some("/bin/zsh"), &[], None, Some(&hooks), ShellStart::Plain).unwrap();
        assert_eq!(zsh.get_env("ZDOTDIR"), Some(std::ffi::OsStr::new("/data/unifyterm/shell/hooks/zsh")));
        assert!(zsh.get_env("UNIFYTERM_ZDOTDIR").is_some_and(|own| !own.is_empty()), "where the user's own files are");
        assert_eq!(
            argv(&command(Some("fish"), &[], None, Some(&hooks), ShellStart::Plain).unwrap()),
            ["fish", "--init-command", "source '/data/unifyterm/shell/hooks/hook.fish'"]
        );
        let cmd = command(Some("cmd.exe"), &[], None, Some(&hooks), ShellStart::Plain).unwrap();
        let prompt = cmd.get_env("PROMPT").unwrap().to_string_lossy().into_owned();
        assert!(prompt.contains("]133;A") && prompt.ends_with("$E]133;B$E\\"), "{prompt}");
        let own = ["-NoProfile".to_owned()];
        assert_eq!(
            argv(&command(Some("bash"), &own, None, Some(&hooks), ShellStart::Plain).unwrap()),
            ["bash", "-NoProfile"]
        );
        assert_eq!(
            argv(&command(Some("nu"), &[], None, Some(&hooks), ShellStart::Plain).unwrap()),
            ["nu"],
            "a shell with no hook"
        );
    }

    #[test]
    fn a_login_start_makes_each_shell_read_its_profile_files() {
        let (commands, hooks) = (ready(), hooked());
        let login = ShellStart::Login;
        assert_eq!(argv(&command(Some("/bin/zsh"), &[], None, None, login).unwrap()), ["/bin/zsh", "-l"]);
        let zsh = command(Some("/bin/zsh"), &[], Some(&commands), Some(&hooks), login).unwrap();
        assert_eq!(argv(&zsh), ["/bin/zsh", "-l"]);
        assert_eq!(
            zsh.get_env("ZDOTDIR"),
            Some(std::ffi::OsStr::new("/data/unifyterm/shell/hooks/zsh")),
            "the hook's files still"
        );
        assert_eq!(zsh.get_env(LOGIN_VARIABLE), None, "zsh's own -l is enough");

        let bash = command(Some("bash"), &[], None, Some(&hooks), login).unwrap();
        assert_eq!(
            argv(&bash),
            ["bash", "--rcfile", "/data/unifyterm/shell/hooks/bashrc"],
            "no -l: it would skip the hook"
        );
        assert_eq!(bash.get_env(LOGIN_VARIABLE), Some(std::ffi::OsStr::new("1")), "the hook reads the profile files");
        assert_eq!(argv(&command(Some("bash"), &[], Some(&commands), None, login).unwrap()), ["bash", "-l"]);

        assert_eq!(
            argv(&command(Some("fish"), &[], None, Some(&hooks), login).unwrap()),
            ["fish", "-l", "--init-command", "source '/data/unifyterm/shell/hooks/hook.fish'"]
        );
        assert_eq!(argv(&command(Some("fish"), &[], None, None, login).unwrap()), ["fish", "-l"]);

        let powershell = command(Some("pwsh"), &[], Some(&commands), Some(&hooks), login).unwrap();
        assert_eq!(argv(&powershell)[1..3], ["-Login", "-NoExit"], "-Login is PowerShell's first argument");
        assert_eq!(argv(&command(Some("pwsh"), &[], None, None, login).unwrap()), ["pwsh", "-Login"]);

        assert_eq!(
            argv(&command(Some("nu"), &[], None, Some(&hooks), login).unwrap()),
            ["nu"],
            "a shell it does not know"
        );
        let own = ["-f".to_owned()];
        assert_eq!(
            argv(&command(Some("/bin/zsh"), &own, None, Some(&hooks), login).unwrap()),
            ["/bin/zsh", "-f"],
            "the user's arguments are left as they are"
        );
        // A plain start adds none of it.
        let plain = command(Some("bash"), &[], None, Some(&hooks), ShellStart::Plain).unwrap();
        assert_eq!(plain.get_env(LOGIN_VARIABLE), None);
        assert_eq!(argv(&command(Some("/bin/zsh"), &[], None, None, ShellStart::Plain).unwrap()), ["/bin/zsh"]);
        assert_eq!(ShellStart::here() == ShellStart::Login, cfg!(target_os = "macos"), "macOS only");
    }

    #[test]
    fn the_hooks_put_the_commands_first_again_and_bash_reads_the_profile() {
        for (name, text) in [("bash", BASH_HOOK), ("zsh", ZSH_HOOK), ("fish", FISH_HOOK)] {
            assert!(text.contains("UNIFYTERM_COMMANDS"), "{name}'s hook puts the commands' directory first in PATH");
        }
        assert!(BASH_HOOK.contains(LOGIN_VARIABLE) && BASH_HOOK.contains("~/.bash_profile ~/.bash_login ~/.profile"));
        assert!(
            POWERSHELL_START.contains("$env:UNIFYTERM_COMMANDS) + $rest"),
            "PowerShell's start script does the same"
        );
    }

    #[test]
    fn cmd_keeps_its_own_prompt_inside_the_marks() {
        let default = cmd_prompt(None);
        assert_eq!(default, "$E]633;P;Shell=cmd$E\\$E]9;9;$P$E\\$E]133;A$E\\$P$G$E]133;B$E\\");
        assert_eq!(cmd_prompt(Some("$P$G")), default, "Windows' own default is the same");
        assert_eq!(cmd_prompt(Some("")), default);
        assert!(
            cmd_prompt(Some("[$T] $P$_$$ ")).contains("$E]133;A$E\\[$T] $P$_$$ $E]133;B$E\\"),
            "the user's is kept as it is"
        );
    }

    #[test]
    fn the_hooks_files_are_written_with_unix_line_ends() {
        let dir = tempfile::tempdir().unwrap();
        // Through the real data directory function, moved by its variable for this process only: a child.
        let hooks = Hooks {
            powershell: dir.path().join("hook.ps1"),
            bash: dir.path().join("bashrc"),
            zsh: dir.path().join("zsh"),
            fish: dir.path().join("hook.fish"),
        };
        for text in [POWERSHELL_HOOK, BASH_HOOK, ZSH_HOOK, FISH_HOOK, ZSH_ENV, ZSH_PROFILE] {
            assert!(text.replace("\r\n", "\n").lines().all(|line| !line.ends_with('\r')));
        }
        assert!(BASH_HOOK.contains("133;B") && ZSH_HOOK.contains("133;B") && FISH_HOOK.contains("133;B"));
        assert!(POWERSHELL_HOOK.contains("]133;A") && POWERSHELL_HOOK.contains("633;E"));
        drop(hooks);
    }

    #[test]
    fn the_commands_go_first_in_path() {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let existing = OsString::from(format!("/usr/bin{separator}/bin"));
        let path = path_with(Path::new("/data/bin"), Some(existing)).unwrap();
        assert_eq!(path, OsString::from(format!("/data/bin{separator}/usr/bin{separator}/bin")));
        assert_eq!(path_with(Path::new("/data/bin"), None).unwrap(), OsString::from("/data/bin"), "no PATH yet");
        let holes = OsString::from(format!("{separator}/usr/bin{separator}{separator}"));
        let path = path_with(Path::new("/data/bin"), Some(holes)).unwrap();
        assert_eq!(
            path,
            OsString::from(format!("/data/bin{separator}/usr/bin")),
            "empty entries (the current directory) are dropped"
        );
        // What `PATH` cannot hold in a directory's name: `:` on Unix, a quote on Windows.
        let bad = if cfg!(windows) { "/data\"bin" } else { "/data:bin" };
        assert!(path_with(Path::new(bad), None).is_err(), "{bad} cannot be in PATH");
    }

    #[test]
    fn a_quote_in_the_scripts_path_is_doubled() {
        let args = powershell_arguments(&[Path::new("C:/Users/O'Brien/start.ps1")]);
        assert_eq!(args, ["-NoExit", "-Command", ". 'C:/Users/O''Brien/start.ps1'"]);
    }

    #[test]
    fn powershell_gets_the_start_script_and_other_shells_do_not() {
        let commands = ready();
        let powershell = command(Some("pwsh.exe"), &[], Some(&commands), None, ShellStart::Plain).unwrap();
        assert_eq!(argv(&powershell)[1..], ["-NoExit", "-Command", ". '/data/unifyterm/shell/start.ps1'"]);
        assert_eq!(powershell.get_env(COMMANDS_VARIABLE), Some(std::ffi::OsStr::new("/data/unifyterm/bin")));
        let first = powershell.get_env("PATH").map(|path| std::env::split_paths(path).next());
        assert_eq!(first, Some(Some(PathBuf::from("/data/unifyterm/bin"))));

        assert_eq!(
            argv(&command(Some("bash"), &[], Some(&commands), None, ShellStart::Plain).unwrap()),
            ["bash"],
            "PATH is enough for bash"
        );
        let own = ["-NoProfile".to_owned()];
        let with_own = command(Some("pwsh.exe"), &own, Some(&commands), None, ShellStart::Plain).unwrap();
        assert_eq!(argv(&with_own), ["pwsh.exe", "-NoProfile"], "the user's arguments are left as they are");
    }

    #[test]
    fn without_commands_the_shell_is_started_as_it_is() {
        let plain = command(Some("pwsh.exe"), &[], None, None, ShellStart::Plain).unwrap();
        assert_eq!(argv(&plain), ["pwsh.exe"]);
        for name in [COMMANDS_VARIABLE, LOGIN_VARIABLE, ZDOTDIR_VARIABLE] {
            assert_eq!(
                plain.get_env(name),
                None,
                "{name}: none of its own, and none passed on from a UnifyTerm it was started from"
            );
        }
    }

    #[test]
    fn names_in_system_are_sorted_into_commands_and_typos() {
        let system = ["curl".to_owned(), "cat".to_owned(), "lss".to_owned()];
        let (known, unknown) = known_and_unknown(&system, &["cat", "ls"]);
        assert_eq!((known, unknown), (vec!["cat"], vec!["curl", "lss"]));
    }

    #[test]
    fn commands_turned_off_are_not_prepared() {
        let off = CommandsConfig { builtin: false, system: vec!["cat".into()] };
        assert_eq!(Commands::prepare(&off), (None, Vec::new()));
    }
}
