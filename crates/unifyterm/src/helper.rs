//! Help with commands, as the settings and the user allow it: the local fix
//! for a misspelt command name always; a language model's suggestion, fix
//! and explanation, and its command for a sentence, when `[ai]` names one
//! and the user has agreed, once, to what is sent. The model is a service or
//! a program on this machine (an agent's command line): `ai.provider` says. Every question to the model runs on a thread of its own;
//! the answer comes back on a channel and the window never waits.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use smol::channel::Receiver;
use unifyterm_assist::{
    Anthropic, Ask, Asked, Command, Context, Error, Failure, Fix, OpenAiCompatible, Part, Provider, Tokens,
};
use unifyterm_config::providers::Reach;
use unifyterm_programs::{Explained, Offer, Programs};

use crate::config::{AiConfig, AskEnter, Provider as ProviderSetting};

/// The user's answer to "may this be sent?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consent {
    /// Not asked yet (or the model changed since).
    Unasked,
    Given,
    /// Refused for as long as UnifyTerm runs.
    Refused,
}

/// A feature of the model's help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Feature {
    Suggest,
    Fix,
    Explain,
    /// A command for the sentence on a line that begins with `#`.
    Ask,
}

/// What may be sent, as the window asks the user.
pub(crate) const WHAT_IS_SENT: &str = "The command line you type (secrets replaced), what you write after # at the \
prompt, the directory, your last five commands, and for a command that was not found, the last 40 lines it printed. \
With #, also what programs say of their own commands (ut program).";
/// How many words of the command lists are offered under the input line at a time.
const OFFERED: usize = 6;
/// The most text of the command lists sent along with a sentence, and with a command to explain, in bytes.
const DESCRIBED_WITH_A_SENTENCE: usize = 16 * 1024;
const DESCRIBED_WITH_A_COMMAND: usize = 8 * 1024;
/// The directory, in UnifyTerm's data, where an agent's command line runs: nothing is in it.
const AGENT_DIRECTORY: &str = "agent-cwd";

/// The operating system, as a person calls it: the model writes commands for it.
const SYSTEM: &str = if cfg!(windows) {
    "Windows"
} else if cfg!(target_os = "macos") {
    "macOS"
} else {
    "Linux"
};

/// The help, shared by every tab.
pub(crate) struct Helper {
    settings: AiConfig,
    provider: Option<Arc<dyn Provider>>,
    /// The model's service, name and address, or the program that answers: agreeing is agreeing to these.
    asked_for: String,
    /// What the model is called when the user is asked.
    shown: String,
    consent_file: Option<PathBuf>,
    consent: Mutex<Consent>,
    /// UnifyTerm's commands: their own help goes along with an explanation.
    ut: Option<(PathBuf, Vec<String>)>,
    /// The programs the shells can find, by name: what a misspelt name is compared with.
    known: Arc<OnceLock<Vec<String>>>,
    /// The command lists: what programs say of their own commands (`ut program`).
    programs: Programs,
}

impl std::fmt::Debug for Helper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Helper")
            .field("asked_for", &self.asked_for)
            .field("consent", &self.consent)
            .finish_non_exhaustive()
    }
}

/// The names of the programs in `PATH` (and `first`): file names without
/// their extension, the first of each name kept.
fn programs_in_path(first: Option<PathBuf>) -> Vec<String> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_ascii_lowercase)
            .collect()
    } else {
        Vec::new()
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut names = Vec::new();
    for directory in first.into_iter().chain(std::env::split_paths(&path)) {
        let Ok(entries) = std::fs::read_dir(&directory) else { continue };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().to_lowercase();
            let name = if cfg!(windows) {
                let Some(dot) = file.rfind('.') else { continue };
                if !extensions.iter().any(|extension| *extension == file[dot..]) {
                    continue;
                }
                file[..dot].to_owned()
            } else {
                file
            };
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Output that says the command was not found, in the shells' words (English and Chinese).
fn not_found(output: &str, exit: Option<i32>) -> bool {
    static WORDS: OnceLock<Regex> = OnceLock::new();
    let words = WORDS.get_or_init(|| {
        Regex::new(
            r"(?i)not recognized as (the name of )?a cmdlet|is not recognized as an internal or external command|command not found|CommandNotFoundException|识别为 cmdlet|不是内部或外部命令|未找到命令",
        )
        .expect("the pattern is valid")
    });
    exit == Some(127) || words.is_match(output)
}

impl Helper {
    /// The help the settings ask for, and what the user should be told
    /// (a key that is not set). `commands_directory` and `ut_names`: UnifyTerm's commands.
    pub(crate) fn new(
        settings: &AiConfig,
        ut: Option<(PathBuf, Vec<String>)>,
        commands_directory: Option<PathBuf>,
    ) -> (Self, Vec<String>) {
        let mut told = Vec::new();
        let key = if settings.api_key_env.is_empty() {
            Some(String::new())
        } else {
            let key = std::env::var(&settings.api_key_env).ok().filter(|key| !key.is_empty());
            // A program has its own way to its model: a key named for another provider is not its business.
            if key.is_none() && matches!(settings.provider.reach(), Reach::Anthropic | Reach::Chat { .. }) {
                told.push(format!(
                    "ai.api_key_env names {}, which is not set: the model's help is off",
                    settings.api_key_env
                ));
            }
            key
        };
        let provider: Option<Arc<dyn Provider>> = match (settings.provider.reach(), key) {
            (Reach::Anthropic, Some(key)) => Some(Arc::new(Anthropic::new(&settings.base_url, &settings.model, &key))),
            (Reach::Chat { .. }, Some(key)) => {
                // OpenAI's own service names the answer's length its own way.
                let tokens =
                    if settings.provider == ProviderSetting::Openai { Tokens::MaxCompletion } else { Tokens::Max };
                Some(Arc::new(OpenAiCompatible::new(&settings.address(), &settings.model, &key).with_tokens(tokens)))
            }
            (Reach::Program { .. } | Reach::OwnProgram, _) => settings.program().map(|line| {
                let data = unifyterm_history::data_directory().unwrap_or_else(|_| std::env::temp_dir());
                let program: Arc<dyn Provider> = Arc::new(Command::new(line, data.join(AGENT_DIRECTORY)));
                program
            }),
            _ => None,
        };
        let program = settings.program().unwrap_or_default();
        // As it was before programs could answer, so that who agreed then is not asked again.
        let mut asked_for = format!("{:?} {} {}", settings.provider, settings.model, settings.base_url);
        if !program.is_empty() {
            asked_for = format!("{asked_for} {}", program.join(" "));
        }
        let shown = match program.first() {
            Some(program) if settings.model.trim().is_empty() => program.clone(),
            _ => settings.model.clone(),
        };
        let consent_file = unifyterm_history::data_directory().ok().map(|directory| directory.join("ai-consent"));
        let given = consent_file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .is_some_and(|text| text.trim() == asked_for);
        let known = Arc::new(OnceLock::new());
        let filling = Arc::clone(&known);
        // Reading every PATH directory takes a moment: not on the window's thread.
        let _ = std::thread::Builder::new().name("programs".into()).spawn(move || {
            let _ = filling.set(programs_in_path(commands_directory));
        });
        let helper = Self {
            settings: settings.clone(),
            provider,
            asked_for,
            shown,
            consent_file,
            consent: Mutex::new(if given { Consent::Given } else { Consent::Unasked }),
            ut,
            known,
            programs: unifyterm_history::data_directory()
                .map(|data| Programs::in_directory(unifyterm_programs::directory_in(&data)))
                .unwrap_or_default(),
        };
        (helper, told)
    }

    /// Whether the model is asked for `feature`, as far as the settings go (consent aside).
    pub(crate) fn offers(&self, feature: Feature) -> bool {
        self.provider.is_some()
            && match feature {
                // Not from a program: each question starts it anew, which takes seconds and is
                // counted against the user's plan; a suggestion is asked for at every pause in typing.
                Feature::Suggest => self.settings.suggest && !self.settings.provider.is_program(),
                Feature::Fix => self.settings.fix,
                Feature::Explain => self.settings.explain,
                Feature::Ask => self.settings.ask,
            }
    }

    /// Whether a command made from a sentence may be run without the user's Enter, when it only reads.
    pub(crate) fn runs_what_only_reads(&self) -> bool {
        self.settings.ask_enter == AskEnter::Auto
    }

    /// Whether a model is in use at all (named in the settings, with its key).
    pub(crate) fn has_model(&self) -> bool {
        self.provider.is_some()
    }

    pub(crate) fn consent(&self) -> Consent {
        *self.consent.lock().expect("lock poisoned")
    }

    /// The user agreed: kept in the data directory, for this model.
    pub(crate) fn agree(&self) {
        *self.consent.lock().expect("lock poisoned") = Consent::Given;
        if let Some(file) = &self.consent_file
            && let Err(error) = std::fs::write(file, &self.asked_for)
        {
            log::error!("keep the consent in {}: {error}", file.display());
        }
    }

    /// The user refused: nothing is sent until UnifyTerm starts again.
    pub(crate) fn refuse(&self) {
        *self.consent.lock().expect("lock poisoned") = Consent::Refused;
    }

    /// The model's name (or the program that answers, when no model is named), for the window to show.
    pub(crate) fn model(&self) -> &str {
        &self.shown
    }

    /// The fix that needs no model, for a command that was not found.
    pub(crate) fn local_fix(&self, command: &str, output: &str, exit: Option<i32>, recent: &[String]) -> Option<Fix> {
        if !self.settings.fix || !not_found(output, exit) {
            return None;
        }
        // The commands used lately first: of names as close, the one in use wins.
        let mut known: Vec<String> =
            recent.iter().filter_map(|command| command.split_whitespace().next().map(str::to_owned)).collect();
        known.extend(self.known.get().cloned().unwrap_or_default());
        unifyterm_assist::local_fix(command, &known)
    }

    /// These command lists in place of the data directory's.
    #[cfg(test)]
    pub(crate) fn with_programs(mut self, programs: Programs) -> Self {
        self.programs = programs;
        self
    }

    /// The words the command lists have for the word being typed at the end of `line`: none
    /// when the line's program has no list.
    pub(crate) fn offers_for(&self, line: &str) -> Vec<Offer> {
        unifyterm_programs::complete(&self.programs.now(), line, OFFERED)
    }

    /// For a command that failed: the word its program's list does not have, replaced by
    /// the one near it. Needs no model.
    pub(crate) fn list_fix(&self, command: &str, exit: Option<i32>) -> Option<Fix> {
        if !self.settings.fix || exit.is_none_or(|exit| exit == 0) {
            return None;
        }
        let problem = unifyterm_programs::problem(&self.programs.now(), command)?;
        Some(Fix { command: problem.corrected?, reason: problem.says })
    }

    /// What each word of `command` is, from its program's own list: `None` when the program
    /// has none, or the line is more than one simple command. Needs no model; nothing is sent.
    pub(crate) fn told(&self, command: &str) -> Option<Explained> {
        unifyterm_programs::explain(&self.programs.now(), command)
    }

    /// What a program's list has against a command (one the model wrote): a command or an
    /// option the list does not have.
    pub(crate) fn doubt(&self, command: &str) -> Option<String> {
        unifyterm_programs::problem(&self.programs.now(), command).map(|problem| problem.says)
    }

    /// Whether a failed command is one to ask the model to fix.
    pub(crate) fn wants_fix(&self, output: &str, exit: Option<i32>) -> bool {
        self.offers(Feature::Fix) && not_found(output, exit)
    }

    fn ask<T: Send + 'static>(
        &self,
        question: impl FnOnce(&dyn Provider) -> T + Send + 'static,
    ) -> Option<Receiver<T>> {
        let provider = Arc::clone(self.provider.as_ref()?);
        if self.consent() != Consent::Given {
            return None;
        }
        let (answer, answered) = smol::channel::bounded(1);
        let spawned = std::thread::Builder::new().name("model".into()).spawn(move || {
            let _ = answer.try_send(question(provider.as_ref()));
        });
        if let Err(error) = spawned {
            log::error!("start a thread for the model: {error}");
            return None;
        }
        Some(answered)
    }

    pub(crate) fn suggest(
        &self,
        typed: String,
        cwd: String,
        shell: String,
        recent: Vec<String>,
    ) -> Option<Receiver<Result<Option<String>, Error>>> {
        if !self.offers(Feature::Suggest) {
            return None;
        }
        self.ask(move |provider| {
            unifyterm_assist::suggest(provider, &Context { typed: &typed, cwd: &cwd, shell: &shell, recent: &recent })
        })
    }

    pub(crate) fn fix(
        &self,
        command: String,
        exit: Option<i32>,
        output: String,
        shell: String,
    ) -> Option<Receiver<Result<Option<Fix>, Error>>> {
        if !self.offers(Feature::Fix) {
            return None;
        }
        self.ask(move |provider| {
            unifyterm_assist::fix(provider, &Failure { command: &command, exit, output: &output, shell: &shell })
        })
    }

    /// The command for a sentence the user wrote at the prompt.
    pub(crate) fn command_for(
        &self,
        words: String,
        cwd: String,
        shell: String,
        recent: Vec<String>,
    ) -> Option<Receiver<Result<Option<Asked>, Error>>> {
        if !self.offers(Feature::Ask) {
            return None;
        }
        // UnifyTerm's commands are in every tab's PATH: the model may use them on any system.
        let programs = self.ut.as_ref().map(|(_, names)| names.clone()).unwrap_or_default();
        // The lists of the programs the sentence or the last commands name: the model need not guess their commands.
        let mentions: Vec<&str> =
            std::iter::once(words.as_str()).chain(recent.iter().take(5).map(String::as_str)).collect();
        let described = unifyterm_programs::brief(&self.programs.now(), &mentions, DESCRIBED_WITH_A_SENTENCE);
        self.ask(move |provider| {
            let ask = Ask {
                words: &words,
                cwd: &cwd,
                shell: &shell,
                system: SYSTEM,
                programs: &programs,
                described: &described,
                recent: &recent,
            };
            unifyterm_assist::ask(provider, &ask)
        })
    }

    pub(crate) fn explain(&self, command: String, shell: String) -> Option<Receiver<Result<Vec<Part>, Error>>> {
        if !self.offers(Feature::Explain) {
            return None;
        }
        let ut = self.ut.clone();
        // What the program on the line says of the command used, when it has a list.
        let described = unifyterm_programs::help(&self.programs.now(), &command, DESCRIBED_WITH_A_COMMAND);
        // Starting `ut` takes a moment: on the question's thread, not the window's.
        self.ask(move |provider| {
            let help = ut.as_ref().and_then(|(program, names)| own_help(program, names, &command)).or(described);
            unifyterm_assist::explain(provider, &command, &shell, help.as_deref())
        })
    }
}

/// The help text of one of UnifyTerm's commands, when the command is one. No other program is
/// run: what `--help` does is a program's own business.
fn own_help(ut: &std::path::Path, names: &[String], command: &str) -> Option<String> {
    const MOST: usize = 8 * 1024;
    let name = command.split_whitespace().next()?;
    if !names.iter().any(|known| known == name) {
        return None;
    }
    let mut run = std::process::Command::new(ut);
    run.args([name, "--help"]);
    crate::shell::no_window(&mut run);
    let output = run.output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.chars().take(MOST).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(provider: ProviderSetting) -> AiConfig {
        AiConfig { provider, model: "m".into(), base_url: "http://127.0.0.1:9".into(), ..AiConfig::default() }
    }

    #[test]
    fn not_found_is_read_from_the_output_in_each_shell_s_words() {
        for output in [
            "gti: The term 'gti' is not recognized as a cmdlet, function, script file, or executable program.",
            "'gti' is not recognized as an internal or external command,",
            "bash: gti: command not found",
            "CommandNotFoundException",
            "gti: 无法将“gti”项识别为 cmdlet、函数、脚本文件或可运行程序的名称。",
            "'gti' 不是内部或外部命令，也不是可运行的程序",
        ] {
            assert!(not_found(output, Some(1)), "{output}");
        }
        assert!(not_found("", Some(127)), "127 is the shells' code for it");
        assert!(!not_found("ls: cannot access 'x': No such file or directory", Some(2)));
    }

    /// A helper with one program's command list, whatever is in the data directory.
    fn with_panes(ai: &AiConfig) -> Helper {
        let text = r#"{"opencli": "0.1", "info": {"title": "Panes", "version": "1.2.0"}, "command": {"name": "panes",
            "commands": [{"name": "split-window", "description": "Split a pane in two.",
                          "options": [{"name": "-h", "description": "Side by side."}]},
                         {"name": "kill-pane"}]}}"#;
        let panes = unifyterm_programs::read(text).expect("a description").program;
        Helper::new(ai, None, None).0.with_programs(Programs::fixed(vec![panes]))
    }

    #[test]
    fn a_program_s_own_list_helps_with_no_model_at_all() {
        let helper = with_panes(&AiConfig::default());
        assert!(!helper.has_model());
        let offered: Vec<String> = helper.offers_for("panes sp").into_iter().map(|offer| offer.line).collect();
        assert_eq!(offered, ["panes split-window"]);
        let fixed = helper.list_fix("panes splt-window -h", Some(1)).expect("near a command of the list");
        assert_eq!(fixed.command, "panes split-window -h");
        assert!(fixed.reason.contains("'splt-window'") && fixed.reason.contains("'split-window'"), "{}", fixed.reason);
        assert_eq!(helper.list_fix("panes splt-window", Some(0)), None, "it did not fail");
        assert_eq!(helper.list_fix("panes splt-window", None), None, "a shell that does not say how it ended");
        assert_eq!(helper.list_fix("panes frobnicate", Some(1)), None, "nothing near: nothing to put in its place");
        assert_eq!(helper.list_fix("panes split-window -h", Some(1)), None, "failed for another reason");
        let told = helper.told("panes split-window -h").expect("the list tells it");
        assert_eq!((told.program.as_str(), told.parts.len()), ("panes", 3));
        assert_eq!(told.parts[2], ("-h".to_owned(), "Side by side".to_owned()));
        assert_eq!(helper.told("ls -l"), None, "a program with no list");
        assert_eq!(helper.doubt("panes frobnicate").as_deref(), Some("panes's list has no command 'frobnicate'."));
        assert_eq!(helper.doubt("panes kill-pane"), None);
        assert_eq!(helper.doubt("ls --frobnicate"), None, "no list: no doubt");
    }

    #[test]
    fn the_lists_fix_is_off_with_ai_fix_and_nothing_changes_with_no_list() {
        let off = with_panes(&AiConfig { fix: false, ..AiConfig::default() });
        assert_eq!(off.list_fix("panes splt-window", Some(1)), None, "ai.fix turns every fix off");
        assert!(
            off.told("panes kill-pane").is_some(),
            "telling what a line means sends nothing: not a setting's matter"
        );
        let none = Helper::new(&AiConfig::default(), None, None).0.with_programs(Programs::default());
        assert!(none.offers_for("panes sp").is_empty());
        assert_eq!(none.list_fix("panes splt-window", Some(1)), None);
        assert_eq!(none.told("panes split-window"), None);
        assert_eq!(none.doubt("panes frobnicate"), None);
    }

    #[test]
    fn with_no_model_there_is_the_local_fix_and_nothing_is_asked() {
        let (helper, told) = Helper::new(&AiConfig::default(), None, None);
        assert!(told.is_empty());
        for feature in [Feature::Suggest, Feature::Fix, Feature::Explain, Feature::Ask] {
            assert!(!helper.offers(feature));
        }
        assert!(helper.command_for("list files".into(), String::new(), "pwsh".into(), Vec::new()).is_none());
        assert!(helper.suggest("ls".into(), String::new(), "pwsh".into(), Vec::new()).is_none());
        let recent = vec!["git status".to_owned()];
        let fixed = helper.local_fix("gti log", "bash: gti: command not found", Some(127), &recent).unwrap();
        assert_eq!(fixed.command, "git log");
        assert_eq!(
            helper.local_fix("gti log", "fatal: not a git repository", Some(128), &recent),
            None,
            "found, failed for another reason"
        );
    }

    #[test]
    fn a_key_that_is_not_set_turns_the_model_off_and_says_so() {
        let mut ai = settings(ProviderSetting::Anthropic);
        ai.api_key_env = "UNIFYTERM_TEST_NO_SUCH_VARIABLE_8D2F".into();
        let (helper, told) = Helper::new(&ai, None, None);
        assert!(!helper.offers(Feature::Suggest));
        assert_eq!(told.len(), 1);
        assert!(told[0].contains("UNIFYTERM_TEST_NO_SUCH_VARIABLE_8D2F"), "{told:?}");
    }

    #[test]
    fn nothing_is_sent_before_the_user_agrees() {
        let (helper, _) = Helper::new(&settings(ProviderSetting::OpenaiCompatible), None, None);
        assert!(helper.offers(Feature::Suggest));
        if helper.consent() == Consent::Unasked {
            assert!(helper.suggest("ls".into(), String::new(), "pwsh".into(), Vec::new()).is_none(), "not asked yet");
            helper.refuse();
            assert!(helper.fix("gti".into(), Some(1), String::new(), "pwsh".into()).is_none(), "refused");
        }
    }

    #[test]
    fn a_program_answers_all_but_suggestions() {
        // A program that is not there: nothing is started by these checks.
        let ai = AiConfig {
            provider: ProviderSetting::Command,
            command: vec!["unifyterm-no-such-agent-71b3".into(), "--quiet".into()],
            ..AiConfig::default()
        };
        let (helper, told) = Helper::new(&ai, None, None);
        assert!(told.is_empty(), "{told:?}");
        assert!(helper.has_model());
        // A key's variable left in the file from another provider, and not set: the program answers all the same.
        let left = AiConfig { api_key_env: "UNIFYTERM_TEST_NO_SUCH_VARIABLE_8D2F".into(), ..ai };
        let (with_key_left, told) = Helper::new(&left, None, None);
        assert!(told.is_empty() && with_key_left.has_model(), "{told:?}");
        assert!(!helper.offers(Feature::Suggest), "too slow, and counted, at every pause in typing");
        for feature in [Feature::Fix, Feature::Explain, Feature::Ask] {
            assert!(helper.offers(feature), "{feature:?}");
        }
        assert_eq!(helper.model(), "unifyterm-no-such-agent-71b3", "no model named: the program's name is shown");
        assert!(helper.asked_for.ends_with("unifyterm-no-such-agent-71b3 --quiet"), "{}", helper.asked_for);
        let named =
            AiConfig { provider: ProviderSetting::ClaudeCode, model: "claude-haiku-5-5".into(), ..AiConfig::default() };
        let (helper, _) = Helper::new(&named, None, None);
        assert_eq!(helper.model(), "claude-haiku-5-5");
        assert!(helper.asked_for.contains("--tools") && helper.asked_for.contains("--model claude-haiku-5-5"));
    }

    #[test]
    fn the_settings_turn_the_sentence_off_and_choose_who_presses_enter() {
        let on = settings(ProviderSetting::Ollama);
        let (helper, _) = Helper::new(&on, None, None);
        assert!(helper.offers(Feature::Ask) && !helper.runs_what_only_reads(), "on, and the user presses Enter");
        let (helper, _) = Helper::new(&AiConfig { ask: false, ask_enter: AskEnter::Auto, ..on }, None, None);
        assert!(!helper.offers(Feature::Ask) && helper.runs_what_only_reads());
        // Who agreed before programs could answer is not asked again: the words agreed to are as they were.
        let (helper, _) = Helper::new(&settings(ProviderSetting::OpenaiCompatible), None, None);
        assert_eq!(helper.asked_for, "OpenaiCompatible m http://127.0.0.1:9");
    }
}
