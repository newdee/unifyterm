//! An answer: to whom it goes, what it names, and sending it over SMTP.

use anyhow::{Context as _, Result, bail};
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MessageBuilder};
use std::io::{BufRead as _, BufReader, Write as _};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::account::{Account, Security};
use crate::message::{self, Address};
use crate::net::{self, Stream};
use crate::store::Thread;

/// The most message ids an answer names in `References`.
const MOST_REFERENCES: usize = 20;

/// An answer, ready to be written out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub text: String,
}

/// The answer to a conversation: to whoever wrote its newest mail from outside (or their
/// Reply-To), the others of that mail in Cc; "Re:" before the subject; naming that mail.
pub fn answer(thread: &Thread, me: &str, text: &str) -> Result<Answer> {
    let answered = thread
        .mails
        .iter()
        .rev()
        .find(|mail| !mail.outgoing)
        .or_else(|| thread.mails.last())
        .context("the conversation has no mail to answer")?;
    let not_me = |address: &&Address| address.addr != me && !address.addr.is_empty();
    let to: Vec<Address> = if answered.outgoing {
        answered.to.clone()
    } else {
        vec![answered.reply_to.clone().unwrap_or_else(|| answered.from.clone())]
    };
    let mut cc: Vec<Address> = Vec::new();
    for address in answered.to.iter().chain(&answered.cc).filter(not_me) {
        if !to.iter().chain(&cc).any(|other| other.addr == address.addr) {
            cc.push(address.clone());
        }
    }
    if to.is_empty() {
        bail!("the mail answered names no one to answer");
    }
    let subject = if message::is_answer(&answered.subject) {
        answered.subject.clone()
    } else {
        format!("Re: {}", answered.subject)
    };
    let mut references: Vec<String> = answered.refs.clone();
    references.extend(answered.message_id.clone());
    references.dedup();
    let skip = references.len().saturating_sub(MOST_REFERENCES);
    Ok(Answer {
        from: Address::new("", me),
        to,
        cc,
        subject,
        in_reply_to: answered.message_id.clone(),
        references: references.into_iter().skip(skip).collect(),
        text: text.trim_end().to_owned(),
    })
}

/// A message id for a mail written here: no machine's name in it, only the time and the address's domain.
pub fn new_message_id(me: &str, now_ms: i64) -> String {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let domain = me.rsplit_once('@').map_or("localhost", |(_, domain)| domain);
    format!("unifyterm.{now_ms}.{}.{count}@{domain}", std::process::id())
}

fn mailbox(address: &Address) -> Result<Mailbox> {
    let email = address.addr.parse().with_context(|| format!("\"{}\" is not a mail address", address.addr))?;
    Ok(Mailbox::new((!address.name.is_empty()).then(|| address.name.clone()), email))
}

/// The mail's bytes, as they are sent and kept.
pub fn write(answer: &Answer, message_id: &str) -> Result<lettre::Message> {
    let mut builder: MessageBuilder = lettre::Message::builder()
        .from(mailbox(&answer.from)?)
        .subject(answer.subject.clone())
        .message_id(Some(format!("<{message_id}>")))
        .header(ContentType::TEXT_PLAIN);
    for to in &answer.to {
        builder = builder.to(mailbox(to)?);
    }
    for cc in &answer.cc {
        builder = builder.cc(mailbox(cc)?);
    }
    if let Some(id) = &answer.in_reply_to {
        builder = builder.in_reply_to(format!("<{id}>"));
    }
    if !answer.references.is_empty() {
        builder =
            builder.references(answer.references.iter().map(|id| format!("<{id}>")).collect::<Vec<_>>().join(" "));
    }
    builder.body(answer.text.clone()).context("write the mail")
}

/// Sends a mail through the account's SMTP server, through its proxy when it has one. The
/// conversation is SMTP's own (RFC 5321): EHLO, STARTTLS on 587, AUTH PLAIN or LOGIN, MAIL,
/// RCPT for each one it goes to, DATA.
pub fn send(account: &Account, password: &str, mail: &lettre::Message) -> Result<()> {
    let envelope = mail.envelope();
    let from = envelope.from().map(ToString::to_string).unwrap_or_default();
    let to: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();
    let server = &account.smtp;
    let tcp = net::open(server, account.proxy.as_ref())?;
    let mut smtp = Smtp {
        stream: BufReader::new(match server.security {
            Security::Tls => net::secure(tcp, &server.host)?,
            Security::StartTls | Security::Plain => Stream::Plain(tcp),
        }),
    };
    smtp.expect(220)?;
    let mut offered = smtp.say(EHLO, 250)?;
    if server.security == Security::StartTls {
        smtp.say("STARTTLS", 220)?;
        let Stream::Plain(tcp) = smtp.stream.into_inner() else { bail!("STARTTLS over TLS") };
        smtp = Smtp { stream: BufReader::new(net::secure(tcp, &server.host)?) };
        offered = smtp.say(EHLO, 250)?;
    }
    let mechanisms =
        offered.iter().find_map(|line| line.strip_prefix("AUTH ")).unwrap_or_default().to_ascii_uppercase();
    if mechanisms.split_whitespace().any(|mechanism| mechanism == "PLAIN") {
        let token = BASE64.encode(format!("\u{0}{}\u{0}{password}", account.login));
        smtp.say(&format!("AUTH PLAIN {token}"), 235).map_err(|error| refused(&error))?;
    } else {
        smtp.say("AUTH LOGIN", 334)?;
        smtp.say(&BASE64.encode(&account.login), 334)?;
        smtp.say(&BASE64.encode(password), 235).map_err(|error| refused(&error))?;
    }
    smtp.say(&format!("MAIL FROM:<{from}>"), 250)?;
    for one in &to {
        smtp.say(&format!("RCPT TO:<{one}>"), 250)?;
    }
    smtp.say("DATA", 354)?;
    smtp.write(&stuffed(&mail.formatted()))?;
    smtp.expect(250)?;
    smtp.say("QUIT", 221).ok();
    Ok(())
}

/// What a client says it is: an address of this machine's own, the same for everyone, no machine's name.
const EHLO: &str = "EHLO [127.0.0.1]";

fn refused(error: &anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("the server refused the login: {error:#}")
}

/// A mail's bytes as DATA carries them: lines ended with CRLF, a line that begins with `.` given
/// one more, and the line with `.` alone that ends it.
fn stuffed(raw: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw).replace("\r\n", "\n");
    let mut out = Vec::with_capacity(raw.len() + 16);
    for line in text.split('\n') {
        if line.starts_with('.') {
            out.push(b'.');
        }
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// An SMTP conversation.
struct Smtp {
    stream: BufReader<Stream>,
}

impl Smtp {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let stream = self.stream.get_mut();
        stream.write_all(bytes)?;
        stream.flush()?;
        Ok(())
    }

    /// The server's answer: its lines without the code, when the code is `wanted`.
    fn expect(&mut self, wanted: u16) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            if self.stream.read_line(&mut line)? == 0 {
                bail!("the SMTP server closed the connection");
            }
            let line = line.trim_end();
            let code: u16 = line
                .get(..3)
                .and_then(|code| code.parse().ok())
                .with_context(|| format!("the SMTP server said \"{line}\""))?;
            let rest = line.get(4..).unwrap_or_default().to_owned();
            let last = line.as_bytes().get(3) != Some(&b'-');
            if code != wanted {
                bail!("the SMTP server said {code} {rest}");
            }
            lines.push(rest);
            if last {
                return Ok(lines);
            }
        }
    }

    fn say(&mut self, line: &str, wanted: u16) -> Result<Vec<String>> {
        self.write(format!("{line}\r\n").as_bytes())?;
        self.expect(wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Mail, Summary};

    #[derive(Clone, Copy, PartialEq)]
    enum Way {
        In,
        Out,
    }

    fn mail(from: &str, to: &[&str], cc: &[&str], way: Way, id: &str, refs: &[&str]) -> Mail {
        let outgoing = way == Way::Out;
        Mail {
            id: 1,
            message_id: Some(id.to_owned()),
            from: Address::new("", from),
            reply_to: None,
            to: to.iter().map(|addr| Address::new("", addr)).collect(),
            cc: cc.iter().map(|addr| Address::new("", addr)).collect(),
            subject: "周五的发布".into(),
            at_ms: 0,
            text: String::new(),
            attachments: Vec::new(),
            refs: refs.iter().map(|id| (*id).to_owned()).collect(),
            outgoing,
            seen: true,
        }
    }

    fn thread(mails: Vec<Mail>) -> Thread {
        let summary = Summary {
            id: 1,
            subject: String::new(),
            with: String::new(),
            last_at_ms: 0,
            messages: mails.len(),
            unread: 0,
            waiting: false,
            draft_by: None,
            snippet: String::new(),
            archived: false,
        };
        Thread { summary, mails, draft: None }
    }

    #[test]
    fn an_answer_goes_to_the_writer_with_the_others_in_cc_and_names_what_it_answers() {
        let thread = thread(vec![
            mail(
                "lin@example.com",
                &["me@example.com", "alex@example.com"],
                &["ops@example.com"],
                Way::In,
                "a1",
                &["a0"],
            ),
            mail("me@example.com", &["lin@example.com"], &[], Way::Out, "b1", &["a0", "a1"]),
        ]);
        let answer = answer(&thread, "me@example.com", "可以，周四。\n\n").expect("answer");
        assert_eq!(answer.to, [Address::new("", "lin@example.com")], "the newest mail from outside, not mine");
        assert_eq!(
            answer.cc.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(),
            ["alex@example.com", "ops@example.com"]
        );
        assert_eq!(answer.subject, "Re: 周五的发布");
        assert_eq!(
            (answer.in_reply_to.as_deref(), answer.references.as_slice()),
            (Some("a1"), &["a0".to_owned(), "a1".to_owned()][..])
        );
        assert_eq!(answer.text, "可以，周四。");
        let written = write(&answer, "unifyterm.1.2.3@example.com").expect("written");
        let parsed = message::parse(&written.formatted());
        assert_eq!(parsed.message_id.as_deref(), Some("unifyterm.1.2.3@example.com"));
        assert_eq!(
            (parsed.in_reply_to.as_slice(), parsed.subject.as_str()),
            (&["a1".to_owned()][..], "Re: 周五的发布")
        );
        assert_eq!(parsed.text, "可以，周四。", "Chinese survives the encoding");
    }

    #[test]
    fn data_is_stuffed_and_ended() {
        assert_eq!(stuffed(b"a\n.b\r\n..c"), b"a\r\n..b\r\n...c\r\n.\r\n".to_vec());
    }

    #[test]
    fn reply_to_goes_first_and_a_conversation_of_mine_alone_goes_to_whom_i_wrote() {
        let mut theirs = mail("lin@example.com", &["me@example.com"], &[], Way::In, "a1", &[]);
        theirs.reply_to = Some(Address::new("Desk", "desk@example.com"));
        assert_eq!(
            answer(&thread(vec![theirs]), "me@example.com", "x").expect("answer").to[0].addr,
            "desk@example.com"
        );
        let mine = mail("me@example.com", &["lin@example.com"], &[], Way::Out, "b1", &[]);
        assert_eq!(answer(&thread(vec![mine]), "me@example.com", "x").expect("answer").to[0].addr, "lin@example.com");
        assert!(answer(&thread(Vec::new()), "me@example.com", "x").is_err());
        let id = new_message_id("me@example.com", 5);
        assert!(id.starts_with("unifyterm.5.") && id.ends_with("@example.com"), "the time and the domain alone: {id}");
    }
}
