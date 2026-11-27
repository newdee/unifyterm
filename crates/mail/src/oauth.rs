//! Signing in with `OAuth2`, as Microsoft's mail wants: the device code flow (RFC 8628). UnifyTerm
//! asks for a short code, the person enters it in a browser and signs in there, and UnifyTerm is
//! given tokens: an access token that lasts about an hour, and a refresh token that gets new ones.
//! The refresh token is kept in the system's keychain; no password is ever seen here.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::account::Proxy;

/// UnifyTerm's application at Microsoft: a public client (no secret), allowed the device code
/// flow, for work and personal accounts (Microsoft Entra: any organization's directory and
/// personal Microsoft accounts). An id, not a secret. `mail.oauth_client_id` puts an
/// organization's own application in its place.
pub const CLIENT_ID: &str = "aee8de56-489a-4caa-94ed-b97b9bb4f57b";
/// Where Microsoft signs people in, for work and personal accounts alike.
pub const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
/// IMAP and SMTP at Outlook, and a refresh token.
const SCOPES: &str =
    "offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send";
/// The keychain's name for the refresh tokens; the login is the entry's user.
const SERVICE: &str = "unifyterm-mail-oauth";
/// A token is longer than some keychains take in one entry (Windows': 2560 bytes): kept in pieces.
const PIECE: usize = 1000;
/// A token this close to its end is not used: one is got anew.
const EARLY_MS: i64 = 120_000;
/// How long one request to Microsoft may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Who UnifyTerm is to the sign-in service, and where that service is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub client_id: String,
    /// The service's address, before `/devicecode` and `/token`.
    pub authority: String,
}

impl Provider {
    /// Microsoft, as UnifyTerm or as the organization that registered its own application.
    #[must_use]
    pub fn microsoft(client_id: &str) -> Self {
        let client_id = if client_id.trim().is_empty() { CLIENT_ID } else { client_id.trim() };
        Self { client_id: client_id.to_owned(), authority: AUTHORITY.to_owned() }
    }
}

/// What the person is asked to do: open `verification_uri` and enter `user_code`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    /// Seconds the code is good for.
    pub expires_in: u64,
    /// Seconds between two asks whether the person is done.
    #[serde(default = "five")]
    pub interval: u64,
}

const fn five() -> u64 {
    5
}

/// What the service gives.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    /// A new refresh token, when the service gave one (it may change each time).
    pub refresh: Option<String>,
    pub expires_at_ms: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tokens {{ expires_at_ms: {} }}", self.expires_at_ms)
    }
}

/// Why the service gave no token.
#[derive(Debug)]
pub enum Refusal {
    /// The sign-in is over: declined, expired, or the refresh token no longer works. Sign in again.
    SignInAgain(String),
    /// The network or the service: worth trying again later.
    Other(anyhow::Error),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignInAgain(why) => write!(f, "sign in again: {why}"),
            Self::Other(error) => write!(f, "{error:#}"),
        }
    }
}

#[derive(Deserialize)]
struct Answer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

fn agent(proxy: Option<&Proxy>) -> Result<ureq::Agent> {
    let proxy = proxy.map(|proxy| ureq::Proxy::new(&proxy.url())).transpose().context("the proxy")?;
    Ok(ureq::Agent::config_builder()
        .proxy(proxy)
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into())
}

fn post(proxy: Option<&Proxy>, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)> {
    let mut response = agent(proxy)?.post(url).send_form(form.iter().copied()).with_context(|| format!("ask {url}"))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().with_context(|| format!("read {url}'s answer"))?;
    Ok((status, body))
}

/// Asks for a code the person enters in the browser.
pub fn start(provider: &Provider, proxy: Option<&Proxy>) -> Result<DeviceCode> {
    if provider.client_id.is_empty() {
        bail!(
            "this UnifyTerm has no application at Microsoft to sign in with yet: register one (Microsoft Entra, \
             a public client that allows the device code flow) and set mail.oauth_client_id to its id"
        );
    }
    let url = format!("{}/devicecode", provider.authority);
    let (status, body) = post(proxy, &url, &[("client_id", &provider.client_id), ("scope", SCOPES)])?;
    if status != 200 {
        let answer: Option<Answer> = serde_json::from_str(&body).ok();
        let why = answer.and_then(|answer| answer.error_description.or(answer.error)).unwrap_or(body);
        bail!("Microsoft gave no code ({status}): {}", first_line(&why));
    }
    serde_json::from_str(&body).context("Microsoft's code")
}

/// Waits until the person has signed in with `code`: asks each `interval` seconds, until the code
/// expires or `stopped` says to stop.
pub fn finish(
    provider: &Provider,
    proxy: Option<&Proxy>,
    code: &DeviceCode,
    now_ms: &dyn Fn() -> i64,
    stopped: &dyn Fn() -> bool,
) -> Result<Tokens, Refusal> {
    let url = format!("{}/token", provider.authority);
    let mut interval = Duration::from_secs(code.interval.max(1));
    let end = now_ms() + i64::try_from(code.expires_in).unwrap_or(900) * 1000;
    loop {
        let waited = std::time::Instant::now();
        while waited.elapsed() < interval {
            if stopped() {
                return Err(Refusal::SignInAgain("stopped".to_owned()));
            }
            std::thread::sleep(interval.min(Duration::from_millis(100)));
        }
        if now_ms() > end {
            return Err(Refusal::SignInAgain("the code expired".to_owned()));
        }
        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", provider.client_id.as_str()),
            ("device_code", code.device_code.as_str()),
        ];
        let (status, body) = post(proxy, &url, &form).map_err(Refusal::Other)?;
        match tokens(status, &body, now_ms()) {
            Err(Refusal::SignInAgain(why)) if why.starts_with("authorization_pending") => {}
            Err(Refusal::SignInAgain(why)) if why.starts_with("slow_down") => interval += Duration::from_secs(5),
            other => return other,
        }
    }
}

/// New tokens for a refresh token.
pub fn refresh(provider: &Provider, proxy: Option<&Proxy>, refresh: &str, now_ms: i64) -> Result<Tokens, Refusal> {
    let url = format!("{}/token", provider.authority);
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", provider.client_id.as_str()),
        ("refresh_token", refresh),
        ("scope", SCOPES),
    ];
    let (status, body) = post(proxy, &url, &form).map_err(Refusal::Other)?;
    tokens(status, &body, now_ms)
}

/// The service's answer read: tokens, or why not. An error the service names (`invalid_grant`,
/// `authorization_pending`, ...) begins what is said; one of the service itself (5xx) is worth
/// trying again.
fn tokens(status: u16, body: &str, now_ms: i64) -> Result<Tokens, Refusal> {
    let answer: Answer = serde_json::from_str(body)
        .map_err(|_| Refusal::Other(anyhow::anyhow!("Microsoft answered {status}: {}", first_line(body))))?;
    if let Some(access) = answer.access_token.filter(|_| status == 200) {
        let lasts = answer.expires_in.unwrap_or(3600);
        return Ok(Tokens { access, refresh: answer.refresh_token, expires_at_ms: now_ms + lasts * 1000 });
    }
    let error = answer.error.unwrap_or_else(|| format!("status {status}"));
    let said = format!("{error}: {}", first_line(answer.error_description.as_deref().unwrap_or_default()));
    if status >= 500 || error == "temporarily_unavailable" {
        Err(Refusal::Other(anyhow::anyhow!("Microsoft: {said}")))
    } else {
        Err(Refusal::SignInAgain(said))
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default().trim()
}

/// Access tokens got in this process, by login: one is used until it is near its end.
static ACCESS: Mutex<Option<HashMap<String, (String, i64)>>> = Mutex::new(None);

/// An access token for `login`: the one got before while it lasts, else one from the kept refresh
/// token (the new refresh token is kept in its place).
pub fn access_token(login: &str, provider: &Provider, proxy: Option<&Proxy>, now_ms: i64) -> Result<String, Refusal> {
    {
        let cache = ACCESS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((token, until)) = cache.as_ref().and_then(|cache| cache.get(login))
            && *until - EARLY_MS > now_ms
        {
            return Ok(token.clone());
        }
    }
    let kept = kept(login).map_err(Refusal::Other)?.ok_or_else(|| {
        Refusal::SignInAgain(format!("{login} has not signed in: run ut mail login, or sign in from the mail view"))
    })?;
    let fresh = refresh(provider, proxy, &kept, now_ms)?;
    remember(login, &fresh).map_err(Refusal::Other)?;
    Ok(fresh.access)
}

/// Keeps what a sign-in gave: the refresh token in the keychain, the access token for this process.
pub fn remember(login: &str, tokens: &Tokens) -> Result<()> {
    if let Some(refresh) = &tokens.refresh {
        keep(login, refresh)?;
    }
    ACCESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(login.to_owned(), (tokens.access.clone(), tokens.expires_at_ms));
    Ok(())
}

fn entry(user: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, user).context("the keychain")
}

/// The refresh token kept for `login`: the count of pieces under `login`, the pieces under `login#n`.
fn kept(login: &str) -> Result<Option<String>> {
    let count = match entry(login)?.get_password() {
        Ok(count) => count,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => bail!("the keychain: {error}"),
    };
    let count: usize = count.parse().context("the keychain's entry for the sign-in")?;
    let mut token = String::new();
    for at in 0..count {
        token.push_str(&entry(&format!("{login}#{at}"))?.get_password().context("the keychain")?);
    }
    Ok(Some(token))
}

fn keep(login: &str, token: &str) -> Result<()> {
    forget(login)?;
    let pieces: Vec<String> =
        token.chars().collect::<Vec<_>>().chunks(PIECE).map(|piece| piece.iter().collect()).collect();
    for (at, piece) in pieces.iter().enumerate() {
        entry(&format!("{login}#{at}"))?.set_password(piece).context("the keychain")?;
    }
    entry(login)?.set_password(&pieces.len().to_string()).context("the keychain")
}

/// Takes the kept sign-in away; none kept is not an error.
pub fn forget(login: &str) -> Result<()> {
    ACCESS.lock().unwrap_or_else(PoisonError::into_inner).as_mut().map(|cache| cache.remove(login));
    let count = match entry(login)?.get_password() {
        Ok(count) => count.parse::<usize>().unwrap_or(0),
        Err(keyring::Error::NoEntry) => return Ok(()),
        Err(error) => bail!("the keychain: {error}"),
    };
    for at in 0..count {
        match entry(&format!("{login}#{at}"))?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => bail!("the keychain: {error}"),
        }
    }
    match entry(login)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => bail!("the keychain: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_tokens_or_why_not() {
        let good =
            tokens(200, r#"{"access_token":"tok-a1","refresh_token":"r","expires_in":3599}"#, 1000).expect("tokens");
        assert_eq!(
            (good.access.as_str(), good.refresh.as_deref(), good.expires_at_ms),
            ("tok-a1", Some("r"), 3_600_000)
        );
        assert!(!format!("{good:?}").contains("tok-a1"), "a token is never written out");
        let pending = tokens(400, r#"{"error":"authorization_pending","error_description":"AADSTS70016: x\r\ny"}"#, 0);
        assert!(matches!(pending, Err(Refusal::SignInAgain(why)) if why == "authorization_pending: AADSTS70016: x"));
        assert!(matches!(tokens(400, r#"{"error":"invalid_grant"}"#, 0), Err(Refusal::SignInAgain(_))));
        assert!(matches!(tokens(503, r#"{"error":"server_error"}"#, 0), Err(Refusal::Other(_))), "worth trying again");
        assert!(matches!(tokens(502, "<html>bad gateway</html>", 0), Err(Refusal::Other(_))));
        assert_eq!(Provider::microsoft(" ").client_id, CLIENT_ID);
        let none = Provider { client_id: String::new(), authority: "http://127.0.0.1:9".into() };
        assert!(format!("{:#}", start(&none, None).expect_err("refused")).contains("mail.oauth_client_id"));
        assert_eq!(Provider::microsoft("abc").client_id, "abc", "an organization's own application");
    }
}
