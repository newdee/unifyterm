//! which: where in `PATH` a command's program is.
//!
//! Windows has `where` and PowerShell has `Get-Command`; neither is what
//! scripts written for Unix call.

use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: which [-a] COMMAND...

  -a   print every match in PATH, not only the first";
const USAGE_ERROR: i32 = 2;
/// The extensions tried on Windows when `PATHEXT` is not set.
const DEFAULT_EXTENSIONS: &str = ".COM;.EXE;.BAT;.CMD";

/// Whether a file is one the system can run as a command.
fn runs(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    meta.is_file()
}

/// The programs `command` names, in the order the system would find them.
/// `path`: the directories to look in. `extensions`: what may follow the
/// name (Windows' `PATHEXT`); empty on Unix, where the name is the whole
/// file name. A `command` with a directory in it is looked for there only.
pub(crate) fn find(command: &OsStr, path: &[PathBuf], extensions: &[String]) -> Vec<PathBuf> {
    let names = |base: &Path| -> Vec<PathBuf> {
        let mut names = vec![base.to_path_buf()];
        names.extend(extensions.iter().map(|extension| {
            let mut name = base.as_os_str().to_owned();
            name.push(extension);
            PathBuf::from(name)
        }));
        names
    };
    let given = Path::new(command);
    let directories: Vec<PathBuf> = if given.components().count() > 1 { vec![PathBuf::new()] } else { path.to_vec() };
    let mut found = Vec::new();
    for directory in directories {
        for candidate in names(&directory.join(given)) {
            if runs(&candidate) {
                let program = as_named(candidate);
                if !found.contains(&program) {
                    found.push(program);
                }
            }
        }
    }
    found
}

/// A file's path with the name its directory gives it. On Windows a file
/// is found whatever the letters' case, and with an extension taken from
/// `PATHEXT` (`.EXE`): what is printed is the file's own name (`cat.exe`).
fn as_named(file: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return file;
    }
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else { return file };
    let wanted = name.to_string_lossy().to_lowercase();
    let listed = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
    let own = std::fs::read_dir(listed).ok().and_then(|entries| {
        entries.flatten().map(|entry| entry.file_name()).find(|entry| entry.to_string_lossy().to_lowercase() == wanted)
    });
    match own {
        Some(own) => parent.join(own),
        None => file,
    }
}

/// `PATHEXT` as a list (`.COM;.EXE` to `[".COM", ".EXE"]`).
fn extensions(pathext: &str) -> Vec<String> {
    pathext.split(';').filter(|e| !e.is_empty()).map(str::to_owned).collect()
}

pub(crate) fn main(args: Vec<OsString>) -> i32 {
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    let (mut all, mut commands) = (false, Vec::new());
    let mut options = true;
    for arg in args.into_iter().skip(1) {
        match arg.to_str() {
            Some("--") if options => options = false,
            Some("-a") if options => all = true,
            Some("--help") if options => {
                let _ = writeln!(out, "{USAGE}");
                return 0;
            }
            Some(option) if options && option.starts_with('-') && option.len() > 1 => {
                let _ = writeln!(err, "which: unknown option '{option}'\n{USAGE}");
                return USAGE_ERROR;
            }
            _ => commands.push(arg),
        }
    }
    if commands.is_empty() {
        let _ = writeln!(err, "{USAGE}");
        return USAGE_ERROR;
    }
    let path: Vec<PathBuf> =
        std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    let extensions = if cfg!(windows) {
        extensions(&std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_EXTENSIONS.into()))
    } else {
        Vec::new()
    };
    let mut missing = false;
    for command in &commands {
        let found = find(command, &path, &extensions);
        missing |= found.is_empty();
        for program in found.iter().take(if all { usize::MAX } else { 1 }) {
            if writeln!(out, "{}", crate::slash::shown(program)).is_err() {
                return 0; // The reader stopped reading.
            }
        }
    }
    i32::from(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two directories with a runnable `tool` in each and a plain file in the first.
    fn scratch(test: &str) -> (PathBuf, Vec<PathBuf>, String) {
        let root = std::env::temp_dir().join(format!("ut-which-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = vec![root.join("one"), root.join("two")];
        let tool = if cfg!(windows) { "tool.exe" } else { "tool" };
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(tool), "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(dir.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        std::fs::write(dirs[0].join("readme"), "").unwrap();
        (root, dirs, tool.to_owned())
    }

    fn windows_extensions() -> Vec<String> {
        if cfg!(windows) { extensions(DEFAULT_EXTENSIONS) } else { Vec::new() }
    }

    #[test]
    fn the_first_directory_in_path_wins() {
        let (root, dirs, tool) = scratch("order");
        let found = find(OsStr::new("tool"), &dirs, &windows_extensions());
        assert_eq!(found, [dirs[0].join(&tool), dirs[1].join(&tool)]);
        let reversed: Vec<_> = dirs.iter().rev().cloned().collect();
        assert_eq!(find(OsStr::new("tool"), &reversed, &windows_extensions())[0], dirs[1].join(&tool));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn what_is_not_there_or_not_a_program_is_not_found() {
        let (root, dirs, _) = scratch("missing");
        assert!(find(OsStr::new("no-such-tool"), &dirs, &windows_extensions()).is_empty());
        assert!(find(OsStr::new("tool"), &[], &windows_extensions()).is_empty(), "an empty PATH");
        // On Unix a file that may not be executed is not a command.
        #[cfg(unix)]
        assert!(find(OsStr::new("readme"), &dirs, &[]).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_name_with_a_directory_is_looked_for_there_only() {
        let (root, dirs, tool) = scratch("direct");
        let direct = dirs[1].join("tool");
        assert_eq!(find(direct.as_os_str(), &dirs, &windows_extensions()), [dirs[1].join(&tool)]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pathext_is_a_list() {
        assert_eq!(extensions(".COM;.EXE;;.BAT"), [".COM", ".EXE", ".BAT"]);
        assert!(extensions("").is_empty());
    }
}
