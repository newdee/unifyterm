//! Where a model's help comes from, by the name the settings give it
//! (`ai.provider`): a service's address, or a program on this machine. One
//! table, so that the terminal, `ut setting` and the documents say the same.

use crate::{AiConfig, Provider};

/// Every name `ai.provider` may have, in the order they are shown.
pub const NAMES: [&str; 11] = [
    "none",
    "anthropic",
    "openai",
    "openai-compatible",
    "ollama",
    "lm-studio",
    "vllm",
    "claude-code",
    "codex",
    "pi",
    "command",
];

/// Whether a service takes a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `ai.api_key_env` must name the variable that holds it.
    Needed,
    /// A service on this machine: none, unless `ai.api_key_env` names one.
    Optional,
}

/// How a provider is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// No model is asked.
    Nowhere,
    /// Anthropic's Messages API.
    Anthropic,
    /// A service with `OpenAI`'s chat-completions shape. `address`: where it
    /// is unless `ai.base_url` says; `None` when the settings must say.
    Chat { address: Option<&'static str>, key: Key },
    /// An agent's command line on this machine: the question goes to its
    /// standard input, the answer is what it prints. `line`: the program and
    /// its arguments. `model`: the argument that names a model.
    Program { line: &'static [&'static str], model: &'static str },
    /// The program `ai.command` names, run the same way.
    OwnProgram,
}

/// Claude Code with no tools: it can only answer. Its own instructions (a
/// coding agent's) are replaced by one line; the session is not kept.
const CLAUDE_CODE: &[&str] = &[
    "claude",
    "-p",
    "--output-format",
    "text",
    "--tools",
    "",
    "--no-session-persistence",
    "--system-prompt",
    "You answer with JSON only.",
];
/// Codex, where it can read and not write, with no session kept.
const CODEX: &[&str] =
    &["codex", "exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never"];
/// pi with no tools and no session.
const PI: &[&str] = &["pi", "-p", "--no-tools", "--no-session"];

impl Provider {
    /// How this provider is reached.
    #[must_use]
    pub const fn reach(&self) -> Reach {
        match self {
            Self::None => Reach::Nowhere,
            Self::Anthropic => Reach::Anthropic,
            Self::Openai => Reach::Chat { address: Some("https://api.openai.com/v1"), key: Key::Needed },
            Self::OpenaiCompatible => Reach::Chat { address: None, key: Key::Optional },
            Self::Ollama => Reach::Chat { address: Some("http://localhost:11434/v1"), key: Key::Optional },
            Self::LmStudio => Reach::Chat { address: Some("http://localhost:1234/v1"), key: Key::Optional },
            Self::Vllm => Reach::Chat { address: Some("http://localhost:8000/v1"), key: Key::Optional },
            Self::ClaudeCode => Reach::Program { line: CLAUDE_CODE, model: "--model" },
            Self::Codex => Reach::Program { line: CODEX, model: "-m" },
            Self::Pi => Reach::Program { line: PI, model: "--model" },
            Self::Command => Reach::OwnProgram,
        }
    }

    /// Whether the answers come from a program on this machine.
    #[must_use]
    pub const fn is_program(&self) -> bool {
        matches!(self.reach(), Reach::Program { .. } | Reach::OwnProgram)
    }
}

impl AiConfig {
    /// Where the service is: `ai.base_url`, else the provider's own
    /// address. Empty for a provider that is not a service.
    #[must_use]
    pub fn address(&self) -> String {
        if !self.base_url.trim().is_empty() {
            return self.base_url.clone();
        }
        match self.provider.reach() {
            Reach::Chat { address: Some(address), .. } => address.to_owned(),
            _ => String::new(),
        }
    }

    /// The program that answers and its arguments; `None` for a provider
    /// that is a service. A model named in `ai.model` is passed on to a
    /// provider's own program; `ai.command` is run as it is written.
    #[must_use]
    pub fn program(&self) -> Option<Vec<String>> {
        match self.provider.reach() {
            Reach::Program { line, model } => {
                let mut words: Vec<String> = line.iter().map(|&word| word.to_owned()).collect();
                if !self.model.trim().is_empty() {
                    words.push(model.to_owned());
                    words.push(self.model.trim().to_owned());
                }
                Some(words)
            }
            Reach::OwnProgram => Some(self.command.clone()),
            Reach::Nowhere | Reach::Anthropic | Reach::Chat { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn provider(name: &str) -> Provider {
        toml::from_str::<Config>(&format!("[ai]\nprovider = \"{name}\""))
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .ai
            .provider
    }

    #[test]
    fn every_name_is_a_provider_and_every_provider_has_a_name() {
        let named: Vec<Provider> = NAMES.iter().map(|name| provider(name)).collect();
        for (index, one) in named.iter().enumerate() {
            assert!(!named[..index].contains(one), "{} names a provider twice", NAMES[index]);
            // Written back, a provider is the name it was read from.
            let written = toml::Value::try_from(one).unwrap();
            assert_eq!(written.as_str(), Some(NAMES[index]));
        }
        assert_eq!((named[0].clone(), named.len()), (Provider::None, 11));
    }

    #[test]
    fn a_service_is_at_its_own_address_unless_the_settings_say() {
        let at = |name: &str, base_url: &str| {
            AiConfig { provider: provider(name), base_url: base_url.into(), ..AiConfig::default() }.address()
        };
        assert_eq!(at("openai", ""), "https://api.openai.com/v1");
        assert_eq!(at("ollama", ""), "http://localhost:11434/v1");
        assert_eq!(at("lm-studio", ""), "http://localhost:1234/v1");
        assert_eq!(at("vllm", ""), "http://localhost:8000/v1");
        assert_eq!(at("ollama", "http://box:11434/v1"), "http://box:11434/v1", "the settings' address wins");
        assert_eq!(at("openai-compatible", ""), "", "it has none of its own");
        assert_eq!(at("claude-code", ""), "", "a program is at no address");
        for (name, key) in [("openai", Key::Needed), ("ollama", Key::Optional), ("openai-compatible", Key::Optional)] {
            assert!(matches!(provider(name).reach(), Reach::Chat { key: its, .. } if its == key), "{name}");
        }
    }

    #[test]
    fn an_agent_s_program_can_only_answer() {
        // What turns each agent's tools off, and what keeps its session out of the user's list.
        for (name, no_tools, no_session) in [
            ("claude-code", vec!["--tools", ""], "--no-session-persistence"),
            ("codex", vec!["--sandbox", "read-only"], "--ephemeral"),
            ("pi", vec!["--no-tools"], "--no-session"),
        ] {
            let ai = AiConfig { provider: provider(name), ..AiConfig::default() };
            let line = ai.program().unwrap();
            let has = |words: &[&str]| {
                line.windows(words.len()).any(|part| part.iter().map(String::as_str).eq(words.iter().copied()))
            };
            assert!(has(&no_tools), "{name} keeps its tools: {line:?}");
            assert!(has(&[no_session]), "{name} keeps a session: {line:?}");
            assert!(provider(name).is_program());
            assert!(!line.iter().any(|word| word == "--model" || word == "-m"), "{name}: no model is named");
        }
        let named = AiConfig { provider: provider("codex"), model: " gpt-5 ".into(), ..AiConfig::default() };
        assert_eq!(named.program().unwrap()[8..], ["-m".to_owned(), "gpt-5".to_owned()]);
        let own = AiConfig {
            provider: provider("command"),
            command: vec!["my-agent".into(), "--quiet".into()],
            model: "ignored".into(),
            ..AiConfig::default()
        };
        assert_eq!(own.program().unwrap(), ["my-agent", "--quiet"], "as written: the model is the command's business");
        assert_eq!(AiConfig { provider: provider("ollama"), ..AiConfig::default() }.program(), None);
        assert!(!provider("ollama").is_program() && !provider("none").is_program());
    }
}
