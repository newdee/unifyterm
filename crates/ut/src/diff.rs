//! diff and cmp: uutils diffutils' formats and comparisons, driven as its
//! own `main` drives them. That `main` takes the process's arguments as
//! they are, so it cannot be called from a program that is many commands;
//! the two drivers are repeated here over the same library functions.
//! (uutils diffutils is MIT or Apache-2.0.)

use std::ffi::OsString;
use std::io::{self, Read as _, Write as _};

use diffutilslib::params::{Format, parse_params};
use diffutilslib::utils::report_failure_to_read_input_file;
use diffutilslib::{cmp as compare, context_diff, ed_diff, normal_diff, side_by_side_diff, unified_diff};

/// diff's and cmp's exit codes: nothing differs, something differs, trouble.
const SAME: i32 = 0;
const DIFFERENT: i32 = 1;
const TROUBLE: i32 = 2;

/// The file's bytes; `-` is standard input.
fn read(file: &OsString) -> io::Result<Vec<u8>> {
    if file == "-" {
        let mut content = Vec::new();
        io::stdin().read_to_end(&mut content).map(|_| content)
    } else {
        std::fs::read(file)
    }
}

/// The same file twice, or standard input twice: equal without reading.
fn one_file(from: &OsString, to: &OsString) -> bool {
    from == "-" && to == "-" || same_file::is_same_file(from, to).unwrap_or(false)
}

/// diff's help. uutils diffutils has none of its own for diff; these are
/// the options its parser takes.
const DIFF_USAGE: &str = "usage: diff [OPTION]... FILE1 FILE2

Formats:  --normal (the default)
          -u, -U NUM, --unified[=NUM]   unified, with NUM lines of context (3)
          -c, -C NUM, --context[=NUM]   context, with NUM lines of context (3)
          -e, --ed                      an ed script
          -y, --side-by-side            two columns
Output:   -q, --brief                   only say whether the files differ
          -s, --report-identical-files  say so when they do not
          -t, --expand-tabs             tabs as spaces
          --tabsize=NUM                 a tab is NUM columns wide (8)

FILE1 or FILE2 may be - for standard input.
Exit status: 0 the files are the same, 1 they differ, 2 trouble.";

/// Whether `option` is among the arguments, before any `--`.
fn asks_for(args: &[OsString], option: &str) -> bool {
    args.iter().skip(1).take_while(|arg| *arg != "--").any(|arg| arg == option)
}

pub(crate) fn diff(args: Vec<OsString>) -> i32 {
    let mut out = io::stdout().lock();
    if asks_for(&args, "--help") {
        let _ = writeln!(out, "{DIFF_USAGE}");
        return SAME;
    }
    if asks_for(&args, "--version") {
        let _ = writeln!(out, "diff (uutils diffutils, in ut {})", env!("CARGO_PKG_VERSION"));
        return SAME;
    }
    let params = match parse_params(args.into_iter().peekable()) {
        Ok(params) => params,
        Err(error) => {
            let _ = writeln!(io::stderr(), "{error}");
            return TROUBLE;
        }
    };
    let (from_name, to_name) = (params.from.to_string_lossy().into_owned(), params.to.to_string_lossy().into_owned());
    let identical = |out: &mut io::StdoutLock<'_>| {
        if params.report_identical_files {
            let _ = writeln!(out, "Files {from_name} and {to_name} are identical");
        }
    };
    if one_file(&params.from, &params.to) {
        identical(&mut out);
        return SAME;
    }
    let mut contents = Vec::new();
    for file in [&params.from, &params.to] {
        match read(file) {
            Ok(content) => contents.push(content),
            Err(error) => report_failure_to_read_input_file(&params.executable, file, &error),
        }
    }
    let [from, to] = contents.as_slice() else { return TROUBLE };
    let result = match params.format {
        Format::Normal => normal_diff(from, to, &params),
        Format::Unified => unified_diff(from, to, &params),
        Format::Context => context_diff(from, to, &params),
        Format::SideBySide => side_by_side_diff(from, to, &mut out, &params),
        Format::Ed => match ed_diff(from, to, &params) {
            Ok(result) => result,
            Err(error) => {
                let _ = writeln!(io::stderr(), "{error}");
                return TROUBLE;
            }
        },
    };
    if result.is_empty() {
        identical(&mut out);
        return SAME;
    }
    // A reader that stopped reading does not change the answer.
    let _ = if params.brief { writeln!(out, "Files {from_name} and {to_name} differ") } else { out.write_all(&result) };
    DIFFERENT
}

/// Whether cmp's arguments ask for silence (`-s`, `--quiet`, `--silent`,
/// each as a word of its own: the library's parser takes no other form).
/// The library parses them too, but keeps the answer to itself.
fn cmp_is_quiet(args: &[OsString]) -> bool {
    ["-s", "--quiet", "--silent"].iter().any(|option| asks_for(args, option))
}

/// Whether cmp is asked to compare standard input with itself, which is
/// equal without reading (read twice, the second read would be empty).
fn cmp_of_stdin_twice(args: &[OsString]) -> bool {
    let files: Vec<&OsString> =
        args.iter().skip(1).filter(|arg| *arg == "-" || !arg.to_string_lossy().starts_with('-')).collect();
    files.len() == 2 && files.iter().all(|file| *file == "-")
}

pub(crate) fn cmp(args: Vec<OsString>) -> i32 {
    let (quiet, stdin_twice) = (cmp_is_quiet(&args), cmp_of_stdin_twice(&args));
    let params = match compare::parse_params(args.into_iter().peekable()) {
        Ok(params) => params,
        Err(error) => {
            let _ = writeln!(io::stderr(), "{error}");
            return TROUBLE;
        }
    };
    if stdin_twice {
        return SAME;
    }
    match compare::cmp(&params) {
        Ok(compare::Cmp::Equal) => SAME,
        Ok(compare::Cmp::Different) => DIFFERENT,
        Err(error) => {
            if !quiet {
                let _ = writeln!(io::stderr(), "{error}");
            }
            TROUBLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn cmp_knows_when_it_is_asked_for_silence() {
        assert!(cmp_is_quiet(&args(&["cmp", "-s", "a", "b"])));
        assert!(cmp_is_quiet(&args(&["cmp", "--quiet", "a", "b"])));
        assert!(cmp_is_quiet(&args(&["cmp", "--silent", "a", "b"])));
        assert!(!cmp_is_quiet(&args(&["cmp", "-l", "a", "b"])));
        assert!(!cmp_is_quiet(&args(&["cmp", "a", "--", "-s"])), "after --, -s is a file's name");
        assert!(!cmp_is_quiet(&args(&["cmp", "settings", "b"])), "an s in a file's name is not the option");
    }

    #[test]
    fn diff_answers_questions_about_itself() {
        assert_eq!(diff(args(&["diff", "--help"])), SAME);
        assert_eq!(diff(args(&["diff", "--version"])), SAME);
        assert!(asks_for(&args(&["diff", "-u", "--help"]), "--help"));
        assert!(!asks_for(&args(&["diff", "--", "--help", "b"]), "--help"), "after --, --help is a file's name");
        assert!(!asks_for(&args(&["--help"]), "--help"), "the first word is the command's own name");
    }

    #[test]
    fn standard_input_twice_is_equal() {
        assert!(cmp_of_stdin_twice(&args(&["cmp", "-", "-"])));
        assert!(cmp_of_stdin_twice(&args(&["cmp", "-s", "-", "-"])));
        assert!(!cmp_of_stdin_twice(&args(&["cmp", "-", "b"])));
        assert!(!cmp_of_stdin_twice(&args(&["cmp", "a", "b"])));
    }

    #[test]
    fn files_are_compared() {
        let dir = std::env::temp_dir().join(format!("ut-diff-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, text: &str| {
            std::fs::write(dir.join(name), text).unwrap();
            dir.join(name).into_os_string()
        };
        let (a, same, other) = (file("a", "one\ntwo\n"), file("same", "one\ntwo\n"), file("other", "one\nTWO\n"));
        assert_eq!(cmp(vec!["cmp".into(), "-s".into(), a.clone(), same.clone()]), SAME);
        assert_eq!(cmp(vec!["cmp".into(), "-s".into(), a.clone(), other]), DIFFERENT);
        assert_eq!(cmp(vec!["cmp".into(), "-s".into(), a.clone(), dir.join("missing").into()]), TROUBLE);
        assert_eq!(diff(vec!["diff".into(), "-q".into(), a.clone(), same]), SAME);
        assert_eq!(diff(vec!["diff".into(), "-q".into(), a.clone(), a]), SAME, "one file with itself");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
