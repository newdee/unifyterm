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
    /// Keys, each with the keys held with it (`ctrl-shift-t`); none: no key.
    Keys,
    /// The prompt's segments, in order: `dir`, `git`, ... ([`crate::prompt::Segment`]); none: an empty side.
    Segments,
    /// From `least` to `most` colors, each one of the theme's 16 by its name or `#rrggbb`
    /// ([`crate::prompt::Paint`]), or nothing: the theme's.
    Paints { least: usize, most: usize },
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
            // A tab's shell is told when it starts whether UnifyTerm writes its prompt; how the
            // prompt looks is read again at every prompt (below).
            ("shell", _) | ("terminal", "scrollback") | ("prompt", "enabled") => Applies::NextTab,
            // The window starts its mail sync again with new mail settings.
            ("font" | "theme" | "keys" | "prompt" | "mail", _)
            | ("terminal", "padding")
            | ("window", "tab_bar")
            | ("history", "candidates" | "limit")
            | ("agents", "focus") => Applies::AtOnce,
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

/// A segment's colors: one, or two for a gradient of its own.
const SEGMENT_PAINTS: Kind = Kind::Paints { least: 1, most: 2 };

/// All of them, in the order they are shown.
pub const KEYS: [Key; 88] = [
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
    setting(
        "window.tab_bar",
        Kind::OneOf(&["always", "auto-hide"]),
        "The row of tabs: always there, or out of the way until the pointer is at the window's top edge.",
    ),
    setting(
        "keys.new_tab",
        Kind::Keys,
        "The keys for a new tab. Written as ctrl-shift-t; several with commas; none: no key.",
    ),
    setting("keys.close_tab", Kind::Keys, "The keys that close the tab."),
    setting("keys.next_tab", Kind::Keys, "The keys for the next tab."),
    setting("keys.previous_tab", Kind::Keys, "The keys for the tab before."),
    setting("keys.copy", Kind::Keys, "The keys that copy the selection."),
    setting("keys.paste", Kind::Keys, "The keys that paste."),
    setting("keys.agents", Kind::Keys, "The keys that show or hide the agents' sidebar."),
    setting("keys.tab_bar", Kind::Keys, "The keys that put the row of tabs out of the way, or back."),
    setting("keys.full_screen", Kind::Keys, "The keys for full screen, and out of it."),
    setting("keys.explain", Kind::Keys, "The keys that explain the command on the line."),
    setting("keys.mail", Kind::Keys, "The keys that show the mail in the window, or the terminal again."),
    setting(
        "theme.name",
        Kind::OptionalText,
        "The theme: one of UnifyTerm's, or a file in the themes folder (ut theme). Not set: paper-dark.",
    ),
    setting("theme.foreground", Kind::Color, "The color of text."),
    setting("theme.background", Kind::Color, "The color behind it."),
    setting("theme.cursor", Kind::Color, "The cursor's color."),
    setting("theme.selection", Kind::Color, "The color behind selected text."),
    setting("theme.bar", Kind::Color, "The color behind the row of tabs and the sidebar."),
    setting("theme.panel", Kind::Color, "The color behind a list or a box over the text, and behind a card."),
    setting("theme.well", Kind::Color, "The color behind the tab in use, and behind what the pointer is over."),
    setting("theme.rule", Kind::Color, "The color of a line between two parts."),
    setting("theme.edge", Kind::Color, "The color of the line around a list or a box."),
    setting("theme.muted", Kind::Color, "The color of words that matter less."),
    setting("theme.symbol", Kind::Color, "The color of the small signs, as the one in the tab in use."),
    setting(
        "theme.ansi",
        Kind::Colors,
        "The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.",
    ),
    setting(
        "theme.opacity",
        Kind::Number { least: 0.1, most: 1.0 },
        "How much the window hides what is behind it: 1 hides all. Text, lists and boxes are never seen through.",
    ),
    setting("theme.blur", Kind::Switch, "What shows through the window is blurred, as through frosted glass."),
    setting(
        "theme.picture",
        Kind::OptionalText,
        "A picture's file, shown at the foot of the sidebar: beside this file, or one of UnifyTerm's own (aurora.png).",
    ),
    setting(
        "theme.rounding",
        Kind::Number { least: 0.0, most: 18.0 },
        "How round the corners of the tabs, the cards, the lists and the boxes are, in pixels: 0 is square.",
    ),
    setting(
        "theme.corners",
        Kind::OneOf(&["round", "small", "square"]),
        "The window's own corners: round, round with a smaller radius, or square. Windows 11 only.",
    ),
    setting(
        "theme.wallpaper",
        Kind::OptionalText,
        "A picture's file, drawn behind the text over the whole window and made as large as the window.",
    ),
    setting(
        "theme.wallpaper_opacity",
        Kind::Number { least: 0.0, most: 1.0 },
        "How much of the wallpaper shows over the color behind the text: 0 nothing, 1 all of it. Not set: 0.25.",
    ),
    setting(
        "prompt.enabled",
        Kind::Switch,
        "UnifyTerm writes the prompt (the line left of the cursor) in new tabs: PowerShell, bash, zsh, fish. Off: your own.",
    ),
    setting(
        "prompt.style",
        Kind::OneOf(&["lean", "classic", "rainbow", "plain"]),
        "lean: colored words; classic: one band, thin lines; rainbow: a color for each segment; plain: ASCII only. Not set: the theme's, else lean.",
    ),
    setting(
        "prompt.separator",
        Kind::OneOf(&["sharp", "round", "slant"]),
        "The shape between segments of classic and rainbow. UnifyTerm draws it: no font needs it.",
    ),
    setting(
        "prompt.lines",
        Kind::Whole { least: 1, most: 2 },
        "1: the input after the segments. 2: the segments above, the input below. Not set: 2.",
    ),
    setting(
        "prompt.left",
        Kind::Segments,
        "The segments on the left, in order: dir, git, status, duration, venv, time.",
    ),
    setting(
        "prompt.right",
        Kind::Segments,
        "The segments on the right: at the end of the first line with 2 lines; at the line's end in zsh and fish with 1.",
    ),
    setting(
        "prompt.gradient",
        Kind::OneOf(&["off", "segments", "whole"]),
        "off: each segment in its color; segments: each to the next one's color; whole: prompt.colors over the whole side.",
    ),
    setting(
        "prompt.colors",
        Kind::Paints { least: 2, most: 8 },
        "The colors of gradient whole, left to right: the theme's by name (cyan, bright-blue) or #rrggbb.",
    ),
    setting("prompt.dir_color", SEGMENT_PAINTS, "The directory's color; two: a gradient of its own. Not set: blue."),
    setting("prompt.git_color", SEGMENT_PAINTS, "Git's color with no changes. Not set: green."),
    setting("prompt.git_dirty_color", SEGMENT_PAINTS, "Git's color with changes. Not set: yellow."),
    setting("prompt.status_color", SEGMENT_PAINTS, "The color of a failed command's exit code. Not set: red."),
    setting("prompt.duration_color", SEGMENT_PAINTS, "The color of how long the command took. Not set: yellow."),
    setting("prompt.venv_color", SEGMENT_PAINTS, "The Python environment's color. Not set: cyan."),
    setting("prompt.time_color", SEGMENT_PAINTS, "The time's color. Not set: bright-black."),
    setting(
        "prompt.shine",
        Kind::OneOf(&["off", "once", "loop"]),
        "A light crosses the newest prompt's words: once for each new prompt, or again every prompt.shine_every seconds.",
    ),
    setting(
        "prompt.shine_every",
        Kind::Number { least: 1.0, most: 600.0 },
        "Seconds between two lights, with shine loop. Not set: 4.",
    ),
    setting("commands.builtin", Kind::Switch, "Put UnifyTerm's commands before the system's in each shell's PATH."),
    setting("commands.system", Kind::Words, "Commands that stay the system's although builtin is on: find, sort."),
    setting("history.enabled", Kind::Switch, "Keep the commands shells run. Off: no shell hook, no history file."),
    setting("history.candidates", Kind::Switch, "Show the list of earlier commands while typing."),
    setting("history.limit", Kind::Whole { least: 1, most: 20 }, "The most commands the list shows."),
    setting(
        "ai.provider",
        Kind::OneOf(&crate::providers::NAMES),
        "Which model helps with commands: a service (anthropic, openai, openai-compatible), a model server on this machine (ollama, lm-studio, vllm), an agent's command line (claude-code, codex, pi, command), or none.",
    ),
    setting(
        "ai.model",
        Kind::Text,
        "The model's name, as the service knows it: claude-opus-5-5. An agent's command line uses its own when none is named.",
    ),
    setting(
        "ai.base_url",
        Kind::Text,
        "Where the service is: needed for openai-compatible; else only for another address than the provider's own.",
    ),
    setting(
        "ai.api_key_env",
        Kind::Text,
        "The environment variable that holds the key. The key itself is never in this file.",
    ),
    setting(
        "ai.command",
        Kind::Words,
        "For the provider command: the program that answers, with its arguments. It reads the question, or gets it in place of {prompt}.",
    ),
    setting(
        "ai.suggest",
        Kind::Switch,
        "The rest of the command in grey while typing. Not from an agent's command line: that is too slow.",
    ),
    setting("ai.fix", Kind::Switch, "A corrected command when one is not found."),
    setting("ai.explain", Kind::Switch, "What each part of a command means (Ctrl-Shift-E)."),
    setting("ai.ask", Kind::Switch, "A line that begins with # is a sentence: Enter turns it into a command."),
    setting(
        "ai.ask_enter",
        Kind::OneOf(&["manual", "auto"]),
        "The command made from a sentence: manual puts it on the line; auto also runs it when it only reads.",
    ),
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
    setting("mail.address", Kind::Text, "Your mail address: me@example.com. Empty: no mail."),
    setting(
        "mail.imap",
        Kind::Text,
        "The IMAP server as host:port (993). Empty: the address's domain's: qq.com, 163.com, 126.com, gmail.com, icloud.com and others.",
    ),
    setting("mail.smtp", Kind::Text, "The SMTP server as host:port (465, or 587 with STARTTLS). Empty: as mail.imap."),
    setting("mail.login", Kind::Text, "The name to log in with. Empty: the address."),
    setting(
        "mail.password_command",
        Kind::Words,
        "A program that prints the password (an app password or authorization code). Empty: the one ut mail login keeps in the keychain.",
    ),
    setting(
        "mail.proxy",
        Kind::Text,
        "socks5://host:port or http://host:port: the servers are reached through it, as for a mailbox abroad. Empty: directly.",
    ),
    setting("mail.days", Kind::Whole { least: 1, most: 3650 }, "How many days of mail the first sync reads."),
    setting(
        "mail.on_new",
        Kind::Words,
        "A program run for each new mail: {thread} {from} {subject} {snippet}; the mail as JSON on its input.",
    ),
    setting("mail.notify", Kind::Switch, "A note in the window when mail comes."),
];
