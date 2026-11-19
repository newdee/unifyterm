//! The settings file: `config.toml` in the user's configuration directory
//! (`%APPDATA%\unifyterm`, `~/Library/Application Support/unifyterm`,
//! `~/.config/unifyterm`). Every key is optional; an unknown key is an error,
//! so a misspelt one does not pass silently.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::theme::{Rgb, Theme};

const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;
const LINE_HEIGHTS: std::ops::RangeInclusive<f32> = 1.0..=2.0;
const MAX_SCROLLBACK: usize = 1_000_000;
const MAX_PADDING: f32 = 64.0;
const CANDIDATE_LIMITS: std::ops::RangeInclusive<usize> = 1..=20;
const POLL_TIMES: std::ops::RangeInclusive<u64> = 500..=60_000;

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) font: FontConfig,
    pub(crate) shell: ShellConfig,
    pub(crate) terminal: TerminalConfig,
    pub(crate) window: WindowConfig,
    pub(crate) theme: ThemeConfig,
    pub(crate) commands: CommandsConfig,
    pub(crate) history: HistoryConfig,
    pub(crate) ai: AiConfig,
    pub(crate) agents: AgentsConfig,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FontConfig {
    pub(crate) family: String,
    /// In pixels.
    pub(crate) size: f32,
    /// A line's height as a multiple of `size`.
    pub(crate) line_height: f32,
    /// `->`, `!=` and the like drawn as one shape, where the font has them.
    pub(crate) ligatures: bool,
    /// Fonts tried, in order, for characters `family` lacks.
    pub(crate) fallbacks: Vec<String>,
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

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ShellConfig {
    /// The program a new tab runs. Not set: PowerShell on Windows, `$SHELL` elsewhere.
    pub(crate) program: Option<String>,
    pub(crate) args: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TerminalConfig {
    /// Lines of history kept for each tab.
    pub(crate) scrollback: usize,
    /// Pixels between the window's edge and the text.
    pub(crate) padding: f32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self { scrollback: 10_000, padding: 6.0 }
    }
}

/// The window itself.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WindowConfig {
    /// The system's title bar above the tabs. Without it (the default on
    /// Windows and macOS) the row of tabs is the title bar.
    pub(crate) system_title_bar: bool,
}

/// UnifyTerm's own commands (`cat`, `ls`, `grep`, `jq`, ...): the same ones
/// on Windows, macOS and Linux.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CommandsConfig {
    /// Put UnifyTerm's commands before the system's in each shell's `PATH`.
    pub(crate) builtin: bool,
    /// Commands that stay the system's although `builtin` is on.
    pub(crate) system: Vec<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self { builtin: true, system: Vec::new() }
    }
}

/// The command history (one SQLite file in UnifyTerm's data directory) and
/// the list of commands from it shown while typing.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct HistoryConfig {
    /// Keep the commands shells run. Off: no shell hook, no history file is written.
    pub(crate) enabled: bool,
    /// Show the list while typing at a prompt.
    pub(crate) candidates: bool,
    /// The most commands the list shows.
    pub(crate) limit: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { enabled: true, candidates: true, limit: 8 }
    }
}

/// Which language model helps with commands. None by default: nothing is
/// sent anywhere until the user names one here, and agrees once in the window.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Provider {
    #[default]
    None,
    /// Claude, through Anthropic's Messages API.
    Anthropic,
    /// A service with `OpenAI`'s chat-completions shape: a local model, for one.
    OpenaiCompatible,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AiConfig {
    pub(crate) provider: Provider,
    /// The model's name, as the service knows it (`claude-opus-5-5`).
    pub(crate) model: String,
    /// Where the service is. Needed for `openai-compatible`; for `anthropic`, only to go through a proxy.
    pub(crate) base_url: String,
    /// The environment variable that holds the key: the key itself is never in this file.
    pub(crate) api_key_env: String,
    /// The rest of the command, in grey, while typing.
    pub(crate) suggest: bool,
    /// A corrected command when one is not found.
    pub(crate) fix: bool,
    /// What each part of a command means (Ctrl-Shift-E).
    pub(crate) explain: bool,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: Provider::None,
            model: String::new(),
            base_url: String::new(),
            api_key_env: String::new(),
            suggest: true,
            fix: true,
            explain: true,
        }
    }
}

/// The sidebar with the agents keepane watches (Ctrl-Shift-A).
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AgentsConfig {
    /// Show the sidebar when the window opens.
    pub(crate) sidebar: bool,
    /// How often the list is asked for while the sidebar shows, in milliseconds.
    pub(crate) poll_ms: u64,
    /// The program and its arguments that print the agents as JSON.
    pub(crate) command: Vec<String>,
    /// The program and its arguments that bring an agent's pane to the front; `{pane}` is its number.
    pub(crate) focus: Vec<String>,
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

/// Colors as `#rrggbb`. Any left out keep the built-in theme's value.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ThemeConfig {
    pub(crate) foreground: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) cursor: Option<String>,
    pub(crate) selection: Option<String>,
    /// The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub(crate) ansi: Option<Vec<String>>,
}

impl Config {
    /// Where the settings file is unless `--config` names another.
    pub(crate) fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("unifyterm").join("config.toml"))
    }

    /// The settings in `path`. A file that is not there gives the defaults;
    /// one that cannot be read or is wrong is an error.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub(crate) fn parse(text: &str) -> Result<Self> {
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
        if config.ai.provider != Provider::None {
            if config.ai.model.trim().is_empty() {
                bail!("ai.model is empty: name the model, as the service knows it");
            }
            if config.ai.provider == Provider::OpenaiCompatible && config.ai.base_url.trim().is_empty() {
                bail!("ai.base_url is empty: an openai-compatible service needs its address");
            }
            if config.ai.provider == Provider::Anthropic && config.ai.api_key_env.trim().is_empty() {
                bail!("ai.api_key_env is empty: name the environment variable that holds the key");
            }
        }
        config.theme()?;
        Ok(config)
    }

    /// The built-in theme with this file's colors over it.
    pub(crate) fn theme(&self) -> Result<Theme> {
        let mut theme = Theme::default();
        let color = |key: &str, text: &str| {
            Rgb::parse(text).with_context(|| format!("theme.{key} is \"{text}\": not a color like #1a2b3c"))
        };
        let named = [
            ("foreground", &self.theme.foreground, &mut theme.foreground),
            ("background", &self.theme.background, &mut theme.background),
            ("cursor", &self.theme.cursor, &mut theme.cursor),
            ("selection", &self.theme.selection, &mut theme.selection),
        ];
        for (key, text, slot) in named {
            if let Some(text) = text {
                *slot = color(key, text)?;
            }
        }
        if let Some(ansi) = &self.theme.ansi {
            if ansi.len() != theme.ansi.len() {
                bail!("theme.ansi has {} colors: it must have {}", ansi.len(), theme.ansi.len());
            }
            for (slot, text) in theme.ansi.iter_mut().zip(ansi) {
                *slot = color("ansi", text)?;
            }
        }
        Ok(theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_gives_the_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.theme().unwrap(), Theme::default());
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
        let theme = config.theme().unwrap();
        assert_eq!((theme.background, theme.foreground), (Rgb(0, 0, 0), Theme::default().foreground));
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
