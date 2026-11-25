//! A key with the keys held with it, as the settings write one: `ctrl-shift-t`,
//! `f11`, `cmd-enter`.
//!
//! The settings' `[keys]` give each of the window's own commands its keys.
//! What a person types is read loosely (`Ctrl+Shift+T` is the same key) and
//! kept in one form, the one the window's key table takes.

/// The keys held with a key, in the order they are written.
const HELD: [&str; 4] = ["ctrl", "alt", "shift", "cmd"];

/// The keys that have a name: the ones that type nothing.
const NAMED: [&str; 16] = [
    "enter",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
    "escape",
    "-",
];

/// The highest function key a keyboard has.
const LAST_FUNCTION_KEY: u32 = 35;

/// The name of a held key as it is kept, for any of the names it goes by.
fn held(word: &str) -> Option<&'static str> {
    Some(match word {
        "ctrl" | "control" => "ctrl",
        "alt" | "option" | "opt" => "alt",
        "shift" => "shift",
        "cmd" | "command" | "super" | "win" | "meta" => "cmd",
        _ => return None,
    })
}

/// Whether `word` is a function key: `f1` to `f35`.
fn function_key(word: &str) -> bool {
    word.strip_prefix('f')
        .and_then(|number| number.parse::<u32>().ok())
        .is_some_and(|number| (1..=LAST_FUNCTION_KEY).contains(&number))
}

/// The key `text` writes, in the form it is kept: the held keys in one
/// order, then the key, in small letters with `-` between. Why not, when it
/// is no key or one the window may not take.
///
/// A key that types (a letter, a digit, a sign) needs Ctrl, Alt or Cmd with
/// it, and so do Enter, Tab, the arrows and the like: alone, or with Shift
/// only, they are what the shell is typed to with. A function key and Insert
/// may stand alone.
pub fn chord(text: &str) -> Result<String, String> {
    let small = text.trim().to_lowercase();
    if small.is_empty() {
        return Err("it is empty".into());
    }
    // `ctrl+shift+t` as well as `ctrl-shift-t`. A key that ends in `-` or `+` is that key itself.
    let (before, last) = match small.strip_suffix(['-', '+']) {
        Some(before) if before.is_empty() || before.ends_with(['-', '+']) => (before, &small[before.len()..]),
        _ => match small.rfind(['-', '+']) {
            Some(at) => (&small[..=at], &small[at + 1..]),
            None => ("", small.as_str()),
        },
    };
    let mut with = Vec::new();
    for word in before.split(['-', '+']).filter(|word| !word.is_empty()) {
        let name = held(word).ok_or_else(|| format!("'{word}' is not ctrl, alt, shift or cmd"))?;
        if with.contains(&name) {
            return Err(format!("'{name}' is there twice"));
        }
        with.push(name);
    }
    let key = match last {
        "esc" => "escape",
        "return" => "enter",
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        "del" => "delete",
        "ins" => "insert",
        other => other,
    };
    if held(key).is_some() {
        return Err(format!("'{key}' is a key that is held: it needs a key to go with"));
    }
    let types = key.chars().count() == 1 && key != "-";
    if !(types || key == "-" || NAMED.contains(&key) || function_key(key)) {
        return Err(format!(
            "'{key}' is no key: a letter, a digit, a sign, f1 to f{LAST_FUNCTION_KEY}, or one of enter, tab, space, backspace, delete, insert, home, end, pageup, pagedown, up, down, left, right, escape"
        ));
    }
    let strong = with.iter().any(|name| matches!(*name, "ctrl" | "alt" | "cmd"));
    if !(strong || function_key(key) || key == "insert") {
        return Err("the shell needs that key: hold ctrl, alt or cmd with it".into());
    }
    let mut written: Vec<&str> = HELD.iter().copied().filter(|name| with.contains(name)).collect();
    written.push(key);
    Ok(written.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_read_loosely_and_kept_in_one_form() {
        for (typed, kept) in [
            ("ctrl-shift-t", "ctrl-shift-t"),
            ("Ctrl+Shift+T", "ctrl-shift-t"),
            ("shift-ctrl-t", "ctrl-shift-t"),
            (" CMD-T ", "cmd-t"),
            ("command+enter", "cmd-enter"),
            ("win-e", "cmd-e"),
            ("Option-Return", "alt-enter"),
            ("f11", "f11"),
            ("F35", "f35"),
            ("shift-insert", "shift-insert"),
            ("ctrl-cmd-f", "ctrl-cmd-f"),
            ("ctrl-alt-Esc", "ctrl-alt-escape"),
            ("ctrl-pgdn", "ctrl-pagedown"),
            ("ctrl--", "ctrl--"),
            ("ctrl+-", "ctrl--"),
            ("ctrl-+", "ctrl-+"),
            ("ctrl++", "ctrl-+"),
            ("alt-[", "alt-["),
            ("ctrl-shift-tab", "ctrl-shift-tab"),
            ("ctrl-1", "ctrl-1"),
        ] {
            assert_eq!(chord(typed).as_deref(), Ok(kept), "{typed}");
            assert_eq!(chord(kept).as_deref(), Ok(kept), "{kept}: what is kept reads as itself");
        }
    }

    #[test]
    fn what_is_no_key_or_the_shells_key_is_refused_with_the_reason() {
        for (typed, says) in [
            ("", "it is empty"),
            ("ctrl", "is a key that is held"),
            ("ctrl-shift", "is a key that is held"),
            ("ctrl-shft-a", "'shft' is not ctrl, alt, shift or cmd"),
            ("ctrl-ctrl-a", "'ctrl' is there twice"),
            ("ctrl-banana", "'banana' is no key"),
            ("f36", "'f36' is no key"),
            ("f0", "'f0' is no key"),
            ("a", "the shell needs that key"),
            ("shift-a", "the shell needs that key"),
            ("enter", "the shell needs that key"),
            ("shift-tab", "the shell needs that key"),
            ("escape", "the shell needs that key"),
            ("up", "the shell needs that key"),
            ("-", "the shell needs that key"),
        ] {
            let why = chord(typed).expect_err(typed);
            assert!(why.contains(says), "{typed}: {why}");
        }
    }
}
