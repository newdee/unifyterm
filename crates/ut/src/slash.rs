//! Paths printed with `/` on Windows, as on every other system.
//!
//! The commands are other people's and print a path the way the system
//! writes it: `C:\Users\me` on Windows. A file's name cannot have a `\` in
//! it there, so in what these commands print a `\` is always a separator
//! and can be written `/`, which Windows reads just as well.
//!
//! `UT_NATIVE_PATHS` (set to anything) keeps the system's own form.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{Command, Stdio};

/// The environment variable that keeps Windows' own separators.
pub(crate) const NATIVE_PATHS: &str = "UT_NATIVE_PATHS";

/// uutils commands whose standard output is paths, and nothing a `\` could
/// otherwise be in. (`ls` is not among them: what it prints depends on
/// whether it writes to a terminal, which a filter in between would hide.)
pub(crate) const PRINT_PATHS: [&str; 7] = ["df", "dirname", "du", "mktemp", "pwd", "readlink", "realpath"];

/// Whether paths are to be printed with `/`: on Windows, unless the user asked for its own form.
pub(crate) fn wanted() -> bool {
    cfg!(windows) && std::env::var_os(NATIVE_PATHS).is_none()
}

/// Passes on what is written with every `\` as `/`.
#[derive(Debug)]
pub(crate) struct Slashes<W: Write>(pub(crate) W);

impl<W: Write> Write for Slashes<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let slashed: Vec<u8> = bytes.iter().map(|byte| if *byte == b'\\' { b'/' } else { *byte }).collect();
        self.0.write_all(&slashed)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// A path as text, with `/` where [`wanted`] says so.
pub(crate) fn shown(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if wanted() { text.replace('\\', "/") } else { text.into_owned() }
}

/// What this program ends with when it cannot start itself again.
const CANNOT_RERUN: i32 = 126;

/// Runs this program again with the same arguments, its paths in the
/// system's form, and prints what it prints with `/`. The exit code is its.
///
/// A second process, because a uutils command writes to standard output
/// itself: there is no place inside this one to change what it writes.
pub(crate) fn rerun(arguments: &[OsString]) -> i32 {
    let started = std::env::current_exe().and_then(|program| {
        Command::new(program).args(arguments).env(NATIVE_PATHS, "1").stdout(Stdio::piped()).spawn()
    });
    let mut child = match started {
        Ok(child) => child,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut: cannot start itself again: {error}");
            return CANNOT_RERUN;
        }
    };
    if let Some(mut printed) = child.stdout.take() {
        // A reader that stopped reading ends the copy; the command's exit code still counts.
        let _ = io::copy(&mut printed, &mut Slashes(io::stdout().lock()));
    }
    child.wait().ok().and_then(|status| status.code()).unwrap_or(CANNOT_RERUN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backslash_becomes_a_slash() {
        let mut out = Slashes(Vec::new());
        out.write_all(br"C:\Users\me\a b.txt").unwrap();
        out.write_all(b"\nno/change here\n").unwrap();
        out.write_all("中文\\路径\n".as_bytes()).unwrap();
        assert_eq!(String::from_utf8(out.0).unwrap(), "C:/Users/me/a b.txt\nno/change here\n中文/路径\n");
    }

    #[test]
    fn a_write_reports_all_its_bytes_taken() {
        let mut out = Slashes(Vec::new());
        assert_eq!(out.write(br"a\b").unwrap(), 3, "the caller's count, not the translated one");
        assert_eq!(out.write(b"").unwrap(), 0);
    }

    #[test]
    fn the_commands_that_print_paths_are_commands() {
        let mut sorted = PRINT_PATHS;
        sorted.sort_unstable();
        assert_eq!(sorted, PRINT_PATHS, "kept in order, so a name is easy to find");
    }
}
