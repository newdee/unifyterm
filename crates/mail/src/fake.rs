//! A pretend IMAP and SMTP server on this machine (127.0.0.1, no TLS), for
//! the tests and for looking at the window with mail in it. It speaks the
//! part of each protocol UnifyTerm uses: LOGIN, LIST, SELECT, UID SEARCH,
//! UID FETCH, UID STORE, APPEND, IDLE; EHLO, AUTH PLAIN, MAIL, RCPT, DATA.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// One mail in a folder.
#[derive(Debug, Clone)]
struct Stored {
    uid: u32,
    seen: bool,
    raw: Vec<u8>,
}

#[derive(Debug, Default)]
struct Folder {
    validity: u32,
    next: u32,
    mails: Vec<Stored>,
    /// Marked `\Sent` in LIST.
    sent: bool,
}

#[derive(Debug, Default)]
struct State {
    folders: BTreeMap<String, Folder>,
    /// What was sent through SMTP: (from, to, bytes).
    sent: Vec<(String, Vec<String>, Vec<u8>)>,
    /// The IMAP server closes each connection at once, as one that is down.
    down: bool,
}

/// Whether the IMAP server answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Down {
    Down,
    Up,
}

/// Whether a mail put into a folder is read already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    Seen,
    Unseen,
}

/// The two servers, and what they hold.
#[derive(Debug, Clone)]
pub struct Fake {
    state: Arc<(Mutex<State>, Condvar)>,
    pub imap_port: u16,
    pub smtp_port: u16,
    password: Arc<str>,
}

impl Fake {
    /// Both servers, listening; `login`/`password` is the only account they let in.
    pub fn start(login: &str, password: &str) -> std::io::Result<Self> {
        Self::start_at(login, password, 0)
    }

    /// The same, with IMAP on `imap_port` (0: any free one).
    pub fn start_at(login: &str, password: &str, imap_port: u16) -> std::io::Result<Self> {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        {
            let mut held = state.0.lock().unwrap_or_else(PoisonError::into_inner);
            held.folders.insert("INBOX".into(), Folder { validity: 7, next: 1, ..Folder::default() });
            held.folders
                .insert("Sent Messages".into(), Folder { validity: 9, next: 1, sent: true, ..Folder::default() });
        }
        let imap = TcpListener::bind(("127.0.0.1", imap_port))?;
        let smtp = TcpListener::bind("127.0.0.1:0")?;
        let fake = Self {
            state,
            imap_port: imap.local_addr()?.port(),
            smtp_port: smtp.local_addr()?.port(),
            password: Arc::from(format!("{login}\u{0}{password}")),
        };
        let served = fake.clone();
        std::thread::spawn(move || {
            for stream in imap.incoming().flatten() {
                let fake = served.clone();
                std::thread::spawn(move || fake.imap(stream));
            }
        });
        let served = fake.clone();
        std::thread::spawn(move || {
            for stream in smtp.incoming().flatten() {
                let fake = served.clone();
                std::thread::spawn(move || fake.smtp(stream));
            }
        });
        Ok(fake)
    }

    fn held(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a mail into a folder, as if it had come; its uid.
    pub fn deliver(&self, folder: &str, raw: &[u8], read: Read) -> u32 {
        let seen = read == Read::Seen;
        let mut held = self.held();
        let target = held.folders.entry(folder.to_owned()).or_insert_with(|| Folder {
            validity: 11,
            next: 1,
            ..Folder::default()
        });
        let uid = target.next;
        target.next += 1;
        target.mails.push(Stored { uid, seen, raw: raw.to_vec() });
        self.state.1.notify_all();
        uid
    }

    /// Whether the mail `uid` of `folder` is marked read.
    pub fn is_seen(&self, folder: &str, uid: u32) -> bool {
        self.held().folders.get(folder).and_then(|f| f.mails.iter().find(|m| m.uid == uid)).is_some_and(|m| m.seen)
    }

    /// How many mails a folder has.
    pub fn count(&self, folder: &str) -> usize {
        self.held().folders.get(folder).map_or(0, |f| f.mails.len())
    }

    /// What was sent through SMTP: (from, recipients, bytes).
    pub fn sent(&self) -> Vec<(String, Vec<String>, Vec<u8>)> {
        self.held().sent.clone()
    }

    /// The IMAP server goes down (each connection closed at once), or comes up again.
    pub fn set_down(&self, down: Down) {
        self.held().down = down == Down::Down;
    }

    /// The folder numbered anew: what a client read of it is no longer good.
    pub fn renumber(&self, folder: &str) {
        if let Some(target) = self.held().folders.get_mut(folder) {
            target.validity += 1;
        }
    }

    fn imap(&self, stream: TcpStream) {
        let _ = self.serve_imap(stream);
    }

    fn serve_imap(&self, stream: TcpStream) -> std::io::Result<()> {
        if self.held().down {
            return stream.shutdown(std::net::Shutdown::Both);
        }
        let mut out = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        let mut selected: Option<String> = None;
        // How many mails the selected folder had when it was selected: IDLE tells of more.
        let mut known = 0usize;
        out.write_all(b"* OK fake IMAP ready\r\n")?;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let line = line.trim_end().to_owned();
            let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
            let (command, arguments) = rest.split_once(' ').unwrap_or((rest, ""));
            let reply: String = match command.to_ascii_uppercase().as_str() {
                "CAPABILITY" => format!("* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done\r\n"),
                "LOGIN" => {
                    let words: Vec<String> =
                        arguments.split(' ').map(|word| word.trim_matches('"').replace("\\\"", "\"")).collect();
                    let given = format!(
                        "{}\u{0}{}",
                        words.first().map_or("", String::as_str),
                        words.get(1).map_or("", String::as_str)
                    );
                    if *given == *self.password {
                        format!("{tag} OK logged in\r\n")
                    } else {
                        format!("{tag} NO [AUTHENTICATIONFAILED] wrong password\r\n")
                    }
                }
                "LIST" => {
                    let mut text = String::new();
                    for (name, folder) in &self.held().folders {
                        let flags = if folder.sent { "\\HasNoChildren \\Sent" } else { "\\HasNoChildren" };
                        let _ = write!(text, "* LIST ({flags}) \"/\" \"{name}\"\r\n");
                    }
                    format!("{text}{tag} OK listed\r\n")
                }
                "SELECT" | "EXAMINE" => {
                    let name = arguments.trim_matches('"').to_owned();
                    let held = self.held();
                    match held.folders.get(&name) {
                        Some(folder) => {
                            selected = Some(name);
                            known = folder.mails.len();
                            format!(
                                "* {} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY {}] ok\r\n* OK [UIDNEXT {}] ok\r\n* FLAGS (\\Seen)\r\n{tag} OK [READ-WRITE] selected\r\n",
                                folder.mails.len(),
                                folder.validity,
                                folder.next
                            )
                        }
                        None => format!("{tag} NO no such folder\r\n"),
                    }
                }
                "UID" => {
                    out.write_all(&self.uid_command(tag, arguments, selected.as_deref()))?;
                    continue;
                }
                "APPEND" => {
                    let name = arguments.split('"').nth(1).unwrap_or_default().to_owned();
                    let size: usize =
                        arguments.rsplit('{').next().and_then(|s| s.trim_end_matches('}').parse().ok()).unwrap_or(0);
                    out.write_all(b"+ go on\r\n")?;
                    let mut raw = vec![0; size];
                    reader.read_exact(&mut raw)?;
                    let mut end = String::new();
                    reader.read_line(&mut end)?;
                    let read = if arguments.contains("\\Seen") { Read::Seen } else { Read::Unseen };
                    let uid = self.deliver(&name, &raw, read);
                    format!("{tag} OK [APPENDUID 9 {uid}] appended\r\n")
                }
                "IDLE" => {
                    out.write_all(b"+ idling\r\n")?;
                    known = self.idle(&mut reader, &mut out, selected.as_deref(), known)?;
                    format!("{tag} OK idle done\r\n")
                }
                "NOOP" => format!("{tag} OK\r\n"),
                "LOGOUT" => {
                    out.write_all(format!("* BYE\r\n{tag} OK bye\r\n").as_bytes())?;
                    return Ok(());
                }
                _ => format!("{tag} BAD unknown command\r\n"),
            };
            out.write_all(reply.as_bytes())?;
        }
    }

    /// IDLE: more mails than `known` are said as EXISTS; DONE ends it. The count told is returned.
    fn idle(
        &self,
        reader: &mut BufReader<TcpStream>,
        out: &mut TcpStream,
        folder: Option<&str>,
        known: usize,
    ) -> std::io::Result<usize> {
        reader.get_ref().set_read_timeout(Some(Duration::from_millis(50)))?;
        let mut told = known;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return Ok(told),
                Ok(_) if line.trim().eq_ignore_ascii_case("DONE") => break,
                Ok(_) => {}
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                }
                Err(error) => return Err(error),
            }
            let exists = folder.and_then(|name| self.held().folders.get(name).map(|f| f.mails.len())).unwrap_or(0);
            if exists > told {
                out.write_all(format!("* {exists} EXISTS\r\n").as_bytes())?;
                told = exists;
            }
        }
        reader.get_ref().set_read_timeout(None)?;
        Ok(told)
    }

    fn uid_command(&self, tag: &str, arguments: &str, selected: Option<&str>) -> Vec<u8> {
        self.uid_reply(tag, arguments, selected).unwrap_or_else(String::into_bytes)
    }

    /// The reply's bytes; `Err` holds a reply that is text alone.
    fn uid_reply(&self, tag: &str, arguments: &str, selected: Option<&str>) -> Result<Vec<u8>, String> {
        let Some(name) = selected else { return Err(format!("{tag} BAD no folder selected\r\n")) };
        let (verb, rest) = arguments.split_once(' ').unwrap_or((arguments, ""));
        let mut held = self.held();
        let Some(folder) = held.folders.get_mut(name) else { return Err(format!("{tag} NO gone\r\n")) };
        let last = folder.mails.last().map_or(0, |mail| mail.uid);
        match verb.to_ascii_uppercase().as_str() {
            "SEARCH" => {
                let found: Vec<String> = if let Some(set) = rest.strip_prefix("UID ") {
                    // As servers do: `n:*` gives the last mail even when its uid is below n.
                    let mut uids: Vec<u32> =
                        folder.mails.iter().map(|m| m.uid).filter(|uid| in_set(set, *uid, last)).collect();
                    if uids.is_empty() && set.ends_with(":*") {
                        uids.extend(folder.mails.last().map(|m| m.uid));
                    }
                    uids.iter().map(u32::to_string).collect()
                } else if let Some(id) = rest.strip_prefix("HEADER Message-ID ") {
                    let id = id.trim_matches('"');
                    folder
                        .mails
                        .iter()
                        .filter(|m| String::from_utf8_lossy(&m.raw).contains(id))
                        .map(|m| m.uid.to_string())
                        .collect()
                } else {
                    folder.mails.iter().map(|m| m.uid.to_string()).collect()
                };
                Err(format!("* SEARCH {}\r\n{tag} OK searched\r\n", found.join(" ")))
            }
            "FETCH" => {
                let (set, items) = rest.split_once(' ').unwrap_or((rest, ""));
                let mut text = String::new();
                let mut bytes = Vec::new();
                for (index, mail) in folder.mails.iter().enumerate() {
                    if !in_set(set, mail.uid, last) {
                        continue;
                    }
                    let flags = if mail.seen { "\\Seen" } else { "" };
                    let mut parts = format!("UID {} FLAGS ({flags})", mail.uid);
                    if items.contains("RFC822.SIZE") {
                        let _ = write!(parts, " RFC822.SIZE {}", mail.raw.len());
                    }
                    if items.contains("INTERNALDATE") {
                        parts.push_str(" INTERNALDATE \"26-Nov-2026 10:42:00 +0800\"");
                    }
                    let _ = write!(text, "* {} FETCH ({parts}", index + 1);
                    let body: Option<(&str, &[u8])> = if items.contains("BODY.PEEK[HEADER]") {
                        let end =
                            mail.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(mail.raw.len(), |at| at + 4);
                        Some(("BODY[HEADER]", &mail.raw[..end]))
                    } else if items.contains("BODY.PEEK[]") {
                        Some(("BODY[]", &mail.raw))
                    } else {
                        None
                    };
                    bytes.extend_from_slice(text.as_bytes());
                    text.clear();
                    if let Some((label, content)) = body {
                        bytes.extend_from_slice(format!(" {label} {{{}}}\r\n", content.len()).as_bytes());
                        bytes.extend_from_slice(content);
                    }
                    bytes.extend_from_slice(b")\r\n");
                }
                bytes.extend_from_slice(format!("{tag} OK fetched\r\n").as_bytes());
                Ok(bytes)
            }
            "STORE" => {
                let (set, _) = rest.split_once(' ').unwrap_or((rest, ""));
                for mail in &mut folder.mails {
                    if in_set(set, mail.uid, last) {
                        mail.seen = true;
                    }
                }
                Err(format!("{tag} OK stored\r\n"))
            }
            _ => Err(format!("{tag} BAD unknown UID command\r\n")),
        }
    }

    fn smtp(&self, stream: TcpStream) {
        let _ = self.serve_smtp(stream);
    }

    fn serve_smtp(&self, stream: TcpStream) -> std::io::Result<()> {
        let mut out = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        let (mut from, mut to) = (String::new(), Vec::new());
        out.write_all(b"220 fake SMTP ready\r\n")?;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let upper = line.trim_end().to_ascii_uppercase();
            let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n"
            } else if let Some(given) = line.trim_end().strip_prefix("AUTH PLAIN ") {
                let decoded = decode_plain(given);
                if *decoded == *self.password { b"235 ok\r\n" } else { b"535 no\r\n" }
            } else if upper.starts_with("MAIL FROM:") {
                line.trim_end()[10..].trim().trim_matches(['<', '>']).clone_into(&mut from);
                b"250 ok\r\n"
            } else if upper.starts_with("RCPT TO:") {
                to.push(line.trim_end()[8..].trim().trim_matches(['<', '>']).to_owned());
                b"250 ok\r\n"
            } else if upper == "DATA" {
                out.write_all(b"354 go on\r\n")?;
                let mut data = Vec::new();
                loop {
                    let mut part = String::new();
                    if reader.read_line(&mut part)? == 0 || part == ".\r\n" {
                        break;
                    }
                    data.extend_from_slice(part.strip_prefix('.').unwrap_or(&part).as_bytes());
                }
                self.held().sent.push((std::mem::take(&mut from), std::mem::take(&mut to), data));
                b"250 queued\r\n"
            } else if upper == "QUIT" {
                out.write_all(b"221 bye\r\n")?;
                return Ok(());
            } else {
                b"250 ok\r\n"
            };
            out.write_all(reply)?;
        }
    }
}

/// A proxy on this machine that passes connections on, and counts them: SOCKS5 or HTTP CONNECT.
#[derive(Debug, Clone)]
pub struct FakeProxy {
    pub port: u16,
    passed: Arc<Mutex<Vec<String>>>,
}

impl FakeProxy {
    /// A SOCKS5 proxy (no login) when `socks`, else an HTTP one.
    pub fn start(socks: Socks) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let proxy = Self { port: listener.local_addr()?.port(), passed: Arc::new(Mutex::new(Vec::new())) };
        let passed = Arc::clone(&proxy.passed);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let passed = Arc::clone(&passed);
                std::thread::spawn(move || {
                    let _ = pass_on(stream, socks, &passed);
                });
            }
        });
        Ok(proxy)
    }

    /// The places it passed connections to: `host:port`.
    pub fn passed(&self) -> Vec<String> {
        self.passed.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Which proxy [`FakeProxy`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Socks {
    Five,
    HttpConnect,
}

fn pass_on(mut client: TcpStream, kind: Socks, passed: &Mutex<Vec<String>>) -> std::io::Result<()> {
    let place = if kind == Socks::Five {
        let mut hello = [0u8; 2];
        client.read_exact(&mut hello)?;
        let mut methods = vec![0u8; usize::from(hello[1])];
        client.read_exact(&mut methods)?;
        client.write_all(&[5, 0])?;
        let mut head = [0u8; 5];
        client.read_exact(&mut head)?;
        let mut host = vec![0u8; usize::from(head[4])];
        client.read_exact(&mut host)?;
        let mut port = [0u8; 2];
        client.read_exact(&mut port)?;
        client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
        format!("{}:{}", String::from_utf8_lossy(&host), u16::from_be_bytes(port))
    } else {
        let mut reader = BufReader::new(client.try_clone()?);
        let mut first = String::new();
        reader.read_line(&mut first)?;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                break;
            }
        }
        client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;
        first.split_whitespace().nth(1).unwrap_or_default().to_owned()
    };
    passed.lock().unwrap_or_else(PoisonError::into_inner).push(place.clone());
    let server = TcpStream::connect(&place)?;
    let (mut from_client, mut to_server) = (client.try_clone()?, server.try_clone()?);
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut from_client, &mut to_server);
    });
    let (mut from_server, mut to_client) = (server, client);
    let _ = std::io::copy(&mut from_server, &mut to_client);
    Ok(())
}

/// Whether `uid` is in an IMAP set (`1:3,5,7:*`): in a range `*` has no end; alone it is `last`.
fn in_set(set: &str, uid: u32, last: u32) -> bool {
    let value = |text: &str, star: u32| if text == "*" { star } else { text.parse().unwrap_or(0) };
    set.split(',').any(|part| match part.split_once(':') {
        Some((low, high)) => {
            let (low, high) = (value(low, u32::MAX), value(high, u32::MAX));
            (low.min(high)..=low.max(high)).contains(&uid)
        }
        None => value(part, last) == uid,
    })
}

/// AUTH PLAIN's base64 as `login\0password` (the first part, who acts for whom, left out).
fn decode_plain(given: &str) -> String {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bytes, mut buffer, mut held) = (Vec::new(), 0u32, 0u32);
    for c in given.trim().bytes().filter(|c| *c != b'=') {
        let Some(at) = table.iter().position(|t| *t == c) else { continue };
        buffer = (buffer << 6) | u32::try_from(at).unwrap_or(0);
        held += 6;
        if held >= 8 {
            held -= 8;
            bytes.push(u8::try_from((buffer >> held) & 0xff).unwrap_or(0));
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    text.split_once('\u{0}').map_or(text.clone(), |(_, rest)| rest.to_owned())
}
