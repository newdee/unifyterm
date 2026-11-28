//! A pretend mailbox on this machine, with a few conversations in it, for
//! looking at the window's mail (the checks, the site's pictures):
//!
//! ```text
//! cargo run -p unifyterm-mail --example fake_mailbox -- IMAP_PORT [LATER_SECONDS]
//! ```
//!
//! The account is `me@example.com`, password `s3cret`; the SMTP port is
//! printed. With `LATER_SECONDS`, one more mail comes that many seconds after
//! the start. `FAKE_MAILBOX_LOGIN` lets another login in, for a second mailbox;
//! `FAKE_MAILBOX_IDS` (first letters of the mails' ids: `abg`) puts only those mails in. It runs until stopped.

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

const LETTERS: [Letter; 11] = [
    // A long one, newest: how bubbles wrap at each width.
    Letter {
        folder: "INBOX",
        id: "n1@example.com",
        from: ALEX,
        to: ME,
        cc: "",
        subject: "0.9 的发布说明，帮我看一下",
        answers: &[],
        date: "Fri, 27 Nov 2026 12:30:00 +0800",
        text: "发布说明我先写了一版：多邮箱、在邮箱界面里添加、Outlook 用浏览器登录，再加上四个筛选。有两处我拿不准：一是 Outlook 的那段要不要写明需要管理员同意，二是 mail.db 升到第 2 版那段，要不要提醒大家先备份。你看完直接在这里回我就行，不急，今天下班前给我就好。\r\nThe English version follows the same order; the line about the database moving to version 2 is the one I am least sure of, so please read that one twice before we ship it.",
        read: Read::Unseen,
    },
    // Sent, not answered yet: the newest mail of its conversation is mine.
    Letter {
        folder: "Sent Messages",
        id: "e1@example.com",
        from: ME,
        to: "印刷厂 <print@example.com>",
        cc: "",
        subject: "贴纸的报价请再确认一下",
        answers: &[],
        date: "Thu, 26 Nov 2026 18:20:00 +0800",
        text: "500 张圆形贴纸，直径 5 厘米，含运费一共多少？下周三之前能到吗？",
        read: Read::Seen,
    },
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

/// A mail with two files: the icon and a log. Its id begins with p.
const WITH_FILES: &str = concat!(
    "Message-ID: <p1@example.com>\r\nFrom: =?utf-8?B?6L+Q57u05YC854+t?= <ops@example.com>\r\nTo: me@example.com\r\n",
    "Subject: =?utf-8?B?5LiL6L296aG16K+K5pat?=\r\nDate: Fri, 27 Nov 2026 11:20:00 +0800\r\nMIME-Version: 1.0\r\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
    "日志和当时的截图在附件里。\r\n",
    "--b\r\nContent-Type: text/plain; name=\"nginx.log\"\r\nContent-Disposition: attachment; filename=\"nginx.log\"\r\n\r\n",
    "10:58:02 502 GET /download\r\n10:58:07 502 GET /download\r\n11:01:13 200 GET /download\r\n",
    "--b\r\nContent-Type: image/png; name=\"icon.png\"\r\nContent-Disposition: attachment; filename=\"icon.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
    "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\r\nBklEQVQDAAZRu4gaJQxUAAAAAElFTkSuQmCC\r\n",
    "--b--\r\n",
);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let imap = args.first().and_then(|port| port.parse::<u16>().ok()).unwrap_or(0);
    let login = std::env::var("FAKE_MAILBOX_LOGIN").unwrap_or_else(|_| ME.to_owned());
    let fake = Fake::start_at(&login, "s3cret", imap).expect("the pretend servers");
    let only = std::env::var("FAKE_MAILBOX_IDS").ok();
    for letter in &LETTERS {
        if only.as_ref().is_none_or(|only| letter.id.chars().next().is_some_and(|first| only.contains(first))) {
            fake.deliver(letter.folder, &letter.bytes(), letter.read);
        }
    }
    if only.as_ref().is_none_or(|only| only.contains('p')) {
        fake.deliver("INBOX", WITH_FILES.as_bytes(), Read::Unseen);
    }
    println!("imap 127.0.0.1:{} smtp 127.0.0.1:{} ({login} / s3cret)", fake.imap_port, fake.smtp_port);
    if let Some(seconds) = args.get(1).and_then(|seconds| seconds.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_secs(seconds));
        fake.deliver(LATER.folder, &LATER.bytes(), LATER.read);
        println!("one more mail came");
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
