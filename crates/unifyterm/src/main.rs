//! UnifyTerm: one terminal for Windows, macOS and Linux. The window and its
//! tabs are here; a tab's view is in `terminal`, its screen and pty in
//! `session`.

// A release build on Windows is a window program: no console opens with it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agents;
mod boxdraw;
mod candidates;
mod config;
mod frame;
mod graphics;
mod helper;
mod keys;
mod logging;
mod marks;
mod prompt;
mod recorder;
mod session;
mod shell;
mod terminal;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Application, Bounds, ClipboardItem, Context, Entity, KeyBinding, MouseButton, SharedString, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowOptions, actions, div, point, prelude::*, px, rgb,
    size,
};

use config::Config;
use shell::{Commands, Hooks};
use terminal::{Settings, TerminalEvent, TerminalView};

actions!(unifyterm, [NewTab, CloseTab, NextTab, PreviousTab, Copy, Paste, ToggleAgents]);

const APP_NAME: &str = "UnifyTerm";
const WINDOW_SIZE: (f32, f32) = (960., 620.);
const TAB_BAR_HEIGHT: f32 = 34.;
/// Room at the left of the tabs for macOS's three window buttons, and where they sit.
const TRAFFIC_LIGHTS_WIDTH: f32 = 76.;
const TRAFFIC_LIGHTS_AT: (f32, f32) = (13., 11.);
/// One of the window buttons UnifyTerm draws on Windows (minimize, maximize, close).
const WINDOW_BUTTON_WIDTH: f32 = 46.;
/// The face Windows keeps its window buttons' glyphs in (10 and 11 both have it).
const WINDOW_GLYPHS: &str = "Segoe MDL2 Assets";
/// The close button under the pointer: Windows' own red, with white on it.
const CLOSE_RED: u32 = 0x00c4_2b1c;
const CLOSE_WHITE: u32 = 0x00ff_ffff;
/// The agents' sidebar, in pixels.
const SIDEBAR_WIDTH: f32 = 300.;
/// The longest title a tab shows, in characters.
const TAB_TITLE_CHARS: usize = 28;
const USAGE: &str = "usage: unifyterm [--config FILE] [--cwd DIRECTORY] [-e PROGRAM [ARGS...]]
       unifyterm --replay FILE [--size COLUMNSxLINES]

  --config FILE        read settings from FILE, not the default config.toml
  --cwd DIRECTORY      start the tabs in DIRECTORY, not in your home directory
  -e PROGRAM [ARGS...] run PROGRAM in the first tab, not the shell
  --replay FILE        print the screen that FILE's bytes draw, and leave:
                       FILE is what a program printed, as UNIFYTERM_RAW_LOG saves it
  --size CxL           the replay's screen size (default 100x30)
  -V, --version        print the version
  -h, --help           print this text

Environment:
  UNIFYTERM_DATA_DIR    keep UnifyTerm's data (command links, start scripts, history,
                       log) in this directory instead of the system's one
  UNIFYTERM_RAW_LOG     a file to append every byte the first tab's program prints to;
                       later tabs write to the name with .2, .3 added";

/// The replay's screen size when `--size` is not given.
const REPLAY_SIZE: (u16, u16) = (100, 30);
/// History kept by a replay's screen: none, it prints the last screen only.
const REPLAY_HISTORY: usize = 0;

/// `unifyterm 0.1.0 (c5a6513)`: the version and the commit it was built from.
fn version() -> String {
    format!("{APP_NAME} {} ({})", env!("CARGO_PKG_VERSION"), env!("UNIFYTERM_COMMIT"))
}

/// What the command line asked for.
#[derive(Debug, PartialEq, Default)]
struct Args {
    config: Option<PathBuf>,
    /// The first tab's program and its arguments.
    command: Option<(String, Vec<String>)>,
    /// The directory the tabs start in, when one was asked for.
    cwd: Option<PathBuf>,
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
            "-V" | "--version" => return Err(Early::Print(version())),
            "--config" => {
                let file = args.next().ok_or_else(|| Early::Usage("--config needs a file".into()))?;
                parsed.config = Some(file.into());
            }
            "--cwd" => {
                let directory = args.next().ok_or_else(|| Early::Usage("--cwd needs a directory".into()))?;
                parsed.cwd = Some(directory.into());
            }
            "--replay" => {
                let file = args.next().ok_or_else(|| Early::Usage("--replay needs a file".into()))?;
                parsed.replay = Some(file.into());
            }
            "--size" => {
                let size = args.next().ok_or_else(|| Early::Usage("--size needs a size like 100x30".into()))?;
                parsed.size = Some(
                    parse_size(&size).ok_or_else(|| Early::Usage(format!("--size {size}: not a size like 100x30")))?,
                );
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

/// The directory the tabs' programs start in: the one `--cwd` names, else
/// the home directory.
///
/// A terminal is where work starts. It is opened from the Dock, the Start
/// menu or a launcher far more often than from another shell, and the
/// directory such a start leaves it in (`/` from the Finder, its own folder
/// from a Windows shortcut) is nobody's choice: so that directory is not
/// used, on any system. The current directory is only the last resort, for
/// an account with no home directory.
fn tab_directory(asked: Option<&std::path::Path>, home: Option<PathBuf>) -> Result<PathBuf, String> {
    let Some(asked) = asked else {
        return home
            .filter(|home| home.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "no home directory and no current directory to start in".to_owned());
    };
    let directory = std::path::absolute(asked).map_err(|error| format!("--cwd {}: {error}", asked.display()))?;
    if directory.is_dir() { Ok(directory) } else { Err(format!("--cwd {}: not a directory", asked.display())) }
}

/// What is above the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleBar {
    /// The row of tabs is the title bar: the window is moved by it, and on
    /// Windows it holds the window's three buttons (macOS keeps its own).
    Tabs,
    /// The system's title bar, with the row of tabs under it.
    System,
}

impl TitleBar {
    /// The tabs, where GPUI can take the system's bar away (Windows, macOS)
    /// and the settings do not ask for it. Elsewhere the window manager
    /// draws the bar and the borders, and a window without them could not
    /// be moved or sized.
    const fn choose(config: &config::WindowConfig) -> Self {
        if cfg!(any(target_os = "windows", target_os = "macos")) && !config.system_title_bar {
            Self::Tabs
        } else {
            Self::System
        }
    }
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
    /// UnifyTerm's commands, when the settings have them on and they could be made ready.
    commands: Option<Commands>,
    /// The history file's keeper, when the settings have the history on and the file opened.
    recorder: Option<recorder::Recorder>,
    /// The shells' hooks: with the history on.
    hooks: Option<Hooks>,
    /// Where each tab's program starts (see [`tab_directory`]).
    directory: PathBuf,
    title_bar: TitleBar,
    /// Help with commands: the local fix always, a model's help when the settings name one.
    helper: Arc<helper::Helper>,
    sidebar: Sidebar,
}

/// The agents' sidebar.
#[derive(Default)]
struct Sidebar {
    shown: bool,
    agents: Option<agents::Agents>,
    /// Counts the times it was shown: a poll started for an earlier time stops.
    shown_times: u64,
    quiet: agents::Quiet,
    /// What the last click did.
    note: Option<String>,
}

/// What the window starts with.
struct Start {
    settings: Arc<Settings>,
    commands: Option<Commands>,
    recorder: Option<recorder::Recorder>,
    hooks: Option<Hooks>,
    directory: PathBuf,
    helper: Arc<helper::Helper>,
    /// The first tab's program and its arguments; the shell when `None`.
    command: Option<(String, Vec<String>)>,
    /// Lines to show at the top of the first tab: what is wrong with the settings.
    notices: Vec<String>,
}

impl Workspace {
    /// A window with one tab.
    fn new(start: Start, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let Start { settings, commands, recorder, hooks, directory, helper, command, notices } = start;
        let sidebar = Sidebar::default();
        let title_bar = TitleBar::choose(&settings.config.window);
        let mut workspace = Self {
            tabs: Vec::new(),
            active: 0,
            settings,
            commands,
            recorder,
            hooks,
            directory,
            title_bar,
            helper,
            sidebar,
        };
        workspace.open(command, window, cx);
        if workspace.settings.config.agents.sidebar {
            workspace.toggle_agents(cx);
        }
        if let (Some(tab), false) = (workspace.tabs.first(), notices.is_empty()) {
            let text: Vec<String> = notices.iter().map(|notice| format!("{APP_NAME}: {notice}")).collect();
            tab.view.update(cx, |view, _| view.announce(&text.join("\n")));
        }
        workspace
    }

    /// Opens a tab running `command`, or the configured shell.
    ///
    /// A configured shell that does not start is replaced by the default
    /// one, with a line in the tab saying so: a mistake in the settings
    /// must not leave the user without a terminal. A `command` that does
    /// not start is reported; with no tab open, UnifyTerm then ends in failure.
    fn open(&mut self, command: Option<(String, Vec<String>)>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let shell = &self.settings.config.shell;
        let asked = command.is_some();
        let (program, args) =
            command.unwrap_or_else(|| (shell.program.clone().unwrap_or_default(), shell.args.clone()));
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
                log::error!("{error:#}");
                if self.tabs.is_empty() {
                    std::process::exit(1);
                }
                return;
            }
        };
        let settings = Arc::clone(&self.settings);
        let recorder = self.recorder.clone();
        let helper = Arc::clone(&self.helper);
        let view = cx.new(|cx| TerminalView::new(session, events, settings, recorder, helper, cx));
        if let Some(notice) = notice {
            view.update(cx, |view, _| view.announce(&notice));
        }
        let events = cx.subscribe_in(&view, window, Self::terminal_event);
        self.tabs.push(Tab { view, name, _events: events });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    /// Starts `program` (the default shell when `None`) in a session; the
    /// session, and the tab's name.
    fn start(
        &self,
        program: Option<&str>,
        args: &[String],
    ) -> anyhow::Result<((session::Session, smol::channel::Receiver<session::PtyEvent>), String)> {
        let mut command =
            shell::command(program, args, self.commands.as_ref(), self.hooks.as_ref(), shell::ShellStart::here())?;
        command.cwd(&self.directory);
        let name = command.get_argv().first().map(|program| tab_name(&program.to_string_lossy())).unwrap_or_default();
        Ok((TerminalView::start(command, &self.settings)?, name))
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        self.active = index;
        window.focus(&tab.view.read(cx).focus);
        window.set_window_title(&self.title(index, cx));
        cx.notify();
    }

    /// Closes a tab; with the last one, the window and the app.
    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
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
            Some(title) if names_a_program(title) => tab_name(title),
            Some(title) => title.to_owned(),
            None => tab.name.clone(),
        }
    }

    fn terminal_event(
        &mut self,
        view: &Entity<TerminalView>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| &tab.view == view) else { return };
        match event {
            TerminalEvent::Exited => self.close(index, window, cx),
            TerminalEvent::Finished(finished) => {
                if let Some(recorder) = &self.recorder {
                    recorder.record(finished, &self.tabs[index].name);
                }
            }
            TerminalEvent::Title => {
                if index == self.active {
                    window.set_window_title(&self.title(index, cx));
                }
                cx.notify();
            }
        }
    }

    /// Shows or hides the agents' sidebar. While it shows, the agents are
    /// asked for every `agents.poll_ms`; while it is hidden, nothing runs.
    fn toggle_agents(&mut self, cx: &mut Context<'_, Self>) {
        let sidebar = &mut self.sidebar;
        sidebar.shown = !sidebar.shown;
        cx.notify();
        if !sidebar.shown {
            return;
        }
        sidebar.shown_times += 1;
        sidebar.note = None;
        // What it showed before may be old by now: "asking" until the first answer.
        sidebar.agents = None;
        let time = sidebar.shown_times;
        let command = self.settings.config.agents.command.clone();
        let every = Duration::from_millis(self.settings.config.agents.poll_ms);
        cx.spawn(async move |this, cx| {
            let wanted = |this: &Workspace| this.sidebar.shown && this.sidebar.shown_times == time;
            while this.read_with(cx, |this, _| wanted(this)).unwrap_or(false) {
                let command = command.clone();
                let found = cx.background_spawn(async move { agents::ask(&command) }).await;
                let kept = this.update(cx, |this, cx| {
                    if !wanted(this) {
                        return false;
                    }
                    this.sidebar.quiet.log(&found);
                    this.sidebar.agents = Some(found);
                    cx.notify();
                    true
                });
                if !kept.unwrap_or(false) {
                    break;
                }
                cx.background_executor().timer(every).await;
            }
        })
        .detach();
    }

    /// Brings an agent's pane to the front (`agents.focus`); when that fails,
    /// its place goes to the clipboard instead.
    fn go_to(&mut self, agent: &agents::Agent, cx: &mut Context<'_, Self>) {
        let command = agents::focus_command(&self.settings.config.agents.focus, &agent.pane);
        let (pane, place) = (agent.pane.clone(), agent.target.clone());
        cx.spawn(async move |this, cx| {
            let done = cx.background_spawn(async move { agents::focus(&command) }).await;
            let _ = this.update(cx, |this, cx| {
                this.sidebar.note = Some(match done {
                    Ok(()) => format!("Went to {pane}."),
                    Err(error) => {
                        log::warn!("agents: going to {pane}: {error}");
                        let copied = if place.is_empty() { pane } else { place };
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                        format!("Could not go there ({error}); {copied} is copied.")
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The sidebar: a card for each agent, or a line saying why there are none.
    fn sidebar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = &self.settings.theme;
        let (background, foreground) = (theme.background, theme.foreground);
        let dim = foreground.toward(background, 0.45);
        let card_color = background.toward(foreground, 0.06);
        let listed = match &self.sidebar.agents {
            Some(agents::Agents::Listed(agents)) => agents.clone(),
            _ => Vec::new(),
        };
        let notice =
            self.sidebar.agents.as_ref().map_or_else(|| agents::Agents::Asking.notice(), agents::Agents::notice);
        let count = if listed.is_empty() { String::new() } else { format!(" ({})", listed.len()) };
        let cards = listed.into_iter().enumerate().map(|(index, agent)| {
            let (title, place, figures) = (agent.title(), agent.place(), agent.figures());
            div()
                .id(("agent", index))
                .p_2()
                .rounded_md()
                .bg(rgb(card_color.hex()))
                .hover(|card| card.bg(rgb(background.toward(foreground, 0.12).hex())))
                .cursor_pointer()
                .flex()
                .flex_col()
                .gap_1()
                .on_click(cx.listener(move |this, _, _, cx| this.go_to(&agent, cx)))
                .child(div().text_color(rgb(foreground.hex())).child(SharedString::from(title)))
                .child(div().text_color(rgb(dim.hex())).child(SharedString::from(place)))
                .child(div().text_color(rgb(dim.hex())).text_size(px(12.)).child(SharedString::from(figures)))
        });
        div()
            .id("agents")
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .bg(rgb(background.toward(theme::Rgb(0, 0, 0), 0.2).hex()))
            .text_color(rgb(foreground.hex()))
            .child(div().child(SharedString::from(format!("Agents{count}"))))
            .children(notice.map(|notice| div().text_color(rgb(dim.hex())).child(SharedString::from(notice))))
            .children(cards)
            .children(
                self.sidebar.note.clone().map(|note| div().text_color(rgb(dim.hex())).child(SharedString::from(note))),
            )
    }

    /// Goes to the tab beside the active one, around the ends.
    fn step(&mut self, step: Step, window: &mut Window, cx: &mut Context<'_, Self>) {
        let count = self.tabs.len();
        let next = match step {
            Step::Next => (self.active + 1) % count,
            Step::Previous => (self.active + count - 1) % count,
        };
        self.activate(next, window, cx);
    }
}

/// Which neighbor of the active tab to go to.
#[derive(Clone, Copy, Debug)]
enum Step {
    Next,
    Previous,
}

/// What follows the last dot of a Windows program's file name, in any case.
const PROGRAM_EXTENSION: &str = "exe";

/// A file name without `.exe` (in any case of letters), if it ends in that.
fn without_program_extension(file: &str) -> Option<&str> {
    file.rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case(PROGRAM_EXTENSION))
        .map(|(stem, _)| stem)
}

/// Whether a title is a program's file name or path (`...\nvim.EXE`).
fn names_a_program(title: &str) -> bool {
    without_program_extension(title).is_some()
}

/// A program's path as a tab's name: the file's name without `.exe`.
fn tab_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    without_program_extension(file).unwrap_or(file).to_owned()
}

/// A title cut to what a tab has room for.
fn tab_title(title: &str) -> String {
    if title.chars().count() <= TAB_TITLE_CHARS {
        return title.to_owned();
    }
    let cut: String = title.chars().take(TAB_TITLE_CHARS - 1).collect();
    format!("{cut}…")
}

/// The empty part of the row of tabs. When the tabs are the title bar it
/// moves the window, as a title bar does: Windows takes it for one (drag,
/// double click, right click), macOS is told to start the move.
fn window_handle(title_bar: TitleBar) -> gpui::Stateful<gpui::Div> {
    div().id("title-bar").flex_1().h_full().when(title_bar == TitleBar::Tabs, |handle| {
        handle
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
    })
}

/// The window's three buttons on Windows, where UnifyTerm draws them when
/// the tabs are the title bar. Windows itself acts on a click: each button
/// only says which one it is.
fn window_buttons(window: &Window, theme: &theme::Theme) -> gpui::Div {
    let (background, foreground) = (theme.background, theme.foreground);
    let dim = foreground.toward(background, 0.5);
    let lit = foreground.toward(background, 0.85);
    let button = |name: &'static str, glyph: &'static str, area: WindowControlArea| {
        div()
            .id(name)
            .h_full()
            .w(px(WINDOW_BUTTON_WIDTH))
            .flex()
            .items_center()
            .justify_center()
            .font_family(WINDOW_GLYPHS)
            .text_size(px(10.))
            .text_color(rgb(dim.hex()))
            .window_control_area(area)
            .child(glyph)
    };
    // Windows' own glyphs: minimize, maximize or restore, close.
    let middle = if window.is_maximized() { "\u{e923}" } else { "\u{e922}" };
    let quiet = move |button: gpui::StyleRefinement| button.bg(rgb(lit.hex())).text_color(rgb(foreground.hex()));
    div()
        .flex()
        .flex_none()
        .h_full()
        .child(button("minimize", "\u{e921}", WindowControlArea::Min).hover(quiet))
        .child(button("maximize", middle, WindowControlArea::Max).hover(quiet))
        .child(
            button("close", "\u{e8bb}", WindowControlArea::Close)
                .hover(|button| button.bg(rgb(CLOSE_RED)).text_color(rgb(CLOSE_WHITE))),
        )
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
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
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.close(index, window, cx)),
                )
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
        let own_bar = self.title_bar == TitleBar::Tabs;
        let handle = window_handle(self.title_bar);
        let buttons = (own_bar && cfg!(target_os = "windows")).then(|| window_buttons(window, theme));
        let lights = (own_bar && cfg!(target_os = "macos")).then(|| div().flex_none().w(px(TRAFFIC_LIGHTS_WIDTH)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(bar.hex()))
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.open(None, window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(Step::Next, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.step(Step::Previous, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleAgents, _, cx| this.toggle_agents(cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .h(px(TAB_BAR_HEIGHT))
                    .children(lights)
                    .children(tabs)
                    .child(add)
                    .child(handle)
                    .children(buttons),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(
                        self.tabs.get(self.active).map(|tab| div().flex_1().min_w_0().h_full().child(tab.view.clone())),
                    )
                    .when(self.sidebar.shown, |area| area.child(self.sidebar(cx))),
            )
    }
}

/// The keys for the window's own commands. Ctrl-Shift on Windows and Linux,
/// where Ctrl alone belongs to the program in the terminal; Cmd on macOS.
fn key_bindings() -> Vec<KeyBinding> {
    #[cfg(target_os = "macos")]
    let keys = [("cmd-t", 0), ("cmd-w", 1), ("cmd-c", 4), ("cmd-v", 5), ("cmd-shift-a", 6)];
    #[cfg(not(target_os = "macos"))]
    let keys = [
        ("ctrl-shift-t", 0),
        ("ctrl-shift-w", 1),
        ("ctrl-shift-c", 4),
        ("ctrl-shift-v", 5),
        ("shift-insert", 5),
        ("ctrl-shift-a", 6),
    ];
    let common = [("ctrl-tab", 2), ("ctrl-shift-tab", 3)];
    keys.into_iter()
        .chain(common)
        .map(|(key, action)| match action {
            0 => KeyBinding::new(key, NewTab, None),
            1 => KeyBinding::new(key, CloseTab, None),
            2 => KeyBinding::new(key, NextTab, None),
            3 => KeyBinding::new(key, PreviousTab, None),
            4 => KeyBinding::new(key, Copy, None),
            5 => KeyBinding::new(key, Paste, None),
            _ => KeyBinding::new(key, ToggleAgents, None),
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
    let directory = match tab_directory(args.cwd.as_deref(), dirs::home_dir()) {
        Ok(directory) => directory,
        Err(problem) => {
            complain(&format!("{problem}\n{USAGE}"));
            std::process::exit(2);
        }
    };
    logging::init();
    // A settings file with a mistake does not keep the terminal from
    // opening: it opens with the defaults and says what is wrong.
    let path = args.config.or_else(Config::default_path);
    let (config, mut notices) = match path.as_deref().map(Config::load) {
        Some(Err(error)) => (Config::default(), vec![format!("settings not used: {error:#}")]),
        Some(Ok(config)) => (config, Vec::new()),
        None => (Config::default(), Vec::new()),
    };
    let theme = config.theme().unwrap_or_default();
    let (commands, told) = Commands::prepare(&config.commands);
    notices.extend(told);
    // The history and the hooks that feed it go together: one without the other is no use.
    let (recorder, hooks) = if config.history.enabled {
        let opened = unifyterm_history::default_path().and_then(|path| recorder::Recorder::start(&path));
        match opened.and_then(|recorder| Ok((recorder, Hooks::prepare()?))) {
            Ok((recorder, hooks)) => (Some(recorder), Some(hooks)),
            Err(error) => {
                notices.push(format!("the command history is not in use: {error:#}"));
                (None, None)
            }
        }
    } else {
        (None, None)
    };
    let ut = commands.as_ref().and_then(|commands| Some((Commands::program().ok()?, commands.names().to_vec())));
    let (helper, told) = helper::Helper::new(&config.ai, ut, commands.as_ref().map(Commands::directory));
    notices.extend(told);
    let settings = Arc::new(Settings { config, theme });
    let helper = Arc::new(helper);
    let start = Start { settings, commands, recorder, hooks, directory, helper, command: args.command, notices };

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings());
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let own_bar = TitleBar::choose(&start.settings.config.window) == TitleBar::Tabs;
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                appears_transparent: own_bar,
                traffic_light_position: own_bar.then(|| point(px(TRAFFIC_LIGHTS_AT.0), px(TRAFFIC_LIGHTS_AT.1))),
            }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(start, window, cx))).expect("open the window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_names_the_commit() {
        let line = version();
        let commit = line
            .strip_prefix(&format!("{APP_NAME} {} (", env!("CARGO_PKG_VERSION")))
            .and_then(|rest| rest.strip_suffix(')'))
            .expect(&line);
        assert!(commit == "unknown" || (commit.len() == 7 && commit.chars().all(|c| c.is_ascii_hexdigit())), "{line}");
    }
    fn args(list: &[&str]) -> Result<Args, Early> {
        parse_args(list.iter().map(ToString::to_string))
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
    fn the_tabs_start_at_home_unless_a_directory_is_asked_for() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let at_home = Some(home.path().to_path_buf());
        // Whatever directory the program itself was started in (the tests': the crate's) is not used.
        assert_eq!(tab_directory(None, at_home.clone()), Ok(home.path().to_path_buf()));
        assert_ne!(tab_directory(None, at_home.clone()).ok(), std::env::current_dir().ok());
        assert_eq!(tab_directory(Some(other.path()), at_home.clone()), Ok(other.path().to_path_buf()));
        let relative = tab_directory(Some(std::path::Path::new(".")), at_home.clone()).unwrap();
        assert!(relative.is_absolute(), "a relative one is taken from where the command was typed: {relative:?}");
        let missing = other.path().join("not-there");
        let refused = tab_directory(Some(&missing), at_home).unwrap_err();
        assert!(refused.starts_with("--cwd ") && refused.ends_with(": not a directory"), "{refused}");
        // An account whose home directory is gone still gets a tab: where the program was started.
        assert_eq!(tab_directory(None, Some(missing)).ok(), std::env::current_dir().ok());
        assert_eq!(tab_directory(None, None).ok(), std::env::current_dir().ok());

        assert_eq!(args(&["--cwd", "work", "-e", "sh"]).unwrap().cwd, Some("work".into()));
        assert_eq!(args(&["--cwd"]), Err(Early::Usage("--cwd needs a directory".into())));
        assert_eq!(args(&["-e", "sh", "--cwd", "x"]).unwrap().cwd, None, "after -e it is the program's");
    }

    #[test]
    fn mistakes_and_questions_do_not_open_a_window() {
        assert_eq!(args(&["--config"]), Err(Early::Usage("--config needs a file".into())));
        assert_eq!(args(&["-e"]), Err(Early::Usage("-e needs a program".into())));
        assert_eq!(args(&["--nope"]), Err(Early::Usage("unknown argument: --nope".into())));
        assert_eq!(args(&["--help"]), Err(Early::Print(USAGE.into())));
        assert!(matches!(args(&["-V"]), Err(Early::Print(text)) if text.starts_with("UnifyTerm ")));
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

        let file = std::env::temp_dir().join(format!("unifyterm-replay-test-{}.bin", std::process::id()));
        std::fs::write(&file, "one\r\n\x1b[31mtwo\x1b[0m  \r\n\x1b[1;1HONE").unwrap();
        assert_eq!(replay(&file, (20, 3)).unwrap(), "ONE\ntwo\n\n");
        std::fs::remove_file(&file).unwrap();
        assert!(replay(&file, (20, 3)).is_err(), "the file is gone");
    }

    #[test]
    fn a_tab_is_named_after_its_program() {
        assert_eq!(tab_name(r"C:\Program Files\PowerShell\7\pwsh.exe"), "pwsh");
        assert_eq!(tab_name("/bin/zsh"), "zsh");
        assert_eq!(tab_name(r"C:\Program Files\Neovim\bin\nvim.EXE"), "nvim", "whatever the letters' case");
        assert_eq!(tab_name("notes.exe.txt"), "notes.exe.txt", "only a last .exe is the program's");
        assert!(names_a_program(r"C:\Windows\System32\cmd.exe") && names_a_program("nvim.EXE"));
        assert!(!names_a_program("vim README.md") && !names_a_program("pwsh"));
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
