//! The whole way, against the pretend servers on this machine: a first sync,
//! new mail, read here and told there, an answer sent and kept, a folder
//! numbered anew, a wrong password, the running sync woken by IDLE, proxies, a server
//! down and up again, two mailboxes side by side, and `OAuth2`'s sign-in and tokens.

use std::sync::Arc;
use std::time::Duration;

use unifyterm_config::MailAuth;
use unifyterm_mail::fake::{Down, Fake, FakeAuthority, FakeProxy, Read, Socks};
use unifyterm_mail::oauth::{self, Provider, Refusal};
use unifyterm_mail::{Account, Credential, Filter, Plan, Stopper, Store, net, send, sync};

const PASSWORD: &str = "s3cret";
const ME: &str = "me@example.com";

fn mail(id: &str, from: &str, to: &str, subject: &str, answers: &str, text: &str) -> Vec<u8> {
    let refs = if answers.is_empty() {
        String::new()
    } else {
        format!("In-Reply-To: <{answers}>\r\nReferences: <{answers}>\r\n")
    };
    format!(
        "Message-ID: <{id}>\r\n{refs}From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: Thu, 26 Nov 2026 10:42:00 +0800\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n"
    )
    .into_bytes()
}

fn echo(word: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".into(), "/c".into(), format!("echo {word}")]
    } else {
        vec!["echo".into(), word.into()]
    }
}

fn plan(fake: &Fake, folder: &std::path::Path, password: &str) -> Plan {
    plan_through(fake, folder, password, "")
}

fn plan_through(fake: &Fake, folder: &std::path::Path, password: &str, proxy: &str) -> Plan {
    plan_of(ME, fake, folder, password, proxy, MailAuth::Password)
}

fn plan_of(address: &str, fake: &Fake, folder: &std::path::Path, password: &str, proxy: &str, auth: MailAuth) -> Plan {
    Plan {
        name: address.split('@').next().unwrap_or_default().to_owned(),
        account: Account::new(
            address,
            &format!("127.0.0.1:{}", fake.imap_port),
            &format!("127.0.0.1:{}", fake.smtp_port),
            "",
            proxy,
            auth,
        )
        .expect("account"),
        password_command: echo(password),
        provider: Provider { client_id: "test".into(), authority: String::new() },
        store: folder.join("mail.db"),
        days: 30,
        wait: Duration::from_secs(60),
        retry: Duration::from_millis(200),
    }
}

#[test]
fn mail_comes_is_read_answered_and_kept() {
    let fake = Fake::start("me@example.com", PASSWORD).expect("servers");
    let folder = tempfile::tempdir().expect("folder");
    let plan = plan(&fake, folder.path(), PASSWORD);
    fake.deliver(
        "INBOX",
        &mail("a1@example.com", "林晓 <lin@example.com>", "me@example.com", "周五的发布", "", "能提前到周四吗？"),
        Read::Unseen,
    );
    fake.deliver(
        "Sent Messages",
        &mail("b1@example.com", "me@example.com", "lin@example.com", "Re: 周五的发布", "a1@example.com", "可以。"),
        Read::Seen,
    );

    assert!(sync::once(&plan).expect("first sync").is_empty(), "a first sync brings old mail: none is new");
    let store = Store::open(&plan.store).expect("store");
    let threads = store.threads(10, Filter::Inbox, None).expect("list");
    assert_eq!(threads.len(), 1, "inbox and sent in one conversation");
    assert_eq!((threads[0].messages, threads[0].unread, threads[0].with.as_str()), (2, 1, "林晓"));

    let uid = fake.deliver(
        "INBOX",
        &mail(
            "a2@example.com",
            "林晓 <lin@example.com>",
            "me@example.com",
            "Re: 周五的发布",
            "b1@example.com",
            "好，周四 10 点。",
        ),
        Read::Unseen,
    );
    let new = sync::once(&plan).expect("second sync");
    assert_eq!(new.len(), 1, "one new mail");
    assert_eq!((new[0].thread, new[0].snippet.as_str()), (threads[0].id, "好，周四 10 点。"));

    store.mark_seen(threads[0].id).expect("read here");
    sync::once(&plan).expect("third sync");
    assert!(fake.is_seen("INBOX", uid), "read here, told the server");

    store.save_draft(threads[0].id, "claude", "收到", 1).expect("draft");
    unifyterm_mail::send_answer(std::slice::from_ref(&plan), threads[0].id, "收到，周四见。").expect("sent");
    let sent = fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].0.as_str(), sent[0].1.as_slice()), ("me@example.com", &["lin@example.com".to_owned()][..]));
    assert!(String::from_utf8_lossy(&sent[0].2).contains("In-Reply-To: <a2@example.com>"));
    assert_eq!(fake.count("Sent Messages"), 2, "a copy put in the sent folder");
    let thread = store.thread(threads[0].id).expect("read").expect("there");
    assert_eq!((thread.mails.len(), thread.mails.last().map(|m| m.outgoing)), (4, Some(true)), "kept here at once");
    assert!(thread.draft.is_none(), "its draft went with the answer");
    sync::once(&plan).expect("fourth sync");
    assert_eq!(
        store.thread(threads[0].id).expect("read").expect("there").mails.len(),
        4,
        "the server's copy is not kept twice"
    );

    fake.renumber("INBOX");
    sync::once(&plan).expect("after renumbering");
    assert_eq!(
        store.thread(threads[0].id).ok().flatten().map_or(0, |t| t.mails.len())
            + store.threads(10, Filter::All, None).expect("list").len(),
        5,
        "read again"
    );
}

#[test]
fn a_wrong_password_is_refused_and_said() {
    let fake = Fake::start("me@example.com", PASSWORD).expect("servers");
    let folder = tempfile::tempdir().expect("folder");
    let plan = plan(&fake, folder.path(), "wrong");
    match net::login(&plan.account, &Credential::Password("wrong".into())) {
        Err(net::Failure::Refused(_)) => {}
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
    assert!(format!("{:#}", sync::once(&plan).expect_err("refused")).contains("refused"));
}

#[test]
fn the_running_sync_hears_of_new_mail_while_it_waits() {
    let fake = Fake::start("me@example.com", PASSWORD).expect("servers");
    let folder = tempfile::tempdir().expect("folder");
    let plan = plan(&fake, folder.path(), PASSWORD);
    fake.deliver(
        "INBOX",
        &mail("a1@example.com", "lin@example.com", "me@example.com", "hello", "", "first"),
        Read::Seen,
    );
    let stop = Arc::new(Stopper::default());
    let (tell, heard) = std::sync::mpsc::channel();
    let running = {
        let (plan, stop) = (plan.clone(), Arc::clone(&stop));
        std::thread::spawn(move || sync::run(&plan, &stop, |new| tell.send(new).expect("told")))
    };
    // Wait for the first sync to be done (its status), then a mail comes while it waits.
    let store = Store::open(&plan.store).expect("store");
    for _ in 0..100 {
        if store.status(ME).ok().flatten().is_some_and(|status| status.text == "synced") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    fake.deliver(
        "INBOX",
        &mail("a2@example.com", "ops@example.com", "me@example.com", "下载页 502", "", "已恢复"),
        Read::Unseen,
    );
    let new = heard
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| panic!("not heard within 10 s; the sync says {:?}", store.status(ME)));
    assert_eq!(new[0].subject, "下载页 502");
    // Stopped while it waits (60 s): it ends within one piece of the wait, not when the wait is over.
    std::thread::sleep(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    stop.stop();
    running.join().expect("stopped");
    assert!(asked.elapsed() < net::IDLE_SLICE + Duration::from_secs(1), "stopped in {:?}", asked.elapsed());
}

#[test]
fn both_servers_are_reached_through_a_proxy_of_either_kind() {
    for kind in [Socks::Five, Socks::HttpConnect] {
        let fake = Fake::start("me@example.com", PASSWORD).expect("servers");
        let proxy = FakeProxy::start(kind).expect("proxy");
        let folder = tempfile::tempdir().expect("folder");
        let scheme = if kind == Socks::Five { "socks5" } else { "http" };
        let plan = plan_through(&fake, folder.path(), PASSWORD, &format!("{scheme}://127.0.0.1:{}", proxy.port));
        fake.deliver(
            "INBOX",
            &mail("a1@example.com", "lin@example.com", "me@example.com", "hello", "", "through"),
            Read::Unseen,
        );
        sync::once(&plan).expect("synced through the proxy");
        let store = Store::open(&plan.store).expect("store");
        let thread = store.threads(5, Filter::Inbox, None).expect("list")[0].id;
        unifyterm_mail::send_answer(std::slice::from_ref(&plan), thread, "ok").expect("sent through the proxy");
        assert_eq!(fake.sent().len(), 1, "{scheme}: sent");
        let passed = proxy.passed();
        assert!(
            passed.contains(&format!("127.0.0.1:{}", fake.imap_port)),
            "{scheme}: IMAP went through it: {passed:?}"
        );
        assert!(
            passed.contains(&format!("127.0.0.1:{}", fake.smtp_port)),
            "{scheme}: SMTP went through it: {passed:?}"
        );
    }
}

#[test]
fn a_server_that_cannot_be_reached_is_tried_again_until_it_can() {
    // Down: each connection is closed at once, as a server that is down or a network that drops it.
    let fake = Fake::start("me@example.com", PASSWORD).expect("servers");
    fake.set_down(Down::Down);
    let folder = tempfile::tempdir().expect("folder");
    let plan = plan(&fake, folder.path(), PASSWORD);
    let stop = Arc::new(Stopper::default());
    let running = {
        let (plan, stop) = (plan.clone(), Arc::clone(&stop));
        std::thread::spawn(move || sync::run(&plan, &stop, |_| {}))
    };
    let store = Store::open(&plan.store).expect("store");
    let said = |wanted: &dyn Fn(&unifyterm_mail::Status) -> bool| {
        (0..200).any(|_| {
            std::thread::sleep(Duration::from_millis(50));
            store.status(ME).ok().flatten().is_some_and(|status| wanted(&status))
        })
    };
    assert!(said(&|status| status.failed), "the failure is said: {:?}", store.status(ME));
    // It comes up: the next try (200 ms, 400 ms, ... after the failures) reaches it.
    fake.deliver(
        "INBOX",
        &mail("a1@example.com", "lin@example.com", "me@example.com", "back", "", "up again"),
        Read::Unseen,
    );
    fake.set_down(Down::Up);
    assert!(said(&|status| !status.failed && status.text == "synced"), "synced once it can: {:?}", store.status(ME));
    assert_eq!(store.threads(5, Filter::Inbox, None).expect("list").len(), 1);
    stop.stop();
    running.join().expect("stopped");
}

#[test]
fn two_mailboxes_sync_side_by_side_into_one_file() {
    const OTHER: &str = "me@other.example";
    let first = Fake::start(ME, PASSWORD).expect("servers");
    let second = Fake::start(OTHER, "other").expect("servers");
    let folder = tempfile::tempdir().expect("folder");
    let plans = [
        plan_of(ME, &first, folder.path(), PASSWORD, "", MailAuth::Auto),
        plan_of(OTHER, &second, folder.path(), "other", "", MailAuth::Auto),
    ];
    first.deliver("INBOX", &mail("a1@example.com", "lin@example.com", ME, "one", "", "1"), Read::Unseen);
    second.deliver("INBOX", &mail("a1@example.com", "lin@example.com", OTHER, "one", "", "1"), Read::Unseen);
    let locks: Vec<_> = plans
        .iter()
        .map(|plan| sync::Lock::take(folder.path(), &plan.account.address).expect("lock").expect("free"))
        .collect();
    assert!(sync::Lock::take(folder.path(), ME).expect("lock").is_none(), "one sync of a mailbox at a time");
    let stop = Arc::new(Stopper::default());
    let running: Vec<_> = plans
        .iter()
        .map(|plan| {
            let (plan, stop) = (plan.clone(), Arc::clone(&stop));
            std::thread::spawn(move || sync::run(&plan, &stop, |_| {}))
        })
        .collect();
    let store = Store::open(&plans[0].store).expect("store");
    let synced = (0..200).any(|_| {
        std::thread::sleep(Duration::from_millis(50));
        [ME, OTHER].iter().all(|who| store.status(who).ok().flatten().is_some_and(|status| status.text == "synced"))
    });
    assert!(synced, "both synced: {:?} {:?}", store.status(ME), store.status(OTHER));
    assert_eq!(store.threads(9, Filter::Inbox, None).expect("all").len(), 2, "the same mail in each, kept in each");
    assert_eq!(store.threads(9, Filter::Inbox, Some(OTHER)).expect("one")[0].account, OTHER);
    // An answer goes from the mailbox the conversation is in.
    let theirs = store.threads(9, Filter::Inbox, Some(OTHER)).expect("one")[0].id;
    unifyterm_mail::send_answer(&plans, theirs, "ok").expect("sent");
    assert_eq!((first.sent().len(), second.sent().len()), (0, 1));
    assert_eq!(second.sent()[0].0, OTHER);
    stop.stop();
    for one in running {
        one.join().expect("stopped");
    }
    drop(locks);
}

#[test]
fn oauth2_signs_in_with_a_code_and_its_token_lets_in_to_both_servers() {
    let fake = Fake::start(ME, PASSWORD).expect("servers");
    let authority = FakeAuthority::start(PASSWORD).expect("sign-in service");
    let provider = Provider { client_id: "test".into(), authority: authority.url.clone() };
    let code = oauth::start(&provider, None).expect("a code");
    assert_eq!((code.user_code.as_str(), code.interval), ("WDJB-MJHT", 1));
    let approving = {
        let authority = authority.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            authority.approve();
        })
    };
    let tokens = oauth::finish(&provider, None, &code, &unifyterm_history::now_ms, &|| false).expect("signed in");
    approving.join().expect("approved");
    assert_eq!((tokens.access.as_str(), tokens.refresh.as_deref()), (PASSWORD, Some("r1")));
    assert!(authority.asked().iter().filter(|asked| *asked == "device_code").count() >= 2, "asked until done");
    // The token lets in to IMAP and SMTP; another does not.
    net::login(&fake_account(&fake, MailAuth::Oauth2), &Credential::Token(PASSWORD.into())).expect("IMAP with XOAUTH2");
    match net::login(&fake_account(&fake, MailAuth::Oauth2), &Credential::Token("stale".into())) {
        Err(net::Failure::Refused(why)) => assert!(why.contains("refused"), "{why}"),
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    }
    send::check(&fake_account(&fake, MailAuth::Oauth2), &Credential::Token(PASSWORD.into()))
        .expect("SMTP with XOAUTH2");
    let refused = send::check(&fake_account(&fake, MailAuth::Oauth2), &Credential::Token("stale".into()));
    assert!(format!("{:#}", refused.expect_err("refused")).contains("refused"));
    // A refresh token gets a new pair, once; a taken-back sign-in is said as one to do again.
    let again = oauth::refresh(&provider, None, "r1", 0).expect("refreshed");
    assert_eq!(again.refresh.as_deref(), Some("r2"));
    assert!(matches!(oauth::refresh(&provider, None, "r1", 0), Err(Refusal::SignInAgain(_))), "used once");
    authority.revoke();
    assert!(matches!(oauth::refresh(&provider, None, "r2", 0), Err(Refusal::SignInAgain(_))));
    // A sync with a token held (no refresh token: nothing in the keychain).
    let folder = tempfile::tempdir().expect("folder");
    let plan = plan_of(ME, &fake, folder.path(), "unused", "", MailAuth::Oauth2);
    fake.deliver("INBOX", &mail("a1@example.com", "lin@example.com", ME, "hello", "", "token"), Read::Unseen);
    oauth::remember(ME, &oauth::Tokens { access: PASSWORD.into(), refresh: None, expires_at_ms: i64::MAX / 2 })
        .expect("held");
    sync::once(&plan).expect("synced with the token");
    assert_eq!(Store::open(&plan.store).expect("store").threads(5, Filter::Inbox, Some(ME)).expect("list").len(), 1);
}

fn fake_account(fake: &Fake, auth: MailAuth) -> Account {
    Account::new(ME, &format!("127.0.0.1:{}", fake.imap_port), &format!("127.0.0.1:{}", fake.smtp_port), "", "", auth)
        .expect("account")
}
