//! The cells written the way a shell takes them in its prompt.
//!
//! The colors are the terminal's 24-bit escape sequences. A shell that
//! measures its prompt must be told which bytes take no room: bash between
//! `\001` and `\002` (readline's marks), zsh between `%{` and `%}`.
//! PowerShell and fish measure the screen themselves and take them as they are.
//!
//! The hooks put the text in a variable that the prompt names (bash
//! `PS1='${...}'`, zsh with `prompt_subst`): the shell expands it once and
//! does not read it again, so a `$` or a backtick in a folder's name is only
//! a character. zsh still reads `%` in it: it is written `%%`.

use std::fmt::Write as _;

use super::facts::Shell;
use super::render::{Cell, Rendered};

/// Between the left part and the right part of what `ut prompt` prints, for
/// zsh and fish, which put the right part in a place of their own.
pub(crate) const RIGHT_PART: char = '\u{1e}';

/// An escape sequence, marked the way `shell` wants bytes that take no room.
fn marked(sequence: &str, shell: Shell, out: &mut String) {
    match shell {
        Shell::Bash => {
            out.push('\u{1}');
            out.push_str(sequence);
            out.push('\u{2}');
        }
        Shell::Zsh => {
            out.push_str("%{");
            out.push_str(sequence);
            out.push_str("%}");
        }
        Shell::Pwsh | Shell::Fish => out.push_str(sequence),
    }
}

/// The sequence that sets these colors (the terminal's own where `None`).
fn colors(fg: Option<[u8; 3]>, bg: Option<[u8; 3]>) -> String {
    let mut sequence = String::from("\x1b[0");
    if let Some([red, green, blue]) = fg {
        let _ = write!(sequence, ";38;2;{red};{green};{blue}");
    }
    if let Some([red, green, blue]) = bg {
        let _ = write!(sequence, ";48;2;{red};{green};{blue}");
    }
    sequence.push('m');
    sequence
}

/// The colors a cell is written in: a new sequence is written where they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pen {
    fg: Option<[u8; 3]>,
    bg: Option<[u8; 3]>,
}

/// One run of cells, for `shell`: a color sequence where the colors change, and the terminal's own at the end.
pub(crate) fn written(cells: &[Cell], shell: Shell) -> String {
    let mut out = String::new();
    let mut current: Option<Pen> = None;
    for cell in cells {
        let wanted = Pen { fg: cell.fg, bg: cell.bg };
        if current != Some(wanted) {
            marked(&colors(cell.fg, cell.bg), shell, &mut out);
            current = Some(wanted);
        }
        if shell == Shell::Zsh && cell.text == '%' {
            out.push_str("%%");
        } else {
            out.push(cell.text);
        }
    }
    if current.is_some() {
        marked("\x1b[0m", shell, &mut out);
    }
    out
}

/// What `ut prompt` prints for `shell`: the lines, and for a shell with a
/// place of its own at the line's end, [`RIGHT_PART`] and what goes there.
pub(crate) fn output(rendered: &Rendered, shell: Shell) -> String {
    let lines: Vec<String> = rendered.lines.iter().map(|line| written(line, shell)).collect();
    let mut out = lines.join("\n");
    if shell.has_right_prompt() {
        out.push(RIGHT_PART);
        out.push_str(&written(&rendered.right, shell));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn cells() -> Vec<Cell> {
        vec![
            Cell { text: 'a', fg: Some(RED), bg: None },
            Cell { text: '%', fg: Some(RED), bg: None },
            Cell { text: 'b', fg: Some(RED), bg: Some(BLUE) },
            Cell { text: ' ', fg: None, bg: None },
        ]
    }

    #[test]
    fn a_color_sequence_where_the_colors_change_and_the_terminals_own_at_the_end() {
        assert_eq!(
            written(&cells(), Shell::Pwsh),
            "\x1b[0;38;2;255;0;0ma%\x1b[0;38;2;255;0;0;48;2;0;0;255mb\x1b[0m \x1b[0m"
        );
        assert_eq!(written(&cells(), Shell::Fish), written(&cells(), Shell::Pwsh), "fish takes them as they are");
        assert_eq!(written(&[], Shell::Pwsh), "", "nothing: no sequence");
    }

    #[test]
    fn bash_and_zsh_are_told_which_bytes_take_no_room() {
        let bash = written(&cells(), Shell::Bash);
        assert!(bash.starts_with("\u{1}\x1b[0;38;2;255;0;0m\u{2}a%"), "{bash:?}");
        assert_eq!(bash.matches('\u{1}').count(), 4, "each of the four sequences marked");
        let zsh = written(&cells(), Shell::Zsh);
        assert!(zsh.starts_with("%{\x1b[0;38;2;255;0;0m%}a%%"), "{zsh:?}: and % is written %%");
        // What takes room is the same in every shell: the characters themselves.
        let shown = |text: &str| -> String {
            let mut plain = String::new();
            let mut hidden = false;
            for c in text.chars() {
                match c {
                    '\u{1}' => hidden = true,
                    '\u{2}' => hidden = false,
                    _ if !hidden => plain.push(c),
                    _ => {}
                }
            }
            plain
        };
        assert_eq!(shown(&bash), "a%b ");
    }

    #[test]
    fn zsh_and_fish_get_the_right_part_after_a_mark_the_others_do_not() {
        let rendered = Rendered { lines: vec![cells(), vec![Cell { text: '>', fg: None, bg: None }]], right: cells() };
        let powershell = output(&rendered, Shell::Pwsh);
        assert_eq!(powershell.matches('\n').count(), 1, "two lines");
        assert!(!powershell.contains(RIGHT_PART));
        let zsh = output(&rendered, Shell::Zsh);
        let (left, right) = zsh.split_once(RIGHT_PART).expect("the mark");
        assert!(
            left.ends_with(">%{\x1b[0m%}") && right.contains("a%%"),
            "{zsh:?}: the mark, then the terminal's own colors"
        );
    }
}
