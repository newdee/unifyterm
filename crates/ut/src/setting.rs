//! `ut setting`: UnifyTerm's settings, shown and changed.
//!
//! With no more words, in a terminal, it is a list to move in with the
//! arrow keys ([`screen`]). With words it does one thing and ends, for a
//! script or an agent: `list`, `get`, `set`, `reset`, `path`. Both change
//! the file through `unifyterm-config`, with the rules the terminal reads
//! it by: a value the terminal would refuse is never written.

use std::ffi::OsString;
use std::io::{self, IsTerminal as _, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow, bail};
use unifyterm_config::{Config, FILE_VARIABLE, File, KEYS, Key, Kind, default_of, key};

use crate::USAGE_ERROR;

mod choices;
mod screen;

/// The exit code when something went wrong that is not the command line's.
const TROUBLE: i32 = 1;

const USAGE: &str = "usage: ut setting [--config FILE]                 the settings, to change with the keyboard
       ut setting list                            every setting and its value
       ut setting get NAME                        one value
       ut setting set NAME VALUE                  change one (a list: words with commas between them)
       ut setting reset NAME                      back to the default
       ut setting path                            the settings file

NAME is as in the file: font.size, theme.background, agents.command.
The file is UnifyTerm's config.toml, or the one --config or UNIFYTERM_CONFIG names.
A value the terminal would refuse is not written. UnifyTerm takes a saved change at once.";

/// What was asked for.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    Screen,
    List,
    Get(String),
    Set(String, String),
    Reset(String),
    Path,
    Help,
}

/// The words after `ut setting`: the file named with `--config`, and what to do.
fn parse(args: &[OsString]) -> Result<(Option<PathBuf>, Ask), String> {
    let mut words: Vec<String> = Vec::new();
    let mut file = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--config") => file = Some(PathBuf::from(args.next().ok_or("--config needs a file")?)),
            Some("--help" | "-h") => return Ok((file, Ask::Help)),
            Some(word) => words.push(word.to_owned()),
            None => return Err(format!("{}: not text", arg.display())),
        }
    }
    let ask = match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => Ask::Screen,
        ["list"] => Ask::List,
        ["path"] => Ask::Path,
        ["get", name] => Ask::Get((*name).to_owned()),
        ["reset", name] => Ask::Reset((*name).to_owned()),
        // A value typed without quotes arrives as several words: they are one value.
        ["set", name, value @ ..] if !value.is_empty() => Ask::Set((*name).to_owned(), value.join(" ")),
        ["get" | "reset"] => return Err("it needs a setting's name".into()),
        ["set", ..] => return Err("set needs a name and a value".into()),
        [other, ..] => return Err(format!("unknown: {other}")),
    };
    Ok((file, ask))
}

/// The settings file: `--config`'s, else the one the terminal this shell is
/// in reads ([`FILE_VARIABLE`]), else the default one.
fn file_path(asked: Option<PathBuf>) -> Result<PathBuf> {
    asked
        .or_else(|| std::env::var_os(FILE_VARIABLE).filter(|file| !file.is_empty()).map(PathBuf::from))
        .or_else(Config::default_path)
        .ok_or_else(|| anyhow!("this system names no directory for settings: name the file with --config"))
}

fn named(name: &str) -> Result<&'static Key> {
    key(name).ok_or_else(|| {
        // A name close to the one typed, when there is one: most mistakes are a letter or the section.
        let near = KEYS.iter().find(|key| key.field() == name || key.name.eq_ignore_ascii_case(name));
        match near {
            Some(near) => anyhow!("no setting is called {name}: did you mean {}?", near.name),
            None => anyhow!("no setting is called {name}: 'ut setting list' names them"),
        }
    })
}

/// What a setting may be, in a few words.
pub(crate) fn takes(kind: Kind) -> String {
    match kind {
        Kind::Switch => "true or false".into(),
        Kind::Whole { least, most } => format!("a whole number, {least} to {most}"),
        Kind::Number { least, most } => format!("a number, {least} to {most}"),
        Kind::Text => "text".into(),
        Kind::OptionalText => "text; empty: not set".into(),
        Kind::Words => "words with commas between them".into(),
        Kind::OneOf(choices) => choices.join(", "),
        Kind::Color => "#rrggbb; empty: the built-in color".into(),
        Kind::Colors => "16 colors as #rrggbb with commas between them; empty: the built-in ones".into(),
    }
}

/// One line for each setting: its name, the value in use, and `(default)`
/// after one the file does not set.
fn list(file: &File, out: &mut impl Write) -> io::Result<()> {
    let width = KEYS.iter().map(|key| key.name.len()).max().unwrap_or(0);
    for key in &KEYS {
        match file.value(key) {
            Some(value) => writeln!(out, "{:width$}  {value}", key.name)?,
            None => match default_of(key) {
                default if default.is_empty() => writeln!(out, "{:width$}  (not set)", key.name)?,
                default => writeln!(out, "{:width$}  {default}  (default)", key.name)?,
            },
        }
    }
    Ok(())
}

fn run(asked: Option<PathBuf>, ask: Ask, out: &mut impl Write) -> Result<i32> {
    if ask == Ask::Help {
        writeln!(out, "{USAGE}")?;
        return Ok(0);
    }
    let path = file_path(asked)?;
    if ask == Ask::Path {
        writeln!(out, "{}", path.display())?;
        return Ok(0);
    }
    let mut file = File::open(&path)?;
    match ask {
        Ask::List => list(&file, out)?,
        Ask::Get(name) => writeln!(out, "{}", file.in_use(named(&name)?))?,
        Ask::Set(name, value) => {
            file.set(named(&name)?, &value)?;
            file.save()?;
        }
        Ask::Reset(name) => {
            file.reset(named(&name)?)?;
            file.save()?;
        }
        Ask::Screen => {
            // Keys are read from a terminal and the list is drawn on one: without both, the list is printed.
            if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
                list(&file, out)?;
                return Ok(0);
            }
            screen::run(file).context("the settings screen")?;
        }
        Ask::Path | Ask::Help => bail!("answered above"),
    }
    Ok(0)
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let (file, ask) = match parse(args) {
        Ok(parsed) => parsed,
        Err(problem) => {
            let _ = writeln!(io::stderr(), "ut setting: {problem}\n{USAGE}");
            return USAGE_ERROR;
        }
    };
    match run(file, ask, &mut io::stdout().lock()) {
        Ok(code) => code,
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut setting: {error:#}");
            TROUBLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn ran(file: &std::path::Path, ask: Ask) -> (Result<i32>, String) {
        let mut out = Vec::new();
        let result = run(Some(file.to_path_buf()), ask, &mut out);
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn the_words_say_what_to_do() {
        assert_eq!(parse(&words(&[])), Ok((None, Ask::Screen)));
        assert_eq!(parse(&words(&["list"])), Ok((None, Ask::List)));
        assert_eq!(parse(&words(&["get", "font.size"])), Ok((None, Ask::Get("font.size".into()))));
        assert_eq!(
            parse(&words(&["--config", "c.toml", "set", "font.family", "JetBrains", "Mono"])),
            Ok((Some("c.toml".into()), Ask::Set("font.family".into(), "JetBrains Mono".into()))),
            "a value typed without quotes is one value"
        );
        assert_eq!(parse(&words(&["reset", "font.size"])), Ok((None, Ask::Reset("font.size".into()))));
        assert_eq!(parse(&words(&["path"])), Ok((None, Ask::Path)));
        assert_eq!(parse(&words(&["set", "x", "--help"])).unwrap().1, Ask::Help);
        for wrong in [&["set", "font.size"][..], &["get"], &["frobnicate"], &["--config"], &["list", "more"]] {
            assert!(parse(&words(wrong)).is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn one_setting_is_read_changed_and_put_back() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("config.toml");
        assert_eq!(ran(&file, Ask::Get("font.size".into())).1, "14.0\n", "no file yet: the default");
        assert!(!file.exists(), "reading makes no file");
        assert_eq!(ran(&file, Ask::Set("font.size".into(), "18".into())).0.unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[font]\nsize = 18.0\n");
        assert_eq!(ran(&file, Ask::Get("font.size".into())).1, "18.0\n");
        let (_, listed) = ran(&file, Ask::List);
        assert_eq!(listed.lines().count(), KEYS.len(), "one line for each setting");
        assert!(listed.contains("font.size                18.0\n"), "{listed}");
        assert!(listed.contains("font.ligatures           true  (default)\n"), "{listed}");
        assert!(listed.contains("shell.program            (not set)\n"), "{listed}");
        assert_eq!(ran(&file, Ask::Reset("font.size".into())).0.unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "", "the emptied [font] is gone");
        assert_eq!(ran(&file, Ask::Path).1.trim_end(), file.display().to_string());
    }

    #[test]
    fn a_wrong_name_or_value_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("config.toml");
        std::fs::write(&file, "[font]\nsize = 15\n").unwrap();
        let said = |ask: Ask| format!("{:#}", ran(&file, ask).0.unwrap_err());
        assert!(said(Ask::Set("font.size".into(), "500".into())).contains("from 6 to 72"));
        assert!(said(Ask::Set("font.sise".into(), "12".into())).contains("no setting is called font.sise"));
        assert!(said(Ask::Get("size".into())).contains("did you mean font.size?"));
        assert!(said(Ask::Set("ai.provider".into(), "anthropic".into())).contains("ai.model is empty"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[font]\nsize = 15\n", "the file is as it was");
        std::fs::write(&file, "[font]\nsise = 15\n").unwrap();
        assert!(said(Ask::List).contains("sise"), "a file the terminal refuses is not changed blindly");
    }

    #[test]
    fn each_kind_says_what_it_takes() {
        for key in &KEYS {
            assert!(!takes(key.kind).is_empty() && !key.says.is_empty(), "{}", key.name);
        }
        assert_eq!(takes(Kind::Whole { least: 1, most: 20 }), "a whole number, 1 to 20");
    }
}
