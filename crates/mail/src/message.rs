//! One mail as UnifyTerm keeps it: who wrote it to whom, what it answers,
//! and its words as text. HTML is turned into text, and nothing it loads
//! from elsewhere (pictures, styles) is looked at.

use mail_parser::MessageParser;
use serde::{Deserialize, Serialize};

/// Someone a mail is from or to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// The name, as the mail gives it; empty when it gives none.
    pub name: String,
    /// `someone@example.com`, in small letters.
    pub addr: String,
}

impl Address {
    pub fn new(name: &str, addr: &str) -> Self {
        Self { name: name.trim().trim_matches('"').trim().to_owned(), addr: addr.trim().to_lowercase() }
    }

    /// The name when there is one, else the address.
    pub fn shown(&self) -> &str {
        if self.name.is_empty() { &self.addr } else { &self.name }
    }

    /// `Name <addr>`, or the address alone.
    pub fn full(&self) -> String {
        if self.name.is_empty() { self.addr.clone() } else { format!("{} <{}>", self.name, self.addr) }
    }
}

/// A file that came with a mail: only its name and size are kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub bytes: usize,
}

/// What is read from a mail's bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    /// `<id@host>` without the brackets.
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub from: Option<Address>,
    pub reply_to: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    /// When it was written, in milliseconds since 1970.
    pub date_ms: Option<i64>,
    /// Its words: the text part, or the HTML part as text.
    pub text: String,
    pub attachments: Vec<Attachment>,
}

/// The mail in `raw`. Bytes that are no mail at all give one with nothing in it.
pub fn parse(raw: &[u8]) -> Parsed {
    let Some(message) = MessageParser::new().parse(raw) else {
        return Parsed::default();
    };
    let first = |list: Option<mail_parser::AddressList<'_>>| {
        list.and_then(|list| list.first())
            .and_then(|mailbox| mailbox.address().map(|addr| Address::new(mailbox.name().unwrap_or_default(), addr)))
    };
    let all = |list: Option<mail_parser::AddressList<'_>>| -> Vec<Address> {
        list.map(|list| {
            list.mailboxes()
                .filter_map(|mailbox| {
                    mailbox.address().map(|addr| Address::new(mailbox.name().unwrap_or_default(), addr))
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let ids = |list: Option<mail_parser::TextList<'_>>| -> Vec<String> {
        list.map(|list| list.iter().map(bare_id).filter(|id| !id.is_empty()).collect()).unwrap_or_default()
    };
    let text = message.body_text(0).map(std::borrow::Cow::into_owned).unwrap_or_default();
    let attachments = message
        .attachments()
        .map(|part| Attachment {
            name: part.attachment_name().unwrap_or("attachment").to_owned(),
            bytes: part.decoded_len(),
        })
        .collect();
    Parsed {
        message_id: message.message_id().map(bare_id).filter(|id| !id.is_empty()),
        in_reply_to: ids(message.in_reply_to()),
        references: ids(message.references()),
        from: first(message.from()),
        reply_to: first(message.reply_to()),
        to: all(message.to()),
        cc: all(message.cc()),
        subject: message.subject().unwrap_or_default().trim().to_owned(),
        date_ms: message.date().map(|date| date.to_timestamp().saturating_mul(1000)),
        text: tidy(&text),
        attachments,
    }
}

/// A message id without its angle brackets and blanks.
fn bare_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_owned()
}

/// Lines ended with `\n` alone, no blanks at their ends, no more than one empty line in a row,
/// none at the start or the end.
fn tidy(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() && out.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|last| last.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// The words before which a subject says it answers or passes on another mail.
const SUBJECT_PREFIXES: [&str; 8] = ["re:", "fw:", "fwd:", "aw:", "回复:", "答复:", "转发:", "回覆:"];

/// A subject without `Re:`, `Fwd:`, `回复：` and the like before it, however many: what
/// two mails of one conversation share.
pub fn plain_subject(subject: &str) -> String {
    let mut rest = subject.trim().replace('：', ":");
    loop {
        let lower = rest.to_lowercase();
        let Some(prefix) = SUBJECT_PREFIXES.iter().find(|prefix| lower.starts_with(**prefix)) else { break };
        // The prefixes have the same length in small letters: ASCII, or Chinese with no case.
        let cut = rest.get(prefix.len()..).unwrap_or_default().trim_start().to_owned();
        rest = cut;
    }
    rest.trim().to_owned()
}

/// Whether the subject says it answers another mail.
pub fn is_answer(subject: &str) -> bool {
    plain_subject(subject) != subject.trim()
}

/// The lines that begin the quoted mail at the end of an answer.
fn begins_quote(line: &str) -> bool {
    let line = line.trim();
    let lower = line.to_lowercase();
    (lower.starts_with("on ") && lower.ends_with("wrote:"))
        || (line.starts_with('在') && (line.ends_with("写道：") || line.ends_with("写道:")))
        || lower.starts_with("-----original message-----")
        || line.starts_with("------------------ 原始邮件")
        || line.starts_with("-------- 原始邮件")
        || line.starts_with("------ Original")
        || (lower.starts_with("from:") && lower.contains('@'))
}

/// An answer's own words, and how many lines of the mail it answers follow them:
/// the quote is what follows "On ... wrote:", "在 ... 写道：", "-----Original Message-----",
/// or the run of `>` lines at the end.
pub fn fold_quote(text: &str) -> (String, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|line| begins_quote(line)).or_else(|| {
        // The `>` lines at the end, with empty lines among them.
        let mut start = lines.len();
        while start > 0 && (lines[start - 1].starts_with('>') || lines[start - 1].trim().is_empty()) {
            start -= 1;
        }
        (start < lines.len() && lines[start..].iter().any(|line| line.starts_with('>'))).then_some(start)
    });
    match at {
        // A mail that is all quote keeps it: there is nothing else to show.
        Some(0) | None => (text.to_owned(), 0),
        Some(at) => {
            let own = lines[..at].join("\n").trim_end().to_owned();
            let quoted = lines[at..].iter().filter(|line| !line.trim().is_empty()).count();
            (own, quoted)
        }
    }
}

/// The first line with words in an answer's own text, cut to `chars` characters.
pub fn snippet(text: &str, chars: usize) -> String {
    let (own, _) = fold_quote(text);
    let line = own.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    if line.chars().count() <= chars {
        return line.to_owned();
    }
    let cut: String = line.chars().take(chars.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "Message-ID: <b2@example.com>\r\n\
In-Reply-To: <a1@example.com>\r\n\
References: <a0@example.com> <a1@example.com>\r\n\
From: =?UTF-8?B?5p6X5pmT?= <Lin@Example.com>\r\n\
To: me@example.com, \"Alex Chen\" <alex@example.com>\r\n\
Cc: ops@example.com\r\n\
Subject: =?UTF-8?B?UmU6IOWRqOS6lOeahOWPkeW4gw==?=\r\n\
Date: Thu, 26 Nov 2026 10:42:00 +0800\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
OK, Thursday 10:00.\r\n\
\r\n\
On Wed, Nov 25, 2026 at 4:30 PM me <me@example.com> wrote:\r\n\
> Thursday is fine.\r\n\
> Windows unsigned.\r\n";

    #[test]
    fn a_mail_gives_who_what_and_what_it_answers() {
        let mail = parse(ANSWER.as_bytes());
        assert_eq!(mail.message_id.as_deref(), Some("b2@example.com"));
        assert_eq!(mail.in_reply_to, ["a1@example.com"]);
        assert_eq!(mail.references, ["a0@example.com", "a1@example.com"]);
        assert_eq!(
            mail.from,
            Some(Address::new("林晓", "lin@example.com")),
            "the name decoded, the address in small letters"
        );
        assert_eq!(
            mail.to.iter().map(|to| to.addr.as_str()).collect::<Vec<_>>(),
            ["me@example.com", "alex@example.com"]
        );
        assert_eq!(mail.cc, [Address::new("", "ops@example.com")]);
        assert_eq!(mail.subject, "Re: 周五的发布");
        assert_eq!(mail.date_ms, Some(1_795_660_920_000), "Thu, 26 Nov 2026 02:42:00 UTC");
        assert!(mail.text.starts_with("OK, Thursday 10:00.\n\nOn Wed"));
        assert_eq!(fold_quote(&mail.text), ("OK, Thursday 10:00.".to_owned(), 3), "the quote folded: its 3 lines");
        assert_eq!(snippet(&mail.text, 40), "OK, Thursday 10:00.");
    }

    #[test]
    fn html_only_is_text_and_nothing_is_no_mail() {
        let html = "Subject: x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <b>there</b></p><p>Second</p>";
        let mail = parse(html.as_bytes());
        assert!(
            mail.text.contains("Hello there") && mail.text.contains("Second") && !mail.text.contains('<'),
            "{:?}",
            mail.text
        );
        let gbk = b"Subject: =?GBK?B?xOO6ww==?=\r\nContent-Type: text/plain; charset=gbk\r\n\r\n\xc4\xe3\xba\xc3\r\n";
        let mail = parse(gbk);
        assert_eq!((mail.subject.as_str(), mail.text.as_str()), ("你好", "你好"), "GBK, as Chinese mail often is");
        assert_eq!(parse(b""), Parsed::default());
    }

    #[test]
    fn subjects_lose_their_answer_words() {
        assert_eq!(plain_subject("Re: RE: Fwd: 周五的发布"), "周五的发布");
        assert_eq!(plain_subject("回复：答复: 发布计划"), "发布计划");
        assert_eq!(plain_subject("Report"), "Report", "a word that begins with re is not Re:");
        assert!(is_answer("Re: x") && !is_answer("x"));
    }

    #[test]
    fn quotes_fold_in_the_shapes_mail_programs_write_them() {
        assert_eq!(fold_quote("好的\n\n在 2026年11月25日 写道：\n> 原文"), ("好的".to_owned(), 2));
        assert_eq!(fold_quote("Yes\n-----Original Message-----\nFrom: a@b.c\nhello"), ("Yes".to_owned(), 3));
        assert_eq!(fold_quote("Yes\n\n> a\n>\n> b\n"), ("Yes".to_owned(), 3), "the > lines at the end");
        assert_eq!(fold_quote("> only a quote"), ("> only a quote".to_owned(), 0), "all quote: kept");
        assert_eq!(fold_quote("plain\ntext"), ("plain\ntext".to_owned(), 0));
        assert_eq!(snippet("一二三四五六", 4), "一二三…");
    }
}
