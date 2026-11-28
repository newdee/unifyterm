//! IMAP: the connection, and the sync that brings the server's mail into
//! the store. The inbox and the sent folder are read; a first sync takes the
//! last `days` days, later ones what came after. Mails read here are marked
//! read on the server. Between syncs the connection waits (IDLE) for new mail.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;

use crate::account::{Account, Proxy, ProxyKind, Security, Server};
use crate::message::{self, Address};
use crate::store::{Added, Incoming, Store};

/// How long a connection may take to open, and a read to answer.
const CONNECT_WAIT: Duration = Duration::from_secs(20);
const READ_WAIT: Duration = Duration::from_secs(60);
/// The longest one IDLE lasts before the wait looks whether it should stop.
pub const IDLE_SLICE: Duration = Duration::from_secs(5);
/// The most mails asked for in one FETCH.
const BATCH: usize = 20;
/// A mail larger than this is kept by its header only: 8 MiB.
const WHOLE_LIMIT: u32 = 8 * 1024 * 1024;
/// The names a sent folder has where the server does not mark it.
const SENT_NAMES: [&str; 7] =
    ["Sent", "Sent Messages", "Sent Items", "Sent Mail", "已发送", "INBOX.Sent", "[Gmail]/Sent Mail"];

/// A connection to a server: TLS, or plain to this machine.
pub enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Plain(_) => "Stream::Plain",
            Self::Tls(_) => "Stream::Tls",
        })
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

impl imap::extensions::idle::SetReadTimeout for Stream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> imap::Result<()> {
        let tcp = match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => stream.get_mut(),
        };
        // The socket's own, not this trait's: no wait at all is never asked for.
        TcpStream::set_read_timeout(tcp, timeout.or(Some(READ_WAIT))).map_err(imap::Error::Io)
    }
}

/// TLS with Mozilla's roots, made once.
fn tls() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    Arc::clone(CONFIG.get_or_init(|| {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map(|builder| builder.with_root_certificates(roots).with_no_client_auth());
        Arc::new(config.expect("ring has rustls' default TLS versions"))
    }))
}

/// A TCP connection to `server`: directly, or through `proxy` (which is given the server's name).
pub fn open(server: &Server, proxy: Option<&Proxy>) -> Result<TcpStream> {
    let (host, port) = proxy.map_or((server.host.as_str(), server.port), |proxy| (proxy.host.as_str(), proxy.port));
    let address = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("find {host}"))?
        .next()
        .with_context(|| format!("{host} has no address"))?;
    let mut tcp = TcpStream::connect_timeout(&address, CONNECT_WAIT).with_context(|| {
        if proxy.is_some() {
            format!("connect to the proxy {host}:{port}")
        } else {
            format!("connect to {host}:{port}")
        }
    })?;
    tcp.set_read_timeout(Some(READ_WAIT))?;
    tcp.set_write_timeout(Some(READ_WAIT))?;
    match proxy {
        None => {}
        Some(proxy) if proxy.kind == ProxyKind::Socks5 => socks5(&mut tcp, proxy, server)?,
        Some(proxy) => http_connect(&mut tcp, proxy, server)?,
    }
    Ok(tcp)
}

/// SOCKS5 (RFC 1928), with a name and password (RFC 1929) when the proxy has them: CONNECT to the server by its name.
fn socks5(tcp: &mut TcpStream, proxy: &Proxy, server: &Server) -> Result<()> {
    let methods: &[u8] = if proxy.login.is_some() { &[0, 2] } else { &[0] };
    let mut hello = vec![5, u8::try_from(methods.len()).unwrap_or(1)];
    hello.extend_from_slice(methods);
    tcp.write_all(&hello)?;
    let mut chosen = [0u8; 2];
    tcp.read_exact(&mut chosen).context("the SOCKS5 proxy did not answer")?;
    match (chosen, &proxy.login) {
        ([5, 0], _) => {}
        ([5, 2], Some((name, password))) => {
            let mut login = vec![1, short(name)?];
            login.extend_from_slice(name.as_bytes());
            login.push(short(password)?);
            login.extend_from_slice(password.as_bytes());
            tcp.write_all(&login)?;
            let mut said = [0u8; 2];
            tcp.read_exact(&mut said)?;
            if said[1] != 0 {
                bail!("the SOCKS5 proxy refused the name and password");
            }
        }
        _ => bail!("the SOCKS5 proxy wants a way of logging in UnifyTerm does not have"),
    }
    let mut ask = vec![5, 1, 0, 3, short(&server.host)?];
    ask.extend_from_slice(server.host.as_bytes());
    ask.extend_from_slice(&server.port.to_be_bytes());
    tcp.write_all(&ask)?;
    let mut head = [0u8; 4];
    tcp.read_exact(&mut head).context("the SOCKS5 proxy did not answer the CONNECT")?;
    if head[1] != 0 {
        bail!("the SOCKS5 proxy could not reach {}:{} (code {})", server.host, server.port, head[1]);
    }
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut length = [0u8; 1];
            tcp.read_exact(&mut length)?;
            usize::from(length[0]) + 2
        }
        other => bail!("the SOCKS5 proxy answered an address of kind {other}"),
    };
    let mut bound = vec![0u8; rest];
    tcp.read_exact(&mut bound)?;
    Ok(())
}

/// A name SOCKS5 can carry: at most 255 bytes.
fn short(text: &str) -> Result<u8> {
    u8::try_from(text.len()).with_context(|| format!("\"{text}\" is too long for SOCKS5"))
}

/// An HTTP proxy asked to CONNECT to the server.
fn http_connect(tcp: &mut TcpStream, proxy: &Proxy, server: &Server) -> Result<()> {
    use std::fmt::Write as _;

    use base64::Engine as _;
    let place = format!("{}:{}", server.host, server.port);
    let mut ask = format!("CONNECT {place} HTTP/1.1\r\nHost: {place}\r\n");
    if let Some((name, password)) = &proxy.login {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{name}:{password}"));
        let _ = write!(ask, "Proxy-Authorization: Basic {token}\r\n");
    }
    ask.push_str("\r\n");
    tcp.write_all(ask.as_bytes())?;
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        if tcp.read(&mut byte)? == 0 || answer.len() > 16 * 1024 {
            bail!("the HTTP proxy closed the connection");
        }
        answer.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&answer).lines().next().unwrap_or_default().to_owned();
    if status.split_whitespace().nth(1) != Some("200") {
        bail!("the HTTP proxy said \"{status}\" to CONNECT {place}");
    }
    Ok(())
}

/// TLS over a connection already open to `host`.
pub fn secure(tcp: TcpStream, host: &str) -> Result<Stream> {
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .with_context(|| format!("{host} is not a host name"))?;
    let connection = rustls::ClientConnection::new(tls(), name)?;
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp))))
}

/// A connection to the IMAP server, private as its port says.
pub fn connect(server: &Server, proxy: Option<&Proxy>) -> Result<Stream> {
    let tcp = open(server, proxy)?;
    match server.security {
        Security::Plain => Ok(Stream::Plain(tcp)),
        Security::Tls => secure(tcp, &server.host),
        Security::StartTls => bail!("{}:{}: IMAP is read over TLS, on port 993", server.host, server.port),
    }
}

pub type Session = imap::Session<Stream>;

/// Why a sync could not be done.
#[derive(Debug)]
pub enum Failure {
    /// The server said no to the name and password, or there is none to give: trying again will
    /// not help until they change. What is said says which.
    Refused(String),
    /// Anything else: the network, the server. Worth trying again later.
    Other(anyhow::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => f.write_str(why),
            Self::Other(error) => write!(f, "{error:#}"),
        }
    }
}

/// What a login proves itself with.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// A password, an app password or an authorization code.
    Password(String),
    /// An `OAuth2` access token (`XOAUTH2`).
    Token(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never written out, also not in a log.
        f.write_str(match self {
            Self::Password(_) => "Password(..)",
            Self::Token(_) => "Token(..)",
        })
    }
}

/// `XOAUTH2`'s one message: `user=…^Aauth=Bearer …^A^A`. Base64 is the caller's.
pub fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

/// IMAP's AUTHENTICATE `XOAUTH2`: the message at first; after a refusal (the server sends what went
/// wrong as a challenge) nothing, so that the server ends with NO.
struct XOauth2 {
    message: String,
}

impl imap::Authenticator for XOauth2 {
    type Response = String;

    fn process(&self, challenge: &[u8]) -> String {
        if challenge.is_empty() { self.message.clone() } else { String::new() }
    }
}

/// Logged in to the account's IMAP server.
pub fn login(account: &Account, credential: &Credential) -> std::result::Result<Session, Failure> {
    login_holding(account, credential, |_| {})
}

/// Logged in, with `hold` given a handle on the connection's socket first: shutting it down
/// from another thread ends a wait for new mail at once.
pub fn login_holding(
    account: &Account,
    credential: &Credential,
    hold: impl FnOnce(TcpStream),
) -> std::result::Result<Session, Failure> {
    let client = greeted(account, hold).map_err(Failure::Other)?;
    log_in(client, account, credential)
}

/// A connection to the IMAP server that has greeted, not logged in; `hold` is given its socket.
pub fn greeted(account: &Account, hold: impl FnOnce(TcpStream)) -> Result<imap::Client<Stream>> {
    let stream = connect(&account.imap, account.proxy.as_ref())?;
    let socket = match &stream {
        Stream::Plain(tcp) => tcp.try_clone(),
        Stream::Tls(tls) => tls.get_ref().try_clone(),
    };
    hold(socket?);
    let mut client = imap::Client::new(stream);
    client.read_greeting().context("the IMAP server's greeting")?;
    Ok(client)
}

/// Logs in on a connection that has greeted.
pub fn log_in(
    client: imap::Client<Stream>,
    account: &Account,
    credential: &Credential,
) -> std::result::Result<Session, Failure> {
    let refused = |(error, _)| match error {
        imap::Error::No(no) => Failure::Refused(format!("the server refused the login: {}", no.information)),
        imap::Error::Bad(bad) => Failure::Refused(format!("the server refused the login: {}", bad.information)),
        other => Failure::Other(other.into()),
    };
    match credential {
        Credential::Password(password) => client.login(&account.login, password).map_err(refused),
        Credential::Token(token) => {
            client.authenticate("XOAUTH2", &XOauth2 { message: xoauth2(&account.login, token) }).map_err(refused)
        }
    }
}

/// The server's sent folder: the one it marks `\Sent`, else one with a sent folder's name.
pub fn sent_folder(session: &mut Session) -> Result<Option<String>> {
    let names = session.list(Some(""), Some("*"))?;
    let marked = names
        .iter()
        .find(|name| name.attributes().iter().any(|attribute| format!("{attribute:?}") == "Sent"))
        .map(|name| name.name().to_owned());
    if marked.is_some() {
        return Ok(marked);
    }
    Ok(names.iter().map(|name| name.name().to_owned()).find(|name| {
        let shown = decode_folder(name);
        SENT_NAMES.iter().any(|known| shown.eq_ignore_ascii_case(known))
    }))
}

/// A folder's name as the server writes it (modified UTF-7, RFC 3501 5.1.3) in plain text: `&XfJT0ZAB-` is 已发送.
pub fn decode_folder(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let coded = &after[..end];
        if coded.is_empty() {
            out.push('&');
        } else {
            let bits: Vec<u8> = decode_base64(&coded.replace(',', "/"));
            let units: Vec<u16> = bits.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
            out.push_str(&String::from_utf16_lossy(&units));
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Base64 without padding, as modified UTF-7 writes it.
fn decode_base64(text: &str) -> Vec<u8> {
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let (mut out, mut buffer, mut held) = (Vec::new(), 0u32, 0u32);
    for c in text.bytes().filter_map(value) {
        buffer = (buffer << 6) | c;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push(u8::try_from((buffer >> held) & 0xff).unwrap_or_default());
        }
    }
    out
}

/// uids as an IMAP set: `1:5,7,9:12`.
pub fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut at = 0;
    while at < sorted.len() {
        let start = sorted[at];
        let mut end = start;
        while at + 1 < sorted.len() && sorted[at + 1] == end + 1 {
            at += 1;
            end = sorted[at];
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
        at += 1;
    }
    parts.join(",")
}

/// A new mail from someone else: what the window says of it, and what `mail.on_new` is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewMail {
    /// The mailbox's address.
    pub account: String,
    /// The mailbox's name (`qq`, `work`); filled in by the sync, which knows it.
    pub mailbox: String,
    pub thread: i64,
    pub from: Address,
    pub subject: String,
    pub snippet: String,
    pub at_ms: i64,
}

/// Which folder is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Inbox,
    Sent,
}

/// One sync: what was read here goes to the server, then the server's new mail comes here.
/// The new mails from others are returned (none from a first sync: those are old).
pub fn sync(
    session: &mut Session,
    store: &mut Store,
    account: &Account,
    days: u32,
    now_ms: i64,
) -> Result<Vec<NewMail>> {
    tell_seen(session, store, &account.address)?;
    let mut new = sync_folder(session, store, account, "INBOX", Kind::Inbox, days, now_ms)?;
    if let Some(sent) = sent_folder(session)? {
        new.extend(sync_folder(session, store, account, &sent, Kind::Sent, days, now_ms)?);
    }
    Ok(new)
}

/// The mails read here are marked read on the server.
fn tell_seen(session: &mut Session, store: &Store, account: &str) -> Result<()> {
    let untold = store.untold(account)?;
    let mut folders: Vec<&str> = untold.iter().map(|mail| mail.folder.as_str()).collect();
    folders.dedup();
    for folder in folders {
        session.select(folder)?;
        for (seen, change) in [(true, "+FLAGS (\\Seen)"), (false, "-FLAGS (\\Seen)")] {
            let these: Vec<&crate::store::Untold> =
                untold.iter().filter(|mail| mail.folder == folder && mail.seen == seen).collect();
            if these.is_empty() {
                continue;
            }
            let uids: Vec<u32> = these.iter().map(|mail| mail.uid).collect();
            session.uid_store(uid_set(&uids), change)?;
            store.told(&these.iter().map(|mail| mail.id).collect::<Vec<_>>())?;
        }
    }
    Ok(())
}

fn sync_folder(
    session: &mut Session,
    store: &mut Store,
    account: &Account,
    folder: &str,
    kind: Kind,
    days: u32,
    now_ms: i64,
) -> Result<Vec<NewMail>> {
    let mailbox = session.select(folder).with_context(|| format!("open the folder {}", decode_folder(folder)))?;
    let validity = mailbox.uid_validity.unwrap_or_default();
    let me = account.address.as_str();
    let mut known = store.folder(me, folder)?;
    if known.is_some_and(|(was, _)| was != validity) {
        store.forget_folder(me, folder)?;
        known = None;
    }
    let first = known.is_none();
    let last = known.map_or(0, |(_, last)| last);
    let mut uids: Vec<u32> = if mailbox.exists == 0 {
        Vec::new()
    } else if first {
        session.uid_search(format!("SINCE {}", imap_date(now_ms - i64::from(days) * 86_400_000)))?.into_iter().collect()
    } else {
        // `n:*` gives the last mail even when its uid is below n: those are left out.
        session.uid_search(format!("UID {}:*", last + 1))?.into_iter().filter(|uid| *uid > last).collect()
    };
    uids.sort_unstable();
    let mut new = Vec::new();
    let mut highest = last;
    for batch in uids.chunks(BATCH) {
        let sizes = session.uid_fetch(uid_set(batch), "(UID RFC822.SIZE FLAGS INTERNALDATE)")?;
        let mut whole = Vec::new();
        let mut header_only = Vec::new();
        let mut facts = std::collections::HashMap::new();
        for fetch in sizes.iter() {
            let Some(uid) = fetch.uid else { continue };
            let seen = fetch.flags().iter().any(|flag| matches!(flag, imap::types::Flag::Seen));
            let received = fetch.internal_date().map_or(now_ms, |date| date.timestamp_millis());
            facts.insert(uid, (seen, received));
            if fetch.size.unwrap_or_default() > WHOLE_LIMIT { header_only.push(uid) } else { whole.push(uid) }
        }
        for (set, query) in [(whole, "(UID BODY.PEEK[])"), (header_only, "(UID BODY.PEEK[HEADER])")] {
            if set.is_empty() {
                continue;
            }
            let fetches = session.uid_fetch(uid_set(&set), query)?;
            for fetch in fetches.iter() {
                let Some(uid) = fetch.uid else { continue };
                let raw = fetch.body().or_else(|| fetch.header()).unwrap_or_default();
                let mut mail = message::parse(raw);
                if fetch.body().is_none() {
                    "(This mail is larger than 8 MiB: only its header was read. Read it in your mail program.)"
                        .clone_into(&mut mail.text);
                }
                let (seen, received) = facts.get(&uid).copied().unwrap_or((false, now_ms));
                let outgoing =
                    kind == Kind::Sent || mail.from.as_ref().is_some_and(|from| from.addr == account.address);
                let added = store.add(&Incoming {
                    account: me,
                    folder,
                    uidvalidity: validity,
                    uid: Some(uid),
                    mail: &mail,
                    seen,
                    outgoing,
                    received_ms: received,
                })?;
                highest = highest.max(uid);
                if let Added::New { thread, .. } = added
                    && !first
                    && kind == Kind::Inbox
                    && !outgoing
                    && !seen
                {
                    new.push(NewMail {
                        account: me.to_owned(),
                        mailbox: String::new(),
                        thread,
                        from: mail.from.clone().unwrap_or_else(|| Address::new("", "")),
                        subject: mail.subject.clone(),
                        snippet: message::snippet(&mail.text, 120),
                        at_ms: mail.date_ms.unwrap_or(received),
                    });
                }
            }
        }
    }
    // What the server says of the mails kept before: read elsewhere, or not.
    if !first {
        let kept = store.uids(me, folder, validity)?;
        let old: Vec<u32> = kept.iter().copied().filter(|uid| !uids.contains(uid)).collect();
        if !old.is_empty() {
            let flags = session.uid_fetch(uid_set(&old), "(UID FLAGS)")?;
            let seen: Vec<u32> = flags
                .iter()
                .filter(|fetch| fetch.flags().iter().any(|flag| matches!(flag, imap::types::Flag::Seen)))
                .filter_map(|fetch| fetch.uid)
                .collect();
            let checked: Vec<u32> = flags.iter().filter_map(|fetch| fetch.uid).collect();
            store.server_seen(me, folder, validity, &checked, &seen)?;
        }
    }
    store.set_folder(me, folder, validity, highest)?;
    Ok(new)
}

/// A day as IMAP's SEARCH writes it: `26-Nov-2026`.
fn imap_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().format("%d-%b-%Y").to_string()
}

/// Waits for new mail in the inbox: with IDLE until the server says something or `most` has
/// passed; a server without IDLE is asked again after `most`. The wait is cut into pieces of
/// [`IDLE_SLICE`], and ends early when `stopped` says so: a socket shut down from another
/// thread does not always end a read blocked on it (Windows), a read that times out does.
pub fn wait(session: &mut Session, most: Duration, stopped: &dyn Fn() -> bool) -> Result<()> {
    let start = std::time::Instant::now();
    let left = || most.saturating_sub(start.elapsed());
    if !session.capabilities()?.has_str("IDLE") {
        while !left().is_zero() && !stopped() {
            std::thread::sleep(left().min(Duration::from_millis(250)));
        }
        return Ok(());
    }
    session.select("INBOX")?;
    while !left().is_zero() && !stopped() {
        let outcome = session.idle().timeout(left().min(IDLE_SLICE)).keepalive(false).wait_while(|_| false)?;
        if outcome == imap::extensions::idle::WaitOutcome::MailboxChanged {
            break;
        }
    }
    Ok(())
}

/// Puts a mail sent from here into the sent folder, unless the server has put it there itself
/// (Gmail and others keep what SMTP sent): looked for by its message id first.
pub fn keep_sent(account: &Account, credential: &Credential, raw: &[u8], message_id: &str) -> Result<()> {
    let mut session = login(account, credential).map_err(|failure| anyhow::anyhow!("{failure}"))?;
    let Some(sent) = sent_folder(&mut session)? else {
        session.logout().ok();
        return Ok(());
    };
    session.select(&sent)?;
    let there = session.uid_search(format!("HEADER Message-ID \"{message_id}\""))?;
    if there.is_empty() {
        session.append(&sent, raw).flag(imap::types::Flag::Seen).finish()?;
    }
    session.logout().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_and_sets_are_written_as_imap_writes_them() {
        assert_eq!(decode_folder("&XfJT0ZAB-"), "已发送");
        assert_eq!(decode_folder("INBOX.&XfJT0ZAB-"), "INBOX.已发送");
        assert_eq!(decode_folder("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_folder("Sent Messages"), "Sent Messages");
        assert_eq!(uid_set(&[9, 1, 2, 3, 5, 7, 8, 3]), "1:3,5,7:9");
        assert_eq!(uid_set(&[]), "");
        assert_eq!(imap_date(1_795_660_920_000), "26-Nov-2026");
    }
}
