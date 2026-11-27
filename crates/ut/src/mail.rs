//! `ut mail`: the mailbox UnifyTerm keeps (`[mail]` in the settings), as
//! text: for a person in a shell, a script, or an agent. The window shows it
//! as a conversation (Ctrl-Shift-M). A draft may be written here by anyone;
//! `ut mail send` asks on the terminal before it sends.

use std::ffi::OsString;
use std::io::{self, BufRead as _, IsTerminal as _, Read as _, Write as _};

use anyhow::{Context as _, Result, anyhow, bail};
use unifyterm_config::Config;
use unifyterm_mail::{Filter, Plan, Store, Summary, Thread, message};

const USAGE: &str = "usage: ut mail [list] [--unread] [--all] [-n N] [--json]   the conversations, newest first
       ut mail show THREAD [--json]       one conversation's mails, oldest first
       ut mail search TEXT [--json]       the conversations with TEXT in a subject, a mail or a sender
       ut mail draft THREAD FILE|- [--by NAME]  keep FILE (- reads standard input) as the answer waiting in THREAD
       ut mail send THREAD                show THREAD's draft and ask, on this terminal, before sending it
       ut mail sync                       read the server's new mail now (when no UnifyTerm window does)
       ut mail status                     the address, what the sync last said, how many are unread
       ut mail login                      keep the password in the system's keychain (typed, or read from standard input)
       ut mail logout                     take it away

The mailbox is [mail] in the settings (ut setting): mail.address, and for a domain UnifyTerm does
not know, mail.imap and mail.smtp. In UnifyTerm, Ctrl-Shift-M shows it as a conversation.";

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
        ["sync"] => sync(),
        ["status"] => status(),
        ["login"] => login(),
        ["logout"] => logout(),
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

/// The plan of the settings in use; an error that says what to set when there is no mailbox.
fn plan() -> Result<Plan> {
    let file = crate::setting::file_path(None)?;
    let config = Config::load(&file)?;
    unifyterm_mail::plan(&config.mail)?
        .ok_or_else(|| anyhow!("no mailbox is set: ut setting set mail.address me@example.com, then ut mail login"))
}

/// The mailbox's file, opened; an error when no mail was ever read (nothing is made then).
fn store(plan: &Plan) -> Result<Store> {
    if !plan.store.exists() {
        bail!("no mail read yet: open UnifyTerm, or run ut mail sync");
    }
    Store::open(&plan.store)
}

fn thread_id(text: &str) -> Result<i64> {
    text.trim_start_matches('#').parse().with_context(|| format!("\"{text}\" is not a conversation's number"))
}

/// `--json`, `--unread`, `--all`, `-n N` among `rest`; any other word is an error.
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
}

fn flags(rest: &[&str]) -> Result<Flags> {
    let mut flags = Flags { form: Form::Text, filter: Filter::Inbox, limit: LIST };
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        match *word {
            "--json" => flags.form = Form::Json,
            "--unread" => flags.filter = Filter::Unread,
            "--all" => flags.filter = Filter::All,
            "-n" => flags.limit = words.next().and_then(|n| n.parse().ok()).context("-n takes a number")?,
            other => bail!("{other}: not an option here ('ut mail --help')"),
        }
    }
    Ok(flags)
}

fn list(rest: &[&str]) -> Result<()> {
    let flags = flags(rest)?;
    let threads = store(&plan()?)?.threads(flags.limit, flags.filter)?;
    print_list(&threads, flags.form)
}

fn search(text: &str, rest: &[&str]) -> Result<()> {
    let flags = flags(rest)?;
    let threads = store(&plan()?)?.search(text, flags.limit)?;
    print_list(&threads, flags.form)
}

/// A time as a list shows it: local, to the minute.
fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn print_list(threads: &[Summary], form: Form) -> Result<()> {
    let mut out = io::stdout().lock();
    if form == Form::Json {
        writeln!(out, "{}", serde_json::to_string_pretty(threads)?)?;
        return Ok(());
    }
    if threads.is_empty() {
        writeln!(out, "no conversations")?;
    }
    for thread in threads {
        let unread = if thread.unread > 0 { format!("{} new", thread.unread) } else { String::new() };
        let draft = thread.draft_by.as_ref().map(|by| format!("  [draft by {by}]")).unwrap_or_default();
        writeln!(
            out,
            "{:>5}  {}  {:<7} {}  {}{draft}",
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
    let thread = store(&plan()?)?.thread(id)?.with_context(|| format!("there is no conversation {id}"))?;
    print_thread(&thread, flags.form)
}

fn print_thread(thread: &Thread, form: Form) -> Result<()> {
    let mut out = io::stdout().lock();
    if form == Form::Json {
        writeln!(out, "{}", serde_json::to_string_pretty(thread)?)?;
        return Ok(());
    }
    writeln!(out, "{}  (conversation {})", thread.summary.subject, thread.summary.id)?;
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
    store(&plan()?)?.save_draft(id, &by, text.trim_end(), unifyterm_history::now_ms())?;
    writeln!(
        io::stdout(),
        "kept as the draft of conversation {id}: send it in UnifyTerm (Ctrl-Shift-M) or with ut mail send {id}"
    )?;
    Ok(())
}

fn send(thread: &str) -> Result<()> {
    let id = thread_id(thread)?;
    let plan = plan()?;
    let store = store(&plan)?;
    let conversation = store.thread(id)?.with_context(|| format!("there is no conversation {id}"))?;
    let draft = conversation
        .draft
        .clone()
        .with_context(|| format!("conversation {id} has no draft: ut mail draft {id} FILE"))?;
    if !io::stdin().is_terminal() {
        bail!("ut mail send asks before it sends, on a terminal: run it in one");
    }
    let answer = unifyterm_mail::send::answer(&conversation, &plan.account.address, &draft.text)?;
    let mut out = io::stdout().lock();
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
    unifyterm_mail::send_answer(&plan, id, &draft.text)?;
    writeln!(out, "sent")?;
    Ok(())
}

fn sync() -> Result<()> {
    let plan = plan()?;
    let directory = plan.store.parent().context("the mailbox's file has no folder")?.to_path_buf();
    let Some(_lock) = unifyterm_mail::Lock::take(&directory)? else {
        bail!("a UnifyTerm window is syncing this mailbox already");
    };
    let new = unifyterm_mail::sync::once(&plan)?;
    writeln!(io::stdout(), "synced: {} new", new.len())?;
    Ok(())
}

fn status() -> Result<()> {
    let plan = plan()?;
    let mut out = io::stdout().lock();
    writeln!(out, "address: {}", plan.account.address)?;
    writeln!(out, "imap:    {}:{}", plan.account.imap.host, plan.account.imap.port)?;
    writeln!(out, "smtp:    {}:{}", plan.account.smtp.host, plan.account.smtp.port)?;
    if let Some(proxy) = &plan.account.proxy {
        writeln!(out, "proxy:   {}:{}", proxy.host, proxy.port)?;
    }
    if !plan.store.exists() {
        writeln!(out, "no mail read yet")?;
        return Ok(());
    }
    let store = Store::open(&plan.store)?;
    if let Some(status) = store.status()? {
        writeln!(out, "sync:    {} ({})", status.text, when(status.at_ms))?;
    }
    writeln!(out, "unread:  {}", store.unread()?)?;
    Ok(())
}

fn login() -> Result<()> {
    let plan = plan()?;
    let password = if io::stdin().is_terminal() {
        write!(
            io::stdout(),
            "password for {} (an app password or authorization code; not shown): ",
            plan.account.login
        )?;
        io::stdout().flush()?;
        let typed = hidden_line()?;
        writeln!(io::stdout())?;
        typed
    } else {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    if password.is_empty() {
        bail!("no password given");
    }
    match unifyterm_mail::net::login(&plan.account, &password) {
        Ok(mut session) => {
            session.logout().ok();
        }
        Err(failure) => bail!("not kept: {failure}"),
    }
    unifyterm_mail::secret::keep(&plan.account.login, &password)?;
    writeln!(io::stdout(), "the server let {} in; the password is kept in the system's keychain", plan.account.login)?;
    Ok(())
}

fn logout() -> Result<()> {
    let plan = plan()?;
    unifyterm_mail::secret::forget(&plan.account.login)?;
    writeln!(io::stdout(), "the password of {} is no longer kept", plan.account.login)?;
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
