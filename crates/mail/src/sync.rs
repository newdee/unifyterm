//! The sync that keeps running, one for each mailbox: one at a time on a machine for a mailbox
//! (a lock file in the data directory says which), it syncs, waits for new mail, and syncs
//! again; a dropped connection is tried again later, a refused password or sign-in
//! is said and tried again much later.

use std::fs::File;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, Result};

use crate::account::{Account, Auth};
use crate::net::{self, Credential, Failure, NewMail};
use crate::oauth::{self, Provider, Refusal};
use crate::store::{Health, Store};

/// How long a wait for new mail lasts before the server is asked again anyway (what was read
/// here goes then too), unless a plan says otherwise.
pub const WAIT: Duration = Duration::from_mins(5);
/// After a failure, the first wait before trying again (unless a plan says otherwise), and the longest:
/// each failure in a row doubles the wait.
pub const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MOST: Duration = Duration::from_mins(10);
/// After a refused password.
const REFUSED_WAIT: Duration = Duration::from_mins(30);

/// The machine's one sync of a mailbox, while this is held.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

impl Lock {
    /// The lock of the mailbox `address`, when no other sync holds it.
    pub fn take(directory: &Path, address: &str) -> Result<Option<Self>> {
        std::fs::create_dir_all(directory)?;
        let name: String =
            address.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
        let name = format!("mail-{name}.lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(directory.join(&name))
            .with_context(|| format!("open {name}"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error).with_context(|| format!("lock {name}")),
        }
    }
}

/// What a sync of one mailbox needs.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The mailbox's name: `qq`, `work`.
    pub name: String,
    pub account: Account,
    /// `password_command`, for a mailbox that logs in with a password.
    pub password_command: Vec<String>,
    /// Where a mailbox that signs in with `OAuth2` gets its tokens.
    pub provider: Provider,
    pub store: PathBuf,
    pub days: u32,
    /// How long one wait for new mail lasts ([`WAIT`]).
    pub wait: Duration,
    /// The first wait after a failure ([`RETRY_FIRST`]).
    pub retry: Duration,
}

impl Plan {
    /// What the mailbox logs in with now: its password, or an access token (got anew when the one
    /// held is near its end). No password or sign-in kept, or a refused sign-in, is
    /// [`Failure::Refused`]: trying again soon will not help.
    pub fn credential(&self) -> Result<Credential, Failure> {
        match self.account.auth {
            Auth::Password => crate::secret::password(&self.account.login, &self.password_command)
                .map(Credential::Password)
                .map_err(|error| Failure::Refused(format!("{error:#}"))),
            Auth::OAuth2 => oauth::access_token(
                &self.account.login,
                &self.provider,
                self.account.proxy.as_ref(),
                unifyterm_history::now_ms(),
            )
            .map(Credential::Token)
            .map_err(|refusal| match refusal {
                Refusal::SignInAgain(why) => Failure::Refused(why),
                Refusal::Other(error) => Failure::Other(error),
            }),
        }
    }
}

/// One sync, start to end: logs in, syncs, logs out. For `ut mail sync`.
pub fn once(plan: &Plan) -> Result<Vec<NewMail>> {
    let credential = plan.credential().map_err(|failure| anyhow::anyhow!("{failure}"))?;
    let mut store = Store::open(&plan.store)?;
    let mut session = net::login(&plan.account, &credential).map_err(|failure| anyhow::anyhow!("{failure}"))?;
    let new = net::sync(&mut session, &mut store, &plan.account, plan.days, unifyterm_history::now_ms())?;
    store.set_status(&plan.account.address, "synced", Health::Fine, unifyterm_history::now_ms())?;
    session.logout().ok();
    Ok(new)
}

/// Stops a running sync from another thread: at once, also while it waits for new mail.
#[derive(Debug, Default)]
pub struct Stopper {
    stopped: AtomicBool,
    /// The socket of the connection in use: shut down to end a wait.
    socket: Mutex<Option<TcpStream>>,
}

impl Stopper {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(socket) = self.socket.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn hold(&self, socket: TcpStream) {
        *self.socket.lock().unwrap_or_else(PoisonError::into_inner) = Some(socket);
        // Stopped while the connection was opening: it ends now.
        if self.stopped() {
            self.stop();
        }
    }
}

/// Syncs until `stop` is stopped: `on_new` is told of the new mails of each sync.
pub fn run(plan: &Plan, stop: &Stopper, mut on_new: impl FnMut(Vec<NewMail>)) {
    let mut retry = plan.retry;
    while !stop.stopped() {
        let wait = match round(plan, stop, &mut on_new) {
            Ok(()) => {
                retry = plan.retry;
                continue;
            }
            Err(Failure::Refused(why)) => {
                say(plan, &why, Health::Failed);
                REFUSED_WAIT
            }
            Err(Failure::Other(error)) => {
                say(plan, &format!("{error:#}"), Health::Failed);
                let now = retry;
                retry = (retry * 2).min(RETRY_MOST);
                now
            }
        };
        sleep(wait, stop);
    }
}

/// Logged in, sync and wait again and again, until the connection fails or `stop` is stopped.
fn round(plan: &Plan, stop: &Stopper, on_new: &mut impl FnMut(Vec<NewMail>)) -> Result<(), Failure> {
    let credential = plan.credential()?;
    let mut store = Store::open(&plan.store).map_err(Failure::Other)?;
    say(plan, "connecting", Health::Fine);
    let mut session = net::login_holding(&plan.account, &credential, |socket| stop.hold(socket))?;
    while !stop.stopped() {
        let new = net::sync(&mut session, &mut store, &plan.account, plan.days, unifyterm_history::now_ms())
            .map_err(Failure::Other)?;
        store
            .set_status(&plan.account.address, "synced", Health::Fine, unifyterm_history::now_ms())
            .map_err(Failure::Other)?;
        if !new.is_empty() {
            on_new(new.into_iter().map(|mail| NewMail { mailbox: plan.name.clone(), ..mail }).collect());
        }
        if let Err(error) = net::wait(&mut session, plan.wait, &|| stop.stopped()) {
            // A wait ended by `stop` is no failure.
            return if stop.stopped() { Ok(()) } else { Err(Failure::Other(error)) };
        }
    }
    session.logout().ok();
    Ok(())
}

fn say(plan: &Plan, text: &str, health: Health) {
    if health == Health::Failed {
        log::warn!("mail ({}): {text}", plan.name);
    }
    let now = unifyterm_history::now_ms();
    if let Err(error) =
        Store::open(&plan.store).and_then(|store| store.set_status(&plan.account.address, text, health, now))
    {
        log::warn!("mail: the status could not be written: {error:#}");
    }
}

/// Sleeps `most`, waking early when `stop` is stopped.
fn sleep(most: Duration, stop: &Stopper) {
    let tick = Duration::from_millis(250);
    let mut slept = Duration::ZERO;
    while slept < most && !stop.stopped() {
        std::thread::sleep(tick);
        slept += tick;
    }
}
