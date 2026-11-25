//! Themes: a set of colors under a name.
//!
//! A theme is a file with one table, `[theme]`, and the same colors the
//! settings file's `[theme]` takes. The themes UnifyTerm brings are such
//! files too, kept in the program; a user's own are in the `themes` folder
//! beside the settings file, each called `NAME.toml`. A user's file with a
//! built-in theme's name is used in its place.
//!
//! Which theme is in use is one setting, `theme.name`: `ut theme NAME`
//! writes it, and the window follows the settings file as it does for any
//! other setting.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::ThemeConfig;

/// The theme in use when the settings name none.
pub const DEFAULT: &str = "tokyo-night";
/// What a theme file's name ends in.
const EXTENSION: &str = "toml";
/// A name is a file's name too: not longer than this.
const LONGEST_NAME: usize = 64;

macro_rules! built_in {
    ($name:literal) => {
        ($name, include_str!(concat!("../../../demo/themes/", $name, ".toml")))
    };
}

/// The themes UnifyTerm brings, in the order they are listed: each one's name and its file.
pub const BUILT_IN: [(&str, &str); 9] = [
    built_in!("tokyo-night"),
    built_in!("aurora"),
    built_in!("eclipse"),
    built_in!("phosphor"),
    built_in!("riso"),
    built_in!("night"),
    built_in!("day"),
    built_in!("paper-dark"),
    built_in!("paper"),
];

/// Where a theme is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// In the program.
    BuiltIn,
    /// The user's file.
    File(PathBuf),
}

/// A theme there is: its name and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub source: Source,
}

/// The folder of the user's own themes: `themes` beside the settings file.
#[must_use]
pub fn directory(settings: &Path) -> PathBuf {
    settings.parent().map_or_else(|| PathBuf::from("themes"), |beside| beside.join("themes"))
}

/// The file the user's theme `name` is in.
#[must_use]
pub fn file_of(name: &str, directory: &Path) -> PathBuf {
    directory.join(format!("{name}.{EXTENSION}"))
}

/// Whether `text` may be a theme's name: small letters, digits and `-`,
/// beginning with a letter or a digit. It is a file's name on every system,
/// and never an option on a command line.
#[must_use]
pub fn is_name(text: &str) -> bool {
    let plain = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    text.len() <= LONGEST_NAME
        && text.bytes().next().is_some_and(plain)
        && text.bytes().all(|byte| plain(byte) || byte == b'-')
}

/// The table `[theme]` of a theme file's text.
fn table(text: &str) -> Result<toml::Table> {
    let whole: toml::Table =
        toml::from_str(text).context("a theme file is TOML: a table [theme] with the theme's colors")?;
    match whole.get("theme") {
        Some(toml::Value::Table(table)) => Ok(table.clone()),
        _ => bail!("a theme file has a table [theme] with the theme's colors"),
    }
}

/// What a theme file's text gives. A theme gives the text's color, the
/// color behind it and the 16 colors itself: with one of them from another
/// theme, a dark theme and a light one would be mixed.
///
/// A key this version does not know is passed over, and so is a table other
/// than `[theme]`: a theme written for a later UnifyTerm, with what that one
/// added, is still a theme here ([`passed_over`] names them, for who checks
/// a file). A known key with a wrong value is an error.
pub fn read(text: &str) -> Result<ThemeConfig> {
    let known: toml::Table =
        table(text)?.into_iter().filter(|(key, _)| ThemeConfig::KEYS.contains(&key.as_str())).collect();
    let theme: ThemeConfig = known.try_into().context("in [theme]")?;
    if theme.name.is_some() {
        bail!("a theme file gives colors: 'name' is for the settings file, where it says which theme is in use");
    }
    theme.check()?;
    let needed = [
        ("foreground", theme.foreground.is_some()),
        ("background", theme.background.is_some()),
        ("ansi", theme.ansi.is_some()),
    ];
    if let Some((key, _)) = needed.iter().find(|(_, given)| !given) {
        bail!("the theme has no '{key}': a theme gives foreground, background and ansi itself");
    }
    Ok(whole(theme))
}

/// The selection, when a theme does not give one, is this share of the way
/// from the background to the text's color: one fifth.
const SELECTION_SHARE: i32 = 5;

/// A theme with the two colors it may leave out worked out from its own:
/// the cursor is the text's color, the selection a step from the background
/// toward it. Left empty they would be the built-in theme's, a dark theme's
/// blue behind the selected text of a light one.
fn whole(mut theme: ThemeConfig) -> ThemeConfig {
    let (Some(text), Some(behind)) = (theme.foreground.clone(), theme.background.clone()) else { return theme };
    if theme.selection.is_none()
        && let (Some(ink), Some(paper)) = (crate::parse_color(&text), crate::parse_color(&behind))
    {
        let mixed = [0, 1, 2].map(|at| {
            let (from, to) = (i32::from(paper[at]), i32::from(ink[at]));
            // Between two channels: a channel itself.
            u8::try_from(from + (to - from) / SELECTION_SHARE).unwrap_or(paper[at])
        });
        theme.selection = Some(format!("#{:02x}{:02x}{:02x}", mixed[0], mixed[1], mixed[2]));
    }
    theme.cursor.get_or_insert(text);
    theme
}

/// How far apart two colors (`#rrggbb`) are for the eye: the contrast ratio
/// as WCAG 2 works it out, from 1 (the same) to 21 (black on white). Text is
/// read with ease from 7, large text from 4.5. `None` when one is no color.
#[must_use]
pub fn contrast(one: &str, other: &str) -> Option<f64> {
    Some(contrast_of(crate::parse_color(one)?, crate::parse_color(other)?))
}

/// [`contrast`] of two colors given as red, green, blue.
#[must_use]
pub fn contrast_of(one: [u8; 3], other: [u8; 3]) -> f64 {
    let light = |color: [u8; 3]| {
        let [red, green, blue] = color.map(|channel| {
            let part = f64::from(channel) / 255.0;
            if part <= 0.04045 { part / 12.92 } else { ((part + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    };
    let (one, other) = (light(one), light(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// What [`read`] passes over in a theme file's text: the keys of `[theme]`
/// this version does not know, and the other tables. A misspelt key is here
/// too: nothing tells it from a later version's.
#[must_use]
pub fn passed_over(text: &str) -> Vec<String> {
    let Ok(whole) = toml::from_str::<toml::Table>(text) else { return Vec::new() };
    let mut names: Vec<String> = whole.keys().filter(|key| *key != "theme").map(|key| format!("[{key}]")).collect();
    if let Some(toml::Value::Table(table)) = whole.get("theme") {
        names.extend(table.keys().filter(|key| !ThemeConfig::KEYS.contains(&key.as_str())).cloned());
    }
    names
}

/// The files of UnifyTerm's own pictures: one for each built-in theme but
/// the default. The window has the pictures themselves.
pub const PICTURES: [&str; 6] = ["aurora.png", "eclipse.png", "phosphor.png", "riso.png", "night.png", "day.png"];

/// Where a picture is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// One of UnifyTerm's own, by its file's name.
    BuiltIn(&'static str),
    /// A file. It may not be there: who shows it finds out.
    File(PathBuf),
}

/// Where the picture `named` is, for a file in the folder `beside` that
/// names it: that file when it is there; else one of UnifyTerm's own with
/// that name, which are found from every folder (a theme begun as a copy of
/// a built-in one keeps its picture). `None` for an empty name: no picture.
#[must_use]
pub fn picture(named: &str, beside: Option<&Path>) -> Option<Picture> {
    if named.trim().is_empty() {
        return None;
    }
    let file = beside.map_or_else(|| PathBuf::from(named), |beside| beside.join(named));
    let own = PICTURES.iter().find(|own| **own == named);
    match own {
        Some(own) if !file.is_file() => Some(Picture::BuiltIn(own)),
        _ => Some(Picture::File(file)),
    }
}

/// The built-in theme `name`'s file.
fn built_in(name: &str) -> Option<&'static str> {
    BUILT_IN.iter().find(|(known, _)| *known == name).map(|(_, text)| *text)
}

/// The text of the theme `name` and where it is: the user's file in
/// `directory` when there is one, else the built-in theme.
pub fn text(name: &str, directory: Option<&Path>) -> Result<(String, Source)> {
    if !is_name(name) {
        bail!("\"{name}\" is not a theme's name: small letters, digits and -");
    }
    if let Some(file) = directory.map(|directory| file_of(name, directory)) {
        match std::fs::read_to_string(&file) {
            Ok(text) => return Ok((text, Source::File(file))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
        }
    }
    if let Some(text) = built_in(name) {
        return Ok((text.to_owned(), Source::BuiltIn));
    }
    let known: Vec<String> = list(directory).into_iter().map(|theme| theme.name).collect();
    bail!("no theme is called {name}: there is {}", known.join(", "))
}

/// What the theme `name` gives, and where the theme is (see [`text`]).
pub fn found(name: &str, directory: Option<&Path>) -> Result<(ThemeConfig, Source)> {
    let (text, source) = text(name, directory)?;
    let theme = read(&text).with_context(|| match &source {
        Source::BuiltIn => format!("in the built-in theme {name}"),
        Source::File(file) => format!("in {}", file.display()),
    })?;
    Ok((theme, source))
}

/// What the theme `name` gives (see [`found`]).
pub fn find(name: &str, directory: Option<&Path>) -> Result<ThemeConfig> {
    found(name, directory).map(|(theme, _)| theme)
}

/// Every theme there is: the built-in ones in their order, then the user's
/// other ones by name. A built-in theme the user has a file for is listed
/// once, as that file.
#[must_use]
pub fn list(directory: Option<&Path>) -> Vec<Listed> {
    let mut own: Vec<(String, PathBuf)> = directory
        .and_then(|directory| std::fs::read_dir(directory).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|file| file.extension().is_some_and(|extension| extension == EXTENSION))
        .filter_map(|file| {
            let name = file.file_stem()?.to_str().filter(|name| is_name(name))?.to_owned();
            Some((name, file))
        })
        .collect();
    own.sort();
    let mut listed: Vec<Listed> = BUILT_IN
        .iter()
        .map(|(name, _)| {
            let source = own
                .iter()
                .position(|(own, _)| own == name)
                .map_or(Source::BuiltIn, |at| Source::File(own.remove(at).1));
            Listed { name: (*name).to_owned(), source }
        })
        .collect();
    listed.extend(own.into_iter().map(|(name, file)| Listed { name, source: Source::File(file) }));
    listed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ANSI_COLORS, Config};

    const OWN: &str = "[theme]\nforeground = \"#111111\"\nbackground = \"#eeeeee\"\nansi = [\n\
        \"#000000\", \"#000001\", \"#000002\", \"#000003\", \"#000004\", \"#000005\", \"#000006\", \"#000007\",\n\
        \"#000008\", \"#000009\", \"#00000a\", \"#00000b\", \"#00000c\", \"#00000d\", \"#00000e\", \"#00000f\"]\n";

    #[test]
    fn contrast_is_wcags_ratio() {
        let ratio = |one: &str, other: &str| contrast(one, other).expect("two colors");
        assert!((ratio("#000000", "#ffffff") - 21.0).abs() < 1e-9);
        assert!((ratio("#ffffff", "#000000") - 21.0).abs() < 1e-9, "either way round");
        assert!((ratio("#808080", "#808080") - 1.0).abs() < 1e-9);
        assert!((ratio("#777777", "#ffffff") - 4.478).abs() < 0.001, "the grey that just misses 4.5 on white");
        assert_eq!(contrast("red", "#ffffff"), None);
    }

    #[test]
    fn every_built_in_theme_is_a_whole_theme_that_can_be_read() {
        assert_eq!(BUILT_IN[0].0, DEFAULT, "the default one is listed first");
        for (name, text) in BUILT_IN {
            assert!(is_name(name), "{name}");
            let theme = read(text).unwrap_or_else(|error| panic!("{name}: {error:#}"));
            assert_eq!(theme.ansi.as_ref().map(Vec::len), Some(ANSI_COLORS), "{name}");
            let (text_color, behind) = (theme.foreground.expect("given"), theme.background.expect("given"));
            let ratio = |one: &str| contrast(one, &behind).expect("two colors");
            assert!(ratio(&text_color) >= 7.0, "{name}: text on its background can be read");
            if let Some(muted) = &theme.muted {
                assert!(ratio(muted) >= 4.5, "{name}: so can the dimmed words");
            }
        }
    }

    #[test]
    fn the_built_in_themes_around_the_text_are_the_preview_pages() {
        // demo/appearance.html: --bar, --panel, --well, --rule, --edge, --muted of each theme, as #rrggbb.
        let page = [
            ("aurora", ["#10252d", "#16303b", "#203c47", "#2c4953", "#496674", "#a6c5ca"]),
            ("eclipse", ["#2b1a21", "#36232b", "#432b32", "#54353e", "#79535e", "#dcbabe"]),
            ("phosphor", ["#152015", "#1c291c", "#243324", "#374b35", "#536e50", "#b4caa7"]),
            ("riso", ["#f3ead7", "#f7efde", "#ebdcc3", "#cec1a7", "#9a8b70", "#514c50"]),
            ("night", ["#121b26", "#18222f", "#1c2938", "#2b3644", "#394657", "#92a2b4"]),
            ("day", ["#e8eff6", "#ebf3fb", "#dbe6f3", "#c7d2df", "#a4b3c3", "#41546b"]),
        ];
        for (name, colors) in page {
            let theme = find(name, None).expect(name);
            let given = [theme.bar, theme.panel, theme.well, theme.rule, theme.edge, theme.muted];
            assert_eq!(given.map(|color| color.expect("given")), colors.map(str::to_owned), "{name}");
        }
        assert_eq!(find("eclipse", None).expect("there").symbol.as_deref(), Some("#ffa6a0"));
        assert_eq!(find(DEFAULT, None).expect("there").bar, None, "left to be worked out");
    }

    #[test]
    fn a_name_is_small_letters_digits_and_hyphens() {
        for name in ["night", "tokyo-night", "my-theme-2", "3024"] {
            assert!(is_name(name), "{name}");
        }
        for name in ["", "Night", "-night", "my theme", "a/b", "..", "night.toml", "día", &"x".repeat(65)] {
            assert!(!is_name(name), "{name}");
        }
    }

    #[test]
    fn a_theme_file_gives_its_own_look() {
        let theme = read(OWN).expect("a theme");
        assert_eq!((theme.foreground.as_deref(), theme.bar, theme.opacity), (Some("#111111"), None, None));
        let more = OWN.replace("[theme]", "[theme]\nopacity = 0.8\nblur = true\npicture = \"mine.png\"");
        let theme = read(&more).expect("a theme");
        assert_eq!((theme.opacity, theme.blur, theme.picture.as_deref()), (Some(0.8), Some(true), Some("mine.png")));

        let said = |text: &str| format!("{:#}", read(text).expect_err("not a theme"));
        assert!(said("").contains("a table [theme]"));
        assert!(said("foreground = \"#111111\"").contains("a table [theme]"), "the colors are in [theme]");
        assert!(said("theme = 1").contains("a table [theme]"));
        assert!(said("[theme").contains("a theme file is TOML"));
        assert!(said(&OWN.replace("[theme]", "[theme]\nname = \"day\"")).contains("'name' is for the settings file"));
        assert!(said("[theme]\nforeground = \"#111111\"").contains("no 'background'"));
        assert!(said(&OWN.replace("foreground", "foregruond")).contains("no 'foreground'"), "a needed key, misspelt");
        assert!(said(&OWN.replace("#eeeeee", "white")).contains("theme.background is \"white\""));
        assert!(said(&OWN.replace("\"#000008\", ", "")).contains("theme.ansi has 15 colors"));
        assert!(
            said(&OWN.replace("[theme]", "[theme]\nopacity = 3"))
                .contains("theme.opacity is 3: it must be from 0.1 to 1")
        );
        assert!(
            said(&OWN.replace("[theme]", "[theme]\nblur = \"yes\"")).contains("in [theme]"),
            "a known key, a wrong value"
        );
    }

    #[test]
    fn a_theme_that_leaves_out_the_cursor_and_the_selection_has_them_from_its_own_colors() {
        // Text #111111 on #eeeeee: the cursor is the text's color, the selection a fifth of the way to it.
        let theme = read(OWN).expect("a theme");
        assert_eq!((theme.cursor.as_deref(), theme.selection.as_deref()), (Some("#111111"), Some("#c2c2c2")));
        let given = OWN.replace("[theme]", "[theme]\ncursor = \"#ff0000\"\nselection = \"#00ff00\"");
        let theme = read(&given).expect("a theme");
        assert_eq!(
            (theme.cursor.as_deref(), theme.selection.as_deref()),
            (Some("#ff0000"), Some("#00ff00")),
            "its own stay"
        );
        for (name, text) in BUILT_IN {
            let theme = read(text).expect(name);
            assert!(theme.cursor.is_some() && theme.selection.is_some(), "{name}");
        }
    }

    #[test]
    fn what_this_version_does_not_know_in_a_theme_file_is_passed_over_and_named() {
        // A theme written for a later UnifyTerm: a key and a table this one has never heard of.
        let later = format!(
            "{}\n[meta]\nauthor = \"someone\"\n",
            OWN.replace("[theme]", "[theme]\nshadow = \"hard\"\ncursr = \"#ff0000\"")
        );
        let theme = read(&later).expect("still a theme");
        assert_eq!(theme, read(OWN).expect("a theme"), "what it knows is read as before");
        let mut named = passed_over(&later);
        named.sort();
        assert_eq!(named, ["[meta]", "cursr", "shadow"], "a misspelt key is named with them");
        assert!(passed_over(OWN).is_empty());
        assert!(passed_over("not toml [").is_empty(), "what cannot be read names nothing: reading it says why");
        for (name, text) in BUILT_IN {
            assert!(passed_over(text).is_empty(), "{name}: every key is known");
        }
    }

    #[test]
    fn the_tables_keys_are_the_ones_listed() {
        let all = ThemeConfig {
            name: Some("x".into()),
            foreground: Some("#000000".into()),
            background: Some("#000000".into()),
            cursor: Some("#000000".into()),
            selection: Some("#000000".into()),
            bar: Some("#000000".into()),
            panel: Some("#000000".into()),
            well: Some("#000000".into()),
            rule: Some("#000000".into()),
            edge: Some("#000000".into()),
            muted: Some("#000000".into()),
            symbol: Some("#000000".into()),
            ansi: Some(vec!["#000000".into(); ANSI_COLORS]),
            opacity: Some(1.0),
            blur: Some(true),
            picture: Some("x.png".into()),
            rounding: Some(7.0),
            corners: Some(crate::Corners::Small),
        };
        let written = toml::Value::try_from(all).expect("a table");
        let mut keys: Vec<&str> = written.as_table().expect("a table").keys().map(String::as_str).collect();
        let mut listed = ThemeConfig::KEYS.to_vec();
        keys.sort_unstable();
        listed.sort_unstable();
        assert_eq!(keys, listed, "a key the table has and the list lacks would be passed over in every theme file");
    }

    #[test]
    fn a_picture_is_beside_the_file_that_names_it_or_one_of_unifyterms_own() {
        let folder = tempfile::tempdir().expect("a folder");
        let here = Some(folder.path());
        assert_eq!(picture("", here), None);
        assert_eq!(picture("  ", here), None, "no picture");
        assert_eq!(picture("aurora.png", here), Some(Picture::BuiltIn("aurora.png")), "found from every folder");
        assert_eq!(picture("aurora.png", None), Some(Picture::BuiltIn("aurora.png")));
        assert_eq!(picture("mine.png", here), Some(Picture::File(folder.path().join("mine.png"))), "there or not");
        std::fs::write(folder.path().join("aurora.png"), "x").expect("write");
        assert_eq!(
            picture("aurora.png", here),
            Some(Picture::File(folder.path().join("aurora.png"))),
            "a file of that name in the folder comes first"
        );
        let elsewhere = folder.path().join("elsewhere.png");
        assert_eq!(picture(&elsewhere.display().to_string(), here), Some(Picture::File(elsewhere)), "a whole path");
        // The six themes of the preview page each name their picture; the others have none.
        let pictured: Vec<&str> = PICTURES.iter().map(|file| file.trim_end_matches(".png")).collect();
        for name in &pictured {
            let named = find(name, None).expect("built in").picture.expect("it has one");
            assert_eq!(
                picture(&named, None),
                Some(Picture::BuiltIn(PICTURES.iter().find(|own| **own == named).expect("own")))
            );
            assert_eq!(named, format!("{name}.png"));
        }
        for (name, _) in BUILT_IN.iter().filter(|(name, _)| !pictured.contains(name)) {
            assert_eq!(find(name, None).expect("built in").picture, None, "{name}");
        }
    }

    #[test]
    fn the_users_file_is_found_before_the_built_in_theme() {
        let folder = tempfile::tempdir().expect("a folder");
        let here = Some(folder.path());
        assert_eq!(find("day", here).expect("built in"), find("day", None).expect("built in"));
        std::fs::write(file_of("day", folder.path()), OWN).expect("write");
        std::fs::write(file_of("mine", folder.path()), OWN).expect("write");
        std::fs::write(folder.path().join("Not A Name.toml"), OWN).expect("write");
        std::fs::write(folder.path().join("notes.txt"), "x").expect("write");
        assert_eq!(find("day", here).expect("the file").background.as_deref(), Some("#eeeeee"));
        assert_eq!(find("day", None).expect("built in").background.as_deref(), Some("#f4f9fe"));
        assert_eq!(text("mine", here).expect("the file").1, Source::File(file_of("mine", folder.path())));

        let names: Vec<(String, bool)> =
            list(here).into_iter().map(|theme| (theme.name, theme.source == Source::BuiltIn)).collect();
        let wanted = [
            ("tokyo-night", true),
            ("aurora", true),
            ("eclipse", true),
            ("phosphor", true),
            ("riso", true),
            ("night", true),
            ("day", false),
            ("paper-dark", true),
            ("paper", true),
            ("mine", false),
        ];
        assert_eq!(names, wanted.map(|(name, built_in)| (name.to_owned(), built_in)));
        assert_eq!(list(None).len(), BUILT_IN.len());
        assert_eq!(list(Some(&folder.path().join("not-there"))).len(), BUILT_IN.len(), "no folder: the built-in ones");
    }

    #[test]
    fn a_theme_that_is_not_there_or_is_wrong_is_refused_with_the_reason() {
        let folder = tempfile::tempdir().expect("a folder");
        let said = |name: &str| format!("{:#}", find(name, Some(folder.path())).expect_err("refused"));
        assert_eq!(
            said("nosuch"),
            "no theme is called nosuch: there is tokyo-night, aurora, eclipse, phosphor, riso, night, day, paper-dark, paper"
        );
        assert!(said("../day").contains("is not a theme's name"), "a name never leaves the folder");
        std::fs::write(file_of("broken", folder.path()), "[theme]\nforeground = 1").expect("write");
        let broken = said("broken");
        assert!(broken.starts_with(&format!("in {}", file_of("broken", folder.path()).display())), "{broken}");
    }

    #[test]
    fn what_the_settings_give_goes_over_the_named_themes() {
        let named = Config::parse("[theme]\nname = \"day\"\ncursor = \"#ff0000\"\nopacity = 0.5").expect("settings");
        let look = named.look(None).expect("the theme is there");
        assert_eq!(look.colors.cursor.as_deref(), Some("#ff0000"), "the settings' own");
        assert_eq!(look.colors.background.as_deref(), Some("#f4f9fe"), "the theme's");
        assert_eq!(look.colors.bar.as_deref(), Some("#e8eff6"));
        assert_eq!((look.colors.opacity, look.colors.blur), (Some(0.5), None));
        assert_eq!(look.picture, Some(Picture::BuiltIn("day.png")), "the theme's picture");
        let plain = Config::parse("[theme]\ncursor = \"#ff0000\"").expect("settings");
        let look = plain.look(None).expect("nothing to find");
        assert_eq!((look.colors, look.picture), (plain.theme, None), "no theme named: what the settings give");
        let lost = Config::parse("[theme]\nname = \"nosuch\"").expect("a name is not looked up when the file is read");
        assert!(lost.look(None).is_err());
        assert!(Config::parse("[theme]\nname = \"No Such\"").is_err(), "but it must be a name");
        assert!(Config::parse("[theme]\nopacity = 0").is_err());
    }

    #[test]
    fn a_picture_is_looked_for_beside_the_file_that_names_it() {
        // settings/config.toml, settings/themes/paper.toml: each names a picture of its own folder.
        let folder = tempfile::tempdir().expect("a folder");
        let themes = folder.path().join("themes");
        std::fs::create_dir_all(&themes).expect("a folder");
        std::fs::write(file_of("paper", &themes), OWN.replace("[theme]", "[theme]\npicture = \"paper.png\""))
            .expect("write");
        let look =
            |settings: &str| Config::parse(settings).expect("settings").look(Some(&themes)).expect("a look").picture;
        assert_eq!(
            look("[theme]\nname = \"paper\""),
            Some(Picture::File(themes.join("paper.png"))),
            "the theme's folder"
        );
        assert_eq!(
            look("[theme]\nname = \"paper\"\npicture = \"mine.png\""),
            Some(Picture::File(folder.path().join("mine.png"))),
            "the settings' own, beside the settings file"
        );
        assert_eq!(look("[theme]\nname = \"paper\"\npicture = \"\""), None, "the settings turn the theme's off");
        assert_eq!(look("[theme]\nname = \"aurora\""), Some(Picture::BuiltIn("aurora.png")));
        assert_eq!(
            look("[theme]\npicture = \"night.png\""),
            Some(Picture::BuiltIn("night.png")),
            "with no theme named"
        );
        assert_eq!(look(""), None);
    }
}
