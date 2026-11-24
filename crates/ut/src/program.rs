//! `ut program`: the command lists UnifyTerm keeps: what programs say
//! about their own commands (OpenCLI descriptions), one file a program.
//!
//! Nothing here runs a program: a description is read from a file or from
//! standard input, checked, and kept in UnifyTerm's data directory, where
//! the window finds it.

use std::ffi::OsString;
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use unifyterm_programs::{MOST_BYTES, Program, Read, bare, directory_in, load, read, show};

const USAGE: &str = "usage: ut program                      the programs whose commands UnifyTerm knows
       ut program add FILE|-           check a description and keep it (- reads standard input)
       ut program check FILE|-         check it only: for who writes one
       ut program show NAME [WORD...]  a program's commands, or one command: its options, arguments, commands
       ut program remove NAME          forget a program's description
       ut program directory            print the folder the descriptions are kept in

A description is a file in the OpenCLI format (https://opencli.org): the
program's commands, options and arguments. With it UnifyTerm offers the
program's commands as you type, puts a mistyped one right, and explains a
command line without asking a model. UnifyTerm runs no program to get one:
the program writes it (PROGRAM ... | ut program add -).";

const USAGE_ERROR: i32 = 2;
const TROUBLE: i32 = 1;

/// The text of a description: a file's, or standard input's for `-`. Not more than a description may have.
fn text_of(source: &str, input: &mut dyn io::Read) -> Result<String> {
    let most = u64::try_from(MOST_BYTES).unwrap_or(u64::MAX) + 1;
    let mut text = String::new();
    if source == "-" {
        input.take(most).read_to_string(&mut text).context("standard input")?;
    } else {
        let file = std::fs::File::open(source).with_context(|| source.to_owned())?;
        file.take(most).read_to_string(&mut text).with_context(|| source.to_owned())?;
    }
    Ok(text)
}

/// One line about a description: `keepane 0.89.0: 182 commands, 431 options`.
fn line_about(program: &Program) -> String {
    let (commands, options) = program.counts();
    let version = if program.version.is_empty() { String::new() } else { format!(" {}", program.version) };
    format!("{}{version}: {commands} commands, {options} options", program.name)
}

fn left_out(read: &Read, out: &mut dyn Write) -> io::Result<()> {
    if read.left_out > 0 {
        writeln!(
            out,
            "{} entries were left out: a name with a blank or a character a shell reads, or a command more than 8 deep",
            read.left_out
        )?;
    }
    Ok(())
}

/// The files in `directory` that describe the program `key`.
fn files_of(directory: &Path, key: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| file.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")))
        .filter(|file| {
            std::fs::read_to_string(file)
                .ok()
                .and_then(|text| read(&text).ok())
                .is_some_and(|read| read.program.key == key)
        })
        .collect()
}

fn add(source: &str, directory: &Path, input: &mut dyn io::Read, out: &mut dyn Write) -> Result<i32> {
    let text = text_of(source, input)?;
    let read = read(&text).with_context(|| source.to_owned())?;
    std::fs::create_dir_all(directory).with_context(|| directory.display().to_string())?;
    let file = directory.join(format!("{}.json", read.program.key));
    // Written whole or not at all: the window may be reading the folder now.
    let unfinished = directory.join(format!("{}.json.new", read.program.key));
    std::fs::write(&unfinished, &text).with_context(|| unfinished.display().to_string())?;
    // One file a program: a file under another name that describes it would be read in its place.
    for other in files_of(directory, &read.program.key).into_iter().filter(|other| *other != file) {
        std::fs::remove_file(&other).with_context(|| other.display().to_string())?;
    }
    std::fs::rename(&unfinished, &file).with_context(|| file.display().to_string())?;
    writeln!(out, "added {}", line_about(&read.program))?;
    left_out(&read, out)?;
    Ok(0)
}

fn list(directory: &Path, out: &mut dyn Write) -> Result<i32> {
    let (programs, refused) = load(directory);
    for line in &refused {
        let _ = writeln!(io::stderr(), "ut program: not read: {line}");
    }
    if programs.is_empty() {
        writeln!(out, "no program's commands are known yet: 'ut program add FILE' adds a description")?;
    }
    for program in &programs {
        writeln!(out, "{}", line_about(program))?;
    }
    Ok(if refused.is_empty() { 0 } else { TROUBLE })
}

/// Runs `ut program` with the descriptions in the data directory `data`.
fn run(args: &[String], data: &Path, input: &mut dyn io::Read, out: &mut dyn Write) -> Result<i32> {
    let directory = directory_in(data);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["list"] => list(&directory, out),
        ["--help" | "-h"] => {
            writeln!(out, "{USAGE}")?;
            Ok(0)
        }
        ["add", source] => add(source, &directory, input, out),
        ["check", source] => {
            let read = read(&text_of(source, input)?).with_context(|| (*source).to_owned())?;
            writeln!(out, "ok: {}", line_about(&read.program))?;
            left_out(&read, out)?;
            Ok(0)
        }
        ["show", name, path @ ..] => {
            let (programs, _) = load(&directory);
            let key = bare(name);
            let Some(program) = programs.iter().find(|program| program.key == key) else {
                bail!("no description of '{name}': 'ut program' lists the programs there is one of");
            };
            let path: Vec<String> = path.iter().map(|&word| word.to_owned()).collect();
            let Some(text) = show(program, &path) else {
                bail!("{} has no command '{}'", program.name, path.join(" "));
            };
            write!(out, "{text}")?;
            Ok(0)
        }
        ["remove", name] => {
            let files = files_of(&directory, &bare(name));
            if files.is_empty() {
                bail!("no description of '{name}'");
            }
            for file in files {
                std::fs::remove_file(&file).with_context(|| file.display().to_string())?;
            }
            writeln!(out, "removed {name}")?;
            Ok(0)
        }
        ["directory"] => {
            writeln!(out, "{}", directory.display().to_string().replace('\\', "/"))?;
            Ok(0)
        }
        _ => {
            let _ = writeln!(io::stderr(), "{USAGE}");
            Ok(USAGE_ERROR)
        }
    }
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let outcome = unifyterm_history::data_directory()
        .and_then(|data| run(&args, &data, &mut io::stdin().lock(), &mut io::stdout().lock()));
    match outcome {
        Ok(code) => code,
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut program: {error:#}");
            TROUBLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANES: &str = r#"{"opencli": "0.1", "info": {"title": "Panes", "version": "1.2.0"},
        "command": {"name": "panes", "commands": [
            {"name": "split-window", "description": "Split a pane.", "options": [{"name": "-h", "description": "Side by side."}]},
            {"name": "bad name"}]}}"#;

    /// Runs `ut program ARGS` on the data directory `data`, with `input` on standard input.
    fn ran(args: &[&str], data: &Path, input: &str) -> (Result<i32>, String) {
        let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
        let mut out = Vec::new();
        let code = run(&args, data, &mut input.as_bytes(), &mut out);
        (code, String::from_utf8(out).expect("text"))
    }

    #[test]
    fn a_description_is_added_listed_shown_and_removed() {
        let data = tempfile::tempdir().expect("a folder");
        let (code, out) = ran(&[], data.path(), "");
        assert_eq!(code.expect("runs"), 0);
        assert!(out.starts_with("no program's commands are known yet"), "{out}");

        let (code, out) = ran(&["add", "-"], data.path(), PANES);
        assert_eq!(code.expect("runs"), 0);
        assert_eq!(
            out,
            "added panes 1.2.0: 1 commands, 1 options\n\
             1 entries were left out: a name with a blank or a character a shell reads, or a command more than 8 deep\n"
        );
        let kept = data.path().join("programs").join("panes.json");
        assert_eq!(std::fs::read_to_string(&kept).expect("kept"), PANES, "kept as it was given");

        assert_eq!(ran(&[], data.path(), "").1, "panes 1.2.0: 1 commands, 1 options\n");
        assert_eq!(ran(&["list"], data.path(), "").1, "panes 1.2.0: 1 commands, 1 options\n");
        let (code, out) = ran(&["show", "Panes.exe", "split-window"], data.path(), "");
        assert_eq!(code.expect("runs"), 0);
        assert_eq!(out, "panes split-window: Split a pane.\noptions:\n  -h: Side by side.\n");
        assert!(
            ran(&["show", "panes"], data.path(), "")
                .1
                .starts_with("panes 1.2.0\ncommands:\n  split-window: Split a pane\n")
        );

        let (code, out) = ran(&["remove", "panes"], data.path(), "");
        assert_eq!((code.expect("runs"), out.as_str()), (0, "removed panes\n"));
        assert!(!kept.exists());
        assert!(ran(&["remove", "panes"], data.path(), "").0.is_err(), "nothing to remove now");
    }

    #[test]
    fn a_newer_description_takes_the_place_of_the_older_whatever_its_file_is_called() {
        let data = tempfile::tempdir().expect("a folder");
        let folder = data.path().join("programs");
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(folder.join("a-copy.json"), PANES).expect("write");
        let newer = PANES.replace("1.2.0", "1.3.0");
        let file = data.path().join("given.json");
        std::fs::write(&file, &newer).expect("write");
        let (code, _) = ran(&["add", &file.display().to_string()], data.path(), "");
        assert_eq!(code.expect("runs"), 0);
        assert!(!folder.join("a-copy.json").exists(), "the other file that described it is gone");
        assert!(!folder.join("panes.json.new").exists(), "nothing unfinished is left");
        assert_eq!(ran(&[], data.path(), "").1, "panes 1.3.0: 1 commands, 1 options\n");
    }

    #[test]
    fn what_is_no_description_is_not_kept_and_the_reason_is_said() {
        let data = tempfile::tempdir().expect("a folder");
        let (code, out) = ran(&["add", "-"], data.path(), "{\"name\": 1}");
        let said = format!("{:#}", code.expect_err("not a description"));
        assert!(said.contains("not an OpenCLI description") && out.is_empty(), "{said}");
        assert!(!data.path().join("programs").exists(), "nothing was made");
        let (code, out) = ran(&["check", "-"], data.path(), PANES);
        assert_eq!(code.expect("runs"), 0);
        assert!(out.starts_with("ok: panes 1.2.0: 1 commands, 1 options\n"), "{out}");
        assert!(!data.path().join("programs").exists(), "check keeps nothing");
        assert!(ran(&["add", "no-such-file.json"], data.path(), "").0.is_err());
        assert!(ran(&["show", "panes"], data.path(), "").0.is_err(), "no description of it");
        let (code, _) = ran(&["add", "-"], data.path(), PANES);
        assert_eq!(code.expect("runs"), 0);
        let said = format!("{:#}", ran(&["show", "panes", "nosuch"], data.path(), "").0.expect_err("no such command"));
        assert_eq!(said, "panes has no command 'nosuch'");
    }

    #[test]
    fn the_folder_is_told_and_a_wrong_call_is_a_usage_error() {
        let data = tempfile::tempdir().expect("a folder");
        let (code, out) = ran(&["directory"], data.path(), "");
        assert_eq!(code.expect("runs"), 0);
        assert!(out.trim_end().ends_with("/programs") && !out.contains('\\'), "{out}");
        assert_eq!(ran(&["add"], data.path(), "").0.expect("runs"), USAGE_ERROR);
        assert_eq!(ran(&["frobnicate"], data.path(), "").0.expect("runs"), USAGE_ERROR);
        let (code, out) = ran(&["--help"], data.path(), "");
        assert_eq!(code.expect("runs"), 0);
        assert!(out.starts_with("usage: ut program"));
    }
}
