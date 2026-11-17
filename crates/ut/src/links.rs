//! The command directory: one link to this program for every command, so
//! that a shell with the directory first in its `PATH` finds `cat`, `ls`
//! and the rest here. `ut --links DIRECTORY [--skip A,B]` makes the
//! directory match the commands this program has, less the skipped ones,
//! and one link named `ut`: the program itself (`ut history`, `ut --list`).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// The link to this program under its own name.
const OWN_NAME: &str = "ut";

/// The file in the directory that lists the links this program made, one
/// name a line: only those are ever removed.
const MANIFEST: &str = ".ut-links";

#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct Summary {
    /// Links that were made or made again.
    pub(crate) made: usize,
    /// Links that already led to this program.
    pub(crate) kept: usize,
    /// Links to commands that are now skipped or gone.
    pub(crate) removed: usize,
}

/// A command's file in the directory: its name, with `.exe` on Windows.
fn link_name(command: &str) -> String {
    if cfg!(windows) { format!("{command}.exe") } else { command.to_owned() }
}

/// A symbolic link on Unix. A hard link on Windows, where a symbolic link
/// needs a privilege most users do not have; a copy when the directory is
/// on another drive, which a hard link cannot cross.
fn link(program: &Path, at: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(program, at);
    #[cfg(not(unix))]
    return std::fs::hard_link(program, at).or_else(|_| std::fs::copy(program, at).map(|_| ()));
}

fn leads_to(at: &Path, program: &Path) -> bool {
    same_file::is_same_file(at, program).unwrap_or(false)
}

/// Makes `directory` hold a link to `program` for each of `commands` not in
/// `skip`, and no link of an earlier run for anything else.
pub(crate) fn sync(directory: &Path, program: &Path, commands: &[&str], skip: &[String]) -> io::Result<Summary> {
    std::fs::create_dir_all(directory)?;
    let wanted: BTreeSet<&str> = commands.iter().copied().filter(|name| !skip.iter().any(|s| s == name)).collect();
    let manifest = directory.join(MANIFEST);
    let before = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut summary = Summary::default();
    for old in before.lines().filter(|name| !name.is_empty() && !wanted.contains(name)) {
        match std::fs::remove_file(directory.join(link_name(old))) {
            Ok(()) => summary.removed += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for name in &wanted {
        let at = directory.join(link_name(name));
        if leads_to(&at, program) {
            summary.kept += 1;
            continue;
        }
        match std::fs::remove_file(&at) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        link(program, &at)?;
        summary.made += 1;
    }
    let mut listed = String::new();
    for name in &wanted {
        listed.push_str(name);
        listed.push('\n');
    }
    std::fs::write(manifest, listed)?;
    Ok(summary)
}

const USAGE: &str = "usage: ut --links DIRECTORY [--skip COMMAND,COMMAND...]";
const USAGE_ERROR: i32 = 2;

/// `ut --links`: `args` starts after `--links`. `commands`: every command's name.
pub(crate) fn main(args: &[OsString], commands: &[&str]) -> i32 {
    let mut err = io::stderr().lock();
    let mut directory: Option<PathBuf> = None;
    let mut skip: Vec<String> = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--skip" {
            let Some(names) = args.next() else {
                let _ = writeln!(err, "ut: --skip needs command names\n{USAGE}");
                return USAGE_ERROR;
            };
            skip.extend(names.to_string_lossy().split(',').filter(|name| !name.is_empty()).map(str::to_owned));
        } else if directory.is_none() {
            directory = Some(arg.into());
        } else {
            let _ = writeln!(err, "ut: unexpected argument '{}'\n{USAGE}", arg.to_string_lossy());
            return USAGE_ERROR;
        }
    }
    let Some(directory) = directory else {
        let _ = writeln!(err, "{USAGE}");
        return USAGE_ERROR;
    };
    if let Some(unknown) = skip.iter().find(|name| !commands.contains(&name.as_str())) {
        let _ = writeln!(err, "ut: --skip names '{unknown}', which is not one of ut's commands");
        return USAGE_ERROR;
    }
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            let _ = writeln!(err, "ut: cannot find its own file: {error}");
            return 1;
        }
    };
    let mut linked = commands.to_vec();
    linked.push(OWN_NAME);
    match sync(&directory, &program, &linked, &skip) {
        Ok(summary) => {
            let total = summary.made + summary.kept;
            let Summary { made, kept, removed } = summary;
            let _ = writeln!(
                io::stdout(),
                "{total} links in {} (made {made}, kept {kept}, removed {removed})",
                crate::slash::shown(&directory)
            );
            0
        }
        Err(error) => {
            let _ = writeln!(err, "ut: {}: {error}", crate::slash::shown(&directory));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, and a file in it standing for the program.
    fn scratch(test: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ut-links-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("program");
        std::fs::write(&program, "the program").unwrap();
        (root, program)
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn links_are_made_kept_and_removed() {
        let (root, program) = scratch("sync");
        let bin = root.join("bin");
        let first = sync(&bin, &program, &["cat", "ls", "jq"], &[]).unwrap();
        assert_eq!(first, Summary { made: 3, kept: 0, removed: 0 });
        assert_eq!(names(&bin), [MANIFEST.to_owned(), link_name("cat"), link_name("jq"), link_name("ls")]);
        assert_eq!(
            std::fs::read_to_string(bin.join(link_name("cat"))).unwrap(),
            "the program",
            "the link leads to the program"
        );

        let again = sync(&bin, &program, &["cat", "ls", "jq"], &[]).unwrap();
        assert_eq!(again, Summary { made: 0, kept: 3, removed: 0 }, "nothing to do the second time");

        let skipped = sync(&bin, &program, &["cat", "ls", "jq"], &["jq".into()]).unwrap();
        assert_eq!(skipped, Summary { made: 0, kept: 2, removed: 1 });
        assert_eq!(names(&bin), [MANIFEST.to_owned(), link_name("cat"), link_name("ls")]);

        let back = sync(&bin, &program, &["cat", "ls", "jq"], &[]).unwrap();
        assert_eq!(back, Summary { made: 1, kept: 2, removed: 0 });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_the_user_put_there_is_left_alone() {
        let (root, program) = scratch("foreign");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("notes.txt"), "mine").unwrap();
        sync(&bin, &program, &["cat"], &[]).unwrap();
        sync(&bin, &program, &[], &[]).unwrap();
        assert_eq!(
            names(&bin),
            [MANIFEST.to_owned(), "notes.txt".to_owned()],
            "only what the manifest lists is removed"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_link_to_an_older_program_is_made_again() {
        let (root, program) = scratch("stale");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // What an earlier version left: a file of that name that is not this program.
        std::fs::write(bin.join(link_name("cat")), "an older program").unwrap();
        let summary = sync(&bin, &program, &["cat"], &[]).unwrap();
        assert_eq!(summary, Summary { made: 1, kept: 0, removed: 0 });
        assert_eq!(std::fs::read_to_string(bin.join(link_name("cat"))).unwrap(), "the program");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_program_itself_is_linked_too() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        assert_eq!(main(&[bin.clone().into()], &["cat", "ls"]), 0);
        for name in ["cat", "ls", OWN_NAME] {
            assert!(bin.join(link_name(name)).is_file(), "{name} is linked");
        }
        let listed = std::fs::read_to_string(bin.join(MANIFEST)).unwrap();
        assert_eq!(listed, "cat\nls\nut\n");
        assert_eq!(
            main(&[bin.into(), "--skip".into(), "ut".into()], &["cat", "ls"]),
            USAGE_ERROR,
            "ut is no command to skip"
        );
    }

    #[test]
    fn mistakes_in_the_arguments_are_refused() {
        let commands = ["cat", "ls"];
        assert_eq!(main(&[], &commands), USAGE_ERROR);
        assert_eq!(main(&["dir".into(), "--skip".into()], &commands), USAGE_ERROR);
        assert_eq!(main(&["dir".into(), "extra".into()], &commands), USAGE_ERROR);
        assert_eq!(
            main(&["dir".into(), "--skip".into(), "nope".into()], &commands),
            USAGE_ERROR,
            "an unknown command to skip is a typo"
        );
    }
}
