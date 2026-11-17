//! What a tab's shell is started with: uni-term's commands first in its
//! `PATH`, for PowerShell a start script that takes its aliases of the
//! same names out of their way, and a hook that marks the prompt and each
//! command for the history (PowerShell, bash, zsh, fish, cmd.exe).
//!
//! The commands are `ut`, the program beside `uni-term`. A directory of
//! links to it, one a command, is made (by `ut --links`) in the user's data
//! directory; a shell with that directory first in `PATH` finds `cat`,
//! `ls` and the rest there on every system.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use portable_pty::CommandBuilder;

use crate::config::CommandsConfig;
use crate::session;

/// The environment variable that tells programs where uni-term's commands
/// are. The PowerShell start script reads it (`start.ps1`).
const COMMANDS_VARIABLE: &str = "UNI_TERM_COMMANDS";

/// PowerShell's aliases that hide a command of uni-term's, each with the
/// cmdlet it stands for when nobody has changed it.
const POWERSHELL_ALIASES: [(&str, &str); 14] = [
    ("cat", "Get-Content"),
    ("cp", "Copy-Item"),
    ("diff", "Compare-Object"),
    ("dir", "Get-ChildItem"),
    ("echo", "Write-Output"),
    ("kill", "Stop-Process"),
    ("ls", "Get-ChildItem"),
    ("mv", "Move-Item"),
    ("pwd", "Get-Location"),
    ("rm", "Remove-Item"),
    ("rmdir", "Remove-Item"),
    ("sleep", "Start-Sleep"),
    ("sort", "Sort-Object"),
    ("tee", "Tee-Object"),
];

/// PowerShell's functions that hide a command of uni-term's, each with a
/// piece of text its own definition has.
const POWERSHELL_FUNCTIONS: [(&str, &str); 2] = [("mkdir", "New-Item"), ("more", "more.com")];

/// uni-term's commands, ready for a shell to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Commands {
    /// The directory of links, to go first in `PATH`.
    directory: PathBuf,
    /// The script PowerShell runs at its start.
    powershell_start: PathBuf,
    /// The names of the commands in use.
    names: Vec<String>,
}

/// Where uni-term keeps the command links and the start scripts (and the history, and the log).
fn data_directory() -> Result<PathBuf> {
    uni_history::data_directory()
}

/// `ut`, which is beside `uni-term`.
fn ut_program() -> Result<PathBuf> {
    let own = std::env::current_exe().context("uni-term cannot find its own file")?;
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

/// The script PowerShell runs at its start, with `@NAME@` where a value goes.
const POWERSHELL_START: &str = include_str!("start.ps1");

/// The script PowerShell runs at its start, for these commands.
///
/// PowerShell finds an alias or a function before a program of the same
/// name. For each of `commands` the alias or function is removed, but only
/// while it is still PowerShell's own: one the user defined is theirs to keep.
/// The rest of what the script does is said in the script (`start.ps1`).
fn powershell_start(commands: &[&str]) -> String {
    let pairs = |table: &[(&str, &str)]| -> String {
        let wanted = table.iter().filter(|(name, _)| commands.contains(name));
        wanted.map(|(name, own)| format!("@('{name}','{own}')")).collect::<Vec<_>>().join(", ")
    };
    // Inside the script's single quotes a `'` is written twice.
    let quoted = |text: &str| text.replace('\'', "''");
    let names = commands.iter().map(|name| format!("'{}'", quoted(name))).collect::<Vec<_>>().join(", ");
    POWERSHELL_START
        .replace("@ALIASES@", &pairs(&POWERSHELL_ALIASES))
        .replace("@FUNCTIONS@", &pairs(&POWERSHELL_FUNCTIONS))
        .replace("@NAMES@", &names)
}
impl Commands {
    /// The names of uni-term's commands in use (`cat`, `ls`, ...).
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

    /// Makes uni-term's commands ready as the settings say. `None` when the
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
                    .map(|name| format!("commands.system names '{name}', which is not a command of uni-term's"));
                (Some(commands), told.collect())
            }
            Err(error) => (None, vec![format!("uni-term's commands are not in use: {error:#}")]),
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

/// FNV-1a's starting value and multiplier for 64 bits.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The name of the link directory for a set of skipped commands: `bin` for
/// none, else `bin-` and a number made from the names. Two windows whose
/// settings skip different commands then each have a directory of their own
/// and do not undo each other's links. The number is the same for the same
/// names in any order, on every system and in every version (FNV-1a).
fn directory_name(skipped: &[&str]) -> String {
    if skipped.is_empty() {
        return "bin".into();
    }
    let mut sorted = skipped.to_vec();
    sorted.sort_unstable();
    let bytes = sorted.join(",").into_bytes();
    let hash = bytes.iter().fold(FNV_OFFSET, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME));
    format!("bin-{hash:016x}")
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

/// The hooks' files, written to uni-term's data directory at each start.
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

/// The program a tab runs (the default shell when `program` is `None`),
/// with uni-term's commands in its way when `commands` has them, and the
/// history's hook when `hooks` has it.
///
/// A shell gets start scripts and hooks only when the settings give it no
/// arguments of its own: arguments the user wrote are the user's.
pub(crate) fn command(
    program: Option<&str>,
    args: &[String],
    commands: Option<&Commands>,
    hooks: Option<&Hooks>,
) -> Result<CommandBuilder> {
    let mut command = session::shell_command(program, args)?;
    if let Some(commands) = commands {
        command.env("PATH", path_with(&commands.directory, std::env::var_os("PATH"))?);
        command.env(COMMANDS_VARIABLE, &commands.directory);
    }
    if !args.is_empty() {
        return Ok(command);
    }
    let kind = command.get_argv().first().map_or(ShellKind::Other, |program| shell_kind(&program.to_string_lossy()));
    match (kind, hooks) {
        (ShellKind::PowerShell, _) => {
            // uni-term's commands first: the hook then wraps the prompt they set.
            let scripts: Vec<&Path> =
                [commands.map(|c| c.powershell_start.as_path()), hooks.map(|h| h.powershell.as_path())]
                    .into_iter()
                    .flatten()
                    .collect();
            if !scripts.is_empty() {
                command.args(powershell_arguments(&scripts));
            }
        }
        (ShellKind::Bash, Some(hooks)) => command.args([OsString::from("--rcfile"), hooks.bash.clone().into()]),
        (ShellKind::Zsh, Some(hooks)) => {
            let own = std::env::var_os("ZDOTDIR").or_else(|| dirs::home_dir().map(PathBuf::into_os_string));
            command.env("UNI_TERM_ZDOTDIR", own.unwrap_or_default());
            command.env("ZDOTDIR", &hooks.zsh);
        }
        (ShellKind::Fish, Some(hooks)) => {
            let source = format!("source '{}'", hooks.fish.to_string_lossy().replace('\'', "\\'"));
            command.args(["--init-command".to_owned(), source]);
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

    fn ready() -> Commands {
        Commands {
            directory: PathBuf::from("/data/uni-term/bin"),
            powershell_start: PathBuf::from("/data/uni-term/shell/start.ps1"),
            names: vec!["cat".into(), "ls".into()],
        }
    }

    fn argv(command: &CommandBuilder) -> Vec<String> {
        command.get_argv().iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn the_start_script_removes_only_what_hides_a_command_in_use() {
        let script = powershell_start(&["cat", "ls", "jq", "mkdir", "o'neil"]);
        assert!(script.contains("@('cat','Get-Content'), @('ls','Get-ChildItem')"), "{script}");
        assert!(script.contains("@('mkdir','New-Item')"), "{script}");
        for absent in ["@('rm'", "@('sort'", "@('more'", "@('jq'"] {
            assert!(!script.contains(absent), "{absent} is not hidden by PowerShell or not in use:\n{script}");
        }
        assert!(script.contains("$alias.Definition -eq $pair[1]"), "an alias the user changed is kept");
        assert!(script.contains("UTF8Encoding"), "the console is set to UTF-8");
        assert!(script.contains("function global:prompt {") && script.contains(".Replace('\\', '/')"), "{script}");
        assert!(
            script.contains(".Contains('PS $($executionContext.SessionState.Path.CurrentLocation)$(')"),
            "only PowerShell's own prompt is replaced"
        );
        assert!(
            script.contains("[string[]]@('cat', 'ls', 'jq', 'mkdir', 'o''neil')"),
            "the commands in use are named, quoted:\n{script}"
        );
        assert!(
            script.contains("$found.Source.StartsWith($env:UNI_TERM_COMMANDS"),
            "the directory is read from {COMMANDS_VARIABLE}"
        );
        assert!(script.contains("[char]0xE000"), "the mark is the one `ut` takes off (ut's src/expand.rs)");
        for place in ["@ALIASES@", "@FUNCTIONS@", "@NAMES@"] {
            assert!(!script.contains(place), "{place} was given its value");
        }
        let none = powershell_start(&[]);
        assert!(none.contains("foreach ($pair in @()) {"), "no command in use: nothing to remove\n{none}");
        assert!(none.contains("[string[]]@()"), "{none}");
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
            powershell: PathBuf::from("/data/uni-term/shell/hooks/hook.ps1"),
            bash: PathBuf::from("/data/uni-term/shell/hooks/bashrc"),
            zsh: PathBuf::from("/data/uni-term/shell/hooks/zsh"),
            fish: PathBuf::from("/data/uni-term/shell/hooks/hook.fish"),
        }
    }

    #[test]
    fn each_shell_gets_its_hook_and_only_without_arguments_of_the_users() {
        let (commands, hooks) = (ready(), hooked());
        let powershell = command(Some("pwsh.exe"), &[], Some(&commands), Some(&hooks)).unwrap();
        assert_eq!(
            argv(&powershell)[1..],
            ["-NoExit", "-Command", ". '/data/uni-term/shell/start.ps1'; . '/data/uni-term/shell/hooks/hook.ps1'"],
            "the commands' script first: the hook wraps the prompt it sets"
        );
        let only_hook = command(Some("pwsh.exe"), &[], None, Some(&hooks)).unwrap();
        assert_eq!(argv(&only_hook)[3], ". '/data/uni-term/shell/hooks/hook.ps1'");
        assert_eq!(
            argv(&command(Some("bash"), &[], None, Some(&hooks)).unwrap()),
            ["bash", "--rcfile", "/data/uni-term/shell/hooks/bashrc"]
        );
        let zsh = command(Some("/bin/zsh"), &[], None, Some(&hooks)).unwrap();
        assert_eq!(zsh.get_env("ZDOTDIR"), Some(std::ffi::OsStr::new("/data/uni-term/shell/hooks/zsh")));
        assert!(zsh.get_env("UNI_TERM_ZDOTDIR").is_some_and(|own| !own.is_empty()), "where the user's own files are");
        assert_eq!(
            argv(&command(Some("fish"), &[], None, Some(&hooks)).unwrap()),
            ["fish", "--init-command", "source '/data/uni-term/shell/hooks/hook.fish'"]
        );
        let cmd = command(Some("cmd.exe"), &[], None, Some(&hooks)).unwrap();
        let prompt = cmd.get_env("PROMPT").unwrap().to_string_lossy().into_owned();
        assert!(prompt.contains("]133;A") && prompt.ends_with("$E]133;B$E\\"), "{prompt}");
        let own = ["-NoProfile".to_owned()];
        assert_eq!(argv(&command(Some("bash"), &own, None, Some(&hooks)).unwrap()), ["bash", "-NoProfile"]);
        assert_eq!(argv(&command(Some("nu"), &[], None, Some(&hooks)).unwrap()), ["nu"], "a shell with no hook");
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
        let powershell = command(Some("pwsh.exe"), &[], Some(&commands), None).unwrap();
        assert_eq!(argv(&powershell)[1..], ["-NoExit", "-Command", ". '/data/uni-term/shell/start.ps1'"]);
        assert_eq!(powershell.get_env(COMMANDS_VARIABLE), Some(std::ffi::OsStr::new("/data/uni-term/bin")));
        let first = powershell.get_env("PATH").map(|path| std::env::split_paths(path).next());
        assert_eq!(first, Some(Some(PathBuf::from("/data/uni-term/bin"))));

        assert_eq!(
            argv(&command(Some("bash"), &[], Some(&commands), None).unwrap()),
            ["bash"],
            "PATH is enough for bash"
        );
        let own = ["-NoProfile".to_owned()];
        let with_own = command(Some("pwsh.exe"), &own, Some(&commands), None).unwrap();
        assert_eq!(argv(&with_own), ["pwsh.exe", "-NoProfile"], "the user's arguments are left as they are");
    }

    #[test]
    fn without_commands_the_shell_is_started_as_it_is() {
        let plain = command(Some("pwsh.exe"), &[], None, None).unwrap();
        assert_eq!(argv(&plain), ["pwsh.exe"]);
        assert_eq!(plain.get_env(COMMANDS_VARIABLE), None);
    }

    #[test]
    fn names_in_system_are_sorted_into_commands_and_typos() {
        let system = ["curl".to_owned(), "cat".to_owned(), "lss".to_owned()];
        let (known, unknown) = known_and_unknown(&system, &["cat", "ls"]);
        assert_eq!((known, unknown), (vec!["cat"], vec!["curl", "lss"]));
    }

    #[test]
    fn each_set_of_skipped_commands_has_a_directory() {
        assert_eq!(directory_name(&[]), "bin");
        assert_eq!(directory_name(&["cat"]), "bin-f5e307190ce4a327", "FNV-1a of \"cat\", the same everywhere");
        assert_eq!(directory_name(&["ls", "cat"]), directory_name(&["cat", "ls"]), "the order does not matter");
        assert_ne!(directory_name(&["cat"]), directory_name(&["cat", "ls"]));
        assert_ne!(directory_name(&["a", "bc"]), directory_name(&["ab", "c"]), "the names are kept apart");
    }

    #[test]
    fn commands_turned_off_are_not_prepared() {
        let off = CommandsConfig { builtin: false, system: vec!["cat".into()] };
        assert_eq!(Commands::prepare(&off), (None, Vec::new()));
    }
}
