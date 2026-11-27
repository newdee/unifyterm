//! `ut mail`: the mailboxes UnifyTerm keeps (`[mail]` and `[mail.accounts.NAME]` in the
//! settings), as text: for a person in a shell, a script, or an agent. The window shows them
//! as conversations (Ctrl-Shift-M). A draft may be written here by anyone; `ut mail send` asks
//! on the terminal before it sends.

use std::ffi::OsString;
use std::io::{self, BufRead as _, IsTerminal as _, Read as _, Write as _};

use anyhow::{Context as _, Result, anyhow, bail};
use unifyterm_config::{Config, MailAccount, MailAuth, MailboxSettings};
use unifyterm_mail::{Credential, Filter, Plan, Step, Store, Summary, Thread, message, oauth};

const USAGE: &str = "usage: ut mail [list] [--unread|--waiting|--mine|--all] [--account NAME] [-n N] [--json]
                                          the conversations, newest first; of all mailboxes unless --account
       ut mail show THREAD [--json]       one conversation's mails, oldest first
       ut mail search TEXT [--account NAME] [--json]  the conversations with TEXT in a subject, a mail or a sender
       ut mail draft THREAD FILE|- [--by NAME]  keep FILE (- reads standard input) as the answer waiting in THREAD
       ut mail send THREAD                show THREAD's draft and ask, on this terminal, before sending it
       ut mail sync [--account NAME]      read the servers' new mail now (when no UnifyTerm window does)
       ut mail status                     each mailbox: its address, what its sync last said, how many are unread
       ut mail account list               the mailboxes, by name
       ut mail account add NAME ADDRESS [--imap HOST:PORT] [--smtp HOST:PORT] [--login NAME] [--proxy URL]
                                        [--auth password|oauth2]
                                          try a mailbox, sign in, and keep it in the settings
       ut mail account remove NAME [--yes]  take a mailbox out: its settings, its password, the mail kept here
       ut mail login [--account NAME]     keep the password (typed, or read from standard input), or sign in
                                          in the browser (OAuth2: Outlook, Hotmail, Microsoft 365)
       ut mail logout [--account NAME]    take it away

--waiting: the newest mail is someone else's (it waits for an answer); --mine: the newest is yours.
In UnifyTerm, Ctrl-Shift-M shows the mail as conversations; Ctrl-Alt-Shift-M adds a mailbox there.";

/// The most conversations a list gives unless asked for more.
const LIST: usize = 30;

pub(crate) fn main(args: &[OsString]) -> i32 {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match words.as_slice() {
        ["--help" | "-h" | "help"] => {
            let _ = writeln!(io::stdout(), "{USAGE}");
            return 0;
        }
        [] => list(&[]),
        ["list", rest @ ..] => list(rest),
        ["show", thread, rest @ ..] => show(thread, rest),
        ["search", text, rest @ ..] => search(text, rest),
        ["draft", thread, file, rest @ ..] => draft(thread, file, rest),
        ["send", thread] => send(thread),
        ["sync", rest @ ..] => sync(rest),
        ["status"] => status(),
        ["account" | "accounts", "list"] | ["account" | "accounts"] => accounts(),
        ["account", "add", name, address, rest @ ..] => add(name, address, rest),
        ["account", "remove", name, rest @ ..] => remove(name, rest),
        ["login", rest @ ..] => login(rest),
        ["logout", rest @ ..] => logout(rest),
        _ => {
            let _ = writeln!(io::stderr(), "{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "ut mail: {error:#}");
            1
        }
    }
}

/// The settings in use.
fn config() -> Result<Config> {
    Config::load(&crate::setting::file_path(None)?)
}

/// The mailbox named `name`, or the only one when no name is given.
fn mailbox(config: &Config, name: Option<&str>) -> Result<MailboxSettings> {
    let all = config.mail.mailboxes();
    let names = || all.iter().map(|mailbox| mailbox.name.as_str()).collect::<Vec<_>>().join(", ");
    match name {
        Some(name) => all
            .iter()
            .find(|mailbox| mailbox.name == name)
            .cloned()
            .ok_or_else(|| anyhow!("there is no mailbox {name} (the mailboxes: {})", names())),
        None => match all.as_slice() {
            [] => Err(anyhow!("no mailbox is set: ut mail account add NAME ADDRESS")),
            [one] => Ok(one.clone()),
            _ => Err(anyhow!("there are several mailboxes: say which with --account NAME ({})", names())),
        },
    }
}

/// The plan of the mailbox named `name` (or the only one).
fn plan(config: &Config, name: Option<&str>) -> Result<Plan> {
    unifyterm_mail::plan(&mailbox(config, name)?, &config.mail)
}

/// The mail kept here, opened; an error when no mail was ever read (nothing is made then).
fn store(config: &Config) -> Result<Store> {
    if !unifyterm_mail::default_path()?.exists() {
        bail!("no mail read yet: open UnifyTerm, or run ut mail sync");
    }
    unifyterm_mail::open(&config.mail)
}

fn thread_id(text: &str) -> Result<i64> {
    text.trim_start_matches('#').parse().with_context(|| format!("\"{text}\" is not a conversation's number"))
}

/// How a list or a conversation is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Text,
    Json,
}

struct Flags {
    form: Form,
    filter: Filter,
    limit: usize,
    account: Option<String>,
}

/// `--json`, a filter, `--account NAME`, `-n N` among `rest`; any other word is an error.
fn flags(rest: &[&str]) -> Result<Flags> {
    let mut flags = Flags { form: Form::Text, filter: Filter::Inbox, limit: LIST, account: None };
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match *word {
            "--json" => flags.form = Form::Json,
            "--unread" => flags.filter = Filter::Unread,
            "--waiting" => flags.filter = Filter::Waiting,
            "--mine" => flags.filter = Filter::Mine,
            "--all" => flags.filter = Filter::All,
            "-n" => flags.limit = words.next().and_then(|n| n.parse().ok()).context("-n takes a number")?,
            "--account" => {
                flags.account = Some((*words.next().context("--account takes a mailbox's name")?).to_owned());
            }
            other => bail!("{other}: not an option here ('ut mail --help')"),
        }
    }
    Ok(flags)
}

/// The address of the mailbox `--account` names, or none (all mailboxes).
fn scope(config: &Config, flags: &Flags) -> Result<Option<String>> {
    flags.account.as_deref().map(|name| mailbox(config, Some(name)).map(|mailbox| mailbox.address)).transpose()
}

fn list(rest: &[&str]) -> Result<()> {
    let flags = flags(rest)?;
    let config = config()?;
    let scope = scope(&config, &flags)?;
    let threads = store(&config)?.threads(flags.limit, flags.filter, scope.as_deref())?;
    print_list(&config, &threads, flags.form)
}

fn search(text: &str, rest: &[&str]) -> Result<()> {
    let flags = flags(rest)?;
    let config = config()?;
    let scope = scope(&config, &flags)?;
    let threads = store(&config)?.search(text, flags.limit, scope.as_deref())?;
    print_list(&config, &threads, flags.form)
}

/// A time as a list shows it: local, to the minute.
fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The name of the mailbox at `address`, or the address when the settings no longer name it.
fn name_of(config: &Config, address: &str) -> String {
    config
        .mail
        .mailboxes()
        .into_iter()
        .find(|mailbox| mailbox.address == address)
        .map_or_else(|| address.to_owned(), |mailbox| mailbox.name)
}

fn print_list(config: &Config, threads: &[Summary], form: Form) -> Result<()> {
    let mut out = io::stdout().lock();
    if form == Form::Json {
        writeln!(out, "{}", serde_json::to_string_pretty(threads)?)?;
        return Ok(());
    }
    if threads.is_empty() {
        writeln!(out, "no conversations")?;
    }
    // The mailbox's name only when there are several.
    let several = config.mail.mailboxes().len() > 1;
    for thread in threads {
        let unread = if thread.unread > 0 { format!("{} new", thread.unread) } else { String::new() };
        let draft = thread.draft_by.as_ref().map(|by| format!("  [draft by {by}]")).unwrap_or_default();
        let from = if several { format!("{}  ", cut(&name_of(config, &thread.account), 8)) } else { String::new() };
        writeln!(
            out,
            "{:>5}  {}  {from}{:<7} {}  {}{draft}",
            thread.id,
            when(thread.last_at_ms),
            unread,
            cut(&thread.with, 18),
            thread.subject
        )?;
    }
    Ok(())
}

/// `text` in at most `chars` characters, padded to that many.
fn cut(text: &str, chars: usize) -> String {
    let mut shown: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        shown.pop();
        shown.push('…');
    }
    format!("{shown:<chars$}")
}

fn show(thread: &str, rest: &[&str]) -> Result<()> {
    let flags = flags(rest)?;
    let id = thread_id(thread)?;
    let config = config()?;
    let thread = store(&config)?.thread(id)?.with_context(|| format!("there is no conversation {id}"))?;
    print_thread(&config, &thread, flags.form)
}

fn print_thread(config: &Config, thread: &Thread, form: Form) -> Result<()> {
    let mut out = io::stdout().lock();
    if form == Form::Json {
        writeln!(out, "{}", serde_json::to_string_pretty(thread)?)?;
        return Ok(());
    }
    let mailbox = name_of(config, &thread.summary.account);
    writeln!(out, "{}  (conversation {}, mailbox {mailbox})", thread.summary.subject, thread.summary.id)?;
    for mail in &thread.mails {
        let who = if mail.outgoing { "you".to_owned() } else { mail.from.full() };
        writeln!(out, "\n── {who} · {}", when(mail.at_ms))?;
        let (own, quoted) = message::fold_quote(&mail.text);
        writeln!(out, "{own}")?;
        if quoted > 0 {
            writeln!(out, "   ({quoted} quoted lines)")?;
        }
        for file in &mail.attachments {
            writeln!(out, "   attached: {} ({} KB)", file.name, file.bytes.div_ceil(1024))?;
        }
    }
    if let Some(draft) = &thread.draft {
        writeln!(out, "\n── draft by {}, not sent:\n{}", draft.author, draft.text)?;
    }
    Ok(())
}

fn draft(thread: &str, file: &str, rest: &[&str]) -> Result<()> {
    let by = match rest {
        [] => unifyterm_mail::BY_HAND.to_owned(),
        ["--by", name] if !name.trim().is_empty() => name.trim().to_owned(),
        _ => bail!("after the file only --by NAME may come"),
    };
    let id = thread_id(thread)?;
    let text = if file == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        text
    } else {
        std::fs::read_to_string(file).with_context(|| format!("read {file}"))?
    };
    if text.trim().is_empty() {
        bail!("the draft is empty");
    }
    store(&config()?)?.save_draft(id, &by, text.trim_end(), unifyterm_history::now_ms())?;
    writeln!(
        io::stdout(),
        "kept as the draft of conversation {id}: send it in UnifyTerm (Ctrl-Shift-M) or with ut mail send {id}"
    )?;
    Ok(())
}

fn send(thread: &str) -> Result<()> {
    let id = thread_id(thread)?;
    let config = config()?;
    let store = store(&config)?;
    let conversation = store.thread(id)?.with_context(|| format!("there is no conversation {id}"))?;
    let draft = conversation
        .draft
        .clone()
        .with_context(|| format!("conversation {id} has no draft: ut mail draft {id} FILE"))?;
    if !io::stdin().is_terminal() {
        bail!("ut mail send asks before it sends, on a terminal: run it in one");
    }
    let plans: Vec<Plan> = unifyterm_mail::plans(&config.mail).into_iter().filter_map(|(_, plan)| plan.ok()).collect();
    let answer = unifyterm_mail::send::answer(&conversation, &conversation.summary.account, &draft.text)?;
    let mut out = io::stdout().lock();
    writeln!(out, "From: {}", conversation.summary.account)?;
    writeln!(out, "To: {}", answer.to.iter().map(unifyterm_mail::Address::full).collect::<Vec<_>>().join(", "))?;
    if !answer.cc.is_empty() {
        writeln!(out, "Cc: {}", answer.cc.iter().map(unifyterm_mail::Address::full).collect::<Vec<_>>().join(", "))?;
    }
    writeln!(out, "Subject: {}\n\n{}\n", answer.subject, answer.text)?;
    write!(out, "Send this (written by {})? [y/N] ", draft.author)?;
    out.flush()?;
    let mut said = String::new();
    io::stdin().lock().read_line(&mut said)?;
    if !said.trim().eq_ignore_ascii_case("y") {
        writeln!(out, "not sent")?;
        return Ok(());
    }
    unifyterm_mail::send_answer(&plans, id, &draft.text)?;
    writeln!(out, "sent")?;
    Ok(())
}

fn sync(rest: &[&str]) -> Result<()> {
    let name = match rest {
        [] => None,
        ["--account", name] => Some(*name),
        _ => bail!("ut mail sync takes only --account NAME"),
    };
    let config = config()?;
    let directory = unifyterm_mail::default_path()?.parent().context("the mail's file has no folder")?.to_path_buf();
    let mut failed = 0;
    for (mailbox, plan) in unifyterm_mail::plans(&config.mail) {
        if name.is_some_and(|name| name != mailbox.name) {
            continue;
        }
        let outcome = plan.and_then(|plan| {
            let Some(_lock) = unifyterm_mail::Lock::take(&directory, &plan.account.address)? else {
                bail!("a UnifyTerm window is syncing it already");
            };
            unifyterm_mail::open(&config.mail)?;
            unifyterm_mail::sync::once(&plan)
        });
        match outcome {
            Ok(new) => writeln!(io::stdout(), "{}: synced, {} new", mailbox.name, new.len())?,
            Err(error) => {
                failed += 1;
                writeln!(io::stderr(), "{}: {error:#}", mailbox.name)?;
            }
        }
    }
    if let Some(name) = name {
        mailbox(&config, Some(name))?;
    }
    if failed > 0 {
        bail!("{failed} mailbox(es) not synced");
    }
    Ok(())
}

fn status() -> Result<()> {
    let config = config()?;
    let plans = unifyterm_mail::plans(&config.mail);
    if plans.is_empty() {
        bail!("no mailbox is set: ut mail account add NAME ADDRESS");
    }
    let store = if unifyterm_mail::default_path()?.exists() { Some(unifyterm_mail::open(&config.mail)?) } else { None };
    let mut out = io::stdout().lock();
    for (at, (mailbox, plan)) in plans.iter().enumerate() {
        if at > 0 {
            writeln!(out)?;
        }
        writeln!(out, "{}: {}", mailbox.name, mailbox.address)?;
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                writeln!(out, "  settings: {error:#}")?;
                continue;
            }
        };
        let auth = match plan.account.auth {
            unifyterm_mail::Auth::Password => "password",
            unifyterm_mail::Auth::OAuth2 => "sign-in in the browser (OAuth2)",
        };
        writeln!(out, "  login:  {} ({auth})", plan.account.login)?;
        writeln!(out, "  imap:   {}:{}", plan.account.imap.host, plan.account.imap.port)?;
        writeln!(out, "  smtp:   {}:{}", plan.account.smtp.host, plan.account.smtp.port)?;
        if let Some(proxy) = &plan.account.proxy {
            writeln!(out, "  proxy:  {}:{}", proxy.host, proxy.port)?;
        }
        let Some(store) = &store else {
            writeln!(out, "  no mail read yet")?;
            continue;
        };
        if let Some(status) = store.status(&mailbox.address)? {
            writeln!(out, "  sync:   {} ({})", status.text, when(status.at_ms))?;
        }
        writeln!(out, "  unread: {}", store.unread(Some(&mailbox.address))?)?;
    }
    Ok(())
}

fn accounts() -> Result<()> {
    let config = config()?;
    let mut out = io::stdout().lock();
    let all = config.mail.mailboxes();
    if all.is_empty() {
        writeln!(out, "no mailboxes: ut mail account add NAME ADDRESS")?;
    }
    for mailbox in all {
        writeln!(out, "{:<12} {}", mailbox.name, mailbox.address)?;
    }
    Ok(())
}

fn add(name: &str, address: &str, rest: &[&str]) -> Result<()> {
    let mut account = MailAccount { address: address.trim().to_owned(), ..MailAccount::default() };
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let mut value =
            || words.next().map(|value| (*value).to_owned()).with_context(|| format!("{word} takes a value"));
        match *word {
            "--imap" => account.imap = value()?,
            "--smtp" => account.smtp = value()?,
            "--login" => account.login = value()?,
            "--proxy" => account.proxy = value()?,
            "--auth" => {
                account.auth = match value()?.as_str() {
                    "password" => MailAuth::Password,
                    "oauth2" => MailAuth::Oauth2,
                    "auto" => MailAuth::Auto,
                    other => bail!("--auth is password or oauth2, not {other}"),
                }
            }
            other => bail!("{other}: not an option here ('ut mail --help')"),
        }
    }
    let path = crate::setting::file_path(None)?;
    let mut file = unifyterm_config::File::open(&path)?;
    // Refused here (a name taken, an address set twice) before anything is asked.
    file.set_mailbox(name, &account)?;
    let config = Config::parse(&file.text())?;
    let plan = plan(&config, Some(name))?;
    let (credential, tokens) = sign_in(&plan)?;
    if !tried(&plan, &credential) {
        bail!("not added: nothing was kept");
    }
    unifyterm_mail::keep_credential(&plan.account, &credential, tokens.as_ref())?;
    file.save()?;
    writeln!(io::stdout(), "added the mailbox {name}: a UnifyTerm window reads its mail now; ut mail sync does too")?;
    Ok(())
}

fn remove(name: &str, rest: &[&str]) -> Result<()> {
    let config = config()?;
    let mailbox = mailbox(&config, Some(name))?;
    let sure = match rest {
        ["--yes"] => true,
        [] if io::stdin().is_terminal() => {
            write!(
                io::stdout(),
                "take {name} ({}) out of UnifyTerm: its settings, its kept password, the mail kept here? \
                 The server keeps its mail. [y/N] ",
                mailbox.address
            )?;
            io::stdout().flush()?;
            let mut said = String::new();
            io::stdin().lock().read_line(&mut said)?;
            said.trim().eq_ignore_ascii_case("y")
        }
        [] => bail!("not on a terminal: add --yes to remove {name}"),
        _ => bail!("ut mail account remove NAME takes only --yes"),
    };
    if !sure {
        writeln!(io::stdout(), "not removed")?;
        return Ok(());
    }
    let mut file = unifyterm_config::File::open(&crate::setting::file_path(None)?)?;
    file.remove_mailbox(name)?;
    file.save()?;
    let login = if mailbox.login.trim().is_empty() { mailbox.address.clone() } else { mailbox.login.trim().to_owned() };
    unifyterm_mail::forget_credential(&login)?;
    if unifyterm_mail::default_path()?.exists() {
        Store::open(&unifyterm_mail::default_path()?)?.forget_account(&mailbox.address)?;
    }
    writeln!(io::stdout(), "removed the mailbox {name}")?;
    Ok(())
}

/// What lets the mailbox in, got from the person: a password typed (or read from standard
/// input), or a sign-in in the browser with a code shown here.
fn sign_in(plan: &Plan) -> Result<(Credential, Option<oauth::Tokens>)> {
    let mut out = io::stdout().lock();
    if plan.account.auth == unifyterm_mail::Auth::OAuth2 {
        let code = oauth::start(&plan.provider, plan.account.proxy.as_ref())?;
        writeln!(
            out,
            "sign in as {}: open {} and enter the code {}",
            plan.account.address, code.verification_uri, code.user_code
        )?;
        writeln!(out, "waiting for the sign-in (Ctrl-C stops)…")?;
        out.flush()?;
        let tokens =
            oauth::finish(&plan.provider, plan.account.proxy.as_ref(), &code, &unifyterm_history::now_ms, &|| false)
                .map_err(|refusal| anyhow!("{refusal}"))?;
        return Ok((Credential::Token(tokens.access.clone()), Some(tokens)));
    }
    let password = if io::stdin().is_terminal() {
        write!(out, "password for {} (an app password or authorization code; not shown): ", plan.account.login)?;
        out.flush()?;
        let typed = hidden_line()?;
        writeln!(out)?;
        typed
    } else {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    if password.is_empty() {
        bail!("no password given");
    }
    Ok((Credential::Password(password), None))
}

/// Tries the mailbox, saying each step; whether all went.
fn tried(plan: &Plan, credential: &Credential) -> bool {
    let mut out = io::stdout().lock();
    let mut told = |step: Step, outcome: Result<String, String>| {
        let what = match step {
            Step::Connect => "connect",
            Step::Login => "log in",
            Step::SentFolder => "sent folder",
            Step::Smtp => "smtp",
        };
        let _ = match outcome {
            Ok(word) => writeln!(out, "  ok  {what}: {word}"),
            Err(why) => writeln!(out, "  no  {what}: {why}"),
        };
    };
    unifyterm_mail::check(&plan.account, credential, &mut told)
}

/// `--account NAME`, or nothing.
fn account_flag<'a>(rest: &[&'a str]) -> Result<Option<&'a str>> {
    match rest {
        [] => Ok(None),
        ["--account", name] => Ok(Some(*name)),
        _ => bail!("only --account NAME may come here"),
    }
}

fn login(rest: &[&str]) -> Result<()> {
    let config = config()?;
    let plan = plan(&config, account_flag(rest)?)?;
    let (credential, tokens) = sign_in(&plan)?;
    if !tried(&plan, &credential) {
        bail!("not kept");
    }
    unifyterm_mail::keep_credential(&plan.account, &credential, tokens.as_ref())?;
    writeln!(io::stdout(), "{} is let in; what lets it in is kept in the system's keychain", plan.account.login)?;
    Ok(())
}

fn logout(rest: &[&str]) -> Result<()> {
    let config = config()?;
    let plan = plan(&config, account_flag(rest)?)?;
    unifyterm_mail::forget_credential(&plan.account.login)?;
    writeln!(io::stdout(), "what let {} in is no longer kept", plan.account.login)?;
    Ok(())
}

/// A line typed on the terminal, not shown as it is typed. Ctrl-C ends it with nothing.
fn hidden_line() -> Result<String> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
    crossterm::terminal::enable_raw_mode()?;
    let mut typed = String::new();
    let result = loop {
        match read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => break Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    typed.clear();
                    break Ok(());
                }
                KeyCode::Char(c) => typed.push(c),
                KeyCode::Backspace => {
                    typed.pop();
                }
                _ => {}
            },
            Ok(Event::Paste(text)) => typed.push_str(text.trim_end_matches(['\r', '\n'])),
            Ok(_) => {}
            Err(error) => break Err(error),
        }
    };
    crossterm::terminal::disable_raw_mode()?;
    result?;
    Ok(typed)
}
