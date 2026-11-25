//! What a setting can be chosen from, for the settings screen's list to
//! pick from: the fonts this system has, the shells on `PATH`, UnifyTerm's
//! own commands, a setting's fixed choices. A setting with none of these is
//! typed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use unifyterm_config::{Key, Kind};

/// One thing to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    /// What goes in the file; empty for "not set".
    pub(crate) value: String,
    /// What the list shows.
    pub(crate) shown: String,
    /// A word after it, dimmed: "monospace", where a shell is.
    pub(crate) note: String,
}

impl Choice {
    fn plain(value: &str) -> Self {
        Self { value: value.to_owned(), shown: value.to_owned(), note: String::new() }
    }
}

/// Whether one thing is chosen, or several in an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    One,
    Many,
}

/// What a setting is chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choices {
    pub(crate) items: Vec<Choice>,
    pub(crate) pick: Pick,
}

/// What `key` is chosen from; `None` for a setting that is typed.
pub(crate) fn for_key(key: &Key) -> Option<Choices> {
    let (items, pick) = match key.name {
        "font.family" => (fonts(), Pick::One),
        "font.fallbacks" => (fonts(), Pick::Many),
        "shell.program" => (shells(), Pick::One),
        "commands.system" => (crate::commands().into_iter().map(|(name, _)| Choice::plain(name)).collect(), Pick::Many),
        _ => match key.kind {
            Kind::OneOf(choices) => (choices.iter().map(|choice| Choice::plain(choice)).collect(), Pick::One),
            _ => return None,
        },
    };
    Some(Choices { items, pick })
}

/// The setting whose choices are the themes (see [`themes`]).
pub(crate) const THEME: &str = "theme.name";

/// The themes to choose from for the settings file `settings`: "not set"
/// (the default theme), the ones UnifyTerm brings, and the user's own in
/// the `themes` folder beside that file.
pub(crate) fn themes(settings: &std::path::Path) -> Choices {
    use unifyterm_config::themes::{self, Source};
    let mut items = vec![Choice { value: String::new(), shown: "(default)".into(), note: themes::DEFAULT.into() }];
    items.extend(themes::list(Some(&themes::directory(settings))).into_iter().map(|theme| Choice {
        shown: theme.name.clone(),
        value: theme.name,
        note: if theme.source == Source::BuiltIn { "built in".into() } else { "yours".into() },
    }));
    Choices { items, pick: Pick::One }
}

/// The font families this system has, those whose letters are all one
/// width first (a terminal's font is one of them), each once.
fn fonts() -> Vec<Choice> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    // A family's name in English where the font gives one: that is the name the settings are read with.
    let mut families: BTreeMap<String, bool> = BTreeMap::new();
    for face in database.faces() {
        let english = face.families.iter().find(|(_, language)| *language == fontdb::Language::English_UnitedStates);
        let Some((name, _)) = english.or_else(|| face.families.first()) else { continue };
        // macOS keeps fonts of its own under names that start with a dot: not for people to choose.
        if name.starts_with('.') || name.trim().is_empty() {
            continue;
        }
        *families.entry(name.clone()).or_default() |= face.monospaced;
    }
    let mut fonts: Vec<(String, bool)> = families.into_iter().collect();
    fonts.sort_by(|(a, a_mono), (b, b_mono)| b_mono.cmp(a_mono).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
    fonts
        .into_iter()
        .map(|(name, monospace)| Choice {
            shown: name.clone(),
            value: name,
            note: if monospace { "monospace".into() } else { String::new() },
        })
        .collect()
}

/// The shells UnifyTerm knows that are on `PATH`, after "the default".
fn shells() -> Vec<Choice> {
    const KNOWN: [&str; 8] = ["pwsh", "powershell", "cmd", "zsh", "bash", "fish", "nu", "sh"];
    let default = if cfg!(windows) { "PowerShell" } else { "your login shell" };
    let mut found = vec![Choice { value: String::new(), shown: "(default)".into(), note: default.into() }];
    for name in KNOWN {
        if let Some(path) = on_path(name) {
            found.push(Choice { value: name.into(), shown: name.into(), note: path.display().to_string() });
        }
    }
    found
}

/// Where `PATH` has the program `name`.
fn on_path(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(&file)).find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unifyterm_config::key;

    #[test]
    fn each_setting_with_choices_has_them() {
        let provider = for_key(key("ai.provider").unwrap()).unwrap();
        assert_eq!(provider.pick, Pick::One);
        let names: Vec<&str> = provider.items.iter().map(|choice| choice.value.as_str()).collect();
        assert_eq!(names, unifyterm_config::providers::NAMES, "every provider the settings take, in their order");
        let commands = for_key(key("commands.system").unwrap()).unwrap();
        assert_eq!(commands.pick, Pick::Many);
        assert_eq!(commands.items.len(), crate::commands().len(), "every command of ut's");
        let shells = for_key(key("shell.program").unwrap()).unwrap();
        assert_eq!((shells.items[0].value.as_str(), shells.items[0].shown.as_str()), ("", "(default)"));
        for typed in ["font.size", "ai.model", "theme.background", "agents.command"] {
            assert!(for_key(key(typed).unwrap()).is_none(), "{typed} is typed");
        }
    }

    #[test]
    fn the_themes_to_choose_are_the_built_in_ones_and_the_users_own() {
        let folder = tempfile::tempdir().unwrap();
        let settings = folder.path().join("config.toml");
        let listed = |settings: &std::path::Path| -> Vec<(String, String)> {
            themes(settings).items.into_iter().map(|choice| (choice.value, choice.note)).collect()
        };
        let built_in = listed(&settings);
        assert_eq!(built_in[0], (String::new(), "tokyo-night".to_owned()), "not set: the default one");
        assert_eq!(built_in[1], ("tokyo-night".to_owned(), "built in".to_owned()));
        assert_eq!(built_in.len(), unifyterm_config::themes::BUILT_IN.len() + 1);
        std::fs::create_dir_all(folder.path().join("themes")).unwrap();
        std::fs::write(folder.path().join("themes").join("mine.toml"), "").unwrap();
        assert_eq!(listed(&settings).last(), Some(&("mine".to_owned(), "yours".to_owned())));
        assert!(for_key(key(THEME).unwrap()).is_none(), "the screen asks with the file: not from the key alone");
    }

    #[test]
    fn the_fonts_are_this_systems_one_width_ones_first() {
        let fonts = fonts();
        assert!(!fonts.is_empty(), "the system has fonts");
        let first_proportional = fonts.iter().position(|font| font.note.is_empty()).unwrap_or(fonts.len());
        assert!(fonts[first_proportional..].iter().all(|font| font.note.is_empty()), "one-width fonts all come first");
        let mut names: Vec<&str> = fonts.iter().map(|font| font.value.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "each family once");
        assert!(fonts.iter().all(|font| !font.value.starts_with('.')));
    }
}
