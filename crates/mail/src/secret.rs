//! The mailbox's password: never in the settings file. It is what a program
//! of the user's prints (`mail.password_command`), or what `ut mail login`
//! put in the system's keychain (Windows' Credential Manager, the macOS
//! Keychain, the Secret Service on Linux).

use anyhow::{Context as _, Result, bail};

/// The keychain's name for UnifyTerm's mail passwords; the login is the entry's user.
const SERVICE: &str = "unifyterm-mail";

/// The password for `login`: from `command` when it names a program, else from the keychain.
pub fn password(login: &str, command: &[String]) -> Result<String> {
    if let Some((program, arguments)) = command.split_first().filter(|(program, _)| !program.trim().is_empty()) {
        let output = std::process::Command::new(program)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("mail.password_command: run {program}"))?;
        if !output.status.success() {
            bail!("mail.password_command: {program} ended with {}", output.status);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let first = text.lines().next().unwrap_or_default().trim_end_matches('\r');
        if first.is_empty() {
            bail!("mail.password_command: {program} printed nothing");
        }
        return Ok(first.to_owned());
    }
    keyring::Entry::new(SERVICE, login).and_then(|entry| entry.get_password()).map_err(|error| match error {
        keyring::Error::NoEntry => anyhow::anyhow!("no password kept for {login}: run ut mail login"),
        other => anyhow::anyhow!("the keychain: {other}"),
    })
}

/// Keeps `password` for `login` in the system's keychain.
pub fn keep(login: &str, password: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, login).and_then(|entry| entry.set_password(password)).context("the keychain")
}

/// Takes the kept password away; none kept is not an error.
pub fn forget(login: &str) -> Result<()> {
    match keyring::Entry::new(SERVICE, login).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("the keychain: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_gives_its_first_line_and_a_failing_one_is_said() {
        let echo: Vec<String> = if cfg!(windows) {
            ["cmd", "/c", "echo s3cret& echo second"].map(String::from).to_vec()
        } else {
            ["sh", "-c", "printf 's3cret\\nsecond\\n'"].map(String::from).to_vec()
        };
        assert_eq!(password("me@example.com", &echo).expect("password"), "s3cret");
        let fails: Vec<String> =
            if cfg!(windows) { ["cmd", "/c", "exit 3"] } else { ["sh", "-c", "exit 3"] }.map(String::from).to_vec();
        assert!(format!("{:#}", password("me@example.com", &fails).expect_err("failed")).contains("ended with"));
        let silent: Vec<String> =
            if cfg!(windows) { ["cmd", "/c", "rem"] } else { ["sh", "-c", "true"] }.map(String::from).to_vec();
        assert!(format!("{:#}", password("me@example.com", &silent).expect_err("nothing")).contains("printed nothing"));
    }
}
