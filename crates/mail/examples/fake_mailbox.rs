//! A pretend mailbox on this machine, with a few conversations in it, for
//! looking at the window's mail (the checks, the site's pictures):
//!
//! ```text
//! cargo run -p unifyterm-mail --example fake_mailbox -- IMAP_PORT [LATER_SECONDS]
//! ```
//!
//! The account is `me@example.com`, password `s3cret`; the SMTP port is
//! printed. With `LATER_SECONDS`, one more mail comes that many seconds after
//! the start. It runs until stopped.

use std::fmt::Write as _;
use std::time::Duration;

use unifyterm_mail::fake::{Fake, Read};

/// One mail of the pretend mailbox.
struct Letter {
    folder: &'static str,
    id: &'static str,
    from: &'static str,
    to: &'static str,
    cc: &'static str,
    subject: &'static str,
    answers: &'static [&'static str],
    date: &'static str,
    text: &'static str,
    read: Read,
}

impl Letter {
    fn bytes(&self) -> Vec<u8> {
        let mut head = format!("Message-ID: <{}>\r\nFrom: {}\r\nTo: {}\r\n", self.id, self.from, self.to);
        if !self.cc.is_empty() {
            let _ = write!(head, "Cc: {}\r\n", self.cc);
        }
        if let Some(last) = self.answers.last() {
            let all: Vec<String> = self.answers.iter().map(|id| format!("<{id}>")).collect();
            let _ = write!(head, "In-Reply-To: <{last}>\r\nReferences: {}\r\n", all.join(" "));
        }
        format!(
            "{head}Subject: {}\r\nDate: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n",
            self.subject, self.date, self.text
        )
        .into_bytes()
    }
}

const ME: &str = "me@example.com";
const LIN: &str = "林晓 <lin@example.com>";
const ALEX: &str = "Alex Chen <alex@example.com>";
const WANG: &str = "王工 <wang@example.com>";
const LIN_SUBJECT: &str = "周五的发布，能不能提前到周四？";
const LIN_ANSWER: &str = "Re: 周五的发布，能不能提前到周四？";

const LETTERS: [Letter; 9] = [
    Letter {
        folder: "INBOX",
        id: "w1@example.com",
        from: WANG,
        to: ME,
        cc: "",
        subject: "Linux 上的 Vulkan 问题",
        answers: &[],
        date: "Tue, 24 Nov 2026 14:12:00 +0800",
        text: "在一台老笔记本上（Intel HD 4000）打不开窗口，日志说没有 Vulkan 设备。",
        read: Read::Seen,
    },
    Letter {
        folder: "Sent Messages",
        id: "w2@example.com",
        from: ME,
        to: WANG,
        cc: "",
        subject: "Re: Linux 上的 Vulkan 问题",
        answers: &["w1@example.com"],
        date: "Tue, 24 Nov 2026 15:40:00 +0800",
        text: "试试装最新的 mesa-vulkan-drivers；不行的话这台机器暂时用不了，我记进已知限制。",
        read: Read::Seen,
    },
    Letter {
        folder: "INBOX",
        id: "w3@example.com",
        from: WANG,
        to: ME,
        cc: "",
        subject: "Re: Linux 上的 Vulkan 问题",
        answers: &["w1@example.com", "w2@example.com"],
        date: "Tue, 24 Nov 2026 16:02:00 +0800",
        text: "装了新的驱动可以了，谢谢。",
        read: Read::Seen,
    },
    Letter {
        folder: "INBOX",
        id: "g1@github.example",
        from: "GitHub <notifications@github.example>",
        to: ME,
        cc: "",
        subject: "[newdee/unifyterm] CI passed: main",
        answers: &[],
        date: "Thu, 26 Nov 2026 21:05:00 +0800",
        text: "ci · 3 systems · 21 minutes. windows-latest, macos-latest and ubuntu-latest all passed.",
        read: Read::Seen,
    },
    Letter {
        folder: "INBOX",
        id: "a1@example.com",
        from: LIN,
        to: ME,
        cc: ALEX,
        subject: LIN_SUBJECT,
        answers: &[],
        date: "Thu, 26 Nov 2026 16:05:00 +0800",
        text: "0.8 的发布原来定在周五。市场那边希望周四上午就能发公告。\n\n构建和测试周三晚上能跑完吗？",
        read: Read::Seen,
    },
    Letter {
        folder: "Sent Messages",
        id: "b1@example.com",
        from: ME,
        to: LIN,
        cc: ALEX,
        subject: LIN_ANSWER,
        answers: &["a1@example.com"],
        date: "Thu, 26 Nov 2026 16:30:00 +0800",
        text: "周三晚上能跑完。签名证书还在等，周四发的话，Windows 版先不签名。\n\n在 2026年11月26日 16:05，林晓 写道：\n> 0.8 的发布原来定在周五。\n> 构建和测试周三晚上能跑完吗？",
        read: Read::Seen,
    },
    Letter {
        folder: "INBOX",
        id: "a2@example.com",
        from: LIN,
        to: ME,
        cc: ALEX,
        subject: LIN_ANSWER,
        answers: &["a1@example.com", "b1@example.com"],
        date: "Fri, 27 Nov 2026 10:42:00 +0800",
        text: "不签名可以接受，公告里写一句就行。\n\n那就定周四 10 点？你确认一下我就通知大家。",
        read: Read::Unseen,
    },
    Letter {
        folder: "Sent Messages",
        id: "c1@example.com",
        from: ME,
        to: ALEX,
        cc: "",
        subject: "主题图片的授权",
        answers: &[],
        date: "Fri, 27 Nov 2026 08:50:00 +0800",
        text: "主题里的八张角色图都是我们自己画的，可以放进安装包吗？",
        read: Read::Seen,
    },
    Letter {
        folder: "INBOX",
        id: "c2@example.com",
        from: ALEX,
        to: ME,
        cc: "",
        subject: "Re: 主题图片的授权",
        answers: &["c1@example.com"],
        date: "Fri, 27 Nov 2026 09:17:00 +0800",
        text: "可以。原图和生成记录在仓库外面留一份，以后有人问能拿出来。",
        read: Read::Unseen,
    },
];

/// The mail that comes later.
const LATER: Letter = Letter {
    folder: "INBOX",
    id: "d1@example.com",
    from: "运维值班 <ops@example.com>",
    to: ME,
    cc: "",
    subject: "下载页 502，持续 3 分钟",
    answers: &[],
    date: "Fri, 27 Nov 2026 11:03:00 +0800",
    text: "下载页 10:58 到 11:01 返回 502，现在已恢复。要不要在发布说明里提一句？",
    read: Read::Unseen,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let imap = args.first().and_then(|port| port.parse::<u16>().ok()).unwrap_or(0);
    let fake = Fake::start_at(ME, "s3cret", imap).expect("the pretend servers");
    for letter in &LETTERS {
        fake.deliver(letter.folder, &letter.bytes(), letter.read);
    }
    println!("imap 127.0.0.1:{} smtp 127.0.0.1:{} ({ME} / s3cret)", fake.imap_port, fake.smtp_port);
    if let Some(seconds) = args.get(1).and_then(|seconds| seconds.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_secs(seconds));
        fake.deliver(LATER.folder, &LATER.bytes(), LATER.read);
        println!("one more mail came");
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
