//! UnifyTerm's settings: `config.toml` in the user's configuration directory
//! (`%APPDATA%\unifyterm`, `~/Library/Application Support/unifyterm`,
//! `~/.config/unifyterm`). Every key is optional; an unknown key is an error,
//! so a misspelt one does not pass silently.
//!
//! Two programs use this: the terminal reads the settings, and `ut setting`
//! shows and changes them. Both take what a value may be from here, so that
//! what one writes the other accepts.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

mod edit;
mod keys;
pub mod providers;
pub mod shell;
pub mod themes;

pub use edit::{File, default_of};
pub use keys::{Applies, KEYS, Key, Kind, key};

const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;
const LINE_HEIGHTS: std::ops::RangeInclusive<f32> = 1.0..=2.0;
const MAX_SCROLLBACK: usize = 1_000_000;
const MAX_PADDING: f32 = 64.0;
/// Below this the text has nothing behind it to be read against.
const OPACITIES: std::ops::RangeInclusive<f32> = 0.1..=1.0;
/// A tab is 36 pixels high: past this its corners are half circles and more makes no difference.
const MAX_ROUNDING: f32 = 18.0;
/// `theme.ansi` is this many colors: the eight, then the bright eight.
pub const ANSI_COLORS: usize = 16;
/// The variable the terminal gives its shells: the settings file this window
/// reads, when it is not the default one. `ut setting` then changes that file.
pub const FILE_VARIABLE: &str = "UNIFYTERM_CONFIG";

/// `#rrggbb` (the `#` may be left out) as red, green, blue.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}
const CANDIDATE_LIMITS: std::ops::RangeInclusive<usize> = 1..=20;
const POLL_TIMES: std::ops::RangeInclusive<u64> = 500..=60_000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: FontConfig,
    pub shell: ShellConfig,
    pub terminal: TerminalConfig,
    pub window: WindowConfig,
    pub theme: ThemeConfig,
    pub commands: CommandsConfig,
    pub history: HistoryConfig,
    pub ai: AiConfig,
    pub agents: AgentsConfig,
    pub updates: UpdatesConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    pub family: String,
    /// In pixels.
    pub size: f32,
    /// A line's height as a multiple of `size`.
    pub line_height: f32,
    /// `->`, `!=` and the like drawn as one shape, where the font has them.
    pub ligatures: bool,
    /// Fonts tried, in order, for characters `family` lacks.
    pub fallbacks: Vec<String>,
}

impl Default for FontConfig {
    fn default() -> Self {
        let (family, fallbacks): (&str, &[&str]) = if cfg!(windows) {
            ("Cascadia Code", &["Microsoft YaHei UI", "Segoe UI Emoji", "Segoe UI Symbol"])
        } else if cfg!(target_os = "macos") {
            ("Menlo", &["PingFang SC", "Apple Color Emoji"])
        } else {
            ("DejaVu Sans Mono", &["Noto Sans CJK SC", "Noto Color Emoji"])
        };
        Self {
            family: family.into(),
            size: 14.0,
            line_height: 1.35,
            ligatures: true,
            fallbacks: fallbacks.iter().map(|&name| name.into()).collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    /// The program a new tab runs. Not set: PowerShell on Windows, `$SHELL` elsewhere.
    pub program: Option<String>,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    /// Lines of history kept for each tab.
    pub scrollback: usize,
    /// Pixels between the window's edge and the text.
    pub padding: f32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self { scrollback: 10_000, padding: 6.0 }
    }
}

/// The window itself.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    /// The system's title bar above the tabs. Without it (the default on
    /// Windows and macOS) the row of tabs is the title bar.
    pub system_title_bar: bool,
    pub tab_bar: TabBar,
}

/// When the row of tabs shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TabBar {
    #[default]
    Always,
    /// Out of the way until the pointer is at the window's top edge, or a tab
    /// is opened, closed or gone to. It then lies over the text: the
    /// terminal keeps its size.
    AutoHide,
}

/// UnifyTerm's own commands (`cat`, `ls`, `grep`, `jq`, ...): the same ones
/// on Windows, macOS and Linux.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CommandsConfig {
    /// Put UnifyTerm's commands before the system's in each shell's `PATH`.
    pub builtin: bool,
    /// Commands that stay the system's although `builtin` is on.
    pub system: Vec<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self { builtin: true, system: Vec::new() }
    }
}

/// The command history (one SQLite file in UnifyTerm's data directory) and
/// the list of commands from it shown while typing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    /// Keep the commands shells run. Off: no shell hook, no history file is written.
    pub enabled: bool,
    /// Show the list while typing at a prompt.
    pub candidates: bool,
    /// The most commands the list shows.
    pub limit: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { enabled: true, candidates: true, limit: 8 }
    }
}

/// Which language model helps with commands. None by default: nothing is
/// sent anywhere until the user names one here, and agrees once in the window.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    #[default]
    None,
    /// Claude, through Anthropic's Messages API.
    Anthropic,
    /// `OpenAI`'s own service.
    Openai,
    /// A service with `OpenAI`'s chat-completions shape, at the address `ai.base_url` gives.
    OpenaiCompatible,
    /// Model servers on this machine, each at its usual address.
    Ollama,
    LmStudio,
    Vllm,
    /// An agent's command line on this machine, with its tools off: it only answers.
    ClaudeCode,
    Codex,
    Pi,
    /// The program `ai.command` names: it reads the question, prints the answer.
    Command,
}

/// What happens to the command a sentence was turned into (`# words` at the prompt).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AskEnter {
    /// It is put on the input line; the user presses Enter.
    #[default]
    Manual,
    /// A command that only reads is run at once; any other waits for Enter.
    Auto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch of its own in the settings file, on or off whatever the others are"
)]
pub struct AiConfig {
    pub provider: Provider,
    /// The model's name, as the service knows it (`claude-opus-5-5`). An agent's program may go without.
    pub model: String,
    /// Where the service is. Needed for `openai-compatible`; else only to say another address than the provider's own.
    pub base_url: String,
    /// The environment variable that holds the key: the key itself is never in this file.
    pub api_key_env: String,
    /// The program and its arguments for the provider `command`. The question is written to its
    /// standard input, or put in place of `{prompt}` in an argument; the answer is what it prints.
    pub command: Vec<String>,
    /// The rest of the command, in grey, while typing.
    pub suggest: bool,
    /// A corrected command when one is not found.
    pub fix: bool,
    /// What each part of a command means (Ctrl-Shift-E).
    pub explain: bool,
    /// A line that begins with `#` is a sentence: Enter turns it into a command.
    pub ask: bool,
    pub ask_enter: AskEnter,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: Provider::None,
            model: String::new(),
            base_url: String::new(),
            api_key_env: String::new(),
            command: Vec::new(),
            suggest: true,
            fix: true,
            explain: true,
            ask: true,
            ask_enter: AskEnter::Manual,
        }
    }
}

/// The sidebar with the agents keepane watches (Ctrl-Shift-A).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    /// Show the sidebar when the window opens.
    pub sidebar: bool,
    /// How often the list is asked for while the sidebar shows, in milliseconds.
    pub poll_ms: u64,
    /// The program and its arguments that print the agents as JSON.
    pub command: Vec<String>,
    /// The program and its arguments that bring an agent's pane to the front; `{pane}` is its number.
    pub focus: Vec<String>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        let words = |words: &[&str]| words.iter().map(|&word| word.to_owned()).collect();
        Self {
            sidebar: false,
            poll_ms: 2000,
            command: words(&["keepane", "list-agents", "-J"]),
            focus: words(&["keepane", "focus-pane", "{pane}"]),
        }
    }
}

/// Whether UnifyTerm looks for a newer version of itself.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UpdatesConfig {
    /// Once a day, ask GitHub for the latest release; a newer one is named in the row of tabs.
    pub check: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self { check: true }
    }
}

/// How the window looks: a theme's name, colors as `#rrggbb`, and what is
/// not a color: how much shows through, whether that is blurred, a picture.
/// What is left out is the named theme's; with no theme named, the built-in
/// one's.
///
/// A theme file (see [`themes`]) is this same table without `name`: a theme
/// can give all of it, and the settings file's own keys go over the theme's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// The theme: one UnifyTerm brings, or a file in the `themes` folder beside the settings file.
    pub name: Option<String>,
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub cursor: Option<String>,
    pub selection: Option<String>,
    /// Behind the row of tabs and the sidebar.
    pub bar: Option<String>,
    /// Behind a list or a box that lies over the text, and a card.
    pub panel: Option<String>,
    /// Behind the tab in use, and what the pointer is over.
    pub well: Option<String>,
    /// A line between two parts.
    pub rule: Option<String>,
    /// The line around a list or a box.
    pub edge: Option<String>,
    /// Words that matter less.
    pub muted: Option<String>,
    /// The small signs: the one in the tab in use.
    pub symbol: Option<String>,
    /// The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub ansi: Option<Vec<String>>,
    /// How much of what is behind the window the color behind the text,
    /// the row of tabs and the sidebar hide: 1 is all of it. Text, lists and
    /// boxes are never seen through.
    pub opacity: Option<f32>,
    /// What shows through is blurred, as through frosted glass, where the system can do it.
    pub blur: Option<bool>,
    /// A picture's file, shown at the foot of the sidebar: beside the file
    /// that names it, or one of UnifyTerm's own by its name. Empty: none.
    pub picture: Option<String>,
    /// How round the corners of the tabs, the cards, the lists and the boxes
    /// are, in pixels: 0 is square.
    pub rounding: Option<f32>,
    /// The window's own corners, where the system lets a program choose (Windows 11).
    pub corners: Option<Corners>,
}

/// The window's own corners.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Corners {
    Round,
    /// Round, with a smaller radius.
    Small,
    Square,
}

/// The look the settings ask for, put together (see [`Config::look`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The theme's values with the settings' own over them.
    pub colors: ThemeConfig,
    /// The picture, where it was found to be.
    pub picture: Option<themes::Picture>,
}

impl ThemeConfig {
    /// Every key the table has, as the file writes it. A theme file's key
    /// that is not one of these is passed over (see [`themes::read`]).
    pub const KEYS: [&'static str; 18] = [
        "name",
        "foreground",
        "background",
        "cursor",
        "selection",
        "bar",
        "panel",
        "well",
        "rule",
        "edge",
        "muted",
        "symbol",
        "ansi",
        "opacity",
        "blur",
        "picture",
        "rounding",
        "corners",
    ];

    /// The single colors by their names in the file, in the file's order.
    fn single(&self) -> [(&'static str, &Option<String>); 11] {
        [
            ("foreground", &self.foreground),
            ("background", &self.background),
            ("cursor", &self.cursor),
            ("selection", &self.selection),
            ("bar", &self.bar),
            ("panel", &self.panel),
            ("well", &self.well),
            ("rule", &self.rule),
            ("edge", &self.edge),
            ("muted", &self.muted),
            ("symbol", &self.symbol),
        ]
    }

    /// Every color given is one, `ansi` has all 16, and the name is one a theme may have.
    fn check(&self) -> Result<()> {
        let color = |key: &str, text: &str| {
            parse_color(text)
                .map(|_| ())
                .with_context(|| format!("theme.{key} is \"{text}\": not a color like #1a2b3c"))
        };
        if let Some(name) = &self.name
            && !themes::is_name(name)
        {
            bail!("theme.name is \"{name}\": a theme's name is small letters, digits and -");
        }
        for (key, text) in self.single() {
            if let Some(text) = text {
                color(key, text)?;
            }
        }
        if let Some(ansi) = &self.ansi {
            if ansi.len() != ANSI_COLORS {
                bail!("theme.ansi has {} colors: it must have {ANSI_COLORS}", ansi.len());
            }
            for text in ansi {
                color("ansi", text)?;
            }
        }
        if let Some(opacity) = self.opacity
            && !OPACITIES.contains(&opacity)
        {
            bail!("theme.opacity is {opacity}: it must be from {} to {}", OPACITIES.start(), OPACITIES.end());
        }
        if let Some(rounding) = self.rounding
            && !(0.0..=MAX_ROUNDING).contains(&rounding)
        {
            bail!("theme.rounding is {rounding}: it must be from 0 to {MAX_ROUNDING}");
        }
        Ok(())
    }

    /// These colors, with `under`'s where these leave one out.
    #[must_use]
    pub fn over(self, under: Self) -> Self {
        Self {
            name: self.name.or(under.name),
            foreground: self.foreground.or(under.foreground),
            background: self.background.or(under.background),
            cursor: self.cursor.or(under.cursor),
            selection: self.selection.or(under.selection),
            bar: self.bar.or(under.bar),
            panel: self.panel.or(under.panel),
            well: self.well.or(under.well),
            rule: self.rule.or(under.rule),
            edge: self.edge.or(under.edge),
            muted: self.muted.or(under.muted),
            symbol: self.symbol.or(under.symbol),
            ansi: self.ansi.or(under.ansi),
            opacity: self.opacity.or(under.opacity),
            blur: self.blur.or(under.blur),
            picture: self.picture.or(under.picture),
            rounding: self.rounding.or(under.rounding),
            corners: self.corners.or(under.corners),
        }
    }
}

impl Config {
    /// Where the settings file is unless `--config` names another.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("unifyterm").join("config.toml"))
    }

    /// The settings in `path`. A file that is not there gives the defaults;
    /// one that cannot be read or is wrong is an error.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        if !FONT_SIZES.contains(&config.font.size) {
            bail!("font.size is {}: it must be from {} to {}", config.font.size, FONT_SIZES.start(), FONT_SIZES.end());
        }
        if !LINE_HEIGHTS.contains(&config.font.line_height) {
            let (low, high) = (LINE_HEIGHTS.start(), LINE_HEIGHTS.end());
            bail!("font.line_height is {}: it must be from {low} to {high}", config.font.line_height);
        }
        if config.font.family.trim().is_empty() {
            bail!("font.family is empty");
        }
        if config.terminal.scrollback > MAX_SCROLLBACK {
            bail!("terminal.scrollback is {}: it must be at most {MAX_SCROLLBACK}", config.terminal.scrollback);
        }
        if !(0.0..=MAX_PADDING).contains(&config.terminal.padding) {
            bail!("terminal.padding is {}: it must be from 0 to {MAX_PADDING}", config.terminal.padding);
        }

        if !CANDIDATE_LIMITS.contains(&config.history.limit) {
            let (low, high) = (CANDIDATE_LIMITS.start(), CANDIDATE_LIMITS.end());
            bail!("history.limit is {}: it must be from {low} to {high}", config.history.limit);
        }
        if !POLL_TIMES.contains(&config.agents.poll_ms) {
            let (low, high) = (POLL_TIMES.start(), POLL_TIMES.end());
            bail!("agents.poll_ms is {}: it must be from {low} to {high}", config.agents.poll_ms);
        }
        for (key, words) in [("agents.command", &config.agents.command), ("agents.focus", &config.agents.focus)] {
            if words.first().is_none_or(|program| program.trim().is_empty()) {
                bail!("{key} is empty: it is a program and its arguments, as [\"keepane\", ...]");
            }
        }
        config.check_model()?;
        config.theme.check()?;
        Ok(config)
    }

    /// The look in use: the named theme's, with what the settings give
    /// themselves over it. `themes` is the folder of the user's own themes
    /// (`themes` beside the settings file). No theme named: what the settings
    /// give alone (the built-in theme is under it).
    ///
    /// A picture is looked for beside the file that names it: the settings
    /// file for the settings' own, the theme's file for the theme's.
    pub fn look(&self, themes: Option<&Path>) -> Result<Look> {
        let (under, theme_folder) = match &self.theme.name {
            Some(name) => {
                let (under, source) = themes::found(name, themes)?;
                let folder = match source {
                    themes::Source::File(file) => file.parent().map(Path::to_path_buf),
                    themes::Source::BuiltIn => None,
                };
                (Some(under), folder)
            }
            None => (None, None),
        };
        let theirs = under.as_ref().and_then(|under| under.picture.as_deref());
        let picture = match (self.theme.picture.as_deref(), theirs) {
            (Some(own), _) => themes::picture(own, themes.and_then(Path::parent)),
            (None, Some(theirs)) => themes::picture(theirs, theme_folder.as_deref()),
            (None, None) => None,
        };
        let colors = match under {
            Some(under) => self.theme.clone().over(under),
            None => self.theme.clone(),
        };
        Ok(Look { colors, picture })
    }

    /// What the provider named needs is there: a model, an address, a key's variable, a program.
    fn check_model(&self) -> Result<()> {
        use providers::{Key, Reach};
        let empty = |text: &str| text.trim().is_empty();
        let reach = self.ai.provider.reach();
        if matches!(reach, Reach::Anthropic | Reach::Chat { .. }) && empty(&self.ai.model) {
            bail!("ai.model is empty: name the model, as the service knows it");
        }
        match reach {
            Reach::Chat { address: None, .. } if empty(&self.ai.base_url) => {
                bail!("ai.base_url is empty: an openai-compatible service needs its address")
            }
            Reach::Anthropic | Reach::Chat { key: Key::Needed, .. } if empty(&self.ai.api_key_env) => {
                bail!("ai.api_key_env is empty: name the environment variable that holds the key")
            }
            Reach::OwnProgram if self.ai.command.first().is_none_or(|program| empty(program)) => {
                bail!(
                    "ai.command is empty: it is the program that answers and its arguments, as [\"my-agent\", \"--quiet\"]"
                )
            }
            _ => Ok(()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_gives_the_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!((config.font.size, config.font.ligatures, config.terminal.scrollback), (14.0, true, 10_000));
        assert_eq!(config.shell.program, None);
        assert_eq!(
            config.commands,
            CommandsConfig { builtin: true, system: Vec::new() },
            "UnifyTerm's commands are on"
        );
        assert_eq!(config.history, HistoryConfig { enabled: true, candidates: true, limit: 8 }, "and the history");
        assert_eq!(config.ai.provider, Provider::None, "no model is asked anything until one is named");
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "a size read from text is the same bits as the literal it is compared with")]
    fn set_keys_replace_the_defaults() {
        let config = Config::parse(
            r##"
            [font]
            family = "JetBrains Mono"
            size = 16
            ligatures = false
            [shell]
            program = "nu"
            args = ["-l"]
            [terminal]
            scrollback = 500
            [theme]
            background = "#000000"
            [commands]
            builtin = false
            system = ["curl", "tar"]
            [history]
            candidates = false
            limit = 3
            "##,
        )
        .unwrap();
        assert_eq!(
            (config.font.family.as_str(), config.font.size, config.font.ligatures),
            ("JetBrains Mono", 16.0, false)
        );
        assert_eq!(config.font.line_height, FontConfig::default().line_height, "not set: the default");
        assert_eq!(
            (config.shell.program.as_deref(), config.shell.args.as_slice()),
            (Some("nu"), &["-l".to_string()][..])
        );
        assert_eq!(config.terminal.scrollback, 500);
        assert_eq!(config.commands, CommandsConfig { builtin: false, system: vec!["curl".into(), "tar".into()] });
        assert_eq!(config.history, HistoryConfig { enabled: true, candidates: false, limit: 3 });
        assert_eq!(config.theme.background.as_deref(), Some("#000000"));
        assert_eq!(parse_color("#1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(parse_color("1a2b3c"), Some([0x1a, 0x2b, 0x3c]), "the # may be left out");
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#12345é"), None);
    }

    #[test]
    fn wrong_values_are_refused_with_the_key_named() {
        let error = |text: &str| format!("{:#}", Config::parse(text).unwrap_err());
        assert!(error("[font]\nsize = 2").contains("font.size is 2"));
        assert!(error("[font]\nsize = 200").contains("font.size"));
        assert!(error("[font]\nline_height = 3").contains("font.line_height"));
        assert!(error("[font]\nfamily = \" \"").contains("font.family"));
        assert!(error("[terminal]\nscrollback = 5000000").contains("terminal.scrollback"));
        assert!(error("[terminal]\npadding = -1").contains("terminal.padding"));
        assert!(!Config::default().window.system_title_bar, "the tabs are the title bar unless asked");
        assert!(Config::parse("[window]\nsystem_title_bar = true").unwrap().window.system_title_bar);
        assert!(error("[theme]\ncursor = \"red\"").contains("theme.cursor is \"red\""));
        assert!(error("[theme]\nansi = [\"#000000\"]").contains("theme.ansi has 1 colors"));
        assert!(error("[font]\nsise = 12").contains("sise"), "a misspelt key is named");
        assert!(error("[fonts]").contains("fonts"));
        assert!(error("[commands]\nbuiltin = \"yes\"").contains("builtin"), "a switch is true or false");
        assert!(error("[history]\nlimit = 0").contains("history.limit is 0"));
        assert!(error("[history]\nlimit = 21").contains("history.limit"));
        assert!(error("[ai]\nprovider = \"anthropic\"").contains("ai.model is empty"));
        assert!(error("[ai]\nprovider = \"anthropic\"\nmodel = \"m\"").contains("ai.api_key_env"));
        assert!(error("[ai]\nprovider = \"openai-compatible\"\nmodel = \"m\"").contains("ai.base_url"));
        assert!(error("[ai]\nprovider = \"gpt\"").contains("gpt"), "an unknown provider is named");
        assert!(error("[ai]\nprovider = \"ollama\"").contains("ai.model is empty"));
        assert!(error("[ai]\nprovider = \"openai\"\nmodel = \"m\"").contains("ai.api_key_env"));
        assert!(error("[ai]\nprovider = \"command\"").contains("ai.command is empty"));
        assert!(error("[ai]\nprovider = \"command\"\ncommand = [\" \"]").contains("ai.command is empty"));
        assert!(error("[ai]\nask_enter = \"always\"").contains("always"), "an unknown way to enter is named");
        for fine in [
            "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:1.5b\"",
            "[ai]\nprovider = \"lm-studio\"\nmodel = \"m\"",
            "[ai]\nprovider = \"vllm\"\nmodel = \"m\"\nbase_url = \"http://box:8000/v1\"",
            "[ai]\nprovider = \"openai\"\nmodel = \"m\"\napi_key_env = \"OPENAI_API_KEY\"",
            "[ai]\nprovider = \"claude-code\"",
            "[ai]\nprovider = \"codex\"\nmodel = \"gpt-5\"",
            "[ai]\nprovider = \"pi\"",
            "[ai]\nprovider = \"command\"\ncommand = [\"my-agent\", \"{prompt}\"]",
            // What another provider does not use may stay in the file: changing the provider back finds it again.
            "[ai]\nprovider = \"ollama\"\nmodel = \"m\"\ncommand = [\"my-agent\"]",
        ] {
            Config::parse(fine).unwrap_or_else(|error| panic!("{fine}: {error:#}"));
        }
        let ask = Config::parse("[ai]\nask = false\nask_enter = \"auto\"").unwrap().ai;
        assert_eq!((ask.ask, ask.ask_enter), (false, AskEnter::Auto));
        assert_eq!((AiConfig::default().ask, AiConfig::default().ask_enter), (true, AskEnter::Manual));
        assert!(error("[agents]\npoll_ms = 100").contains("agents.poll_ms is 100"));
        assert!(error("[agents]\ncommand = []").contains("agents.command is empty"));
        assert!(error("[agents]\nfocus = [\"\"]").contains("agents.focus is empty"));
        let agents =
            Config::parse("[agents]\nsidebar = true\ncommand = [\"keepane\", \"-L\", \"x\", \"list-agents\", \"-J\"]")
                .unwrap()
                .agents;
        assert!(agents.sidebar);
        assert_eq!((agents.command.len(), agents.poll_ms, agents.focus[1].as_str()), (5, 2000, "focus-pane"));
        let fine = Config::parse("[ai]\nprovider = \"openai-compatible\"\nmodel = \"llama3\"\nbase_url = \"http://localhost:11434/v1\"\nsuggest = false").unwrap();
        assert_eq!((fine.ai.provider, fine.ai.suggest, fine.ai.fix), (Provider::OpenaiCompatible, false, true));
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_bad_one_names_itself() {
        let dir = std::env::temp_dir().join(format!("unifyterm-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&dir.join("none.toml")).unwrap(), Config::default());
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "[font]\nsize = \"big\"").unwrap();
        assert!(format!("{:#}", Config::load(&bad).unwrap_err()).contains("bad.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
