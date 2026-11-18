//! Help with commands, as the settings and the user allow it: the local fix
//! for a misspelt command name always, a language model's suggestion, fix
//! and explanation when `[ai]` names one and the user has agreed, once, to
//! what is sent. Every question to the model runs on a thread of its own;
//! the answer comes back on a channel and the window never waits.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use smol::channel::Receiver;
use unifyterm_assist::{Anthropic, Context, Error, Failure, Fix, OpenAiCompatible, Part, Provider};

use crate::config::{AiConfig, Provider as ProviderSetting};

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
}

/// What may be sent, as the window asks the user.
pub(crate) const WHAT_IS_SENT: &str = "The command line you type (secrets replaced), the directory, your last \
five commands, and for a command that was not found, the last 40 lines it printed.";

/// The help, shared by every tab.
pub(crate) struct Helper {
    settings: AiConfig,
    provider: Option<Arc<dyn Provider>>,
    /// The model's service, name and address: agreeing is agreeing to these.
    asked_for: String,
    consent_file: Option<PathBuf>,
    consent: Mutex<Consent>,
    /// UnifyTerm's commands: their own help goes along with an explanation.
    ut: Option<(PathBuf, Vec<String>)>,
    /// The programs the shells can find, by name: what a misspelt name is compared with.
    known: Arc<OnceLock<Vec<String>>>,
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
            if key.is_none() && settings.provider != ProviderSetting::None {
                told.push(format!(
                    "ai.api_key_env names {}, which is not set: the model's help is off",
                    settings.api_key_env
                ));
            }
            key
        };
        let provider: Option<Arc<dyn Provider>> = match (&settings.provider, key) {
            (ProviderSetting::Anthropic, Some(key)) => {
                Some(Arc::new(Anthropic::new(&settings.base_url, &settings.model, &key)))
            }
            (ProviderSetting::OpenaiCompatible, Some(key)) => {
                Some(Arc::new(OpenAiCompatible::new(&settings.base_url, &settings.model, &key)))
            }
            _ => None,
        };
        let asked_for = format!("{:?} {} {}", settings.provider, settings.model, settings.base_url);
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
            consent_file,
            consent: Mutex::new(if given { Consent::Given } else { Consent::Unasked }),
            ut,
            known,
        };
        (helper, told)
    }

    /// Whether the model is asked for `feature`, as far as the settings go (consent aside).
    pub(crate) fn offers(&self, feature: Feature) -> bool {
        self.provider.is_some()
            && match feature {
                Feature::Suggest => self.settings.suggest,
                Feature::Fix => self.settings.fix,
                Feature::Explain => self.settings.explain,
            }
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

    /// The model's name, for the window to show.
    pub(crate) fn model(&self) -> &str {
        &self.settings.model
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

    pub(crate) fn explain(&self, command: String, shell: String) -> Option<Receiver<Result<Vec<Part>, Error>>> {
        if !self.offers(Feature::Explain) {
            return None;
        }
        let ut = self.ut.clone();
        // Starting `ut` takes a moment: on the question's thread, not the window's.
        self.ask(move |provider| {
            let help = ut.as_ref().and_then(|(program, names)| own_help(program, names, &command));
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

    #[test]
    fn with_no_model_there_is_the_local_fix_and_nothing_is_asked() {
        let (helper, told) = Helper::new(&AiConfig::default(), None, None);
        assert!(told.is_empty());
        for feature in [Feature::Suggest, Feature::Fix, Feature::Explain] {
            assert!(!helper.offers(feature));
        }
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
}
