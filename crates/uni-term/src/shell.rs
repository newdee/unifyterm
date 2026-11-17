//! What a tab's shell is started with: uni-term's commands first in its
//! `PATH`, and for PowerShell a start script that takes its aliases of the
//! same names out of their way.
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
}

/// Where uni-term keeps the command links and the start scripts.
fn data_directory() -> Result<PathBuf> {
    Ok(dirs::data_local_dir().context("this system has no data directory")?.join("uni-term"))
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

/// Runs `ut` with `args` and gives what it printed. No console window opens for it.
fn run_ut(ut: &Path, args: &[OsString]) -> Result<String> {
    let mut command = Command::new(ut);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// `CREATE_NO_WINDOW`: a console program started by a window program gets no console window.
        const NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(NO_WINDOW);
    }
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
        Ok((Self { directory, powershell_start }, unknown))
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

/// Whether a program's path names PowerShell (`pwsh` or `powershell`, with or without `.exe`).
fn is_powershell(program: &str) -> bool {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    matches!(file.strip_suffix(".exe").unwrap_or(&file), "pwsh" | "powershell")
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

/// The arguments that make PowerShell run `script` at its start and stay open.
fn powershell_arguments(script: &Path) -> [String; 3] {
    // In single quotes PowerShell takes everything as it is, but a quote, which is written twice.
    let quoted = script.to_string_lossy().replace('\'', "''");
    ["-NoExit".into(), "-Command".into(), format!(". '{quoted}'")]
}

/// The program a tab runs (the default shell when `program` is `None`),
/// with uni-term's commands in its way when `commands` has them.
///
/// PowerShell gets the start script only when the settings give it no
/// arguments of its own: arguments the user wrote are the user's.
pub(crate) fn command(program: Option<&str>, args: &[String], commands: Option<&Commands>) -> Result<CommandBuilder> {
    let mut command = session::shell_command(program, args)?;
    let Some(commands) = commands else { return Ok(command) };
    command.env("PATH", path_with(&commands.directory, std::env::var_os("PATH"))?);
    command.env(COMMANDS_VARIABLE, &commands.directory);
    let runs_powershell = command.get_argv().first().is_some_and(|program| is_powershell(&program.to_string_lossy()));
    if runs_powershell && args.is_empty() {
        command.args(powershell_arguments(&commands.powershell_start));
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
    fn powershell_is_known_by_its_files_name() {
        for program in
            ["pwsh", "pwsh.exe", r"C:\Program Files\PowerShell\7\pwsh.exe", "PowerShell.EXE", "/usr/bin/pwsh"]
        {
            assert!(is_powershell(program), "{program}");
        }
        for program in ["cmd.exe", "bash", "/bin/zsh", "pwsh-preview", "nu"] {
            assert!(!is_powershell(program), "{program}");
        }
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
        let args = powershell_arguments(Path::new("C:/Users/O'Brien/start.ps1"));
        assert_eq!(args, ["-NoExit", "-Command", ". 'C:/Users/O''Brien/start.ps1'"]);
    }

    #[test]
    fn powershell_gets_the_start_script_and_other_shells_do_not() {
        let commands = ready();
        let powershell = command(Some("pwsh.exe"), &[], Some(&commands)).unwrap();
        assert_eq!(argv(&powershell)[1..], ["-NoExit", "-Command", ". '/data/uni-term/shell/start.ps1'"]);
        assert_eq!(powershell.get_env(COMMANDS_VARIABLE), Some(std::ffi::OsStr::new("/data/uni-term/bin")));
        let first = powershell.get_env("PATH").map(|path| std::env::split_paths(path).next());
        assert_eq!(first, Some(Some(PathBuf::from("/data/uni-term/bin"))));

        assert_eq!(argv(&command(Some("bash"), &[], Some(&commands)).unwrap()), ["bash"], "PATH is enough for bash");
        let own = ["-NoProfile".to_owned()];
        let with_own = command(Some("pwsh.exe"), &own, Some(&commands)).unwrap();
        assert_eq!(argv(&with_own), ["pwsh.exe", "-NoProfile"], "the user's arguments are left as they are");
    }

    #[test]
    fn without_commands_the_shell_is_started_as_it_is() {
        let plain = command(Some("pwsh.exe"), &[], None).unwrap();
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
