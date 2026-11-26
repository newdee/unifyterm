//! `ut prompt`: the prompt UnifyTerm writes for a shell, when the settings
//! say `prompt.enabled = true`. A shell's hook runs it at every prompt and
//! prints what it prints; `ut prompt preview` shows a person how it looks.
//!
//! How it looks is `[prompt]` in the settings and the theme ([`Prompt`]);
//! the colors are the theme's ([`Palette`]); what it says is [`facts::Facts`].

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use unifyterm_config::prompt::{Palette, Prompt, Segment};
use unifyterm_config::{Config, themes};

use crate::USAGE_ERROR;

mod escape;
mod facts;
mod render;

use facts::{Clock, Facts, Shell};

const USAGE: &str = "usage: ut prompt --shell pwsh|bash|zsh|fish [--status N] [--took MS] [--width N] [--cwd DIR]
       ut prompt preview

The first form is for the shells' hooks: UnifyTerm's tabs run it at every
prompt when the settings say prompt.enabled = true, and print what it
prints. --status is the last command's exit code, --took how long it ran in
milliseconds, --width the terminal's width. preview shows how the prompt
looks with the settings and the theme in use. --config FILE names the
settings file, as for ut setting.";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// The prompt for a shell's hook.
    Write { shell: Shell, status: Option<i32>, took: Option<Duration>, width: Option<usize>, cwd: Option<PathBuf> },
    /// A person's look at it.
    Preview,
}

/// The value after an option.
fn value(args: &mut std::slice::Iter<'_, OsString>, option: &str) -> Result<String> {
    args.next().and_then(|value| value.to_str()).map(str::to_owned).ok_or_else(|| anyhow!("{option} needs a value"))
}

fn parse(args: &[OsString]) -> Result<(Option<PathBuf>, Ask)> {
    let mut config = None;
    let (mut shell, mut status, mut took, mut width, mut cwd) = (None, None, None, None, None);
    let mut preview = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("preview") => preview = true,
            Some("--config") => config = Some(PathBuf::from(value(&mut args, "--config")?)),
            Some("--shell") => {
                let name = value(&mut args, "--shell")?;
                shell = Some(Shell::named(&name).ok_or_else(|| anyhow!("no prompt for the shell {name}"))?);
            }
            Some("--status") => status = Some(value(&mut args, "--status")?.parse().context("--status is a number")?),
            Some("--took") => {
                let milliseconds: u64 = value(&mut args, "--took")?.parse().context("--took is milliseconds")?;
                took = Some(Duration::from_millis(milliseconds));
            }
            Some("--width") => width = Some(value(&mut args, "--width")?.parse().context("--width is a number")?),
            Some("--cwd") => cwd = Some(PathBuf::from(value(&mut args, "--cwd")?)),
            _ => bail!("{} is not something ut prompt takes", arg.to_string_lossy()),
        }
    }
    if preview {
        return Ok((config, Ask::Preview));
    }
    let shell = shell.ok_or_else(|| anyhow!("--shell names the shell the prompt is for"))?;
    Ok((config, Ask::Write { shell, status, took, width, cwd }))
}

/// How the prompt looks and the colors it is in, as the settings file says.
fn look(asked: Option<PathBuf>) -> Result<(Prompt, Palette)> {
    let settings = crate::setting::file_path(asked)?;
    let look = Config::load(&settings)?.look(Some(&themes::directory(&settings)))?;
    Ok((look.prompt.clone(), Palette::of(&look.colors)))
}

/// The time of day now, on this machine's clock.
fn now() -> Clock {
    use chrono::Timelike as _;
    let time = chrono::Local::now();
    Clock(time.hour(), time.minute(), time.second())
}

/// The facts of this prompt: what the hook said, and what is read here.
fn gathered(
    prompt: &Prompt,
    shell: Shell,
    status: Option<i32>,
    took: Option<Duration>,
    width: Option<usize>,
    cwd: PathBuf,
) -> Facts {
    let shows = |segment: Segment| prompt.left.contains(&segment) || prompt.right.contains(&segment);
    let repo = shows(Segment::Git).then(|| facts::repo(&cwd)).flatten();
    let venv = shows(Segment::Venv)
        .then(|| {
            let read = |name: &str| std::env::var(name).ok();
            facts::venv(read("VIRTUAL_ENV").as_deref(), read("CONDA_DEFAULT_ENV").as_deref())
        })
        .flatten();
    Facts { shell, cwd, home: dirs::home_dir(), status, took, repo, venv, width, now: now() }
}

/// The prompt as three moments of a session: a clean repository, a failure
/// that took a while, a change in the tree.
fn preview(prompt: &Prompt, palette: &Palette, out: &mut impl Write) -> Result<()> {
    use facts::{Changes, Head, Repo};
    let width = crossterm::terminal::size().ok().map_or(80, |(columns, _)| usize::from(columns));
    let home = dirs::home_dir();
    let cwd = home.clone().unwrap_or_default().join("projects").join("app");
    let branch = |changes: Changes| Some(Repo { head: Head::Branch("main".into()), changes: Some(changes) });
    let moments = [
        (None, None, branch(Changes::default()), "git pull"),
        (Some(1), Some(Duration::from_secs(38)), branch(Changes::default()), "cargo test"),
        (Some(0), None, branch(Changes { staged: 1, changed: 2, untracked: 1, ahead: 1, behind: 0 }), ""),
    ];
    let mut shown = prompt.clone();
    shown.enabled = true;
    for (status, took, repo, command) in moments {
        let facts = Facts {
            shell: Shell::Pwsh,
            cwd: cwd.clone(),
            home: home.clone(),
            status,
            took,
            repo,
            venv: Some(".venv".into()),
            width: Some(width),
            now: now(),
        };
        let rendered = render::render(&shown, palette, &facts);
        writeln!(out, "{}{command}", escape::output(&rendered, Shell::Pwsh))?;
    }
    if !prompt.enabled {
        writeln!(out, "\nThe prompt is off: ut setting set prompt.enabled true, then open a new tab.")?;
    }
    Ok(())
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let (config, ask) = match parse(args) {
        Ok(parsed) => parsed,
        Err(problem) => {
            let _ = writeln!(io::stderr(), "ut prompt: {problem:#}\n{USAGE}");
            return USAGE_ERROR;
        }
    };
    let mut out = io::stdout().lock();
    match ask {
        Ask::Preview => match look(config).and_then(|(prompt, palette)| preview(&prompt, &palette, &mut out)) {
            Ok(()) => 0,
            Err(problem) => {
                let _ = writeln!(io::stderr(), "ut prompt: {problem:#}");
                1
            }
        },
        Ask::Write { shell, status, took, width, cwd } => {
            // A prompt there must be: with settings that cannot be read, a plain one, and why on the error stream.
            let (prompt, palette) = match look(config) {
                Ok(look) => look,
                Err(problem) => {
                    let _ = writeln!(io::stderr(), "ut prompt: {problem:#}");
                    let _ = write!(out, "> ");
                    return 1;
                }
            };
            let cwd = cwd.or_else(|| std::env::current_dir().ok()).unwrap_or_default();
            let facts = gathered(&prompt, shell, status, took, width, cwd);
            let rendered = render::render(&prompt, &palette, &facts);
            let _ = write!(out, "{}", escape::output(&rendered, shell));
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_hooks_command_line_is_read() {
        let (config, ask) =
            parse(&args(&["--shell", "zsh", "--status", "-1", "--took", "2500", "--width", "120", "--cwd", "/tmp"]))
                .expect("fine");
        assert_eq!(config, None);
        assert_eq!(
            ask,
            Ask::Write {
                shell: Shell::Zsh,
                status: Some(-1),
                took: Some(Duration::from_millis(2500)),
                width: Some(120),
                cwd: Some(PathBuf::from("/tmp")),
            }
        );
        assert_eq!(
            parse(&args(&["preview", "--config", "c.toml"])).expect("fine"),
            (Some("c.toml".into()), Ask::Preview)
        );
        let said = |words: &[&str]| format!("{:#}", parse(&args(words)).expect_err("refused"));
        assert!(said(&[]).contains("--shell names the shell"));
        assert!(said(&["--shell", "cmd"]).contains("no prompt for the shell cmd"));
        assert!(said(&["--shell", "bash", "--status"]).contains("--status needs a value"));
        assert!(said(&["--shell", "bash", "--took", "soon"]).contains("--took is milliseconds"));
        assert!(said(&["--shell", "bash", "--colour"]).contains("--colour is not something"));
    }

    #[test]
    fn the_settings_file_says_how_it_looks() {
        let folder = tempfile::tempdir().expect("a folder");
        let settings = folder.path().join("config.toml");
        std::fs::write(&settings, "[theme]\nname = \"aurora\"\n[prompt]\nenabled = true\nlines = 1\n").expect("write");
        let (prompt, palette) = look(Some(settings.clone())).expect("a look");
        assert!(prompt.enabled);
        assert_eq!(prompt.style, unifyterm_config::prompt::Style::Rainbow, "aurora's own");
        assert_eq!(prompt.lines, unifyterm_config::prompt::Lines::One, "the settings'");
        assert_eq!(palette.background, [0x0c, 0x1c, 0x24], "aurora's colors");
        std::fs::write(&settings, "[prompt]\nlines = 9\n").expect("write");
        assert!(look(Some(settings)).is_err(), "settings that cannot be read are an error to say");
    }

    #[test]
    fn a_preview_shows_three_moments_and_says_when_the_prompt_is_off() {
        let prompt = unifyterm_config::PromptConfig::default().resolved();
        let palette = Palette::of(&unifyterm_config::ThemeConfig::default());
        let mut out = Vec::new();
        preview(&prompt, &palette, &mut out).expect("written");
        let text = String::from_utf8(out).expect("UTF-8");
        assert_eq!(text.matches('❯').count(), 3);
        assert!(text.contains("projects/app") && text.contains("main") && text.contains("38s") && text.contains("✘ 1"));
        assert!(text.contains("The prompt is off"));
    }
}
