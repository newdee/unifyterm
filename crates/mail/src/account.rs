//! The mailbox: the address, and the two servers it is reached through.
//! A server not given is the one the address's domain is known to use.

use anyhow::{Context as _, Result, bail};

/// How a connection to a server is kept private.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte: IMAP on 993, SMTP on 465.
    Tls,
    /// Plain first, then STARTTLS: SMTP on 587 or 25.
    StartTls,
    /// None at all: only to this machine (the tests' servers).
    Plain,
}

/// A server, as `host:port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// Which of the two servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Imap,
    Smtp,
}

impl Server {
    /// `host:port` for `role`. The port says how it is kept private; a
    /// connection that is not is allowed only to this machine.
    pub fn parse(text: &str, role: Role) -> Result<Self> {
        let (host, port) = text.trim().rsplit_once(':').with_context(|| format!("\"{text}\" is not host:port"))?;
        let port: u16 = port.parse().with_context(|| format!("\"{text}\": the port is not a number"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        if host.is_empty() {
            bail!("\"{text}\" has no host");
        }
        let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
        let security = match (role, port) {
            (Role::Imap, 993) | (Role::Smtp, 465) => Security::Tls,
            (Role::Smtp, 587 | 25) => Security::StartTls,
            _ if local => Security::Plain,
            (Role::Imap, _) => bail!("\"{text}\": IMAP is read over TLS, on port 993"),
            (Role::Smtp, _) => bail!("\"{text}\": SMTP is sent on 465 (TLS) or 587 (STARTTLS)"),
        };
        Ok(Self { host, port, security })
    }
}

/// How a proxy is spoken to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    /// SOCKS5; the server's name goes to the proxy, which finds its address.
    Socks5,
    /// An HTTP proxy, asked to CONNECT.
    Http,
}

/// A proxy both servers are reached through: for a mailbox abroad, where the direct way is slow or closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    /// A name and password, when the address gives them (`socks5://name:password@host:port`).
    pub login: Option<(String, String)>,
}

impl Proxy {
    /// The proxy written back as an address, as an HTTP client takes it.
    #[must_use]
    pub fn url(&self) -> String {
        let scheme = match self.kind {
            ProxyKind::Socks5 => "socks5",
            ProxyKind::Http => "http",
        };
        let login = self.login.as_ref().map(|(name, password)| format!("{name}:{password}@")).unwrap_or_default();
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{scheme}://{login}{host}:{}", self.port)
    }

    /// `socks5://host:port`, `socks5h://...` or `http://host:port`, with `name:password@` before the
    /// host when the proxy wants one. Empty: no proxy, the servers are reached directly.
    pub fn parse(text: &str) -> Result<Option<Self>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let (scheme, rest) = text
            .split_once("://")
            .with_context(|| format!("\"{text}\": write the proxy as socks5://host:port or http://host:port"))?;
        let kind = match scheme.to_ascii_lowercase().as_str() {
            "socks5" | "socks5h" => ProxyKind::Socks5,
            "http" => ProxyKind::Http,
            other => bail!("\"{text}\": {other} is not socks5 or http"),
        };
        let rest = rest.trim_end_matches('/');
        let (login, place) = match rest.rsplit_once('@') {
            Some((login, place)) => {
                let (name, password) = login.split_once(':').unwrap_or((login, ""));
                (Some((name.to_owned(), password.to_owned())), place)
            }
            None => (None, rest),
        };
        let (host, port) = place.rsplit_once(':').with_context(|| format!("\"{text}\": the proxy needs its port"))?;
        let port: u16 = port.parse().with_context(|| format!("\"{text}\": the port is not a number"))?;
        if host.is_empty() {
            bail!("\"{text}\" has no host");
        }
        Ok(Some(Self { kind, host: host.trim_start_matches('[').trim_end_matches(']').to_owned(), port, login }))
    }
}

/// The servers of the domains whose mail is often read this way: (domains, IMAP, SMTP).
const KNOWN: [(&[&str], &str, &str); 9] = [
    (&["gmail.com", "googlemail.com"], "imap.gmail.com:993", "smtp.gmail.com:465"),
    (&["qq.com", "foxmail.com", "vip.qq.com"], "imap.qq.com:993", "smtp.qq.com:465"),
    (&["163.com"], "imap.163.com:993", "smtp.163.com:465"),
    (&["126.com"], "imap.126.com:993", "smtp.126.com:465"),
    (&["yeah.net"], "imap.yeah.net:993", "smtp.yeah.net:465"),
    (&["icloud.com", "me.com", "mac.com"], "imap.mail.me.com:993", "smtp.mail.me.com:587"),
    (&["yahoo.com"], "imap.mail.yahoo.com:993", "smtp.mail.yahoo.com:465"),
    (&["fastmail.com", "fastmail.fm"], "imap.fastmail.com:993", "smtp.fastmail.com:465"),
    (&["aliyun.com"], "imap.aliyun.com:993", "smtp.aliyun.com:465"),
];

/// Microsoft's own domains: their mail lets programs in with `OAuth2` only.
const MICROSOFT: [&str; 4] = ["outlook.com", "hotmail.com", "live.com", "msn.com"];
/// Microsoft's servers: IMAP for all its mail; SMTP for its own domains (Outlook.com's settings
/// page), and for an organization's mail there (Microsoft 365).
const MICROSOFT_IMAP: &str = "outlook.office365.com:993";
const MICROSOFT_SMTP: &str = "smtp-mail.outlook.com:587";
const MICROSOFT_365_SMTP: &str = "smtp.office365.com:587";

/// How a mailbox logs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// A password, app password or authorization code.
    Password,
    /// A sign-in in the browser; the token Microsoft gives is kept, never a password.
    OAuth2,
}

/// The mailbox UnifyTerm reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// `me@example.com`, in small letters.
    pub address: String,
    pub imap: Server,
    pub smtp: Server,
    /// The name logged in with.
    pub login: String,
    /// The proxy both servers are reached through; `None`: directly.
    pub proxy: Option<Proxy>,
    pub auth: Auth,
}

impl Account {
    /// The account the settings describe: `imap`, `smtp` and `login` may be
    /// empty, and are then the domain's known servers and the address; `proxy` empty is no proxy.
    /// `auth` auto is `OAuth2` for Microsoft's domains; a mailbox that signs in with `OAuth2` and
    /// gives no servers is at Microsoft's.
    pub fn new(
        address: &str,
        imap: &str,
        smtp: &str,
        login: &str,
        proxy: &str,
        auth: unifyterm_config::MailAuth,
    ) -> Result<Self> {
        let address = address.trim().to_lowercase();
        let (_, domain) = address.split_once('@').with_context(|| format!("\"{address}\" is not a mail address"))?;
        let microsoft = MICROSOFT.contains(&domain);
        let auth = match auth {
            unifyterm_config::MailAuth::Auto if microsoft => Auth::OAuth2,
            unifyterm_config::MailAuth::Auto | unifyterm_config::MailAuth::Password => Auth::Password,
            unifyterm_config::MailAuth::Oauth2 => Auth::OAuth2,
        };
        if microsoft && auth == Auth::Password {
            bail!("{domain} lets mail programs in with OAuth2 only: set auth to oauth2 (or auto)");
        }
        let known = KNOWN.iter().find(|(domains, _, _)| domains.contains(&domain));
        let server = |given: &str, role: Role| -> Result<Server> {
            let text = if given.trim().is_empty() {
                if auth == Auth::OAuth2 {
                    let smtp = if microsoft { MICROSOFT_SMTP } else { MICROSOFT_365_SMTP };
                    return Server::parse(if role == Role::Imap { MICROSOFT_IMAP } else { smtp }, role);
                }
                let (_, imap, smtp) = known.with_context(|| {
                    format!(
                        "the servers of {domain} are not known: give mail.{} as host:port",
                        if role == Role::Imap { "imap" } else { "smtp" }
                    )
                })?;
                if role == Role::Imap { *imap } else { *smtp }
            } else {
                given
            };
            Server::parse(text, role)
        };
        Ok(Self {
            imap: server(imap, Role::Imap)?,
            smtp: server(smtp, Role::Smtp)?,
            login: if login.trim().is_empty() { address.clone() } else { login.trim().to_owned() },
            proxy: Proxy::parse(proxy)?,
            address,
            auth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unifyterm_config::MailAuth;

    #[test]
    fn known_domains_bring_their_servers() {
        let qq = Account::new(" Me@QQ.com ", "", "", "", "", MailAuth::Auto).expect("qq");
        assert_eq!(qq.address, "me@qq.com");
        assert_eq!(qq.imap, Server { host: "imap.qq.com".into(), port: 993, security: Security::Tls });
        assert_eq!(qq.smtp.port, 465);
        assert_eq!(qq.login, "me@qq.com", "no login given: the address");
        let icloud = Account::new("me@icloud.com", "", "", "me", "", MailAuth::Auto).expect("icloud");
        assert_eq!((icloud.smtp.security, icloud.login.as_str()), (Security::StartTls, "me"));
    }

    #[test]
    fn what_is_given_goes_first_and_what_cannot_work_is_said() {
        let own =
            Account::new("me@example.com", "mail.example.com:993", "mail.example.com:587", "", "", MailAuth::Auto)
                .expect("own");
        assert_eq!((own.imap.host.as_str(), own.smtp.security), ("mail.example.com", Security::StartTls));
        let said = |address: &str, imap: &str, smtp: &str| {
            format!("{:#}", Account::new(address, imap, smtp, "", "", MailAuth::Auto).expect_err("refused"))
        };
        assert!(said("me@example.com", "", "").contains("give mail.imap as host:port"));
        let outlook = Account::new("Me@Outlook.com", "", "", "", "", MailAuth::Auto).expect("outlook");
        assert_eq!(
            (outlook.auth, outlook.imap.host.as_str(), outlook.smtp.port, outlook.smtp.security),
            (Auth::OAuth2, "outlook.office365.com", 587, Security::StartTls),
        );
        assert_eq!(outlook.smtp.host, "smtp-mail.outlook.com");
        assert_eq!(
            Account::new("me@corp.example", "", "", "", "", MailAuth::Oauth2).expect("365").smtp.host,
            "smtp.office365.com",
            "Microsoft's domains sign in in the browser, at Microsoft's servers"
        );
        let work = Account::new("me@corp.example", "", "", "", "", MailAuth::Oauth2).expect("Microsoft 365");
        assert_eq!((work.auth, work.imap.host.as_str()), (Auth::OAuth2, "outlook.office365.com"));
        assert_eq!(Account::new("me@qq.com", "", "", "", "", MailAuth::Auto).expect("qq").auth, Auth::Password);
        let password = Account::new("me@hotmail.com", "", "", "", "", MailAuth::Password).expect_err("refused");
        assert!(format!("{password:#}").contains("OAuth2 only"));
        assert!(said("nobody", "", "").contains("not a mail address"));
        assert!(
            said("me@example.com", "mail.example.com:143", "x:465").contains("over TLS"),
            "no plain IMAP to another machine"
        );
        assert!(said("me@example.com", "mail.example.com", "x:465").contains("not host:port"));
        let local = Account::new("me@example.com", "127.0.0.1:1143", "localhost:2525", "", "", MailAuth::Auto)
            .expect("this machine");
        assert_eq!((local.imap.security, local.smtp.security), (Security::Plain, Security::Plain));
    }

    #[test]
    fn proxies_are_read_from_their_address() {
        assert_eq!(Proxy::parse(" ").expect("none"), None, "empty: directly");
        let socks = Proxy::parse("socks5://127.0.0.1:7890").expect("read").expect("a proxy");
        assert_eq!(
            (socks.kind, socks.host.as_str(), socks.port, socks.login),
            (ProxyKind::Socks5, "127.0.0.1", 7890, None)
        );
        let http = Proxy::parse("http://me:p%w@proxy.example.com:8080/").expect("read").expect("a proxy");
        assert_eq!((http.kind, http.port, http.login), (ProxyKind::Http, 8080, Some(("me".into(), "p%w".into()))));
        assert_eq!(Proxy::parse("socks5h://[::1]:1080").expect("read").expect("a proxy").host, "::1");
        for wrong in ["127.0.0.1:7890", "ftp://x:1", "socks5://x", "socks5://:80", "http://x:port"] {
            assert!(Proxy::parse(wrong).is_err(), "{wrong} is refused");
        }
        let gmail =
            Account::new("me@gmail.com", "", "", "", "socks5://127.0.0.1:7890", MailAuth::Auto).expect("account");
        assert_eq!(gmail.proxy.map(|proxy| proxy.port), Some(7890));
    }
}
