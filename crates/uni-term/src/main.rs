//! uni-term: one terminal for Windows, macOS and Linux. The window and its
//! tabs are here; a tab's view is in `terminal`, its screen and pty in
//! `session`.

// A release build on Windows is a window program: no console opens with it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod boxdraw;
mod config;
mod frame;
mod keys;
mod session;
mod terminal;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, Application, Bounds, Context, Entity, KeyBinding, MouseButton, SharedString, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

use config::Config;
use terminal::{Settings, TerminalEvent, TerminalView};

actions!(uni_term, [NewTab, CloseTab, NextTab, PreviousTab, Copy, Paste]);

const APP_NAME: &str = "uni-term";
const WINDOW_SIZE: (f32, f32) = (960., 620.);
const TAB_BAR_HEIGHT: f32 = 34.;
/// The longest title a tab shows, in characters.
const TAB_TITLE_CHARS: usize = 28;
const USAGE: &str = "usage: uni-term [--config FILE] [-e PROGRAM [ARGS...]]
       uni-term --replay FILE [--size COLUMNSxLINES]

  --config FILE        read settings from FILE, not the default config.toml
  -e PROGRAM [ARGS...] run PROGRAM in the first tab, not the shell
  --replay FILE        print the screen that FILE's bytes draw, and leave:
                       FILE is what a program printed, as UNI_TERM_RAW_LOG saves it
  --size CxL           the replay's screen size (default 100x30)
  -V, --version        print the version
  -h, --help           print this text

Environment:
  UNI_TERM_RAW_LOG     a file to append every byte the first tab's program prints to;
                       later tabs write to the name with .2, .3 added";

/// The replay's screen size when `--size` is not given.
const REPLAY_SIZE: (u16, u16) = (100, 30);
/// History kept by a replay's screen: none, it prints the last screen only.
const REPLAY_HISTORY: usize = 0;

/// What the command line asked for.
#[derive(Debug, PartialEq, Default)]
struct Args {
    config: Option<PathBuf>,
    /// The first tab's program and its arguments.
    command: Option<(String, Vec<String>)>,
    /// A file of terminal output to draw and print, with no window.
    replay: Option<PathBuf>,
    /// The replay's screen size as (columns, lines).
    size: Option<(u16, u16)>,
}

/// `100x30` as (100, 30). Neither may be 0.
fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (columns, lines) = text.split_once('x')?;
    let (columns, lines) = (columns.parse::<u16>().ok()?, lines.parse::<u16>().ok()?);
    (columns > 0 && lines > 0).then_some((columns, lines))
}

/// The screen that the bytes in `file` draw, as text.
fn replay(file: &std::path::Path, (columns, lines): (u16, u16)) -> anyhow::Result<String> {
    use anyhow::Context as _;
    let bytes = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    let mut screen = session::Screen::new(columns, lines, REPLAY_HISTORY, theme::Theme::default(), None);
    screen.feed(&bytes);
    Ok(screen.text())
}

/// What to do instead of opening a window.
#[derive(Debug, PartialEq)]
enum Early {
    /// Print this and leave with success.
    Print(String),
    /// Print this to stderr and leave with failure.
    Usage(String),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, Early> {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(Early::Print(USAGE.into())),
            "-V" | "--version" => return Err(Early::Print(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION")))),
            "--config" => {
                let file = args.next().ok_or_else(|| Early::Usage("--config needs a file".into()))?;
                parsed.config = Some(file.into());
            }
            "--replay" => {
                let file = args.next().ok_or_else(|| Early::Usage("--replay needs a file".into()))?;
                parsed.replay = Some(file.into());
            }
            "--size" => {
                let size = args.next().ok_or_else(|| Early::Usage("--size needs a size like 100x30".into()))?;
                parsed.size = Some(parse_size(&size).ok_or_else(|| Early::Usage(format!("--size {size}: not a size like 100x30")))?);
            }
            "-e" => {
                let program = args.next().ok_or_else(|| Early::Usage("-e needs a program".into()))?;
                parsed.command = Some((program, args.by_ref().collect()));
            }
            other => return Err(Early::Usage(format!("unknown argument: {other}"))),
        }
    }
    Ok(parsed)
}

struct Tab {
    view: Entity<TerminalView>,
    /// What the tab is called until its program sets a title.
    name: String,
    _events: Subscription,
}

struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    settings: Arc<Settings>,
}

impl Workspace {
    /// A window with one tab running `command` (the shell when `None`).
    /// `notice`: a line to show at the top of that tab.
    fn new(
        settings: Arc<Settings>,
        command: Option<(String, Vec<String>)>,
        notice: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut workspace = Self { tabs: Vec::new(), active: 0, settings };
        workspace.open(command, window, cx);
        if let (Some(tab), Some(notice)) = (workspace.tabs.first(), notice) {
            tab.view.update(cx, |view, _| view.announce(&notice));
        }
        workspace
    }

    /// Opens a tab running `command`, or the configured shell.
    ///
    /// A configured shell that does not start is replaced by the default
    /// one, with a line in the tab saying so: a mistake in the settings
    /// must not leave the user without a terminal. A `command` that does
    /// not start is reported; with no tab open, uni-term then ends in failure.
    fn open(&mut self, command: Option<(String, Vec<String>)>, window: &mut Window, cx: &mut Context<Self>) {
        let shell = &self.settings.config.shell;
        let asked = command.is_some();
        let (program, args) = command.unwrap_or_else(|| (shell.program.clone().unwrap_or_default(), shell.args.clone()));
        let program = (!program.is_empty()).then_some(program);
        let mut notice = None;
        let started = match self.start(program.as_deref(), &args) {
            Err(error) if !asked && program.is_some() => {
                notice = Some(format!("{APP_NAME}: shell.program did not start, this is the default shell: {error:#}"));
                self.start(None, &[])
            }
            started => started,
        };
        let ((session, events), name) = match started {
            Ok(started) => started,
            Err(error) => {
                eprintln!("{APP_NAME}: {error:#}");
                if self.tabs.is_empty() {
                    std::process::exit(1);
                }
                return;
            }
        };
        let settings = self.settings.clone();
        let view = cx.new(|cx| TerminalView::new(session, events, settings, cx));
        if let Some(notice) = notice {
            view.update(cx, |view, _| view.announce(&notice));
        }
        let events = cx.subscribe_in(&view, window, Self::terminal_event);
        self.tabs.push(Tab { view, name, _events: events });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    /// Starts `program` (the default shell when `None`) in a session; the
    /// session, and the tab's name.
    fn start(&self, program: Option<&str>, args: &[String]) -> anyhow::Result<((session::Session, smol::channel::Receiver<session::PtyEvent>), String)> {
        let command = session::shell_command(program, args)?;
        let name = command.get_argv().first().map(|program| tab_name(&program.to_string_lossy())).unwrap_or_default();
        Ok((TerminalView::start(command, &self.settings)?, name))
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        self.active = index;
        window.focus(&tab.view.read(cx).focus);
        window.set_window_title(&self.title(index, cx));
        cx.notify();
    }

    /// Closes a tab; with the last one, the window and the app.
    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            cx.quit();
            return;
        }
        // The tab to the right takes the closed one's place; past the end, the last.
        let next = if index < self.active { self.active - 1 } else { self.active.min(self.tabs.len() - 1) };
        self.activate(next, window, cx);
    }

    fn title(&self, index: usize, cx: &App) -> String {
        let tab = &self.tabs[index];
        match tab.view.read(cx).title() {
            // ConPTY's first title is the program's whole path: its name says as much.
            Some(title) if title.ends_with(".exe") => tab_name(title),
            Some(title) => title.to_owned(),
            None => tab.name.clone(),
        }
    }

    fn terminal_event(&mut self, view: &Entity<TerminalView>, event: &TerminalEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| &tab.view == view) else { return };
        match event {
            TerminalEvent::Exited => self.close(index, window, cx),
            TerminalEvent::Title => {
                if index == self.active {
                    window.set_window_title(&self.title(index, cx));
                }
                cx.notify();
            }
        }
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.tabs.len();
        let next = if forward { (self.active + 1) % count } else { (self.active + count - 1) % count };
        self.activate(next, window, cx);
    }
}

/// A program's path as a tab's name: the file's name without `.exe`.
fn tab_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    file.strip_suffix(".exe").unwrap_or(file).to_owned()
}

/// A title cut to what a tab has room for.
fn tab_title(title: &str) -> String {
    if title.chars().count() <= TAB_TITLE_CHARS {
        return title.to_owned();
    }
    let cut: String = title.chars().take(TAB_TITLE_CHARS - 1).collect();
    format!("{cut}…")
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.settings.theme;
        let (background, foreground) = (theme.background, theme.foreground);
        let bar = background.toward(theme::Rgb(0, 0, 0), 0.35);
        let dim = foreground.toward(background, 0.5);
        let tabs = (0..self.tabs.len()).map(|index| {
            let active = index == self.active;
            div()
                .id(("tab", index))
                .h_full()
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .text_color(rgb(if active { foreground.hex() } else { dim.hex() }))
                .when(active, |tab| tab.bg(rgb(background.hex())))
                .on_click(cx.listener(move |this, _, window, cx| this.activate(index, window, cx)))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close(index, window, cx)))
                .child(SharedString::from(tab_title(&self.title(index, cx))))
                .child(
                    div()
                        .id(("close", index))
                        .text_color(rgb(dim.hex()))
                        .hover(|close| close.text_color(rgb(foreground.hex())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(index, window, cx);
                        }))
                        .child("×"),
                )
        });
        let add = div()
            .id("new-tab")
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .text_color(rgb(dim.hex()))
            .hover(|add| add.text_color(rgb(foreground.hex())))
            .on_click(cx.listener(|this, _, window, cx| this.open(None, window, cx)))
            .child("+");
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bar.hex()))
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.open(None, window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.step(false, window, cx)))
            .child(div().flex().flex_none().h(px(TAB_BAR_HEIGHT)).children(tabs).child(add))
            .children(self.tabs.get(self.active).map(|tab| div().flex_1().min_h_0().child(tab.view.clone())))
    }
}

/// The keys for the window's own commands. Ctrl-Shift on Windows and Linux,
/// where Ctrl alone belongs to the program in the terminal; Cmd on macOS.
fn key_bindings() -> Vec<KeyBinding> {
    #[cfg(target_os = "macos")]
    let keys = [("cmd-t", 0), ("cmd-w", 1), ("cmd-c", 4), ("cmd-v", 5)];
    #[cfg(not(target_os = "macos"))]
    let keys = [("ctrl-shift-t", 0), ("ctrl-shift-w", 1), ("ctrl-shift-c", 4), ("ctrl-shift-v", 5), ("shift-insert", 5)];
    let common = [("ctrl-tab", 2), ("ctrl-shift-tab", 3)];
    keys.into_iter()
        .chain(common)
        .map(|(key, action)| match action {
            0 => KeyBinding::new(key, NewTab, None),
            1 => KeyBinding::new(key, CloseTab, None),
            2 => KeyBinding::new(key, NextTab, None),
            3 => KeyBinding::new(key, PreviousTab, None),
            4 => KeyBinding::new(key, Copy, None),
            _ => KeyBinding::new(key, Paste, None),
        })
        .collect()
}

/// Lets a release build print to the console it was started from. It is a
/// window program there (so that no console opens with it), which starts
/// with no console of its own: without this, `--help` in a terminal would
/// print nothing. Output that is redirected to a file or a pipe is not
/// affected.
#[cfg(all(windows, not(debug_assertions)))]
fn use_parent_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    /// The process that started this one.
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: AttachConsole takes a process id by value and has no other
    // preconditions; when there is no parent console it fails and changes nothing.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn use_parent_console() {}

/// Prints to stdout. A reader that has gone away (a closed pipe) is not an
/// error worth a panic, which is what `print!` makes of it: the text is
/// dropped.
fn say(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Prints to stderr, with the same care as [`say`].
fn complain(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr().lock(), "{APP_NAME}: {text}");
}

fn main() {
    use_parent_console();
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(Early::Print(text)) => {
            say(&format!("{text}\n"));
            return;
        }
        Err(Early::Usage(problem)) => {
            complain(&format!("{problem}\n{USAGE}"));
            std::process::exit(2);
        }
    };
    if let Some(file) = &args.replay {
        match replay(file, args.size.unwrap_or(REPLAY_SIZE)) {
            Ok(text) => say(&text),
            Err(error) => {
                complain(&format!("{error:#}"));
                std::process::exit(1);
            }
        }
        return;
    }
    // A settings file with a mistake does not keep the terminal from
    // opening: it opens with the defaults and says what is wrong.
    let path = args.config.or_else(Config::default_path);
    let (config, notice) = match path.as_deref().map(Config::load) {
        Some(Ok(config)) => (config, None),
        Some(Err(error)) => (Config::default(), Some(format!("{APP_NAME}: settings not used: {error:#}"))),
        None => (Config::default(), None),
    };
    let theme = config.theme().unwrap_or_default();
    let settings = Arc::new(Settings { config, theme });
    let command = args.command;

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings());
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(APP_NAME.into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(settings, command, notice, window, cx)))
            .expect("open the window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, Early> {
        parse_args(list.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn no_arguments_is_the_shell_and_the_default_settings() {
        assert_eq!(args(&[]), Ok(Args::default()));
    }

    #[test]
    fn everything_after_e_is_the_programs() {
        let parsed = args(&["--config", "c.toml", "-e", "nvim", "--config", "-e", "x"]).unwrap();
        assert_eq!(parsed.config, Some("c.toml".into()));
        assert_eq!(parsed.command, Some(("nvim".into(), vec!["--config".into(), "-e".into(), "x".into()])));
    }

    #[test]
    fn mistakes_and_questions_do_not_open_a_window() {
        assert_eq!(args(&["--config"]), Err(Early::Usage("--config needs a file".into())));
        assert_eq!(args(&["-e"]), Err(Early::Usage("-e needs a program".into())));
        assert_eq!(args(&["--nope"]), Err(Early::Usage("unknown argument: --nope".into())));
        assert_eq!(args(&["--help"]), Err(Early::Print(USAGE.into())));
        assert!(matches!(args(&["-V"]), Err(Early::Print(text)) if text.starts_with("uni-term ")));
    }

    #[test]
    fn a_replay_prints_the_screen_the_bytes_draw() {
        assert_eq!(parse_size("100x30"), Some((100, 30)));
        assert_eq!(parse_size("0x30"), None);
        assert_eq!(parse_size("100"), None);
        assert_eq!(parse_size("ax3"), None);
        let parsed = args(&["--replay", "out.bin", "--size", "20x3"]).unwrap();
        assert_eq!((parsed.replay, parsed.size), (Some("out.bin".into()), Some((20, 3))));
        assert!(matches!(args(&["--size", "big"]), Err(Early::Usage(text)) if text.contains("big")));

        let file = std::env::temp_dir().join(format!("uni-term-replay-test-{}.bin", std::process::id()));
        std::fs::write(&file, "one\r\n\x1b[31mtwo\x1b[0m  \r\n\x1b[1;1HONE").unwrap();
        assert_eq!(replay(&file, (20, 3)).unwrap(), "ONE\ntwo\n\n");
        std::fs::remove_file(&file).unwrap();
        assert!(replay(&file, (20, 3)).is_err(), "the file is gone");
    }

    #[test]
    fn a_tab_is_named_after_its_program() {
        assert_eq!(tab_name(r"C:\Program Files\PowerShell\7\pwsh.exe"), "pwsh");
        assert_eq!(tab_name("/bin/zsh"), "zsh");
        assert_eq!(tab_name("nvim"), "nvim");
    }

    #[test]
    fn a_long_title_is_cut() {
        assert_eq!(tab_title("short"), "short");
        let cut = tab_title(&"长".repeat(40));
        assert_eq!(cut.chars().count(), TAB_TITLE_CHARS);
        assert!(cut.ends_with('…'));
    }
}
