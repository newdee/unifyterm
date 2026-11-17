//! Which command lines are not kept: ones that carry a secret, ones the
//! user marked by starting them with a space (as bash's `HISTCONTROL`
//! does), and very long ones. What is sent to a language model is cleaned
//! by the same patterns: [`redact`].

use std::sync::LazyLock;

use regex::{Regex, RegexSet};

/// The longest command line kept, in bytes.
const LONGEST: usize = 4096;

/// Why a command line is not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It begins with a space: the user's sign that it is not to be kept.
    LeadingSpace,
    /// It looks as if it has a password, a key or a token in it.
    Secret,
    /// It is longer than 4096 bytes.
    TooLong,
}

/// What a secret in a command line looks like. Wrongly keeping a secret
/// costs more than wrongly leaving out a command, so these are wide.
const PATTERNS: [&str; 11] = [
    // NAME=value, NAME: value, $env:NAME = value, for names with these words in them.
    r"(?i)[a-z0-9_]*(password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|credential|authorization|auth\b)[a-z0-9_]*\s*[=:]\s*\S+",
    // --password x, --token=x, -password x
    r"(?i)(^|\s)--?(pass|password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|auth[_-]?token|client[_-]?secret)(=|\s+)\S+",
    // An HTTP authorization header's value.
    r"(?i)\b(bearer|basic)\s+[a-z0-9._~+/=-]{8,}",
    // Cloud and service keys by their shape.
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\b(ghp|gho|ghu|ghs|ghr|github_pat)_[A-Za-z0-9_]{20,}",
    r"\bsk-[A-Za-z0-9_-]{20,}",
    r"\bnpm_[A-Za-z0-9]{20,}",
    r"\bxox[abposr]-[A-Za-z0-9-]{10,}",
    // A JSON web token.
    r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.",
    // user:password@ in a URL.
    r"(?i)[a-z][a-z0-9+.-]*://[^/\s:@]+:[^/\s@]+@",
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
];

static SECRETS: LazyLock<RegexSet> = LazyLock::new(|| RegexSet::new(PATTERNS).expect("the patterns are valid"));
static EACH: LazyLock<Vec<Regex>> =
    LazyLock::new(|| PATTERNS.iter().map(|pattern| Regex::new(pattern).expect("the patterns are valid")).collect());

/// `text` with every part that looks like a secret replaced by `with`.
///
/// Every pattern is matched against the text as given, and parts that
/// overlap are replaced as one: replacing one pattern's match first could
/// take away what another needs to see (`Authorization: Bearer <token>`).
#[must_use]
pub fn redact(text: &str, with: &str) -> String {
    let mut found: Vec<(usize, usize)> =
        EACH.iter().flat_map(|pattern| pattern.find_iter(text).map(|part| (part.start(), part.end()))).collect();
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in found {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for (start, end) in merged {
        out.push_str(&text[done..start]);
        out.push_str(with);
        done = end;
    }
    out.push_str(&text[done..]);
    out
}

/// Why `text` is not to be kept; `None` when it may be.
#[must_use]
pub fn refusal(text: &str) -> Option<Refusal> {
    if text.starts_with(' ') {
        Some(Refusal::LeadingSpace)
    } else if text.len() > LONGEST {
        Some(Refusal::TooLong)
    } else if SECRETS.is_match(text) {
        Some(Refusal::Secret)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100 command lines: `S ` before the 50 with a secret, `C ` before the 50 without.
    const CASES: &str = include_str!("secrets_cases.txt");

    #[test]
    fn the_case_set_is_caught_with_no_command_left_out_wrongly() {
        let (mut secret, mut clean, mut missed, mut wrong) = (0, 0, Vec::new(), Vec::new());
        for line in CASES.lines().filter(|line| !line.is_empty()) {
            let (kind, text) = line.split_at(2);
            match kind {
                "S " => {
                    secret += 1;
                    if refusal(text) != Some(Refusal::Secret) {
                        missed.push(text);
                    }
                }
                "C " => {
                    clean += 1;
                    if refusal(text).is_some() {
                        wrong.push(text);
                    }
                }
                other => panic!("a case begins with 'S ' or 'C ', not {other:?}"),
            }
        }
        assert_eq!((secret, clean), (50, 50), "the set is 50 and 50");
        assert!(missed.is_empty(), "secrets kept: {missed:#?}");
        assert!(wrong.is_empty(), "commands left out wrongly: {wrong:#?}");
    }

    #[test]
    fn every_secret_of_the_set_is_cut_out_and_nothing_else() {
        for line in CASES.lines().filter(|line| !line.is_empty()) {
            let (kind, text) = line.split_at(2);
            let clean = redact(text, "‹›");
            if kind == "S " {
                assert!(refusal(&clean).is_none() || clean.contains("‹›"), "{text} -> {clean}");
                assert!(clean.contains("‹›"), "something was cut from {text}");
            } else {
                assert_eq!(clean, text, "a line without a secret is left as it is");
            }
        }
        assert_eq!(redact("mysql --password hunter2 -e 'select 1'", "X"), "mysqlX -e 'select 1'");
        assert_eq!(redact("export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123 && ls", "X"), "export X && ls");
        assert!(!redact("curl -H 'Authorization: Bearer abcdefghijklmnop' x", "X").contains("abcdefghijklmnop"));
    }

    #[test]
    fn a_leading_space_and_length_are_reasons_too() {
        assert_eq!(refusal(" ls"), Some(Refusal::LeadingSpace));
        assert_eq!(refusal(&"x".repeat(LONGEST)), None);
        assert_eq!(refusal(&"x".repeat(LONGEST + 1)), Some(Refusal::TooLong));
        assert_eq!(refusal(""), None);
        // "auth" alone is a secret' name; "author" is not.
        for secret in ["export MY_AUTH=abc123", "curl -H \"auth: abc123\" x", "AUTHORIZATION=Basic x"] {
            assert_eq!(refusal(secret), Some(Refusal::Secret), "{secret}");
        }
        assert_eq!(refusal("git log --author=tom"), None);
    }
}
