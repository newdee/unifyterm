//! `mail.on_new`: a program run for each new mail from someone else. Its
//! words may hold `{thread}`, `{from}`, `{subject}`, `{snippet}` and `{account}` (the
//! mailbox's name); the mail
//! is also written to its standard input as JSON. It is given its arguments
//! directly, with no shell between: what a mail says is never run as a
//! command. What the program does with it (hand it to an agent) is its own.

use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context as _, Result};

use crate::net::NewMail;

/// How long the program may run before it is stopped.
const MOST: Duration = Duration::from_secs(60);

/// The program's words with the placeholders filled in.
pub fn words(command: &[String], mail: &NewMail) -> Vec<String> {
    command
        .iter()
        .map(|word| {
            word.replace("{thread}", &mail.thread.to_string())
                .replace("{from}", &mail.from.full())
                .replace("{subject}", &mail.subject)
                .replace("{snippet}", &mail.snippet)
                .replace("{account}", &mail.mailbox)
        })
        .collect()
}

/// Starts the program for `mail` and returns; a thread waits for it, and stops it after a minute.
pub fn run(command: &[String], mail: &NewMail) -> Result<()> {
    let words = words(command, mail);
    let Some((program, arguments)) = words.split_first() else { return Ok(()) };
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("mail.on_new: run {program}"))?;
    let json = serde_json::to_vec(mail)?;
    if let Some(mut input) = child.stdin.take() {
        // A program that reads nothing closes its input: that is not a failure.
        let _ = input.write_all(&json);
    }
    let name = program.clone();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    log::warn!("mail.on_new: {name} ended with {status}");
                    return;
                }
                Ok(Some(_)) => return,
                Ok(None) if start.elapsed() > MOST => {
                    log::warn!("mail.on_new: {name} ran for a minute: stopped");
                    let _ = child.kill();
                    return;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(error) => {
                    log::warn!("mail.on_new: {name}: {error}");
                    return;
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Address;

    #[test]
    fn placeholders_are_filled_and_each_word_stays_one_argument() {
        let mail = NewMail {
            account: "me@qq.com".into(),
            mailbox: "qq".into(),
            thread: 42,
            from: Address::new("林晓", "lin@example.com"),
            subject: "周五; rm -rf /".into(),
            snippet: "能提前吗".into(),
            at_ms: 0,
        };
        let command: Vec<String> =
            ["keepane", "send-message", "--to", "%3", "{account} 新邮件 {thread}，来自 {from}：{subject}"]
                .map(String::from)
                .to_vec();
        let filled = words(&command, &mail);
        assert_eq!(filled.len(), 5, "a subject with ; is still one argument: no shell reads it");
        assert_eq!(filled[4], "qq 新邮件 42，来自 林晓 <lin@example.com>：周五; rm -rf /");
        assert!(run(&[], &mail).is_ok(), "no program: nothing to run");
        assert!(run(&["no-such-program-anywhere".into()], &mail).is_err(), "said, not hidden");
    }
}
