//! The keyboard back from a tab's hidden console window (Windows).
//!
//! Each tab's programs share a pseudo console, which has a window of its own
//! (`PseudoConsoleWindow`): never drawn, no size. A program that brings "its
//! console window" to the front (`SetForegroundWindow(GetConsoleWindow())`,
//! as keepane does when it goes to a pane) brings that one, and the keys
//! then go nowhere until the terminal is clicked. Windows Terminal makes its
//! own window that window's owner; the pseudo console UnifyTerm uses cannot
//! be given one. So when UnifyTerm's window loses the keyboard to the hidden
//! window of one of its own programs, it takes the keyboard back. A hidden
//! window of any other program, and every other window, is left alone.

/// The class Windows gives a pseudo console's window.
const HIDDEN_CONSOLE: &str = "PseudoConsoleWindow";

/// How many parents up a program is looked for: a shell, keepane's client
/// and what it starts are a few; this ends a loop in a damaged list.
const MOST_PARENTS: usize = 64;

/// Whether the window in front is the hidden console window of one of this
/// program's own programs (a child, or a child's child, ...).
pub(crate) fn own_hidden_console_in_front() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId};
    /// Longer than any class name looked for.
    const CLASS_ROOM: usize = 64;
    // SAFETY: no argument; the answer may be null.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return false;
    }
    let mut class = [0u16; CLASS_ROOM];
    // SAFETY: `window` is a window handle; `class` has the room the call is told.
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), i32::try_from(CLASS_ROOM).unwrap_or(i32::MAX)) };
    let class = String::from_utf16_lossy(&class[..usize::try_from(length).unwrap_or(0)]);
    if class != HIDDEN_CONSOLE {
        return false;
    }
    let mut process = 0;
    // SAFETY: `window` is a window handle; `process` is a place for the answer.
    unsafe { GetWindowThreadProcessId(window, &raw mut process) };
    descends_from(process, std::process::id(), &parents())
}

/// Each running program's parent, as `(program, parent)`.
fn parents() -> Vec<(u32, u32)> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut found = Vec::new();
    // SAFETY: asks for a list of the programs; the handle is closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return found;
    }
    // SAFETY: an all-zero PROCESSENTRY32W is valid; its size is set before use.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX);
    // SAFETY: `snapshot` is open and `entry` has its size set.
    let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) } != 0;
    while more {
        found.push((entry.th32ProcessID, entry.th32ParentProcessID));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &raw mut entry) } != 0;
    }
    // SAFETY: `snapshot` is open and closed once.
    unsafe { CloseHandle(snapshot) };
    found
}

/// Whether `program` is `ancestor`'s child, or a child's child, ..., by `parents`.
fn descends_from(program: u32, ancestor: u32, parents: &[(u32, u32)]) -> bool {
    let mut at = program;
    for _ in 0..MOST_PARENTS {
        let Some(&(_, parent)) = parents.iter().find(|(id, _)| *id == at) else { return false };
        if parent == ancestor {
            return true;
        }
        if parent == 0 || parent == at {
            return false;
        }
        at = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_known_by_its_line_of_parents() {
        let parents = [(10, 1), (20, 10), (30, 20), (40, 2), (50, 50)];
        assert!(descends_from(10, 1, &parents), "a child");
        assert!(descends_from(30, 1, &parents), "a child's child's child");
        assert!(!descends_from(40, 1, &parents), "another program's");
        assert!(!descends_from(1, 1, &parents), "itself is not its own child");
        assert!(!descends_from(99, 1, &parents), "a program that is gone");
        assert!(!descends_from(50, 1, &parents), "a loop ends");
    }

    /// This test program's own child is found; this program is not its own.
    #[test]
    fn a_running_child_is_found() {
        let mut child = std::process::Command::new("cmd").args(["/c", "ping -n 3 127.0.0.1 >nul"]).spawn().unwrap();
        let list = parents();
        assert!(descends_from(child.id(), std::process::id(), &list));
        assert!(!descends_from(std::process::id(), child.id(), &list));
        child.kill().ok();
        child.wait().ok();
    }
}
