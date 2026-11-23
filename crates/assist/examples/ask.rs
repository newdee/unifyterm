//! Asks a model for the command of each sentence in a file and says how
//! many answers have what was expected, and how long they took. For
//! measuring a model before its numbers are written anywhere.
//!
//!   cargo run -p unifyterm-assist --example ask -- SETTINGS SHELL SENTENCES [JOBS]
//!
//! SETTINGS: a settings file whose `[ai]` names the model (as `config.toml`).
//! SHELL: `pwsh` or `zsh`: the shell the commands are for (pwsh on Windows, zsh on macOS).
//! SENTENCES: a file of lines `sentence<TAB>expected in pwsh<TAB>expected in zsh`; a line
//! that begins with `#` is passed over. "Expected" is words with `|` between them: the
//! answer has what was expected when its command has one of them (letters' case aside).
//! JOBS: how many questions are asked at a time (1 when left out).
//!
//! The sentences go to the model the settings name: with a service's key, or an agent's
//! plan, that is counted. Nothing is run: the commands are only printed.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use unifyterm_assist::{Anthropic, Ask, Command, Error, OpenAiCompatible, Provider, Tokens, ask};
use unifyterm_config::providers::Reach;
use unifyterm_config::{Config, Provider as Named};

/// One sentence and what its command is expected to have.
struct Case {
    words: String,
    expected: Vec<String>,
}

/// What became of one sentence.
struct Outcome {
    words: String,
    took: Duration,
    answer: Result<Option<String>, Error>,
    has_expected: bool,
}

fn provider(config: &Config, directory: PathBuf) -> Result<Arc<dyn Provider>, String> {
    let ai = &config.ai;
    let key = if ai.api_key_env.is_empty() {
        String::new()
    } else {
        std::env::var(&ai.api_key_env).map_err(|_| format!("{} is not set", ai.api_key_env))?
    };
    Ok(match ai.provider.reach() {
        Reach::Nowhere => return Err("the settings name no model: [ai] provider".into()),
        Reach::Anthropic => Arc::new(Anthropic::new(&ai.base_url, &ai.model, &key)),
        Reach::Chat { .. } => {
            let tokens = if ai.provider == Named::Openai { Tokens::MaxCompletion } else { Tokens::Max };
            Arc::new(OpenAiCompatible::new(&ai.address(), &ai.model, &key).with_tokens(tokens))
        }
        Reach::Program { .. } | Reach::OwnProgram => {
            Arc::new(Command::new(ai.program().unwrap_or_default(), directory))
        }
    })
}

fn cases(text: &str, shell: &str) -> Vec<Case> {
    let column = if shell == "pwsh" { 1 } else { 2 };
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let expected = fields.get(column)?.split('|').map(|word| word.trim().to_lowercase()).collect();
            Some(Case { words: (*fields.first()?).to_owned(), expected })
        })
        .collect()
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [settings, shell, sentences, rest @ ..] = arguments.as_slice() else {
        return Err("usage: ask SETTINGS SHELL SENTENCES [JOBS]".into());
    };
    let jobs: usize = rest.first().map_or(Ok(1), |jobs| jobs.parse()).map_err(|_| "JOBS is a number")?;
    let config = Config::load(std::path::Path::new(settings)).map_err(|error| format!("{error:#}"))?;
    let directory = std::env::temp_dir().join(format!("unifyterm-ask-{}", std::process::id()));
    let provider = provider(&config, directory.clone())?;
    let text = std::fs::read_to_string(sentences).map_err(|error| format!("{sentences}: {error}"))?;
    let system = if shell == "pwsh" { "Windows" } else { "macOS" };
    let waiting = Arc::new(Mutex::new(cases(&text, shell)));
    let done = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    let workers: Vec<_> = (0..jobs.max(1))
        .map(|_| {
            let (waiting, done, provider, shell) =
                (Arc::clone(&waiting), Arc::clone(&done), Arc::clone(&provider), shell.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(case) = waiting.lock().expect("lock poisoned").pop() else { break };
                    let asked = Instant::now();
                    let question =
                        Ask { words: &case.words, cwd: "~/work", shell: &shell, system, programs: &[], recent: &[] };
                    let answer = ask(provider.as_ref(), &question).map(|asked| asked.map(|asked| asked.command));
                    let has_expected = matches!(&answer, Ok(Some(command))
                        if case.expected.iter().any(|word| command.to_lowercase().contains(word)));
                    let outcome = Outcome { words: case.words, took: asked.elapsed(), answer, has_expected };
                    done.lock().expect("lock poisoned").push(outcome);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().map_err(|_| "a worker stopped")?;
    }
    let _ = std::fs::remove_dir_all(&directory);
    let mut done = std::mem::take(&mut *done.lock().expect("lock poisoned"));
    done.sort_by(|a, b| a.words.cmp(&b.words));
    for outcome in &done {
        let said = match &outcome.answer {
            Ok(Some(command)) => command.clone(),
            Ok(None) => "(no command)".to_owned(),
            Err(error) => format!("({error})"),
        };
        let mark = if outcome.has_expected { "ok  " } else { "MISS" };
        println!("{mark} {:>5.1}s  {}  =>  {said}", outcome.took.as_secs_f32(), outcome.words);
    }
    let mut times: Vec<Duration> = done.iter().map(|outcome| outcome.took).collect();
    times.sort_unstable();
    let count = |which: fn(&Outcome) -> bool| done.iter().filter(|outcome| which(outcome)).count();
    println!(
        "{shell}: {} sentences; {} with what was expected; {} with no command; {} not usable; {} other failures; \
         median {:.1} s, longest {:.1} s; {:.0} s in all with {jobs} at a time",
        done.len(),
        count(|outcome| outcome.has_expected),
        count(|outcome| matches!(outcome.answer, Ok(None))),
        count(|outcome| matches!(outcome.answer, Err(Error::BadAnswer(_)))),
        count(|outcome| matches!(&outcome.answer, Err(error) if !matches!(error, Error::BadAnswer(_)))),
        times.get(times.len() / 2).map_or(0.0, Duration::as_secs_f32),
        times.last().map_or(0.0, Duration::as_secs_f32),
        started.elapsed().as_secs_f32(),
    );
    Ok(())
}
