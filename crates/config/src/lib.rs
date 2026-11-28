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

pub mod chord;
mod edit;
mod keys;
pub mod prompt;
pub mod providers;
pub mod shell;
pub mod themes;

pub use edit::{File, default_of};
pub use keys::{Applies, KEYS, Key, Kind, key};
pub use prompt::PromptConfig;

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

/// The bar's color, when a theme leaves it out, is this far from the
/// background toward the text's color. The window and the prompt both work it
/// out from here.
pub const BAR_AMOUNT: f32 = 0.035;

/// `from` moved `amount` (0 to 1) of the way to `to`, each channel rounded and held to 0..=255.
#[must_use]
pub fn toward(from: [u8; 3], to: [u8; 3], amount: f32) -> [u8; 3] {
    [0, 1, 2].map(|at| channel(f32::from(from[at]) + (f32::from(to[at]) - f32::from(from[at])) * amount))
}

/// A computed channel value as a channel: rounded, and held to 0..=255.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to a u8's range first, and float-to-int `as` maps NaN to 0"
)]
fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

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
const MAIL_DAYS: std::ops::RangeInclusive<u32> = 1..=3650;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: FontConfig,
    pub shell: ShellConfig,
    pub terminal: TerminalConfig,
    pub window: WindowConfig,
    pub keys: KeysConfig,
    pub theme: ThemeConfig,
    pub prompt: PromptConfig,
    pub commands: CommandsConfig,
    pub history: HistoryConfig,
    pub ai: AiConfig,
    pub agents: AgentsConfig,
    pub updates: UpdatesConfig,
    pub mail: MailConfig,
    /// Default keys that gave way to a key the file gives another command
    /// ([`KeysConfig::give_way`]), each said as the window's first tab says it.
    #[serde(skip)]
    pub keys_given_way: Vec<String>,
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

/// The window's own commands and the keys each has, when the settings do not
/// say: its name in `[keys]`, its keys on Windows and Linux, its keys on
/// macOS. Ctrl-Shift on Windows and Linux, where Ctrl alone belongs to the
/// program in the terminal; Cmd on macOS. Full screen: F11 and Alt-Enter as
/// in Windows Terminal; Cmd-Enter and Ctrl-Cmd-F, the Mac's own key.
pub const DEFAULT_KEYS: [(&str, &[&str], &[&str]); 13] = [
    ("new_tab", &["ctrl-shift-t"], &["cmd-t"]),
    ("close_tab", &["ctrl-shift-w"], &["cmd-w"]),
    ("next_tab", &["ctrl-tab"], &["ctrl-tab"]),
    ("previous_tab", &["ctrl-shift-tab"], &["ctrl-shift-tab"]),
    ("copy", &["ctrl-shift-c"], &["cmd-c"]),
    ("paste", &["ctrl-shift-v", "shift-insert"], &["cmd-v"]),
    // Select all as Windows Terminal, GNOME Terminal and Konsole have it, and as every Mac program.
    ("select_all", &["ctrl-shift-a"], &["cmd-a"]),
    ("agents", &["ctrl-shift-j"], &["cmd-shift-j"]),
    ("tab_bar", &["ctrl-shift-b"], &["cmd-shift-b"]),
    ("full_screen", &["f11", "alt-enter"], &["cmd-enter", "ctrl-cmd-f"]),
    ("explain", &["ctrl-shift-e"], &["ctrl-shift-e"]),
    ("mail", &["ctrl-shift-m"], &["cmd-shift-m"]),
    // Adding a mailbox is seldom done: a key no program is likely to want.
    ("mail_add", &["ctrl-alt-shift-m"], &["alt-cmd-shift-m"]),
];

/// The keys of the window's own commands: each a list, so that a command
/// may have two keys, or none (`[]`: no key does it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct KeysConfig {
    pub new_tab: Vec<String>,
    pub close_tab: Vec<String>,
    pub next_tab: Vec<String>,
    pub previous_tab: Vec<String>,
    pub copy: Vec<String>,
    pub paste: Vec<String>,
    /// Select everything the tab holds, its history too: then copy takes it.
    pub select_all: Vec<String>,
    /// Show or hide the agents' sidebar.
    pub agents: Vec<String>,
    /// The row of tabs out of the way, or always there again.
    pub tab_bar: Vec<String>,
    pub full_screen: Vec<String>,
    /// What each part of the command on the line means.
    pub explain: Vec<String>,
    /// The window shows the mail, or the terminal again.
    pub mail: Vec<String>,
    /// The mail, with the form that adds a mailbox.
    pub mail_add: Vec<String>,
}

impl Default for KeysConfig {
    fn default() -> Self {
        let of = |name: &str| -> Vec<String> {
            let (_, elsewhere, mac) =
                DEFAULT_KEYS.iter().find(|(known, _, _)| *known == name).copied().unwrap_or_default();
            let keys = if cfg!(target_os = "macos") { mac } else { elsewhere };
            keys.iter().map(|&key| key.to_owned()).collect()
        };
        Self {
            new_tab: of("new_tab"),
            close_tab: of("close_tab"),
            next_tab: of("next_tab"),
            previous_tab: of("previous_tab"),
            copy: of("copy"),
            paste: of("paste"),
            select_all: of("select_all"),
            agents: of("agents"),
            tab_bar: of("tab_bar"),
            full_screen: of("full_screen"),
            explain: of("explain"),
            mail: of("mail"),
            mail_add: of("mail_add"),
        }
    }
}

impl KeysConfig {
    /// Every command with its keys, by its name in `[keys]`, in the file's order.
    #[must_use]
    pub fn all(&self) -> [(&'static str, &Vec<String>); 13] {
        [
            ("new_tab", &self.new_tab),
            ("close_tab", &self.close_tab),
            ("next_tab", &self.next_tab),
            ("previous_tab", &self.previous_tab),
            ("copy", &self.copy),
            ("paste", &self.paste),
            ("select_all", &self.select_all),
            ("agents", &self.agents),
            ("tab_bar", &self.tab_bar),
            ("full_screen", &self.full_screen),
            ("explain", &self.explain),
            ("mail", &self.mail),
            ("mail_add", &self.mail_add),
        ]
    }

    /// Every command with its keys, to change them.
    fn all_mut(&mut self) -> [(&'static str, &mut Vec<String>); 13] {
        [
            ("new_tab", &mut self.new_tab),
            ("close_tab", &mut self.close_tab),
            ("next_tab", &mut self.next_tab),
            ("previous_tab", &mut self.previous_tab),
            ("copy", &mut self.copy),
            ("paste", &mut self.paste),
            ("select_all", &mut self.select_all),
            ("agents", &mut self.agents),
            ("tab_bar", &mut self.tab_bar),
            ("full_screen", &mut self.full_screen),
            ("explain", &mut self.explain),
            ("mail", &mut self.mail),
            ("mail_add", &mut self.mail_add),
        ]
    }

    /// A key the file gives a command (`written`: the commands `[keys]` names)
    /// is taken from a command that has it only by default: what the person
    /// wrote wins over what a later version added. Without this, a version that
    /// gives a new command a default key the person had already given another
    /// would find two commands with one key, and use none of the settings.
    /// Says each key taken, and from which command.
    fn give_way(&mut self, written: &[String]) -> Vec<String> {
        let held: Vec<(&'static str, String)> = self
            .all()
            .into_iter()
            .filter(|(name, _)| written.iter().any(|given| given == name))
            .flat_map(|(name, keys)| keys.iter().map(move |key| (name, key.clone())))
            .collect();
        let mut said = Vec::new();
        for (name, keys) in self.all_mut() {
            if written.iter().any(|given| given == name) {
                continue;
            }
            let mut taken = Vec::new();
            keys.retain(|key| match held.iter().find(|(_, given)| given == key) {
                Some((owner, _)) => {
                    taken.push(format!("\"{key}\" is keys.{owner}'s"));
                    false
                }
                None => true,
            });
            if taken.is_empty() {
                continue;
            }
            let left = if keys.is_empty() {
                format!("; keys.{name} has no key now (give it one in [keys])")
            } else {
                String::new()
            };
            said.push(format!("keys.{name}: {}, as your settings say{left}", taken.join(", ")));
        }
        said
    }
    /// Every key is one the window may take, in the form it is kept
    /// ([`chord::chord`]), and no key is given to two commands.
    fn check(&self) -> Result<()> {
        let mut taken: Vec<(&str, &str)> = Vec::new();
        for (name, keys) in self.all() {
            for key in keys {
                match chord::chord(key) {
                    Ok(kept) if kept == *key => {}
                    Ok(kept) => bail!("keys.{name} has \"{key}\": write it \"{kept}\""),
                    Err(why) => bail!("keys.{name} has \"{key}\": {why}"),
                }
                if let Some((other, _)) = taken.iter().find(|(_, held)| *held == key.as_str()) {
                    bail!("keys.{other} and keys.{name} both have \"{key}\": a key does one thing");
                }
                taken.push((name, key));
            }
        }
        Ok(())
    }
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

/// The mailbox UnifyTerm reads (`Ctrl-Shift-M`): none until `address` is set. The password is
/// never here: a program prints it (`password_command`), or `ut mail login` keeps it in the system's keychain.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MailConfig {
    /// `me@example.com`; empty: no mail.
    pub address: String,
    /// The IMAP server as `host:port`; empty: the address's domain's, when it is a known one.
    pub imap: String,
    /// The SMTP server as `host:port`; empty: as `imap`.
    pub smtp: String,
    /// The name logged in with; empty: the address.
    pub login: String,
    /// A program and its arguments that print the password.
    pub password_command: Vec<String>,
    /// `socks5://host:port` or `http://host:port`: both servers are reached through it. Empty: directly.
    pub proxy: String,
    /// How many days of mail the first sync reads.
    pub days: u32,
    /// A program run for each new mail from someone else, with `{thread}`, `{from}`, `{subject}`, `{snippet}`.
    pub on_new: Vec<String>,
    /// A note in the window when mail comes.
    pub notify: bool,
    /// How the mailbox above logs in.
    pub auth: MailAuth,
    /// The application id `OAuth2` signs in with, for an organization that registers its own; empty: UnifyTerm's.
    pub oauth_client_id: String,
    /// More mailboxes, by name: `[mail.accounts.NAME]`. Added in the window or with `ut mail account add`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub accounts: std::collections::BTreeMap<String, MailAccount>,
}

/// How a mailbox logs in.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MailAuth {
    /// `OAuth2` for Microsoft's domains (outlook.com, hotmail.com, live.com, msn.com), a password elsewhere.
    #[default]
    Auto,
    /// A password, an app password or an authorization code.
    Password,
    /// A browser sign-in (Microsoft): UnifyTerm keeps the token it is given, never a password.
    Oauth2,
}

/// One mailbox of `[mail.accounts.NAME]`: as the first one in `[mail]`, and what it leaves out is `[mail]`'s.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MailAccount {
    pub address: String,
    pub imap: String,
    pub smtp: String,
    pub login: String,
    pub password_command: Vec<String>,
    pub proxy: String,
    pub days: Option<u32>,
    pub auth: MailAuth,
}

/// A mailbox in use: its name and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxSettings {
    pub name: String,
    pub address: String,
    pub imap: String,
    pub smtp: String,
    pub login: String,
    pub password_command: Vec<String>,
    pub proxy: String,
    pub days: u32,
    pub auth: MailAuth,
}

/// Whether `name` may name a mailbox: small letters, digits and `-`.
#[must_use]
pub fn is_mailbox_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl Default for MailConfig {
    fn default() -> Self {
        Self {
            address: String::new(),
            imap: String::new(),
            smtp: String::new(),
            login: String::new(),
            password_command: Vec::new(),
            proxy: String::new(),
            days: 30,
            on_new: Vec::new(),
            notify: true,
            auth: MailAuth::Auto,
            oauth_client_id: String::new(),
            accounts: std::collections::BTreeMap::new(),
        }
    }
}

impl MailConfig {
    /// The mailboxes in use, in order: the one in `[mail]` first (named after its domain, `qq`
    /// for me@qq.com, unless a named one has that name), then `[mail.accounts]` by name.
    #[must_use]
    pub fn mailboxes(&self) -> Vec<MailboxSettings> {
        let mut all = Vec::new();
        if !self.address.trim().is_empty() {
            let domain = self.address.trim().rsplit_once('@').map_or("mail", |(_, domain)| domain);
            let mut name: String = domain.split('.').next().unwrap_or("mail").to_lowercase();
            if !is_mailbox_name(&name) {
                "mail".clone_into(&mut name);
            }
            while self.accounts.contains_key(&name) {
                name.push_str("-1");
            }
            all.push(MailboxSettings {
                name,
                address: self.address.trim().to_lowercase(),
                imap: self.imap.clone(),
                smtp: self.smtp.clone(),
                login: self.login.clone(),
                password_command: self.password_command.clone(),
                proxy: self.proxy.clone(),
                days: self.days,
                auth: self.auth,
            });
        }
        for (name, account) in &self.accounts {
            all.push(MailboxSettings {
                name: name.clone(),
                address: account.address.trim().to_lowercase(),
                imap: account.imap.clone(),
                smtp: account.smtp.clone(),
                login: account.login.clone(),
                password_command: account.password_command.clone(),
                proxy: account.proxy.clone(),
                days: account.days.unwrap_or(self.days),
                auth: account.auth,
            });
        }
        all
    }

    /// What can be told from the text alone: an address with `@`, servers as `host:port`, a proxy
    /// with its kind and port, the days in range, names a mailbox may have, no address twice.
    /// Whether the servers answer is found when they are asked.
    fn check(&self) -> Result<()> {
        check_mailbox("mail", &self.address, &self.imap, &self.smtp, &self.proxy, self.days)?;
        for (name, account) in &self.accounts {
            if !is_mailbox_name(name) {
                bail!("[mail.accounts.{name}]: a mailbox's name is small letters, digits and -");
            }
            if account.address.trim().is_empty() {
                bail!("[mail.accounts.{name}] has no address");
            }
            let prefix = format!("mail.accounts.{name}");
            let days = account.days.unwrap_or(self.days);
            check_mailbox(&prefix, &account.address, &account.imap, &account.smtp, &account.proxy, days)?;
        }
        let addresses: Vec<String> = self.mailboxes().iter().map(|mailbox| mailbox.address.clone()).collect();
        if let Some((_, twice)) = addresses.iter().enumerate().find(|(at, address)| addresses[..*at].contains(address))
        {
            bail!("the mailbox {twice} is set twice");
        }
        if self.on_new.first().is_some_and(|program| program.trim().is_empty()) {
            bail!("mail.on_new begins with an empty word: it is a program and its arguments");
        }
        Ok(())
    }
}

/// One mailbox's text checked: `prefix` names it in what is said (`mail`, `mail.accounts.qq`).
fn check_mailbox(prefix: &str, address: &str, imap: &str, smtp: &str, proxy: &str, days: u32) -> Result<()> {
    let address = address.trim();
    if !address.is_empty()
        && !address.split_once('@').is_some_and(|(name, domain)| !name.is_empty() && domain.contains('.'))
    {
        bail!("{prefix}.address is \"{address}\": not a mail address like me@example.com");
    }
    for (key, text) in [("imap", imap), ("smtp", smtp)] {
        let text = text.trim();
        if !text.is_empty()
            && !text.rsplit_once(':').is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok())
        {
            bail!("{prefix}.{key} is \"{text}\": write it as host:port, as imap.example.com:993");
        }
    }
    let proxy = proxy.trim();
    if !proxy.is_empty() {
        let (scheme, place) = proxy.split_once("://").unwrap_or(("", proxy));
        let known = matches!(scheme.to_ascii_lowercase().as_str(), "socks5" | "socks5h" | "http");
        let port = place.trim_end_matches('/').rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok());
        if !known || !port {
            bail!("{prefix}.proxy is \"{proxy}\": write it as socks5://127.0.0.1:7890 or http://127.0.0.1:7890");
        }
    }
    if !MAIL_DAYS.contains(&days) {
        bail!("{prefix}.days is {days}: it must be from {} to {}", MAIL_DAYS.start(), MAIL_DAYS.end());
    }
    Ok(())
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
    /// A picture's file, drawn behind the text over the whole window: made
    /// as large as the window, what goes past its sides cut off. Found as
    /// `picture` is. Empty: none.
    pub wallpaper: Option<String>,
    /// How much of the wallpaper shows over the color behind the text: 0 is
    /// nothing of it, 1 all of it.
    pub wallpaper_opacity: Option<f32>,
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
    /// The wallpaper, where it was found to be.
    pub wallpaper: Option<themes::Picture>,
    /// The prompt: the settings' `[prompt]` over the theme's over the defaults.
    pub prompt: prompt::Prompt,
}

impl ThemeConfig {
    /// Every key the table has, as the file writes it. A theme file's key
    /// that is not one of these is passed over (see [`themes::read`]).
    pub const KEYS: [&'static str; 20] = [
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
        "wallpaper",
        "wallpaper_opacity",
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
        if let Some(shows) = self.wallpaper_opacity
            && !(0.0..=1.0).contains(&shows)
        {
            bail!("theme.wallpaper_opacity is {shows}: it must be from 0 to 1");
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
            wallpaper: self.wallpaper.or(under.wallpaper),
            wallpaper_opacity: self.wallpaper_opacity.or(under.wallpaper_opacity),
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
        let mut config: Self = toml::from_str(text)?;
        let table: toml::Table = toml::from_str(text)?;
        let written: Vec<String> = table
            .get("keys")
            .and_then(toml::Value::as_table)
            .map(|keys| keys.keys().cloned().collect())
            .unwrap_or_default();
        config.keys_given_way = config.keys.give_way(&written);
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
        config.keys.check()?;
        config.mail.check()?;
        config.theme.check()?;
        config.prompt.check(prompt::Layer::Settings)?;
        Ok(config)
    }

    /// The look in use: the named theme's, with what the settings give
    /// themselves over it. `themes` is the folder of the user's own themes
    /// (`themes` beside the settings file). No theme named: the default one
    /// ([`themes::DEFAULT`]), found as a named one is.
    ///
    /// A picture is looked for beside the file that names it: the settings
    /// file for the settings' own, the theme's file for the theme's.
    pub fn look(&self, themes: Option<&Path>) -> Result<Look> {
        let name = self.theme.name.as_deref().unwrap_or(themes::DEFAULT);
        let (under, their_prompt, source) = themes::found_whole(name, themes)?;
        let theme_folder = match source {
            themes::Source::File(file) => file.parent().map(Path::to_path_buf),
            themes::Source::BuiltIn => None,
        };
        // A picture's file: the settings' own goes first, looked for beside the settings file;
        // else the theme's, beside the theme's file.
        let found = |own: Option<&str>, theirs: Option<&str>| match (own, theirs) {
            (Some(own), _) => themes::picture(own, themes.and_then(Path::parent)),
            (None, Some(theirs)) => themes::picture(theirs, theme_folder.as_deref()),
            (None, None) => None,
        };
        let picture = found(self.theme.picture.as_deref(), under.picture.as_deref());
        let wallpaper = found(self.theme.wallpaper.as_deref(), under.wallpaper.as_deref());
        let colors = self.theme.clone().over(under);
        let prompt = self.prompt.clone().over(their_prompt).resolved();
        Ok(Look { colors, picture, wallpaper, prompt })
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
    fn the_default_keys_of_every_system_are_keys_in_the_form_they_are_kept() {
        // Both tables, whatever system the tests run on: the other system's defaults are data here.
        for column in [0, 1] {
            let mut taken: Vec<&str> = Vec::new();
            for (name, elsewhere, mac) in DEFAULT_KEYS {
                for key in if column == 0 { elsewhere } else { mac } {
                    assert_eq!(chord::chord(key).as_deref(), Ok(*key), "keys.{name}: {key}");
                    assert!(!taken.contains(key), "keys.{name}: {key} is given twice in one table");
                    taken.push(key);
                }
            }
        }
        let names: Vec<&str> = DEFAULT_KEYS.iter().map(|(name, _, _)| *name).collect();
        let fields: Vec<&str> = KeysConfig::default().all().iter().map(|(name, _)| *name).collect();
        assert_eq!(names, fields, "a default for each command, in the commands' order");
        assert!(Config::parse("").is_ok(), "this system's defaults are settings the terminal takes");
    }

    #[test]
    fn keys_are_checked_when_the_settings_are_read() {
        let keys = |text: &str| Config::parse(text).map(|config| config.keys);
        assert_eq!(keys("[keys]\nagents = [\"ctrl-alt-g\", \"f9\"]").expect("settings").agents, ["ctrl-alt-g", "f9"]);
        assert!(keys("[keys]\nexplain = []").expect("no key").explain.is_empty());
        let said = |text: &str| format!("{:#}", keys(text).expect_err("refused"));
        assert!(
            said("[keys]\nagents = [\"Ctrl+Alt+G\"]").contains("write it \"ctrl-alt-g\""),
            "the one form, in the file"
        );
        assert!(said("[keys]\nagents = [\"g\"]").contains("keys.agents has \"g\": the shell needs that key"));
        assert!(said("[keys]\nagents = [\"ctrl-shft-g\"]").contains("'shft' is not ctrl, alt, shift or cmd"));
        assert!(
            said("[keys]\ncopy = [\"ctrl-alt-x\"]\npaste = [\"ctrl-alt-x\"]")
                .contains("keys.copy and keys.paste both have")
        );
        assert!(said("[keys]\ncopy = [\"ctrl-alt-x\", \"ctrl-alt-x\"]").contains("keys.copy and keys.copy both have"));
        assert!(said("[keys]\nfrobnicate = [\"f9\"]").contains("unknown field"), "a command that is none");
    }

    #[test]
    fn a_key_the_file_gives_takes_it_from_a_default_and_the_rest_of_the_file_stays() {
        // As on a Mac after 0.9.4: agents had been given select all's new default (cmd-a there).
        let select_all = KeysConfig::default().select_all;
        let new = select_all.first().expect("select all has a default key").clone();
        let text = format!("[keys]\nagents = [\"{new}\"]\n\n[mail]\naddress = \"me@example.com\"\n");
        let config = Config::parse(&text).expect("the file is used, mailbox and all");
        assert_eq!(config.keys.agents, std::slice::from_ref(&new), "the key the person wrote");
        assert!(!config.keys.select_all.contains(&new), "the default gave it way");
        assert_eq!(config.mail.address, "me@example.com", "the rest of the file is used");
        assert_eq!(config.keys_given_way.len(), 1, "{:?}", config.keys_given_way);
        let said = &config.keys_given_way[0];
        assert!(said.starts_with(&format!("keys.select_all: \"{new}\" is keys.agents's")), "{said}");
        assert_eq!(
            said.contains("has no key now"),
            select_all.len() == 1,
            "said when the command is left with no key: {said}"
        );
        // What was right before is as it was.
        assert!(Config::parse("").expect("defaults").keys_given_way.is_empty(), "no file: nothing given way");
        let same = format!("[keys]\nselect_all = [\"{new}\"]");
        assert!(Config::parse(&same).expect("its own default").keys_given_way.is_empty());
        // Two keys the person wrote are still a mistake to name.
        let both = format!("[keys]\nselect_all = [\"{new}\"]\nagents = [\"{new}\"]");
        let refused = format!("{:#}", Config::parse(&both).expect_err("refused"));
        assert!(refused.contains("both have"), "{refused}");
    }

    #[test]
    fn mail_settings_are_checked_as_far_as_text_can_tell() {
        let mail = Config::parse("[mail]\naddress = \"me@qq.com\"\nproxy = \"socks5://127.0.0.1:7890\"")
            .expect("settings")
            .mail;
        assert_eq!(
            (mail.address.as_str(), mail.days, mail.notify),
            ("me@qq.com", 30, true),
            "days and notify by default"
        );
        assert_eq!(Config::default().mail.address, "", "no mail until an address is set");
        let said = |text: &str| format!("{:#}", Config::parse(text).expect_err("refused"));
        assert!(said("[mail]\naddress = \"me\"").contains("mail.address is \"me\""));
        assert!(said("[mail]\nimap = \"imap.qq.com\"").contains("mail.imap"));
        assert!(said("[mail]\nsmtp = \"x:port\"").contains("mail.smtp"));
        assert!(said("[mail]\nproxy = \"127.0.0.1:7890\"").contains("mail.proxy"));
        assert!(said("[mail]\nproxy = \"ftp://x:1\"").contains("mail.proxy"));
        assert!(said("[mail]\ndays = 0").contains("mail.days is 0"));
        assert!(said("[mail]\non_new = [\"\"]").contains("mail.on_new"));
        assert!(said("[mail]\npassword = \"x\"").contains("unknown field"), "no password in the file");
    }

    #[test]
    fn mailboxes_are_the_first_one_then_the_named_ones() {
        let mail = Config::parse(
            "[mail]\naddress = \"Me@QQ.com\"\ndays = 7\n\
             [mail.accounts.work]\naddress = \"me@outlook.com\"\nauth = \"oauth2\"\n\
             [mail.accounts.gmail]\naddress = \"me@gmail.com\"\nproxy = \"socks5://127.0.0.1:7890\"\ndays = 90",
        )
        .expect("settings")
        .mail;
        let mailboxes = mail.mailboxes();
        let names: Vec<&str> = mailboxes.iter().map(|mailbox| mailbox.name.as_str()).collect();
        assert_eq!(names, ["qq", "gmail", "work"], "the first one named after its domain, the others by name");
        assert_eq!(mailboxes[0].address, "me@qq.com", "an address is kept in small letters");
        assert_eq!((mailboxes[1].days, mailboxes[2].days), (90, 7), "days its own, else [mail]'s");
        assert_eq!((mailboxes[0].auth, mailboxes[2].auth), (MailAuth::Auto, MailAuth::Oauth2));
        assert_eq!(mailboxes[1].proxy, "socks5://127.0.0.1:7890", "each mailbox its own proxy");
        // A named one with the first one's name: the first one steps aside.
        let clash = Config::parse("[mail]\naddress = \"me@qq.com\"\n[mail.accounts.qq]\naddress = \"two@qq.com\"")
            .expect("settings")
            .mail;
        let names: Vec<String> = clash.mailboxes().into_iter().map(|mailbox| mailbox.name).collect();
        assert_eq!(names, ["qq-1", "qq"]);
        // Only named ones, and none at all.
        let only_named = Config::parse("[mail.accounts.a]\naddress = \"a@example.com\"").expect("settings").mail;
        assert_eq!(only_named.mailboxes().len(), 1);
        assert!(Config::default().mail.mailboxes().is_empty());
        let said = |text: &str| format!("{:#}", Config::parse(text).expect_err("refused"));
        assert!(said("[mail.accounts.Work]\naddress = \"a@example.com\"").contains("small letters"));
        assert!(said("[mail.accounts.a]\nimap = \"x:1\"").contains("has no address"));
        assert!(
            said("[mail.accounts.a]\naddress = \"a@example.com\"\nproxy = \"x\"").contains("mail.accounts.a.proxy")
        );
        assert!(said("[mail.accounts.a]\naddress = \"a@example.com\"\ndays = 0").contains("mail.accounts.a.days"));
        assert!(
            said("[mail]\naddress = \"A@example.com\"\n[mail.accounts.b]\naddress = \"a@example.com\"")
                .contains("set twice")
        );
        assert!(said("[mail.accounts.a]\naddress = \"a@example.com\"\npassword = \"x\"").contains("unknown field"));
        assert!(said("[mail]\nauth = \"token\"").contains("auth"));
        assert!(
            is_mailbox_name("qq-2")
                && !is_mailbox_name("")
                && !is_mailbox_name("a b")
                && !is_mailbox_name(&"a".repeat(33))
        );
    }

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
