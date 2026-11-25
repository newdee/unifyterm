//! Changing the settings file in place: one setting at a time, each change
//! checked against the same rules the terminal reads the file with, and the
//! comments and order its owner gave the file left as they are.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::{ANSI_COLORS, Config, Key, Kind, parse_color};

/// A settings file, open to be changed.
#[derive(Debug, Clone)]
pub struct File {
    path: PathBuf,
    document: DocumentMut,
    /// The file's text when it was opened or last saved.
    saved: String,
}

impl File {
    /// The file at `path`. One that is not there yet is an empty one: it is
    /// made when it is saved. One the terminal would refuse is refused here
    /// too, with what is wrong in it: changing it blindly could not mend it.
    pub fn open(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Config::parse(&text).with_context(|| format!("in {}", path.display()))?;
        let document: DocumentMut = text.parse().with_context(|| format!("in {}", path.display()))?;
        Ok(Self { path: path.to_path_buf(), document, saved: text })
    }

    /// The file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the file gives `key`, written the way [`File::set`] takes it;
    /// `None` when the file leaves it out (the default is then in use).
    #[must_use]
    pub fn value(&self, key: &Key) -> Option<String> {
        self.document.get(key.section())?.get(key.field())?.as_value().map(shown)
    }

    /// The value in use: the file's, else the default.
    #[must_use]
    pub fn in_use(&self, key: &Key) -> String {
        self.value(key).unwrap_or_else(|| default_of(key))
    }

    /// Gives `key` the value `input` names. Nothing is changed when the value
    /// is not one `key` takes, or when the settings as a whole would be
    /// refused with it (a model's name is needed once a provider is chosen).
    pub fn set(&mut self, key: &Key, input: &str) -> Result<()> {
        let value = parse(key, input)?;
        let before = self.document.clone();
        let table = self.document.entry(key.section()).or_insert_with(|| Item::Table(Table::new()));
        let Some(table) = table.as_table_like_mut() else { bail!("[{}] is not a table in the file", key.section()) };
        match value {
            Some(mut value) => {
                // A setting already in the file keeps its place: the comment above it belongs to
                // its name, the one after it to its value, and both stay.
                if let Some(item) = table.get_mut(key.field()) {
                    if let Some(old) = item.as_value() {
                        *value.decor_mut() = old.decor().clone();
                    }
                    *item = Item::Value(value);
                } else {
                    table.insert(key.field(), Item::Value(value));
                }
            }
            None => {
                table.remove(key.field());
            }
        }
        self.tidy(key.section());
        if let Err(error) = Config::parse(&self.document.to_string()) {
            self.document = before;
            return Err(error);
        }
        Ok(())
    }

    /// Takes `key` out of the file: the default is in use again.
    pub fn reset(&mut self, key: &Key) -> Result<()> {
        let before = self.document.clone();
        if let Some(table) = self.document.get_mut(key.section()).and_then(Item::as_table_like_mut) {
            table.remove(key.field());
        }
        self.tidy(key.section());
        if let Err(error) = Config::parse(&self.document.to_string()) {
            self.document = before;
            return Err(error);
        }
        Ok(())
    }

    /// A table left with nothing in it goes: the file does not keep empty headings.
    fn tidy(&mut self, section: &str) {
        let empty =
            self.document.get(section).and_then(Item::as_table_like).is_some_and(toml_edit::TableLike::is_empty);
        if empty {
            self.document.remove(section);
        }
    }

    /// Whether there are changes that are not saved.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.document.to_string() != self.saved
    }

    /// The file's text as it will be saved.
    #[must_use]
    pub fn text(&self) -> String {
        self.document.to_string()
    }

    /// Writes the file. It is written beside itself and moved into place, so
    /// that a terminal reading it never sees half of it.
    pub fn save(&mut self) -> Result<()> {
        let text = self.document.to_string();
        if let Some(directory) = self.path.parent().filter(|directory| !directory.as_os_str().is_empty()) {
            std::fs::create_dir_all(directory).with_context(|| format!("make {}", directory.display()))?;
        }
        let mut beside = self.path.clone().into_os_string();
        beside.push(".new");
        let beside = PathBuf::from(beside);
        std::fs::write(&beside, &text).with_context(|| format!("write {}", beside.display()))?;
        std::fs::rename(&beside, &self.path).with_context(|| format!("put {} in place", self.path.display()))?;
        self.saved = text;
        Ok(())
    }
}

/// The value `key` has when the file leaves it out, written the way
/// [`File::set`] takes it. Empty for a setting that is then not set at all.
#[must_use]
pub fn default_of(key: &Key) -> String {
    toml::Value::try_from(Config::default())
        .ok()
        .and_then(|all| all.get(key.section())?.get(key.field()).cloned())
        .and_then(|value| value.to_string().parse::<Value>().ok())
        .map(|value| shown(&value))
        .unwrap_or_default()
}

/// A value as a person writes it: text without its quotes, a list with commas.
fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.value().clone(),
        Value::Array(words) => {
            let words: Vec<String> = words.iter().map(shown).collect();
            // A word with a comma in it, or space at its ends, would not come back the same: TOML's own form then.
            if words.iter().any(|word| word.contains(',') || word.trim() != word || word.is_empty()) {
                let mut array = Array::new();
                array.extend(words.iter().map(String::as_str));
                array.to_string()
            } else {
                words.join(", ")
            }
        }
        // The settings keep numbers as f32: read as one, 1.35 is shown as 1.35 and not as 1.350000023841858.
        Value::Float(number) => {
            let text = number.value().to_string();
            text.parse::<f32>().map_or(text, |number| format!("{number:?}"))
        }
        other => other.clone().decorated("", "").to_string(),
    }
}

/// `input` as a value of `key`'s kind; `None` for "not set", which the kinds
/// that may be left out take as empty text.
fn parse(key: &Key, input: &str) -> Result<Option<Value>> {
    let text = input.trim();
    let name = key.name;
    // Left empty, a setting that may be left out is taken out of the file.
    if text.is_empty() && matches!(key.kind, Kind::OptionalText | Kind::Color | Kind::Colors) {
        return Ok(None);
    }
    Ok(Some(match key.kind {
        Kind::Switch => Value::from(match text.to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => true,
            "false" | "off" | "no" | "0" => false,
            _ => bail!("{name} is true or false, not \"{text}\""),
        }),
        Kind::Whole { least, most } => {
            let number: u64 =
                text.replace('_', "").parse().map_err(|_| anyhow!("{name} is a whole number, not \"{text}\""))?;
            if !(least..=most).contains(&number) {
                bail!("{name} is {number}: it must be from {least} to {most}");
            }
            Value::from(i64::try_from(number).with_context(|| format!("{name} is too large"))?)
        }
        Kind::Number { least, most } => {
            let number: f64 = text.parse().map_err(|_| anyhow!("{name} is a number, not \"{text}\""))?;
            // Compared as the settings keep it, an f32: a limit like 0.1 is not the same number as an
            // f64, and "0.1" typed would be under it.
            let kept: f32 = text.parse().map_err(|_| anyhow!("{name} is a number, not \"{text}\""))?;
            if !(least..=most).contains(&kept) {
                bail!("{name} is {number}: it must be from {least} to {most}");
            }
            Value::from(number)
        }
        Kind::Text | Kind::OptionalText => Value::from(text),
        Kind::OneOf(choices) => {
            if !choices.contains(&text) {
                bail!("{name} is one of {}, not \"{text}\"", choices.join(", "));
            }
            Value::from(text)
        }
        Kind::Color => {
            if parse_color(text).is_none() {
                bail!("{name} is \"{text}\": not a color like #1a2b3c");
            }
            Value::from(text)
        }
        Kind::Words => Value::Array(words(name, text)?),
        Kind::Keys => {
            // Each key in the one form it is kept: "Ctrl+Shift+T" is written ctrl-shift-t.
            let mut kept = Array::new();
            for key in words(name, text)?.iter().filter_map(Value::as_str) {
                kept.push(crate::chord::chord(key).map_err(|why| anyhow!("{name} has \"{key}\": {why}"))?);
            }
            Value::Array(kept)
        }
        Kind::Colors => {
            let colors = words(name, text)?;
            if colors.len() != ANSI_COLORS {
                bail!("{name} has {} colors: it must have {ANSI_COLORS}", colors.len());
            }
            if let Some(bad) = colors.iter().filter_map(Value::as_str).find(|color| parse_color(color).is_none()) {
                bail!("{name} has \"{bad}\": not a color like #1a2b3c");
            }
            Value::Array(colors)
        }
    }))
}

/// A list: TOML's own form (`["a", "b, c"]`), or words with commas between them.
fn words(name: &str, text: &str) -> Result<Array> {
    if text.starts_with('[') {
        let value: Value = text.parse().map_err(|error| anyhow!("{name}: {error}"))?;
        let Value::Array(array) = value else { bail!("{name} is a list") };
        if array.iter().any(|word| !word.is_str()) {
            bail!("{name} is a list of words: each in quotes");
        }
        return Ok(array.iter().filter_map(Value::as_str).collect());
    }
    Ok(text.split(',').map(str::trim).filter(|word| !word.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KEYS, key};

    fn named(name: &str) -> &'static Key {
        key(name).unwrap()
    }

    fn opened(text: &str) -> (tempfile::TempDir, File) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        if !text.is_empty() {
            std::fs::write(&path, text).unwrap();
        }
        let file = File::open(&path).unwrap();
        (directory, file)
    }

    #[test]
    fn every_setting_is_named_and_every_name_is_a_setting() {
        // Each name takes a value of its kind in an empty file: so the terminal knows the name.
        for key in &KEYS {
            let (_directory, mut file) = opened("");
            let sample = match key.kind {
                Kind::Switch => "true".to_owned(),
                Kind::Whole { least, .. } => least.to_string(),
                Kind::Number { least, .. } => least.to_string(),
                Kind::Text | Kind::OptionalText => "text".to_owned(),
                Kind::Words => "one, two".to_owned(),
                // A provider needs a model: the choice that needs nothing else.
                Kind::OneOf(choices) => choices[0].to_owned(),
                Kind::Color => "#102030".to_owned(),
                Kind::Colors => vec!["#102030"; ANSI_COLORS].join(", "),
                // A key no command has by default.
                Kind::Keys => "ctrl-alt-f9".to_owned(),
            };
            file.set(key, &sample).unwrap_or_else(|error| panic!("{}: {error:#}", key.name));
            assert!(file.value(key).is_some(), "{} is in the file", key.name);
        }
        // Every setting the terminal reads has a name here: the defaults, written out, name only these
        // (a setting that is not set by default is not written, so those are counted apart).
        let all = toml::Value::try_from(Config::default()).unwrap();
        let mut written = Vec::new();
        for (section, table) in all.as_table().unwrap() {
            for field in table.as_table().unwrap().keys() {
                written.push(format!("{section}.{field}"));
            }
        }
        for name in &written {
            assert!(key(name).is_some(), "{name} has no entry in KEYS");
        }
        let unset_by_default = KEYS.iter().filter(|key| !written.iter().any(|name| name == key.name)).count();
        assert_eq!((written.len(), unset_by_default), (40, 19), "40 with a default written, 19 that are not set");
        let mut names: Vec<&str> = KEYS.iter().map(|key| key.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), KEYS.len(), "no name twice");
        // When a change is used: counted, so that a new setting is given its answer on purpose.
        let used = |when: crate::Applies| KEYS.iter().filter(|key| key.applies() == when).count();
        assert_eq!(
            (
                used(crate::Applies::AtOnce),
                used(crate::Applies::NextTab),
                used(crate::Applies::SidebarShown),
                used(crate::Applies::NextStart)
            ),
            (38, 3, 2, 16)
        );
    }

    #[test]
    fn a_value_is_shown_the_way_it_is_typed() {
        let (_directory, mut file) = opened("");
        assert_eq!(file.value(named("font.size")), None, "not in the file");
        assert_eq!(file.in_use(named("font.size")), "14.0", "the default");
        assert_eq!(default_of(named("agents.command")), "keepane, list-agents, -J");
        assert_eq!(default_of(named("font.line_height")), "1.35", "not 1.350000023841858");
        assert_eq!(default_of(named("ai.provider")), "none");
        assert_eq!(default_of(named("shell.program")), "", "not set");
        assert_eq!(default_of(named("theme.ansi")), "");
        for (name, typed, kept) in [
            ("font.size", "16", "16.0"),
            ("font.ligatures", "off", "false"),
            ("font.family", "JetBrains Mono", "JetBrains Mono"),
            ("terminal.scrollback", "50_000", "50000"),
            ("commands.system", "find,  sort", "find, sort"),
            ("agents.command", r#"["keepane", "-L", "a, b"]"#, r#"["keepane", "-L", "a, b"]"#),
            ("theme.background", "#000000", "#000000"),
            ("shell.program", "nu", "nu"),
            ("theme.name", "day", "day"),
            ("theme.bar", "#e8eff6", "#e8eff6"),
            ("window.tab_bar", "auto-hide", "auto-hide"),
            ("theme.opacity", "0.85", "0.85"),
            ("theme.opacity", "0.1", "0.1"),
            ("theme.opacity", "1", "1.0"),
            ("theme.blur", "on", "true"),
            ("theme.picture", "mine.png", "mine.png"),
            ("theme.rounding", "0", "0.0"),
            ("theme.corners", "square", "square"),
            ("keys.agents", "Ctrl+Shift+G", "ctrl-shift-g"),
            ("keys.paste", "ctrl-shift-v, Shift-Insert", "ctrl-shift-v, shift-insert"),
            ("keys.explain", "", ""),
        ] {
            file.set(named(name), typed).unwrap();
            assert_eq!(file.value(named(name)).as_deref(), Some(kept), "{name}");
            // What is shown, typed again, is the same value.
            file.set(named(name), kept).unwrap();
            assert_eq!(file.value(named(name)).as_deref(), Some(kept), "{name}, typed back");
        }
        assert!(Config::parse(&file.text()).is_ok());
    }

    #[test]
    fn a_wrong_value_changes_nothing_and_says_why() {
        let (_directory, mut file) = opened("[font]\nsize = 15\n");
        let before = file.text();
        for (name, typed, says) in [
            ("font.size", "200", "from 6 to 72"),
            ("font.size", "big", "is a number"),
            ("font.ligatures", "maybe", "true or false"),
            ("history.limit", "0", "from 1 to 20"),
            ("terminal.scrollback", "-1", "whole number"),
            ("ai.provider", "gpt", "one of none, anthropic, openai, openai-compatible, ollama"),
            ("theme.cursor", "red", "not a color"),
            ("theme.edge", "12", "not a color"),
            ("theme.name", "My Theme", "small letters, digits and -"),
            ("window.tab_bar", "sometimes", "one of always, auto-hide"),
            ("theme.opacity", "0", "from 0.1 to 1"),
            ("theme.rounding", "40", "from 0 to 18"),
            ("theme.corners", "pointed", "one of round, small, square"),
            ("keys.agents", "ctrl-shft-g", "'shft' is not ctrl, alt, shift or cmd"),
            ("keys.agents", "g", "the shell needs that key"),
            ("keys.agents", "ctrl-shift-t", "keys.new_tab and keys.agents both have"),
            ("theme.ansi", "#000000, #111111", "has 2 colors"),
            ("font.family", " ", "font.family is empty"),
            // Right for its kind, wrong for the settings as a whole: a provider needs a model.
            ("ai.provider", "anthropic", "ai.model is empty"),
            ("agents.command", "", "agents.command is empty"),
        ] {
            let error = format!("{:#}", file.set(named(name), typed).unwrap_err());
            assert!(error.contains(says), "{name} = {typed}: {error}");
            assert_eq!(file.text(), before, "{name} = {typed} left the file as it was");
        }
        assert!(!file.changed());
    }

    #[test]
    fn the_owners_comments_and_order_stay() {
        let text = "# my settings\n\n[font]\n# big, for the projector\nsize = 18 # points? no: pixels\nfamily = \"Menlo\"\n\n[history]\nlimit = 5\n";
        let (_directory, mut file) = opened(text);
        file.set(named("font.size"), "20").unwrap();
        file.set(named("terminal.padding"), "10").unwrap();
        file.reset(named("history.limit")).unwrap();
        assert_eq!(
            file.text(),
            "# my settings\n\n[font]\n# big, for the projector\nsize = 20.0 # points? no: pixels\nfamily = \"Menlo\"\n\n[terminal]\npadding = 10.0\n",
            "the comments stay, the changed value is in its place, the emptied [history] is gone"
        );
        assert!(file.changed());
        file.reset(named("font.size")).unwrap();
        file.reset(named("font.size")).unwrap();
        assert_eq!(file.value(named("font.size")), None, "taken out; taking it out again is not an error");
        // Optional ones are taken out by an empty value.
        file.set(named("shell.program"), "nu").unwrap();
        file.set(named("shell.program"), "").unwrap();
        assert_eq!(file.value(named("shell.program")), None);
    }

    #[test]
    fn saving_makes_the_file_and_its_folder_and_leaves_nothing_beside_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("made").join("config.toml");
        let mut file = File::open(&path).unwrap();
        assert!(!file.changed(), "a file that is not there is an empty one");
        file.set(named("font.size"), "12").unwrap();
        file.save().unwrap();
        assert!(!file.changed());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[font]\nsize = 12.0\n");
        let left: Vec<_> =
            std::fs::read_dir(path.parent().unwrap()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, ["config.toml"], "the file written beside it was moved into place");
        assert_eq!(Config::load(&path).unwrap().font.size.to_bits(), 12.0_f32.to_bits(), "the terminal reads it");
    }

    #[test]
    fn a_file_the_terminal_refuses_is_not_opened() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "[font]\nsise = 12\n").unwrap();
        let error = format!("{:#}", File::open(&path).unwrap_err());
        assert!(error.contains("sise") && error.contains("config.toml"), "{error}");
    }
}
