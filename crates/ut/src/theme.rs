//! `ut theme`: the themes there are, and which one is in use.
//!
//! A theme is a set of colors under a name: the ones UnifyTerm brings, and
//! the user's own files in the `themes` folder beside the settings file.
//! Which one is in use is the setting `theme.name`: `ut theme NAME` writes
//! it, after it has read the theme, and a running UnifyTerm takes the change
//! at once. A theme that is not there or is wrong is never written.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use unifyterm_config::themes::{self, Source};
use unifyterm_config::{Config, File, ThemeConfig, key, parse_color};

use crate::USAGE_ERROR;

const USAGE: &str = "usage: ut theme                       the themes there are; * marks the one in use
       ut theme NAME                  use the theme NAME
       ut theme show NAME             print a theme's file
       ut theme new NAME [--from NAME]  begin a theme of your own, from the one in use or the one named
       ut theme check FILE            check a theme file only: for who writes one
       ut theme directory             print the folder your own themes are in

A theme is a file with a table, [theme], and the keys of the settings file's
[theme]: foreground, background and ansi (the 16 colors) it must give; it may
give cursor, selection, the colors around the text (bar, panel, well, rule,
edge, muted, symbol), opacity, blur, and picture (a picture's file beside the
theme's, shown at the foot of the sidebar). A key this version does not know
is passed over, and check names it. Your own themes are NAME.toml in the
themes folder beside the settings file; one with a built-in theme's name is
used in its place. --config FILE names the settings file, as for ut setting.";

const TROUBLE: i32 = 1;
/// The setting that says which theme is in use.
const SETTING: &str = "theme.name";
/// Under this, text is hard to read against what is behind it (WCAG's ratio for large text).
const READABLE: f64 = 4.5;
/// The words `ut theme` takes for itself: a theme of the user's own is not called one of them.
const TAKEN: [&str; 6] = ["show", "new", "check", "directory", "list", "use"];
/// Which of the 16 colors a theme's line shows after its text sample: red, green, yellow, blue, magenta, cyan.
const SAMPLED: std::ops::Range<usize> = 1..7;

/// Whether a theme's colors are shown beside its name: in a terminal they are, in a pipe they are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Samples {
    Shown,
    Left,
}

/// A color as the two escape-sequence numbers lists want: `38;2;r;g;b` for text, `48;2;...` behind it.
fn in_color(kind: u8, color: &str) -> Option<String> {
    let [red, green, blue] = parse_color(color)?;
    Some(format!("{kind};2;{red};{green};{blue}"))
}

/// A theme's colors as a short strip: its text on its background, then six of its 16 colors.
fn sample(theme: &ThemeConfig) -> Option<String> {
    let (text, behind) = (theme.foreground.as_deref()?, theme.background.as_deref()?);
    let mut strip = format!("\x1b[{};{}m Aa ", in_color(38, text)?, in_color(48, behind)?);
    for color in theme.ansi.as_ref()?.get(SAMPLED)? {
        let _ = write!(strip, "\x1b[{}m█", in_color(38, color)?);
    }
    strip.push_str(" \x1b[0m");
    Some(strip)
}

/// How well the theme's text reads against its background; a line of warning when it does not.
fn hard_to_read(theme: &ThemeConfig) -> Option<String> {
    let ratio = themes::contrast(theme.foreground.as_deref()?, theme.background.as_deref()?)?;
    (ratio < READABLE).then(|| {
        format!(
            "the text is hard to read on this background: their contrast is {ratio:.1} to 1, and {READABLE} is little"
        )
    })
}

/// What is worth a word about a theme file's text that was read: the keys
/// passed over, and a picture it names that is not there. `beside` is the
/// folder the theme's file is in.
fn notes(text: &str, colors: &ThemeConfig, beside: Option<&Path>) -> Vec<String> {
    let mut notes = Vec::new();
    let passed = themes::passed_over(text);
    if !passed.is_empty() {
        notes.push(format!(
            "passed over, not known to this version: {} (misspelt, or a later UnifyTerm's)",
            passed.join(", ")
        ));
    }
    let picture = colors.picture.as_deref().and_then(|named| themes::picture(named, beside));
    if let Some(themes::Picture::File(file)) = picture
        && !file.is_file()
    {
        notes.push(format!("its picture is not there: {}", file.display().to_string().replace('\\', "/")));
    }
    notes
}

/// The theme the settings name; the default one when they name none.
fn in_use(settings: &Path) -> Result<String> {
    Ok(Config::load(settings)?.theme.name.unwrap_or_else(|| themes::DEFAULT.to_owned()))
}

fn list(settings: &Path, samples: Samples, out: &mut dyn Write) -> Result<i32> {
    let folder = themes::directory(settings);
    let current = in_use(settings)?;
    let listed = themes::list(Some(&folder));
    let widest = listed.iter().map(|theme| theme.name.len()).max().unwrap_or(0);
    let from_of = |theme: &themes::Listed| match &theme.source {
        Source::BuiltIn => "built in",
        Source::File(_) if themes::BUILT_IN.iter().any(|(name, _)| *name == theme.name) => "yours, for the built-in",
        Source::File(_) => "yours",
    };
    let widest_from = listed.iter().map(|theme| from_of(theme).len()).max().unwrap_or(0);
    let mut wrong = 0;
    for theme in &listed {
        let mark = if theme.name == current { '*' } else { ' ' };
        let from = from_of(theme);
        let colors = match themes::find(&theme.name, Some(&folder)) {
            Ok(colors) if samples == Samples::Shown => sample(&colors).unwrap_or_default(),
            Ok(_) => String::new(),
            Err(error) => {
                wrong += 1;
                format!("cannot be used: {error:#}")
            }
        };
        writeln!(out, "{}", format!("{mark} {:widest$}  {from:<widest_from$}  {colors}", theme.name).trim_end())?;
    }
    if !listed.iter().any(|theme| theme.name == current) {
        writeln!(out, "the settings name the theme {current}, which is not there: the colors in use stay")?;
        wrong += 1;
    }
    Ok(if wrong == 0 { 0 } else { TROUBLE })
}

/// Writes `theme.name` after the theme was read: one that is not there or is wrong changes nothing.
fn choose(name: &str, settings: &Path, out: &mut dyn Write) -> Result<i32> {
    let folder = themes::directory(settings);
    let (text, source) = themes::text(name, Some(&folder))?;
    let colors = themes::find(name, Some(&folder))?;
    let beside = match &source {
        Source::File(file) => file.parent(),
        Source::BuiltIn => None,
    };
    let said = notes(&text, &colors, beside);
    let mut file = File::open(settings)?;
    let Some(setting) = key(SETTING) else { bail!("{SETTING} is no setting") };
    file.set(setting, name)?;
    if file.changed() {
        file.save()?;
    }
    writeln!(out, "theme: {name}")?;
    if let Some(warning) = hard_to_read(&colors) {
        writeln!(out, "{warning}")?;
    }
    for note in said {
        writeln!(out, "{note}")?;
    }
    // Colors the settings give themselves stay over any theme: said, so that a theme that seems not to take is explained.
    let own = Config::load(settings)?.theme;
    let kept: Vec<&str> = [
        ("foreground", own.foreground.is_some()),
        ("background", own.background.is_some()),
        ("cursor", own.cursor.is_some()),
        ("selection", own.selection.is_some()),
        ("bar", own.bar.is_some()),
        ("panel", own.panel.is_some()),
        ("well", own.well.is_some()),
        ("rule", own.rule.is_some()),
        ("edge", own.edge.is_some()),
        ("muted", own.muted.is_some()),
        ("symbol", own.symbol.is_some()),
        ("ansi", own.ansi.is_some()),
        ("opacity", own.opacity.is_some()),
        ("blur", own.blur.is_some()),
        ("picture", own.picture.is_some()),
    ]
    .iter()
    .filter(|(_, given)| *given)
    .map(|(name, _)| *name)
    .collect();
    if !kept.is_empty() {
        writeln!(
            out,
            "the settings give {} themselves: these stay over the theme's ('ut setting reset theme.{}' takes one away)",
            kept.join(", "),
            kept[0]
        )?;
    }
    Ok(0)
}

/// Begins a theme of the user's own: a copy of `from`'s file (the theme in use when `None`), under `name`.
fn begin(name: &str, from: Option<&str>, settings: &Path, out: &mut dyn Write) -> Result<i32> {
    if !themes::is_name(name) {
        bail!("\"{name}\" is not a theme's name: small letters, digits and -");
    }
    if TAKEN.contains(&name) {
        bail!("{name} is a word of ut theme's own: another name");
    }
    let folder = themes::directory(settings);
    let file = themes::file_of(name, &folder);
    if file.exists() {
        bail!("{} is there already", file.display());
    }
    let from = match from {
        Some(from) => from.to_owned(),
        None => in_use(settings)?,
    };
    let (text, _) = themes::text(&from, Some(&folder))?;
    themes::read(&text).with_context(|| format!("the theme {from} cannot be copied"))?;
    // The copy says what it is; the comments at the top of the one it came from were about that one.
    let colors = text.lines().skip_while(|line| line.trim().is_empty() || line.trim_start().starts_with('#'));
    let mut copy = format!("# {name}: a theme of your own, begun from {from}. 'ut theme {name}' uses it.\n");
    for line in colors {
        copy.push_str(line);
        copy.push('\n');
    }
    std::fs::create_dir_all(&folder).with_context(|| folder.display().to_string())?;
    std::fs::write(&file, copy).with_context(|| file.display().to_string())?;
    writeln!(out, "{}", file.display().to_string().replace('\\', "/"))?;
    Ok(0)
}

fn check(file: &str, out: &mut dyn Write) -> Result<i32> {
    let text = std::fs::read_to_string(file).with_context(|| file.to_owned())?;
    let colors = themes::read(&text).with_context(|| file.to_owned())?;
    writeln!(out, "ok: {file}")?;
    if let Some(warning) = hard_to_read(&colors) {
        writeln!(out, "{warning}")?;
    }
    for note in notes(&text, &colors, Path::new(file).parent()) {
        writeln!(out, "{note}")?;
    }
    let stem = Path::new(file).file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
    if !themes::is_name(stem) {
        writeln!(out, "its name, {stem}, is not one a theme may have: small letters, digits and -")?;
    }
    Ok(0)
}

/// Runs `ut theme` on the settings file `settings`.
fn run(words: &[&str], settings: &Path, samples: Samples, out: &mut dyn Write) -> Result<i32> {
    match words {
        [] | ["list"] => list(settings, samples, out),
        ["show", name] => {
            let (text, _) = themes::text(name, Some(&themes::directory(settings)))?;
            write!(out, "{text}")?;
            Ok(0)
        }
        ["new", name] => begin(name, None, settings, out),
        ["new", name, "--from", from] | ["new", "--from", from, name] => begin(name, Some(from), settings, out),
        ["check", file] => check(file, out),
        ["directory"] => {
            writeln!(out, "{}", themes::directory(settings).display().to_string().replace('\\', "/"))?;
            Ok(0)
        }
        ["use", name] => choose(name, settings, out),
        [name] if !TAKEN.contains(name) => choose(name, settings, out),
        _ => {
            let _ = writeln!(io::stderr(), "{USAGE}");
            Ok(USAGE_ERROR)
        }
    }
}

pub(crate) fn main(args: &[OsString]) -> i32 {
    let mut words: Vec<String> = Vec::new();
    let mut named: Option<PathBuf> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--config") => {
                let Some(file) = args.next() else {
                    let _ = writeln!(io::stderr(), "ut theme: --config needs a file\n{USAGE}");
                    return USAGE_ERROR;
                };
                named = Some(PathBuf::from(file));
            }
            Some("--help" | "-h") => {
                let _ = writeln!(io::stdout(), "{USAGE}");
                return 0;
            }
            Some(word) => words.push(word.to_owned()),
            None => {
                let _ = writeln!(io::stderr(), "ut theme: {}: not text", arg.display());
                return USAGE_ERROR;
            }
        }
    }
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let samples = if io::stdout().is_terminal() { Samples::Shown } else { Samples::Left };
    let outcome =
        crate::setting::file_path(named).and_then(|settings| run(&words, &settings, samples, &mut io::stdout().lock()));
    match outcome {
        Ok(code) => code,
        Err(error) if error.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut theme: {error:#}");
            TROUBLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file in a folder of its own, with `text` in it (none when empty).
    fn settings(text: &str) -> (tempfile::TempDir, PathBuf) {
        let folder = tempfile::tempdir().expect("a folder");
        let file = folder.path().join("config.toml");
        if !text.is_empty() {
            std::fs::write(&file, text).expect("write");
        }
        (folder, file)
    }

    fn ran(words: &[&str], settings: &Path) -> (Result<i32>, String) {
        let mut out = Vec::new();
        let code = run(words, settings, Samples::Left, &mut out);
        (code, String::from_utf8(out).expect("text"))
    }

    #[test]
    fn the_themes_are_listed_with_the_one_in_use_marked() {
        let (_folder, file) = settings("");
        let (code, out) = ran(&[], &file);
        assert_eq!(code.expect("runs"), 0);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), themes::BUILT_IN.len());
        assert_eq!(lines[0], "* tokyo-night  built in", "nothing named: the default one is in use");
        assert_eq!(lines[1], "  aurora       built in");
        assert!(!out.contains('\x1b'), "no colors through a pipe");
        assert!(!file.exists(), "listing writes nothing");

        let mut colored = Vec::new();
        run(&[], &file, Samples::Shown, &mut colored).expect("runs");
        let colored = String::from_utf8(colored).expect("text");
        // Tokyo Night's text on its background, then its red.
        assert!(colored.contains("\x1b[38;2;192;202;245;48;2;22;22;30m Aa \x1b[38;2;247;118;142m█"), "{colored:?}");
    }

    #[test]
    fn a_theme_is_chosen_by_writing_its_name_into_the_settings() {
        let (_folder, file) = settings("# mine\n[font]\nsize = 15 # big\n");
        let (code, out) = ran(&["aurora"], &file);
        assert_eq!((code.expect("runs"), out.as_str()), (0, "theme: aurora\n"));
        let text = std::fs::read_to_string(&file).expect("written");
        assert!(text.starts_with("# mine\n[font]\nsize = 15 # big\n"), "what was there stays: {text}");
        assert_eq!(Config::parse(&text).expect("settings").theme.name.as_deref(), Some("aurora"));
        assert!(ran(&[], &file).1.contains("* aurora"), "and it is the one marked");
        assert_eq!(ran(&["use", "day"], &file).1, "theme: day\n", "'use' says the same");
        assert_eq!(in_use(&file).expect("settings"), "day");
    }

    #[test]
    fn a_theme_that_is_not_there_or_is_wrong_changes_nothing() {
        let (folder, file) = settings("[theme]\nname = \"night\"\n");
        let before = std::fs::read_to_string(&file).expect("there");
        let said = format!("{:#}", ran(&["nosuch"], &file).0.expect_err("refused"));
        assert!(said.starts_with("no theme is called nosuch: there is tokyo-night, aurora"), "{said}");
        let themes_folder = folder.path().join("themes");
        std::fs::create_dir_all(&themes_folder).expect("a folder");
        std::fs::write(themes_folder.join("broken.toml"), "[theme]\nforeground = \"#111111\"\n").expect("write");
        let said = format!("{:#}", ran(&["broken"], &file).0.expect_err("refused"));
        assert!(said.contains("broken.toml") && said.contains("no 'background'"), "{said}");
        assert_eq!(std::fs::read_to_string(&file).expect("there"), before, "the settings are as they were");
        let (code, out) = ran(&[], &file);
        assert_eq!(code.expect("runs"), TROUBLE, "a theme that cannot be used is said by the exit code too");
        assert!(out.contains("  broken") && out.contains("cannot be used:"), "{out}");
        assert!(out.contains("* night"), "the one in use is still marked");
    }

    #[test]
    fn a_theme_of_ones_own_is_begun_from_another_and_can_be_chosen() {
        let (folder, file) = settings("[theme]\nname = \"riso\"\n");
        let (code, out) = ran(&["new", "sheet"], &file);
        assert_eq!(code.expect("runs"), 0);
        let made = folder.path().join("themes").join("sheet.toml");
        assert_eq!(out.trim_end(), made.display().to_string().replace('\\', "/"));
        let text = std::fs::read_to_string(&made).expect("made");
        assert!(
            text.starts_with("# sheet: a theme of your own, begun from riso. 'ut theme sheet' uses it.\n[theme]\n")
        );
        assert_eq!(themes::read(&text).expect("a theme"), themes::find("riso", None).expect("built in"));
        assert!(ran(&["new", "sheet"], &file).0.is_err(), "not over one that is there");
        assert_eq!(ran(&["new", "ink", "--from", "night"], &file).0.expect("runs"), 0);
        let ink = std::fs::read_to_string(folder.path().join("themes").join("ink.toml")).expect("made");
        assert!(ink.contains("begun from night") && ink.contains("#0e1620"), "{ink}");
        for wrong in ["My Theme", "show", "../x"] {
            assert!(ran(&["new", wrong], &file).0.is_err(), "{wrong}");
        }
        assert!(ran(&["new", "x", "--from", "nosuch"], &file).0.is_err());

        assert_eq!(ran(&["sheet"], &file).1, "theme: sheet\n");
        let listed = ran(&[], &file).1;
        assert!(listed.contains("* sheet        yours"), "{listed}");
        let (code, out) = ran(&["show", "sheet"], &file);
        assert_eq!((code.expect("runs"), out), (0, text));
        assert!(ran(&["show", "aurora"], &file).1.contains("#0c1c24"), "a built-in theme's file is shown too");
    }

    #[test]
    fn what_stands_in_a_themes_way_is_said() {
        let (folder, file) = settings("[theme]\nbackground = \"#000000\"\ncursor = \"#ff0000\"\n");
        let out = ran(&["day"], &file).1;
        assert_eq!(
            out,
            "theme: day\nthe settings give background, cursor themselves: these stay over the theme's \
             ('ut setting reset theme.background' takes one away)\n"
        );
        let themes_folder = folder.path().join("themes");
        std::fs::create_dir_all(&themes_folder).expect("a folder");
        let faint = themes::text("day", None).expect("built in").0.replace("#14253a", "#d0d8e0");
        std::fs::write(themes_folder.join("faint.toml"), &faint).expect("write");
        let out = ran(&["faint"], &file).1;
        assert!(out.contains("the text is hard to read on this background: their contrast is 1.4 to 1"), "{out}");
        let checked = ran(&["check", &themes_folder.join("faint.toml").display().to_string()], &file);
        assert_eq!(checked.0.expect("runs"), 0, "a theme that is hard to read is still a theme");
        assert!(checked.1.starts_with("ok: ") && checked.1.contains("hard to read"), "{}", checked.1);
    }

    #[test]
    fn a_later_versions_keys_and_a_missing_picture_are_said_not_refused() {
        let (folder, file) = settings("");
        let themes_folder = folder.path().join("themes");
        std::fs::create_dir_all(&themes_folder).expect("a folder");
        let night = themes::text("night", None).expect("built in").0;
        // Begun from a built-in theme: its picture is one of UnifyTerm's own, found from any folder.
        std::fs::write(themes_folder.join("copy.toml"), &night).expect("write");
        assert_eq!(ran(&["copy"], &file).1, "theme: copy\n", "nothing to say");
        // A key of a later version, a misspelt one, and a picture that is not there.
        let later =
            night.replace("picture = \"night.png\"", "picture = \"mine.png\"\nshadow = \"hard\"\ncursr = \"#ff0000\"");
        let path = themes_folder.join("later.toml");
        std::fs::write(&path, &later).expect("write");
        let out = ran(&["later"], &file).1;
        assert!(out.starts_with("theme: later\n"), "it is used: {out}");
        assert!(out.contains("passed over, not known to this version: cursr, shadow"), "{out}");
        assert!(out.contains("its picture is not there: ") && out.contains("/themes/mine.png"), "{out}");
        let checked = ran(&["check", &path.display().to_string()], &file).1;
        assert!(
            checked.starts_with("ok: ") && checked.contains("passed over") && checked.contains("mine.png"),
            "{checked}"
        );
        std::fs::write(themes_folder.join("mine.png"), "x").expect("write");
        assert!(!ran(&["later"], &file).1.contains("its picture"), "the picture is there now");
        let out = ran(&["tokyo-night"], &file).1;
        assert_eq!(out, "theme: tokyo-night\n");
    }

    #[test]
    fn a_file_is_checked_the_folder_is_told_and_a_wrong_call_is_a_usage_error() {
        let (folder, file) = settings("");
        let wrong = folder.path().join("Wrong Name.toml");
        std::fs::write(&wrong, themes::text("night", None).expect("built in").0).expect("write");
        let (code, out) = ran(&["check", &wrong.display().to_string()], &file);
        assert_eq!(code.expect("runs"), 0);
        assert!(out.contains("its name, Wrong Name, is not one a theme may have"), "{out}");
        std::fs::write(&wrong, "[theme]\nforeground = \"red\"").expect("write");
        assert!(ran(&["check", &wrong.display().to_string()], &file).0.is_err());
        assert!(ran(&["check", "no-such-file.toml"], &file).0.is_err());
        let (code, out) = ran(&["directory"], &file);
        assert_eq!(code.expect("runs"), 0);
        assert!(out.trim_end().ends_with("/themes") && !out.contains('\\'), "{out}");
        for wrong in [&["show"][..], &["new"], &["check"], &["a", "b"], &["new", "a", "b", "c"]] {
            assert_eq!(ran(wrong, &file).0.expect("runs"), USAGE_ERROR, "{wrong:?}");
        }
        assert!(!folder.path().join("themes").exists(), "nothing was made on the way");
    }
}
