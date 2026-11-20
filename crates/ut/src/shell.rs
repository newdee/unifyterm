//! `ut shell enable` / `disable`: UnifyTerm's commands in every shell, not
//! only in the shells UnifyTerm's tabs start (another terminal's, keepane's
//! panes, an agent's).
//!
//! `enable` does three things, and `disable` takes each of them back:
//! 1. makes the directory of links (as the terminal does, `ut --links`);
//! 2. writes a start script for each kind of shell into UnifyTerm's data
//!    directory, and puts a line that loads it at the end of each shell's
//!    start file (PowerShell's profile, `.zshrc`, `.bashrc`, fish's
//!    `config.fish`), between two marked lines: `disable` removes only those;
//! 3. on Windows, puts the directory first in the user's `PATH`, for
//!    programs started without a start file (`cmd.exe`, a program an agent
//!    runs) once their parent has the new `PATH`.
//!
//! A start script puts the directory first in `PATH`; PowerShell's also
//! takes PowerShell's own aliases of the commands' names out of the way
//! (the same script as the terminal's, `unifyterm-config`'s `start.ps1`).

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use unifyterm_config::Config;
use unifyterm_config::shell::{directory_name, powershell_start};

use crate::USAGE_ERROR;

/// The exit code when something went wrong that is not the command line's.
const TROUBLE: i32 = 1;

const USAGE: &str = "usage: ut shell enable    UnifyTerm's commands in every new shell, not only in UnifyTerm's tabs
       ut shell disable   take out what enable put in
       ut shell status    whether they are enabled, and where

enable puts a line at the end of each shell's start file (PowerShell's profile,
.zshrc, .bashrc, fish's config.fish) between two marked lines, and on Windows
the commands' directory first in your PATH. disable removes only those.
A shell that is open now keeps what it has: open a new one.";

/// The line before what `enable` adds to a start file.
const BEGIN: &str = "# >>> unifyterm: added by 'ut shell enable'; 'ut shell disable' takes it out >>>";
/// The line after it.
const END: &str = "# <<< unifyterm <<<";

/// A kind of shell, as far as its start file and its script go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    PowerShell,
    /// bash and zsh: one script in the language they share.
    Posix,
    Fish,
}

impl Kind {
    /// The start script's file name in the data directory's `shell` folder.
    const fn script(self) -> &'static str {
        match self {
            Self::PowerShell => "init.ps1",
            Self::Posix => "init.sh",
            Self::Fish => "init.fish",
        }
    }

    /// `text` as a literal in this shell's language: in single quotes.
    fn quoted(self, text: &str) -> String {
        match self {
            Self::PowerShell => format!("'{}'", text.replace('\'', "''")),
            Self::Posix => format!("'{}'", text.replace('\'', r"'\''")),
            Self::Fish => format!("'{}'", text.replace('\\', r"\\").replace('\'', r"\'")),
        }
    }

    /// What a file this shell reads starts with, empty. Windows PowerShell
    /// takes a file without UTF-8's mark for the system's code page: a path
    /// with letters outside ASCII (a user's name) would come out wrong.
    const fn empty_file(self) -> &'static str {
        match self {
            Self::PowerShell => "\u{feff}",
            Self::Posix | Self::Fish => "",
        }
    }

    /// The line a start file gets: load the script, when it is there (it
    /// is not once UnifyTerm is gone, and the shell then starts as before).
    fn line(self, script: &Path) -> String {
        let script = self.quoted(&script.display().to_string());
        match self {
            Self::PowerShell => format!("if (Test-Path -LiteralPath {script}) {{ . {script} }}"),
            Self::Posix => format!("if [ -f {script} ]; then . {script}; fi"),
            Self::Fish => format!("test -f {script}; and source {script}"),
        }
    }

    /// The start script, for the commands in `directory` of which `names` are in use.
    fn script_text(self, directory: &Path, names: &[&str]) -> String {
        let head = format!("{}# Written by 'ut shell enable': changes here are lost.\n", self.empty_file());
        let directory = self.quoted(&directory.display().to_string());
        match self {
            // The terminal sets UNIFYTERM_COMMANDS for its tabs; elsewhere this does. Then the terminal's script.
            Self::PowerShell => format!(
                "{head}if (-not $env:UNIFYTERM_COMMANDS) {{ $env:UNIFYTERM_COMMANDS = {directory} }}\n{}",
                powershell_start(names)
            ),
            // A shell started from this one has the directory first already: PATH is not made longer each time.
            Self::Posix => format!(
                "{head}[ -n \"${{UNIFYTERM_COMMANDS-}}\" ] || UNIFYTERM_COMMANDS={directory}\nexport UNIFYTERM_COMMANDS\n\
                 case \"$PATH\" in\n    \"$UNIFYTERM_COMMANDS\" | \"$UNIFYTERM_COMMANDS\":*) ;;\n    \
                 *) PATH=\"$UNIFYTERM_COMMANDS:$PATH\"; export PATH ;;\nesac\n"
            ),
            Self::Fish => format!(
                "{head}set -q UNIFYTERM_COMMANDS; or set -gx UNIFYTERM_COMMANDS {directory}\n\
                 if test \"$PATH[1]\" != \"$UNIFYTERM_COMMANDS\"\n    set -gx PATH $UNIFYTERM_COMMANDS $PATH\nend\n"
            ),
        }
    }
}

/// A shell's start file, and which kind of shell reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartFile {
    path: PathBuf,
    kind: Kind,
}

/// Where `enable` and `disable` work. On a real system [`Places::here`];
/// a test makes its own, in a directory of its own.
#[derive(Debug)]
struct Places {
    /// UnifyTerm's data directory: the links and the start scripts go in it.
    data: PathBuf,
    /// The start files of the shells this system has.
    start_files: Vec<StartFile>,
    /// The commands' names, and those the settings leave to the system.
    commands: Vec<&'static str>,
    skipped: Vec<String>,
    /// The program the links lead to.
    program: PathBuf,
    /// Where the user's `PATH` is kept (Windows); `None` elsewhere.
    #[cfg(windows)]
    environment: Option<winreg::RegKey>,
}

/// Whether `program` is in a directory of `PATH`.
fn on_path(program: &str) -> bool {
    let file = if cfg!(windows) { format!("{program}.exe") } else { program.to_owned() };
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&file).is_file()))
}

/// The start files of the shells this system has: a shell's file when the
/// shell is on `PATH` or the file is there already.
fn start_files(home: &Path) -> Vec<StartFile> {
    let mut found = Vec::new();
    let mut add = |path: PathBuf, kind: Kind, shell: bool| {
        if shell || path.is_file() {
            found.push(StartFile { path, kind });
        }
    };
    // The profile every PowerShell of the user's reads (`CurrentUserAllHosts`).
    if cfg!(windows) {
        if let Some(documents) = dirs::document_dir() {
            add(documents.join("WindowsPowerShell").join("profile.ps1"), Kind::PowerShell, true);
            add(documents.join("PowerShell").join("profile.ps1"), Kind::PowerShell, on_path("pwsh"));
        }
    } else {
        add(home.join(".config").join("powershell").join("profile.ps1"), Kind::PowerShell, on_path("pwsh"));
        let zsh =
            std::env::var_os("ZDOTDIR").filter(|dir| !dir.is_empty()).map_or_else(|| home.to_path_buf(), PathBuf::from);
        add(zsh.join(".zshrc"), Kind::Posix, on_path("zsh"));
        add(home.join(".bashrc"), Kind::Posix, on_path("bash"));
        add(home.join(".config").join("fish").join("config.fish"), Kind::Fish, on_path("fish"));
    }
    found
}

impl Places {
    /// This system's: the settings say which commands stay the system's.
    fn here() -> Result<Self> {
        let settings = crate::setting::file_path(None)?;
        let config = Config::load(&settings).with_context(|| format!("read {}", settings.display()))?;
        if !config.commands.builtin {
            bail!("commands.builtin is false in UnifyTerm's settings: 'ut setting set commands.builtin true' first");
        }
        let home = dirs::home_dir().context("this system names no home directory")?;
        Ok(Self {
            data: unifyterm_history::data_directory()?,
            start_files: start_files(&home),
            commands: crate::commands().into_iter().map(|(name, _)| name).collect(),
            skipped: config.commands.system,
            program: std::env::current_exe().context("ut cannot find its own file")?,
            #[cfg(windows)]
            environment: Some(
                winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Environment", winreg::enums::KEY_READ | winreg::enums::KEY_WRITE)
                    .context("open the user's environment (HKCU\\Environment)")?,
            ),
        })
    }

    /// The directory of links for these settings (the terminal's name for it).
    fn links(&self) -> PathBuf {
        let skipped: Vec<&str> =
            self.skipped.iter().map(String::as_str).filter(|name| self.commands.contains(name)).collect();
        self.data.join(directory_name(&skipped))
    }

    fn script(&self, kind: Kind) -> PathBuf {
        self.data.join("shell").join(kind.script())
    }
}

/// The newline `text` uses: Windows' when it has one, else `\n`; a new file
/// gets the system's.
fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") || (text.is_empty() && cfg!(windows)) { "\r\n" } else { "\n" }
}

/// Where the marked lines are in `text`: from the start of the first to the
/// end of the second, with its newline. `None` when there are none.
fn block_at(text: &str) -> Result<Option<(usize, usize)>> {
    let mut begin = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        // The first line may start with UTF-8's mark (`Kind::empty_file`).
        let bare = line.trim_end_matches(['\r', '\n']).trim_start_matches('\u{feff}');
        if bare == BEGIN && begin.is_none() {
            // The mark stays where it is: the block starts after it.
            begin = Some(at + line.len() - line.trim_start_matches('\u{feff}').len());
        } else if bare == END
            && let Some(begin) = begin
        {
            return Ok(Some((begin, at + line.len())));
        }
        at += line.len();
    }
    match begin {
        Some(_) => bail!("it has the line '{BEGIN}' but not '{END}' after it: put that right by hand, then try again"),
        None => Ok(None),
    }
}

/// `text` with `line` between the marked lines: in place of the marked
/// lines that are there, else at the end.
fn with_block(text: &str, line: &str) -> Result<String> {
    let newline = newline_of(text);
    let block = format!("{BEGIN}{newline}{line}{newline}{END}{newline}");
    Ok(match block_at(text)? {
        Some((begin, end)) => format!("{}{block}{}", &text[..begin], &text[end..]),
        None if text.is_empty() || text == "\u{feff}" => format!("{text}{block}"),
        None if text.ends_with('\n') => format!("{text}{newline}{block}"),
        None => format!("{text}{newline}{newline}{block}"),
    })
}

/// `text` without the marked lines, and without the empty line `with_block` put before them.
fn without_block(text: &str) -> Result<String> {
    let Some((mut begin, end)) = block_at(text)? else { return Ok(text.to_owned()) };
    let before = &text[..begin];
    let newline = newline_of(text);
    if before.ends_with(&format!("{newline}{newline}")) {
        begin -= newline.len();
    }
    Ok(format!("{}{}", &text[..begin], &text[end..]))
}

/// Reads a start file; an absent one is empty.
fn read(path: &Path) -> Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not UTF-8 text: not changed", path.display()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Writes `text` to `path` whole or not at all: a new file beside it, then
/// renamed over it. A link (a dotfile manager's) stays a link: the file it
/// leads to is written.
fn write(path: &Path, text: &str) -> Result<()> {
    let is_link = path.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink());
    let path = if is_link {
        std::fs::canonicalize(path).with_context(|| format!("follow {}", path.display()))?
    } else {
        path.to_path_buf()
    };
    let folder = path.parent().context("a start file is in a folder")?;
    std::fs::create_dir_all(folder).with_context(|| format!("make {}", folder.display()))?;
    let mut new = path.clone().into_os_string();
    new.push(".unifyterm-new");
    let written = std::fs::write(&new, text).and_then(|()| std::fs::rename(&new, &path));
    if written.is_err() {
        // Nothing of this attempt stays beside the user's file.
        let _ = std::fs::remove_file(&new);
    }
    written.with_context(|| format!("replace {}", path.display()))
}

/// `PATH`'s separator.
#[cfg(any(windows, test))]
const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// Two of `PATH`'s entries are one directory: the same but for case (Windows) and a last separator.
#[cfg(any(windows, test))]
fn same_entry(a: &str, b: &str) -> bool {
    let bare = |entry: &str| entry.trim_end_matches(['\\', '/']).to_owned();
    if cfg!(windows) { bare(a).eq_ignore_ascii_case(&bare(b)) } else { bare(a) == bare(b) }
}

/// `path` (a `PATH` value) with `directory` first, and nowhere else.
#[cfg(any(windows, test))]
fn with_directory(path: &str, directory: &str) -> String {
    let rest = path.split(SEPARATOR).filter(|entry| !entry.is_empty() && !same_entry(entry, directory));
    std::iter::once(directory).chain(rest).collect::<Vec<_>>().join(&SEPARATOR.to_string())
}

/// `path` without `directory`.
#[cfg(any(windows, test))]
fn without_directory(path: &str, directory: &str) -> String {
    path.split(SEPARATOR)
        .filter(|entry| !entry.is_empty() && !same_entry(entry, directory))
        .collect::<Vec<_>>()
        .join(&SEPARATOR.to_string())
}

/// The user's `PATH` in the registry, as written (`%USERPROFILE%` not
/// expanded), and whether it is to be expanded (`REG_EXPAND_SZ`).
#[cfg(windows)]
fn user_path(key: &winreg::RegKey) -> Result<(String, winreg::enums::RegType)> {
    use winreg::enums::RegType;
    match key.get_raw_value("Path") {
        Ok(raw) => {
            let units: Vec<u16> = raw.bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
            let text = String::from_utf16(&units).context("the user's PATH is not text")?;
            let kind = if raw.vtype == RegType::REG_SZ { RegType::REG_SZ } else { RegType::REG_EXPAND_SZ };
            Ok((text.trim_end_matches('\0').to_owned(), kind))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((String::new(), RegType::REG_EXPAND_SZ)),
        Err(error) => Err(error).context("read the user's PATH"),
    }
}

#[cfg(windows)]
fn set_user_path(key: &winreg::RegKey, value: &str, kind: winreg::enums::RegType) -> Result<()> {
    let bytes: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    key.set_raw_value("Path", &winreg::RegValue { bytes: bytes.into(), vtype: kind }).context("write the user's PATH")
}

/// Tells the programs that are running (Explorer first) that the user's
/// environment changed: a program Explorer starts after this has the new `PATH`.
#[cfg(windows)]
fn say_environment_changed() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    /// How long a window that does not answer is waited for.
    const WAIT_MS: u32 = 2000;
    let name: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
    let mut answer = 0;
    // SAFETY: `name` is a NUL-ended string that lives through the call; `answer` is a place for the result.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            name.as_ptr().expose_provenance().cast_signed(),
            SMTO_ABORTIFHUNG,
            WAIT_MS,
            &raw mut answer,
        );
    }
}

/// What `enable` or `disable` did, one line a change, to be printed.
type Done = Vec<String>;

/// Each start file's new text, where it changes. Every file is read and
/// changed in memory before any is written: a file that cannot be changed
/// (not UTF-8, a mark without its end) leaves all of them as they were.
fn new_texts(places: &Places, edit: impl Fn(&StartFile, &str) -> Result<String>) -> Result<Vec<(&StartFile, String)>> {
    let mut changes = Vec::new();
    for file in &places.start_files {
        let before = read(&file.path)?;
        let after = edit(file, &before).with_context(|| file.path.display().to_string())?;
        if after != before {
            changes.push((file, after));
        }
    }
    Ok(changes)
}

fn enable(places: &Places) -> Result<Done> {
    let changes = new_texts(places, |file, before| {
        let start = if before.is_empty() { file.kind.empty_file() } else { before };
        with_block(start, &file.kind.line(&places.script(file.kind)))
    })?;
    let mut done = Done::new();
    let links = places.links();
    let mut linked = places.commands.clone();
    linked.push("ut");
    crate::links::sync(&links, &places.program, &linked, &places.skipped)
        .with_context(|| format!("make {}", links.display()))?;
    let in_use: Vec<&str> =
        places.commands.iter().copied().filter(|name| !places.skipped.iter().any(|s| s == name)).collect();
    done.push(format!("{} commands in {}", in_use.len(), links.display()));
    // The scripts first: a start file's line loads one.
    for file in &places.start_files {
        let script = places.script(file.kind);
        let folder = script.parent().context("the scripts are in a folder")?;
        std::fs::create_dir_all(folder).with_context(|| format!("make {}", folder.display()))?;
        std::fs::write(&script, file.kind.script_text(&links, &in_use))
            .with_context(|| format!("write {}", script.display()))?;
    }
    for (file, after) in changes {
        write(&file.path, &after)?;
        done.push(format!("added a line to {}", file.path.display()));
    }
    #[cfg(windows)]
    if let Some(key) = &places.environment {
        let (path, kind) = user_path(key)?;
        let wanted = with_directory(&path, &links.display().to_string());
        if wanted != path {
            set_user_path(key, &wanted, kind)?;
            say_environment_changed();
            done.push("put it first in your PATH".into());
        }
    }
    Ok(done)
}

fn disable(places: &Places) -> Result<Done> {
    let mut done = Done::new();
    for (file, after) in new_texts(places, |_, before| without_block(before))? {
        write(&file.path, &after)?;
        done.push(format!("took the line out of {}", file.path.display()));
    }
    for kind in [Kind::PowerShell, Kind::Posix, Kind::Fish] {
        match std::fs::remove_file(places.script(kind)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", places.script(kind).display())),
        }
    }
    #[cfg(windows)]
    if let Some(key) = &places.environment {
        let (path, kind) = user_path(key)?;
        // Every directory of links, whichever settings it was made for.
        let ours = |entry: &str| {
            Path::new(entry)
                .parent()
                .is_some_and(|parent| same_entry(&parent.display().to_string(), &places.data.display().to_string()))
        };
        let mut wanted = path.clone();
        for entry in path.split(SEPARATOR).filter(|entry| ours(entry)) {
            wanted = without_directory(&wanted, entry);
        }
        if wanted != path {
            set_user_path(key, &wanted, kind)?;
            say_environment_changed();
            done.push("took it out of your PATH".into());
        }
    }
    Ok(done)
}

fn status(places: &Places, out: &mut impl io::Write) -> Result<()> {
    for file in &places.start_files {
        let has = block_at(&read(&file.path)?).ok().flatten().is_some();
        writeln!(out, "{}  {}", if has { "enabled " } else { "disabled" }, file.path.display())?;
    }
    #[cfg(windows)]
    if let Some(key) = &places.environment {
        let (path, _) = user_path(key)?;
        let links = places.links().display().to_string();
        let has = path.split(SEPARATOR).next().is_some_and(|first| same_entry(first, &links));
        writeln!(out, "{}  PATH: {links}", if has { "enabled " } else { "disabled" })?;
    }
    Ok(())
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let ask: Vec<&str> = args.iter().filter_map(|arg| arg.to_str()).collect();
    let mut out = io::stdout().lock();
    if matches!(ask.as_slice(), ["--help" | "-h"]) {
        let _ = writeln!(out, "{USAGE}");
        return 0;
    }
    if !matches!(ask.as_slice(), ["enable" | "disable" | "status"]) || ask.len() != args.len() {
        let _ = writeln!(io::stderr(), "{USAGE}");
        return USAGE_ERROR;
    }
    let result = Places::here().and_then(|places| match ask[0] {
        "enable" => enable(&places).map(|done| {
            let _ = writeln!(
                out,
                "{}",
                if done.len() > 1 { done.join("\n") } else { format!("{}\nalready enabled", done.join("\n")) }
            );
            let _ = writeln!(out, "A new shell has them; one that is open now does not.");
        }),
        "disable" => disable(&places).map(|done| {
            let _ = writeln!(out, "{}", if done.is_empty() { "nothing to take out".into() } else { done.join("\n") });
        }),
        _ => status(&places, &mut out),
    });
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut shell: {error:#}");
            TROUBLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_goes_in_once_and_comes_out_whole() {
        let line = "if [ -f '/d/init.sh' ]; then . '/d/init.sh'; fi";
        let mine = "export EDITOR=vim\nalias ll='ls -l'\n";
        let once = with_block(mine, line).unwrap();
        assert_eq!(once, format!("{mine}\n{BEGIN}\n{line}\n{END}\n"));
        assert_eq!(with_block(&once, line).unwrap(), once, "enable twice: one block");
        assert_eq!(without_block(&once).unwrap(), mine, "disable gives the file back as it was");
        assert_eq!(without_block(mine).unwrap(), mine, "nothing to take out: nothing changes");
        let changed = with_block(&once, "other").unwrap();
        assert!(changed.contains("\nother\n") && !changed.contains(line), "a new line takes the old one's place");
        assert_eq!(
            without_block(&format!("{once}echo after\n")).unwrap(),
            format!("{mine}echo after\n"),
            "what comes after stays"
        );
    }

    #[test]
    fn a_files_own_newlines_and_mark_are_kept() {
        let windows = "\u{feff}Set-Alias g git\r\n";
        let once = with_block(windows, "x").unwrap();
        assert_eq!(once, format!("\u{feff}Set-Alias g git\r\n\r\n{BEGIN}\r\nx\r\n{END}\r\n"));
        assert_eq!(without_block(&once).unwrap(), windows);
        assert_eq!(
            with_block("no newline at the end", "x").unwrap(),
            format!("no newline at the end\n\n{BEGIN}\nx\n{END}\n")
        );
        let new = with_block("", "x").unwrap();
        assert!(new.starts_with(BEGIN), "a new file starts with the block");
    }

    #[test]
    fn a_begin_without_an_end_is_not_touched() {
        let broken = format!("a\n{BEGIN}\nb\n");
        assert!(with_block(&broken, "x").is_err());
        assert!(without_block(&broken).is_err());
    }

    #[test]
    fn each_shell_gets_a_line_in_its_language() {
        let script = Path::new("/data/it's/init.sh");
        assert_eq!(Kind::Posix.line(script), r"if [ -f '/data/it'\''s/init.sh' ]; then . '/data/it'\''s/init.sh'; fi");
        assert_eq!(Kind::Fish.line(script), r"test -f '/data/it\'s/init.sh'; and source '/data/it\'s/init.sh'");
        assert_eq!(
            Kind::PowerShell.line(script),
            "if (Test-Path -LiteralPath '/data/it''s/init.sh') { . '/data/it''s/init.sh' }"
        );
        let powershell = Kind::PowerShell.script_text(Path::new("C:/d/bin"), &["cat"]);
        assert!(
            powershell.contains("$env:UNIFYTERM_COMMANDS = 'C:/d/bin'")
                && powershell.contains("@('cat','Get-Content')")
        );
        assert!(powershell.contains("if ($global:__unifyterm_loaded) { return }"), "once in each PowerShell");
    }

    #[test]
    fn the_directory_goes_first_in_path_once() {
        let s = SEPARATOR;
        let path = format!("/usr/bin{s}/d/bin{s}/bin");
        assert_eq!(with_directory(&path, "/d/bin"), format!("/d/bin{s}/usr/bin{s}/bin"));
        assert_eq!(with_directory("", "/d/bin"), "/d/bin");
        assert_eq!(without_directory(&format!("/d/bin{s}/usr/bin{s}{s}/d/bin/"), "/d/bin"), "/usr/bin");
    }

    /// A data directory and a home of the test's own, and the places in them.
    fn places(root: &Path) -> Places {
        let program = root.join("program");
        std::fs::write(&program, "the program").unwrap();
        Places {
            data: root.join("data"),
            start_files: vec![
                StartFile { path: root.join("home").join(".zshrc"), kind: Kind::Posix },
                StartFile { path: root.join("home").join(".config/fish/config.fish"), kind: Kind::Fish },
                StartFile { path: root.join("home").join("profile.ps1"), kind: Kind::PowerShell },
            ],
            commands: vec!["cat", "ls", "grep"],
            skipped: vec!["grep".into()],
            program,
            #[cfg(windows)]
            environment: None,
        }
    }

    #[test]
    fn enable_then_disable_leaves_the_files_as_they_were() {
        let root = tempfile::tempdir().unwrap();
        let places = places(root.path());
        let zshrc = &places.start_files[0].path;
        std::fs::create_dir_all(zshrc.parent().unwrap()).unwrap();
        std::fs::write(zshrc, "export A=1\n").unwrap();

        let done = enable(&places).unwrap();
        assert_eq!(done.len(), 4, "the links, and a line in each of 3 files: {done:?}");
        let links = places.links();
        assert_ne!(links, places.data.join("bin"), "grep is the system's: the settings' own directory");
        assert!(links.join(if cfg!(windows) { "cat.exe" } else { "cat" }).exists());
        assert!(!links.join(if cfg!(windows) { "grep.exe" } else { "grep" }).exists(), "a skipped command has no link");
        let zsh = std::fs::read_to_string(zshrc).unwrap();
        assert!(zsh.starts_with("export A=1\n\n") && zsh.contains(&Kind::Posix.line(&places.script(Kind::Posix))));
        let script = std::fs::read_to_string(places.script(Kind::Posix)).unwrap();
        assert!(script.contains(&links.display().to_string()), "{script}");
        assert!(places.script(Kind::PowerShell).is_file() && places.script(Kind::Fish).is_file());
        let profile = std::fs::read_to_string(&places.start_files[2].path).unwrap();
        assert!(
            profile.starts_with(&format!("\u{feff}{BEGIN}")),
            "a new PowerShell profile has UTF-8's mark: {profile:?}"
        );
        assert!(std::fs::read_to_string(places.script(Kind::PowerShell)).unwrap().starts_with('\u{feff}'));
        assert!(
            !std::fs::read_to_string(places.script(Kind::Posix)).unwrap().starts_with('\u{feff}'),
            "sh has no use for it"
        );
        let again = enable(&places).unwrap();
        assert_eq!(again.len(), 1, "a second enable changes no file: {again:?}");

        let mut shown = Vec::new();
        status(&places, &mut shown).unwrap();
        assert_eq!(String::from_utf8(shown).unwrap().matches("enabled ").count(), 3);

        assert_eq!(disable(&places).unwrap().len(), 3);
        assert_eq!(std::fs::read_to_string(zshrc).unwrap(), "export A=1\n", "the user's file is as it was");
        assert_eq!(
            std::fs::read_to_string(&places.start_files[1].path).unwrap(),
            "",
            "a file enable made is left empty"
        );
        assert_eq!(
            std::fs::read_to_string(&places.start_files[2].path).unwrap(),
            "\u{feff}",
            "PowerShell's keeps UTF-8's mark"
        );
        assert!(!places.script(Kind::Posix).exists(), "the scripts are gone");
        assert!(disable(&places).unwrap().is_empty(), "a second disable changes nothing");
    }

    #[test]
    fn a_start_file_that_is_not_utf8_is_not_touched() {
        let root = tempfile::tempdir().unwrap();
        let places = places(root.path());
        let zshrc = &places.start_files[0].path;
        std::fs::create_dir_all(zshrc.parent().unwrap()).unwrap();
        // "中文" in GBK, as an old profile may be.
        let gbk = b"echo \xd6\xd0\xce\xc4\n".to_vec();
        std::fs::write(zshrc, &gbk).unwrap();
        let said = format!("{:#}", enable(&places).unwrap_err());
        assert!(said.contains("not UTF-8"), "{said}");
        assert_eq!(std::fs::read(zshrc).unwrap(), gbk, "not one byte changed");
        assert!(disable(&places).is_err(), "disable leaves it too");
        assert_eq!(std::fs::read(zshrc).unwrap(), gbk);
    }

    #[test]
    fn one_file_that_cannot_be_changed_leaves_every_file_as_it_was() {
        let root = tempfile::tempdir().unwrap();
        let places = places(root.path());
        let (zshrc, fish) = (&places.start_files[0].path, &places.start_files[1].path);
        std::fs::create_dir_all(fish.parent().unwrap()).unwrap();
        std::fs::create_dir_all(zshrc.parent().unwrap()).unwrap();
        std::fs::write(zshrc, "export A=1\n").unwrap();
        std::fs::write(fish, format!("{BEGIN}\nno end\n")).unwrap();
        assert!(enable(&places).is_err(), "the second file has a mark without its end");
        assert_eq!(std::fs::read_to_string(zshrc).unwrap(), "export A=1\n", "the first file is not changed either");
        assert!(!places.start_files[2].path.exists(), "nor the third made");
        assert!(!places.script(Kind::Posix).exists(), "no script written");
    }

    #[test]
    fn a_failed_write_leaves_nothing_beside_the_file() {
        let root = tempfile::tempdir().unwrap();
        // A directory where the file would be: the new file is written, the rename over a directory fails.
        let file = root.path().join("profile.ps1");
        std::fs::create_dir_all(file.join("in-the-way")).unwrap();
        assert!(write(&file, "x").is_err());
        let left: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["profile.ps1"], "no .unifyterm-new stays");
    }
    #[test]
    fn an_end_without_a_begin_and_a_second_begin_are_read_safely() {
        let end_only = format!("a\n{END}\n");
        assert_eq!(without_block(&end_only).unwrap(), end_only, "an end alone is not a block");
        let once = with_block(&end_only, "x").unwrap();
        assert_eq!(once.matches(BEGIN).count(), 1);
        assert_eq!(without_block(&once).unwrap(), end_only);
    }
    #[cfg(unix)]
    #[test]
    fn a_linked_start_file_stays_a_link() {
        let root = tempfile::tempdir().unwrap();
        let places = places(root.path());
        let zshrc = &places.start_files[0].path;
        let real = root.path().join("dotfiles-zshrc");
        std::fs::write(&real, "export A=1\n").unwrap();
        std::fs::create_dir_all(zshrc.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, zshrc).unwrap();
        enable(&places).unwrap();
        assert!(zshrc.symlink_metadata().unwrap().file_type().is_symlink(), "still a link");
        assert!(std::fs::read_to_string(&real).unwrap().contains(BEGIN), "the file it leads to has the line");
    }

    /// The scripts in real shells: `cat` is UnifyTerm's after the script, and
    /// a shell started from that one (it has `UNIFYTERM_COMMANDS`, not the
    /// script's mark) runs the script for itself. Skipped where the shell is not.
    #[test]
    fn a_shell_started_from_one_runs_the_script_again() {
        let root = tempfile::tempdir().unwrap();
        // A name outside ASCII, as a user's name can be: Windows PowerShell reads it right only with UTF-8's mark.
        let root = root.path().join("用户");
        std::fs::create_dir_all(&root).unwrap();
        let mut places = places(&root);
        places.program = std::env::current_exe().unwrap();
        places.start_files.clear();
        enable(&places).unwrap();
        let links = places.links();
        let in_use = ["cat", "ls"];
        for kind in [Kind::PowerShell, Kind::Posix] {
            std::fs::create_dir_all(places.script(kind).parent().unwrap()).unwrap();
            std::fs::write(places.script(kind), kind.script_text(&links, &in_use)).unwrap();
        }
        let run = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .env_remove("UNIFYTERM_COMMANDS")
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().replace('\\', "/"))
        };
        let script = places.script(Kind::PowerShell).display().to_string();
        // Where `cat` is found, and the mark. Without the script it is not ours: an alias on
        // Windows (no source), the system's program elsewhere (PowerShell on Linux has no such alias).
        let ours = links.join(if cfg!(windows) { "cat.exe" } else { "cat" }).display().to_string().replace('\\', "/");
        let ask = "(Get-Command cat).Source + '|' + [bool]$global:__unifyterm_loaded";
        let child = format!("pwsh -NoProfile -Command \"{}\"", ask.replace('$', "`$"));
        let command = format!(
            ". '{script}'; . '{script}'; {ask}; pwsh -NoProfile -Command \". '{script}'; {}\"",
            ask.replace('$', "`$")
        );
        // PowerShell 7, and on Windows PowerShell 5.1 too; a shell that is not there is skipped.
        for shell in ["pwsh", "powershell"] {
            let command = command.replace("pwsh -NoProfile", &format!("{shell} -NoProfile"));
            let Some(said) = run(shell, &["-NoProfile", "-Command", &command]) else { continue };
            let found = format!("{ours}|True");
            assert_eq!(said.lines().collect::<Vec<_>>(), [found.as_str(), found.as_str()], "{shell}: {said}");
            let child = child.replace("pwsh -NoProfile", &format!("{shell} -NoProfile"));
            let without = run(shell, &["-NoProfile", "-Command", &format!(". '{script}'; {child}")]).unwrap();
            assert!(
                !without.starts_with(&ours) && without.ends_with("|False"),
                "{shell}: a shell that does not run it does not find ours: {without}"
            );
        }
        #[cfg(unix)]
        {
            let script = places.script(Kind::Posix).display().to_string();
            let said = run("sh", &["-c", &format!(". '{script}'; . '{script}'; command -v cat")]).unwrap();
            assert_eq!(said, links.join("cat").display().to_string());
        }
    }
    /// The registry part, in a key of the test's own (never the user's environment).
    #[cfg(windows)]
    #[test]
    fn the_users_path_keeps_its_kind_and_the_rest() {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, RegType};
        let name = format!("Software\\unifyterm-test-{}", std::process::id());
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(&name).unwrap();
        set_user_path(&key, "%USERPROFILE%\\bin;C:\\tools", RegType::REG_EXPAND_SZ).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut places = places(root.path());
        places.start_files.clear();
        places.environment = Some(key);
        enable(&places).unwrap();
        let key = places.environment.as_ref().unwrap();
        let links = places.links().display().to_string();
        assert_eq!(user_path(key).unwrap(), (format!("{links};%USERPROFILE%\\bin;C:\\tools"), RegType::REG_EXPAND_SZ));
        disable(&places).unwrap();
        assert_eq!(user_path(key).unwrap(), ("%USERPROFILE%\\bin;C:\\tools".to_owned(), RegType::REG_EXPAND_SZ));
        RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&name).unwrap();
    }
}
