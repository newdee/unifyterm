//! What keys and the mouse send to the program in the pty, as xterm sends it.

/// The modifier keys held with a key or a click.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Mods {
    pub(crate) shift: bool,
    pub(crate) control: bool,
    pub(crate) alt: bool,
}

impl Mods {
    /// xterm's number for these modifiers in a key sequence; `None` for no modifier.
    fn parameter(self) -> Option<u8> {
        let bits = u8::from(self.shift) | u8::from(self.alt) << 1 | u8::from(self.control) << 2;
        (bits != 0).then_some(bits + 1)
    }
}

/// Which form of the arrow keys the program asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum CursorKeys {
    /// `ESC [ A`.
    #[default]
    Normal,
    /// `ESC O A`: what full-screen programs set.
    Application,
}

/// Which form of mouse report the program asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum MouseProtocol {
    /// One byte a number: no cell past column or line 223.
    #[default]
    Legacy,
    /// Numbers as text: no limit.
    Sgr,
}

const ESCAPE: u8 = 0x1b;

/// The bytes for one key press. `key` is the key's name ("a", "enter",
/// "up"), `key_char` the text it types, if any.
///
/// `None` for a key that sends nothing by itself (a modifier), and for a
/// key that only types text: that text reaches the program through the
/// input method's path, which also carries composed text (Chinese).
pub(crate) fn encode(key: &str, key_char: Option<&str>, mods: Mods, cursor_keys: CursorKeys) -> Option<Vec<u8>> {
    if let Some(bytes) = named(key, mods, cursor_keys) {
        return Some(bytes);
    }
    // Ctrl and Alt together with text is AltGr: the text is what counts.
    if mods.control && !(mods.alt && key_char.is_some()) {
        let mut bytes = vec![control_byte(key)?];
        if mods.alt {
            bytes.insert(0, ESCAPE);
        }
        return Some(bytes);
    }
    if mods.alt && !mods.control {
        let text = if key == "space" { " " } else { key };
        // Alt as Meta: escape, then the key without Alt.
        return (text.chars().count() == 1).then(|| [&[ESCAPE], text.as_bytes()].concat());
    }
    None
}

fn named(key: &str, mods: Mods, cursor_keys: CursorKeys) -> Option<Vec<u8>> {
    let parameter = mods.parameter();
    // `ESC [ 1 ; modifiers LETTER`, or without modifiers `ESC [ LETTER`
    // (`ESC O LETTER` in the application form).
    let letter = |letter: char, keys: CursorKeys| match (parameter, keys) {
        (Some(p), _) => format!("\x1b[1;{p}{letter}"),
        (None, CursorKeys::Application) => format!("\x1bO{letter}"),
        (None, CursorKeys::Normal) => format!("\x1b[{letter}"),
    };
    // `ESC [ NUMBER ~`, with modifiers `ESC [ NUMBER ; modifiers ~`.
    let tilde = |number: u8| match parameter {
        Some(p) => format!("\x1b[{number};{p}~"),
        None => format!("\x1b[{number}~"),
    };
    let text = match key {
        "up" => letter('A', cursor_keys),
        "down" => letter('B', cursor_keys),
        "right" => letter('C', cursor_keys),
        "left" => letter('D', cursor_keys),
        "home" => letter('H', cursor_keys),
        "end" => letter('F', cursor_keys),
        // F1 to F4 are always in the application form.
        "f1" => letter('P', CursorKeys::Application),
        "f2" => letter('Q', CursorKeys::Application),
        "f3" => letter('R', CursorKeys::Application),
        "f4" => letter('S', CursorKeys::Application),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "tab" if mods.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        // Shift-Enter is a line feed: a new line in programs that take one
        // (Claude Code), Enter in the shells.
        "enter" if mods.shift => "\n".into(),
        "enter" if mods.alt => "\x1b\r".into(),
        "enter" => "\r".into(),
        "backspace" if mods.control => "\x08".into(),
        "backspace" if mods.alt => "\x1b\x7f".into(),
        "backspace" => "\x7f".into(),
        "escape" => "\x1b".into(),
        _ => return None,
    };
    Some(text.into_bytes())
}

/// The bits of a key's code that Ctrl keeps: Ctrl-C is 3, Ctrl-[ is escape.
const CONTROL_MASK: u8 = 0x1f;

/// Ctrl with a key: the key's code with the top three bits cleared;
/// Ctrl-Space is 0. `None` for a key Ctrl gives no code (a digit).
fn control_byte(key: &str) -> Option<u8> {
    if key == "space" {
        return Some(0);
    }
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let c = c.to_ascii_uppercase();
    let code = u8::try_from(c).ok()?;
    ('@'..='_').contains(&c).then_some(code & CONTROL_MASK)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MouseButton {
    Left,
    Middle,
    Right,
    /// Movement with no button held.
    None,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MouseAction {
    Press,
    Release,
    Move,
}

/// Added to a button's code for movement, and for each modifier.
const MOTION: u8 = 32;
const SHIFT: u8 = 4;
const ALT: u8 = 8;
const CONTROL: u8 = 16;
/// The legacy form sends each number as one byte, this much above it, so
/// it cannot say a coordinate past 255 - 32 = 223.
const LEGACY_OFFSET: usize = 32;
/// In the legacy form a release does not say which button.
const LEGACY_RELEASE: u8 = 3;

/// The report of a mouse event at a cell (column, line; both from 0) for a
/// program that asked for the mouse. `None` when the legacy form cannot say
/// the position.
pub(crate) fn mouse(
    button: MouseButton,
    action: MouseAction,
    (column, line): (usize, usize),
    mods: Mods,
    protocol: MouseProtocol,
) -> Option<Vec<u8>> {
    let mut code = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if action == MouseAction::Move {
        code += MOTION;
    }
    let modifiers = u8::from(mods.shift) * SHIFT + u8::from(mods.alt) * ALT + u8::from(mods.control) * CONTROL;
    let (x, y) = (column + 1, line + 1);
    match protocol {
        MouseProtocol::Sgr => {
            let end = if action == MouseAction::Release { 'm' } else { 'M' };
            Some(format!("\x1b[<{};{x};{y}{end}", code + modifiers).into_bytes())
        }
        MouseProtocol::Legacy => {
            if action == MouseAction::Release {
                code = LEGACY_RELEASE;
            }
            // A number that does not fit its byte cannot be said.
            let byte = |number: usize| u8::try_from(number + LEGACY_OFFSET).ok();
            Some(vec![ESCAPE, b'[', b'M', byte(usize::from(code + modifiers))?, byte(x)?, byte(y)?])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { shift: false, control: false, alt: false };
    const SHIFT: Mods = Mods { shift: true, ..NONE };
    const CTRL: Mods = Mods { control: true, ..NONE };
    const ALT: Mods = Mods { alt: true, ..NONE };

    fn sent(key: &str, key_char: Option<&str>, mods: Mods) -> Option<String> {
        encode(key, key_char, mods, CursorKeys::Normal).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn text_keys_are_left_to_the_input_method() {
        assert_eq!(sent("a", Some("a"), NONE), None);
        assert_eq!(sent("a", Some("A"), SHIFT), None);
        assert_eq!(sent("space", Some(" "), NONE), None);
        assert_eq!(sent("q", Some("@"), Mods { control: true, alt: true, shift: false }), None, "AltGr types text");
    }

    #[test]
    fn named_keys_send_their_sequences() {
        assert_eq!(sent("enter", None, NONE).unwrap(), "\r");
        assert_eq!(sent("backspace", None, NONE).unwrap(), "\x7f");
        assert_eq!(sent("tab", None, NONE).unwrap(), "\t");
        assert_eq!(sent("tab", None, SHIFT).unwrap(), "\x1b[Z");
        assert_eq!(sent("up", None, NONE).unwrap(), "\x1b[A");
        assert_eq!(sent("delete", None, NONE).unwrap(), "\x1b[3~");
        assert_eq!(sent("f1", None, NONE).unwrap(), "\x1bOP");
        assert_eq!(sent("f5", None, NONE).unwrap(), "\x1b[15~");
        assert_eq!(sent("f12", None, NONE).unwrap(), "\x1b[24~");
        assert_eq!(sent("enter", None, SHIFT).unwrap(), "\n");
        assert_eq!(sent("enter", None, ALT).unwrap(), "\x1b\r");
        assert_eq!(sent("backspace", None, CTRL).unwrap(), "\x08");
        assert_eq!(sent("backspace", None, ALT).unwrap(), "\x1b\x7f");
    }

    #[test]
    fn modifiers_go_in_the_sequence() {
        assert_eq!(sent("up", None, SHIFT).unwrap(), "\x1b[1;2A");
        assert_eq!(sent("right", None, CTRL).unwrap(), "\x1b[1;5C");
        assert_eq!(sent("left", None, Mods { shift: true, control: true, alt: true }).unwrap(), "\x1b[1;8D");
        assert_eq!(sent("delete", None, CTRL).unwrap(), "\x1b[3;5~");
        assert_eq!(sent("f1", None, SHIFT).unwrap(), "\x1b[1;2P");
        assert_eq!(sent("f5", None, ALT).unwrap(), "\x1b[15;3~");
    }

    #[test]
    fn the_application_form_changes_unmodified_arrows_only() {
        let app = |key, mods| String::from_utf8(encode(key, None, mods, CursorKeys::Application).unwrap()).unwrap();
        assert_eq!(app("up", NONE), "\x1bOA");
        assert_eq!(app("home", NONE), "\x1bOH");
        assert_eq!(app("up", CTRL), "\x1b[1;5A");
        assert_eq!(app("delete", NONE), "\x1b[3~");
    }

    #[test]
    fn control_keys_send_control_codes() {
        let control = |key, mods| encode(key, None, mods, CursorKeys::Normal);
        assert_eq!(control("c", CTRL).unwrap(), [3]);
        assert_eq!(control("[", CTRL).unwrap(), [0x1b]);
        assert_eq!(control("space", CTRL).unwrap(), [0]);
        assert_eq!(control("1", CTRL), None, "no control code for a digit");
        assert_eq!(control("é", CTRL), None, "nor for a letter outside ASCII");
        assert_eq!(control("c", Mods { control: true, alt: true, shift: false }).unwrap(), [0x1b, 3]);
    }

    #[test]
    fn alt_puts_escape_first() {
        assert_eq!(sent("b", Some("b"), ALT).unwrap(), "\x1bb");
        assert_eq!(sent("space", Some(" "), ALT).unwrap(), "\x1b ");
    }

    #[test]
    fn modifiers_alone_send_nothing() {
        assert_eq!(sent("shift", None, SHIFT), None);
        assert_eq!(sent("control", None, CTRL), None);
        assert_eq!(sent("alt", None, ALT), None);
    }

    fn sgr(button: MouseButton, action: MouseAction, mods: Mods) -> Vec<u8> {
        mouse(button, action, (9, 4), mods, MouseProtocol::Sgr).unwrap()
    }

    #[test]
    fn sgr_mouse_reports() {
        assert_eq!(sgr(MouseButton::Left, MouseAction::Press, NONE), b"\x1b[<0;10;5M");
        assert_eq!(sgr(MouseButton::Left, MouseAction::Release, NONE), b"\x1b[<0;10;5m");
        assert_eq!(sgr(MouseButton::Right, MouseAction::Press, CTRL), b"\x1b[<18;10;5M");
        assert_eq!(sgr(MouseButton::Left, MouseAction::Move, NONE), b"\x1b[<32;10;5M");
        assert_eq!(sgr(MouseButton::None, MouseAction::Move, NONE), b"\x1b[<35;10;5M");
        assert_eq!(sgr(MouseButton::WheelUp, MouseAction::Press, NONE), b"\x1b[<64;10;5M");
        assert_eq!(sgr(MouseButton::WheelDown, MouseAction::Press, SHIFT), b"\x1b[<69;10;5M");
        let far = mouse(MouseButton::Left, MouseAction::Press, (500, 300), NONE, MouseProtocol::Sgr);
        assert_eq!(far.unwrap(), b"\x1b[<0;501;301M", "no limit on the cell");
    }

    #[test]
    fn legacy_mouse_reports() {
        let legacy = |action, cell| mouse(MouseButton::Left, action, cell, NONE, MouseProtocol::Legacy);
        assert_eq!(legacy(MouseAction::Press, (9, 4)).unwrap(), [0x1b, b'[', b'M', 32, 42, 37]);
        assert_eq!(legacy(MouseAction::Release, (9, 4)).unwrap(), [0x1b, b'[', b'M', 35, 42, 37]);
        assert_eq!(legacy(MouseAction::Press, (222, 0)).unwrap()[4], 255, "the last column one byte can say");
        assert_eq!(legacy(MouseAction::Press, (223, 0)), None, "past what one byte says");
        assert_eq!(legacy(MouseAction::Press, (0, 223)), None, "the same for the line");
    }
}
