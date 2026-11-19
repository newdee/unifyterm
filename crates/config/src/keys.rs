//! Every setting by its name, for a program that shows and changes them.
//!
//! The settings themselves are the structs in the crate's root, which the
//! terminal reads. This list says the same thing to a person: what a
//! setting is called, what it may be and what it does. A test holds the two
//! together: every name here is a setting, and every setting is named here.

/// What a setting's value may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// `true` or `false`.
    Switch,
    /// A whole number from `least` to `most`.
    Whole { least: u64, most: u64 },
    /// A number from `least` to `most`.
    Number { least: f32, most: f32 },
    /// Any text.
    Text,
    /// Text, or nothing: left out of the file, the terminal chooses.
    OptionalText,
    /// Words in order: a program and its arguments, a list of fonts.
    Words,
    /// One of these.
    OneOf(&'static [&'static str]),
    /// A color as `#rrggbb`, or nothing: the built-in one.
    Color,
    /// The 16 terminal colors, or nothing: the built-in ones.
    Colors,
}

/// When a running UnifyTerm uses a setting that was changed. It reads the
/// file again whenever the file changes; what it does with the new value
/// depends on when the old one was put to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// Drawn from the settings at every frame: the font, the colors.
    AtOnce,
    /// Used when a tab is opened: its shell, how many lines it keeps.
    NextTab,
    /// Used when the agents' sidebar is shown.
    SidebarShown,
    /// Set up when the window opens: the commands, the history, the model, the title bar.
    NextStart,
}

impl Applies {
    /// In a few words, after "a change is used".
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::AtOnce => "at once",
            Self::NextTab => "by the next tab",
            Self::SidebarShown => "when the sidebar is shown again",
            Self::NextStart => "when UnifyTerm starts again",
        }
    }
}

/// One setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    /// `section.name`, as in the file: `[section]`, then `name = ...`.
    pub name: &'static str,
    pub kind: Kind,
    /// What it does, in a line.
    pub says: &'static str,
}

impl Key {
    /// The part before the dot: the table it is in.
    #[must_use]
    pub fn section(&self) -> &'static str {
        self.name.split_once('.').map_or(self.name, |(section, _)| section)
    }

    /// The part after the dot.
    #[must_use]
    pub fn field(&self) -> &'static str {
        self.name.split_once('.').map_or("", |(_, field)| field)
    }

    /// When a running UnifyTerm uses a change to it.
    #[must_use]
    pub fn applies(&self) -> Applies {
        match (self.section(), self.field()) {
            ("font" | "theme", _)
            | ("terminal", "padding")
            | ("history", "candidates" | "limit")
            | ("agents", "focus") => Applies::AtOnce,
            ("shell", _) | ("terminal", "scrollback") => Applies::NextTab,
            ("agents", "poll_ms" | "command") => Applies::SidebarShown,
            _ => Applies::NextStart,
        }
    }
}

/// The setting called `name`.
#[must_use]
pub fn key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.name == name)
}

const fn setting(name: &'static str, kind: Kind, says: &'static str) -> Key {
    Key { name, kind, says }
}

/// All of them, in the order they are shown.
pub const KEYS: [Key; 32] = [
    setting("font.family", Kind::Text, "The font."),
    setting("font.size", Kind::Number { least: 6.0, most: 72.0 }, "The font's size, in pixels."),
    setting("font.line_height", Kind::Number { least: 1.0, most: 2.0 }, "A line's height as a multiple of the size."),
    setting("font.ligatures", Kind::Switch, "-> and != and the like as one shape, where the font has them."),
    setting("font.fallbacks", Kind::Words, "Fonts tried, in order, for characters the font lacks."),
    setting(
        "shell.program",
        Kind::OptionalText,
        "The program a new tab runs. Not set: PowerShell on Windows, your login shell elsewhere.",
    ),
    setting(
        "shell.args",
        Kind::Words,
        "Its arguments. With arguments of your own the shell gets nothing from UnifyTerm: no history hook, no login start.",
    ),
    setting("terminal.scrollback", Kind::Whole { least: 0, most: 1_000_000 }, "Lines kept for each tab."),
    setting(
        "terminal.padding",
        Kind::Number { least: 0.0, most: 64.0 },
        "Pixels between the window's edge and the text.",
    ),
    setting(
        "window.system_title_bar",
        Kind::Switch,
        "The system's title bar above the tabs. Without it the row of tabs is the title bar. On Linux it is always there.",
    ),
    setting("theme.foreground", Kind::Color, "The color of text."),
    setting("theme.background", Kind::Color, "The color behind it."),
    setting("theme.cursor", Kind::Color, "The cursor's color."),
    setting("theme.selection", Kind::Color, "The color behind selected text."),
    setting(
        "theme.ansi",
        Kind::Colors,
        "The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.",
    ),
    setting("commands.builtin", Kind::Switch, "Put UnifyTerm's commands before the system's in each shell's PATH."),
    setting("commands.system", Kind::Words, "Commands that stay the system's although builtin is on: find, sort."),
    setting("history.enabled", Kind::Switch, "Keep the commands shells run. Off: no shell hook, no history file."),
    setting("history.candidates", Kind::Switch, "Show the list of earlier commands while typing."),
    setting("history.limit", Kind::Whole { least: 1, most: 20 }, "The most commands the list shows."),
    setting(
        "ai.provider",
        Kind::OneOf(&["none", "anthropic", "openai-compatible"]),
        "Which model helps with commands: anthropic (Claude), openai-compatible (a local model, for one), or none.",
    ),
    setting("ai.model", Kind::Text, "The model's name, as the service knows it: claude-opus-5-5."),
    setting(
        "ai.base_url",
        Kind::Text,
        "Where the service is: needed for openai-compatible; for anthropic, only through a proxy.",
    ),
    setting(
        "ai.api_key_env",
        Kind::Text,
        "The environment variable that holds the key. The key itself is never in this file.",
    ),
    setting("ai.suggest", Kind::Switch, "The rest of the command in grey while typing."),
    setting("ai.fix", Kind::Switch, "A corrected command when one is not found."),
    setting("ai.explain", Kind::Switch, "What each part of a command means (Ctrl-Shift-E)."),
    setting("agents.sidebar", Kind::Switch, "Show the agents' sidebar when the window opens."),
    setting(
        "agents.poll_ms",
        Kind::Whole { least: 500, most: 60_000 },
        "How often the agents are asked for while the sidebar shows, in milliseconds.",
    ),
    setting("agents.command", Kind::Words, "The program, with its arguments, that prints the agents as JSON."),
    setting("agents.focus", Kind::Words, "The program that brings an agent's pane to the front; {pane} is its number."),
    setting(
        "updates.check",
        Kind::Switch,
        "Once a day, ask GitHub whether a newer UnifyTerm is out, and name it in the row of tabs.",
    ),
];
