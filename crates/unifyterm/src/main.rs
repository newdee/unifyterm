//! UnifyTerm: one terminal for Windows, macOS and Linux. The window and its
//! tabs are here; a tab's view is in `terminal`, its screen and pty in
//! `session`.

// A release build on Windows is a window program: no console opens with it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agents;
mod art;
mod boxdraw;
mod candidates;
mod config;
#[cfg(windows)]
mod console_focus;
mod corners;
mod frame;
mod graphics;
mod helper;
mod keys;
mod lights;
mod logging;
mod marks;
mod prompt;
mod recorder;
mod session;
mod shell;
mod terminal;
mod theme;
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Application, Bounds, ClipboardItem, Context, Entity, FontWeight, Hsla, KeyBinding, MouseButton,
    MouseMoveEvent, Pixels, Point, SharedString, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowOptions, actions, div, point, prelude::*, px, rgb, size,
};

use config::Config;
use lights::Lights;
use shell::{Commands, Hooks};
use terminal::{Backdrop, Settings, TerminalEvent, TerminalView};

actions!(
    unifyterm,
    [NewTab, CloseTab, NextTab, PreviousTab, Copy, Paste, ToggleAgents, ToggleFullScreen, ToggleTabBar, Explain]
);

const APP_NAME: &str = "UnifyTerm";
const WINDOW_SIZE: (f32, f32) = (960., 620.);
/// The row of tabs, and a tab in it: as the preview page (demo/appearance.html)
/// draws them, but for the row's height, which is 54 there. The row is the
/// window's title bar too, and that much would be taken from the text.
const TAB_BAR_HEIGHT: f32 = 46.;
const TAB_HEIGHT: f32 = 36.;
const TAB_LEAST_WIDTH: f32 = 112.;
/// The short line under the tab in use: how wide, how thick.
const TAB_MARK: (f32, f32) = (30., 2.);
/// The sign before a tab's name.
const TAB_SIGN: &str = "›_";
/// A tab's close button and the new-tab button: each one's width and height.
const TAB_CLOSE: (f32, f32) = (25., 26.);
const TAB_ADD: (f32, f32) = (30., 32.);
/// With `window.tab_bar = "auto-hide"`: the row comes when the pointer has
/// been this near the window's top edge for this long. Windows keeps the top
/// 4 pixels of a window that is not maximized for sizing it, and tells the
/// window nothing of the pointer there: the edge is wider than that.
const BAR_EDGE: f32 = 10.;
const BAR_DWELL: Duration = Duration::from_millis(150);
/// It goes when the pointer is this far below it.
const BAR_LEAVE: f32 = 12.;
/// How long it shows after a tab was opened, closed or gone to.
const BAR_PEEK: Duration = Duration::from_millis(1500);
/// Room at the left of the tabs for macOS's three window buttons, and where they sit.
const TRAFFIC_LIGHTS_WIDTH: f32 = 76.;
const TRAFFIC_LIGHTS_AT: (f32, f32) = (13., 17.);
/// One of the window buttons UnifyTerm draws on Windows (minimize, maximize, close).
const WINDOW_BUTTON_WIDTH: f32 = 46.;
/// The face Windows keeps its window buttons' glyphs in (10 and 11 both have it).
const WINDOW_GLYPHS: &str = "Segoe MDL2 Assets";
/// The close button under the pointer: Windows' own red, with white on it.
const CLOSE_RED: u32 = 0x00c4_2b1c;
const CLOSE_WHITE: u32 = 0x00ff_ffff;
/// The agents' sidebar, in pixels: its width, and its heading's height.
const SIDEBAR_WIDTH: f32 = 268.;
const SIDEBAR_HEAD: f32 = 56.;
/// The theme's picture at the sidebar's foot: its height, and how far down
/// it the sidebar's own color fades out over it (as on the preview page).
const SIDEBAR_PICTURE: f32 = 180.;
const SIDEBAR_PICTURE_FADE: f32 = 0.54;
/// A window lower than this has no room for the picture beside the agents' cards.
const SIDEBAR_PICTURE_FROM: f32 = 480.;
/// How often the settings file is looked at for a change.
const SETTINGS_LOOK: Duration = Duration::from_millis(500);
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

/// The settings file the window reads.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SettingsFile {
    path: PathBuf,
    /// Named with `--config`: the shells are told, so that `ut setting` changes this file.
    asked: Asked,
}

/// Whether the settings file is the default one or one `--config` named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Default,
    WithConfig,
}

/// What tells one state of a file from the next: when it was written, and its size.
/// `None` for a file that is not there.
fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let about = std::fs::metadata(path).ok()?;
    Some((about.modified().ok()?, about.len()))
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
    /// The settings file, when `--config` named it: the tabs' shells are told (see [`Workspace::start`]).
    named_settings: Option<PathBuf>,
    /// The folder of the user's own themes: `themes` beside the settings file.
    themes: Option<PathBuf>,
    title_bar: TitleBar,
    /// Help with commands: the local fix always, a model's help when the settings name one.
    helper: Arc<helper::Helper>,
    sidebar: Sidebar,
    /// A newer UnifyTerm, named in the row of tabs (`updates.check`).
    newer: Option<update::Newer>,
    bar: Bar,
    /// What the system was last told of the window: that what is behind it shows through, or not.
    behind: Behind,
    /// Where the look's pictures are, and the pictures: the one at the foot
    /// of the sidebar, and the wallpaper behind the text.
    pictures_at: config::Pictures,
    picture: Option<Arc<gpui::Image>>,
    wallpaper: Option<Arc<gpui::Image>>,
    pictures: art::Shown,
    wallpapers: art::Shown,
    corners_told: CornersTold,
    /// Whether the system's three buttons show (macOS; see [`lights`]).
    lights: Lights,
}

/// What the system was last told of the window's corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CornersTold {
    /// Nothing yet: the window has just opened.
    NotYet,
    /// These (`None`: its own choice).
    These(Option<unifyterm_config::Corners>),
}

/// The row of tabs while it keeps out of the way (`window.tab_bar = "auto-hide"`).
#[derive(Default)]
struct Bar {
    /// It lies over the text now.
    shown: bool,
    /// The pointer came to the top edge, and the wait before the row comes has begun.
    coming: bool,
    /// Shown for a tab that changed: it stays at least until then.
    kept_until: Option<std::time::Instant>,
    /// Counts the times it was shown for a tab: a wait begun for an earlier time hides nothing.
    peeks: u64,
    /// What the key asked of this window (Ctrl-Shift-B): it goes over the
    /// settings' `window.tab_bar` until that setting is changed.
    asked: Option<config::TabBar>,
}

/// Where the pointer is, for a row of tabs that keeps out of the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pointer {
    /// In the window, within `BAR_EDGE` of its top: it calls the row.
    AtTheEdge,
    /// In the window, on the row or within `BAR_LEAVE` under it: it keeps the row.
    OnTheRow,
    /// Lower in the window, or not in the window at all: the row may go.
    Elsewhere,
}

impl Pointer {
    /// Where a pointer at `at` is in a window of `size`. A pointer outside
    /// the window is elsewhere, above the window too: the system still says
    /// where it is (a negative height), and that is not on the row.
    fn at(at: Point<Pixels>, size: gpui::Size<Pixels>) -> Self {
        let inside = at.x >= px(0.) && at.x < size.width && at.y >= px(0.) && at.y < size.height;
        if !inside {
            Self::Elsewhere
        } else if at.y <= px(BAR_EDGE) {
            Self::AtTheEdge
        } else if at.y <= px(TAB_BAR_HEIGHT + BAR_LEAVE) {
            Self::OnTheRow
        } else {
            Self::Elsewhere
        }
    }
}

/// Whether what is behind the window shows through it, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behind {
    Hidden,
    Seen,
    /// Seen as through frosted glass.
    Blurred,
}

impl Behind {
    /// What the look asks for: seen through with `theme.opacity` under 1,
    /// blurred with `theme.blur` too. With nothing seen through there is
    /// nothing to blur.
    fn of(theme: &theme::Theme) -> Self {
        match (theme.opacity < 1.0, theme.blur) {
            (false, _) => Self::Hidden,
            (true, false) => Self::Seen,
            (true, true) => Self::Blurred,
        }
    }

    /// What GPUI is asked for.
    const fn appearance(self) -> WindowBackgroundAppearance {
        match self {
            Self::Hidden => WindowBackgroundAppearance::Opaque,
            Self::Seen => WindowBackgroundAppearance::Transparent,
            Self::Blurred => WindowBackgroundAppearance::Blurred,
        }
    }
}

/// A color as GPUI takes it.
fn solid(color: theme::Rgb) -> Hsla {
    rgb(color.hex()).into()
}

/// A color that lets `1 - opacity` of what is behind the window through.
fn seen_through(color: theme::Rgb, opacity: f32) -> Hsla {
    Hsla { a: opacity, ..solid(color) }
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
    /// Where the look's pictures are, when it has them.
    pictures: config::Pictures,
    commands: Option<Commands>,
    recorder: Option<recorder::Recorder>,
    hooks: Option<Hooks>,
    directory: PathBuf,
    /// The settings file, to read again when it changes, and whether it was named with `--config`.
    file: Option<SettingsFile>,
    helper: Arc<helper::Helper>,
    /// The first tab's program and its arguments; the shell when `None`.
    command: Option<(String, Vec<String>)>,
    /// Lines to show at the top of the first tab: what is wrong with the settings.
    notices: Vec<String>,
}

impl Workspace {
    /// A window with one tab.
    fn new(start: Start, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let Start {
            settings,
            pictures: pictures_at,
            commands,
            recorder,
            hooks,
            directory,
            file,
            helper,
            command,
            notices,
        } = start;
        let sidebar = Sidebar::default();
        let title_bar = TitleBar::choose(&settings.config.window);
        let named_settings = file.as_ref().filter(|file| file.asked == Asked::WithConfig).map(|file| file.path.clone());
        let themes = file.as_ref().map(|file| unifyterm_config::themes::directory(&file.path));
        let theme_named = settings.config.theme.name.clone();
        // The window was opened with what the settings asked for then (see `main`).
        let behind = Behind::of(&settings.theme);
        let (mut pictures, mut wallpapers) = (art::Shown::default(), art::Shown::default());
        let shown = pictures.of(pictures_at.picture.as_ref());
        let wallpaper = wallpapers.of(pictures_at.wallpaper.as_ref());
        // With a wallpaper the window draws what is behind the text: the tabs are told before the first one opens.
        let settings = if wallpaper.is_some() {
            Arc::new(Settings {
                config: settings.config.clone(),
                theme: settings.theme.clone(),
                backdrop: Backdrop::Wallpaper,
            })
        } else {
            settings
        };
        let mut workspace = Self {
            tabs: Vec::new(),
            active: 0,
            settings,
            commands,
            recorder,
            hooks,
            directory,
            named_settings,
            themes,
            title_bar,
            helper,
            sidebar,
            newer: None,
            bar: Bar::default(),
            behind,
            pictures_at,
            picture: shown,
            wallpaper,
            pictures,
            wallpapers,
            corners_told: CornersTold::NotYet,
            lights: Lights::Shown,
        };
        if let Some(file) = file {
            Self::follow_settings(file.path, theme_named, cx);
        }
        workspace.open(command, window, cx);
        if workspace.settings.config.agents.sidebar {
            workspace.toggle_agents(cx);
        }
        if workspace.settings.config.updates.check {
            Self::look_for_update(cx);
        }
        #[cfg(windows)]
        Self::keep_keyboard_from_hidden_consoles(window, cx);
        if let (Some(tab), false) = (workspace.tabs.first(), notices.is_empty()) {
            let text: Vec<String> = notices.iter().map(|notice| format!("{APP_NAME}: {notice}")).collect();
            tab.view.update(cx, |view, _| view.announce(&text.join("\n")));
        }
        workspace
    }

    /// When a program in a tab brings its hidden console window to the front
    /// (keepane going to a pane does), the window takes the keyboard back (see `console_focus`).
    #[cfg(windows)]
    fn keep_keyboard_from_hidden_consoles(window: &mut Window, cx: &mut Context<'_, Self>) {
        /// The window in front may change a moment after this one loses the keyboard: it is looked at again then.
        const LOOK_AGAIN: Duration = Duration::from_millis(100);
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                return;
            }
            cx.spawn_in(window, async move |_, cx| {
                for wait in [Duration::ZERO, LOOK_AGAIN] {
                    cx.background_executor().timer(wait).await;
                    if console_focus::own_hidden_console_in_front() {
                        let _ = cx.update(|window, _| window.activate_window());
                        break;
                    }
                }
            })
            .detach();
        })
        .detach();
    }

    /// Asks in the background whether a newer UnifyTerm is out; a newer one is named in the row of tabs.
    fn look_for_update(cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async {
                    let data = unifyterm_history::data_directory()?;
                    let program = std::env::current_exe()?;
                    let now = std::time::SystemTime::now();
                    update::check(&data, now, env!("CARGO_PKG_VERSION"), &program, update::ask_github)
                })
                .await;
            match found {
                Ok(newer) => {
                    let _ = this.update(cx, |this, cx| {
                        this.newer = newer;
                        cx.notify();
                    });
                }
                Err(error) => log::warn!("update: no word of a newer version: {error:#}"),
            }
        })
        .detach();
    }

    /// The notice of a newer version was clicked: the release page opens, and that version is not named again.
    fn open_update(&mut self, cx: &mut Context<'_, Self>) {
        let Some(newer) = self.newer.take() else { return };
        cx.open_url(update::RELEASE_PAGE);
        if let Err(error) = unifyterm_history::data_directory().and_then(|data| update::dismiss(&data, &newer.version))
        {
            log::warn!("update: {error:#}");
        }
        cx.notify();
    }

    /// Reads the settings file again whenever it changes (`ut setting`
    /// writes it; so does an editor), and gives every tab the new settings.
    ///
    /// The font, the colors and the padding change at once. The shell and
    /// its arguments are used by the next tab. What was set up when the
    /// window opened (UnifyTerm's commands, the history, the model, the
    /// title bar) stays until UnifyTerm starts again. A file that is wrong
    /// changes nothing: the settings in use stay, and the log says why.
    ///
    /// The file of the theme in use, when it is one of the user's own
    /// (`theme_named` is the theme as the window opens), is followed the
    /// same way: who writes a theme sees each change as it is saved.
    fn follow_settings(path: PathBuf, theme_named: Option<String>, cx: &mut Context<'_, Self>) {
        use unifyterm_config::themes;
        type Stamp = Option<(std::time::SystemTime, u64)>;
        cx.spawn(async move |this, cx| {
            let folder = themes::directory(&path);
            let file_of = |name: Option<&str>| name.map(|name| themes::file_of(name, &folder));
            let stamps = |theme: Option<&std::path::Path>| -> [Stamp; 2] { [stamp(&path), theme.and_then(stamp)] };
            let young = |stamps: &[Stamp; 2]| {
                stamps.iter().flatten().any(|(written, _)| unifyterm_programs::stamp_is_young(*written))
            };
            let mut theme = file_of(theme_named.as_deref());
            let mut seen = stamps(theme.as_deref());
            // Read while its time was young: a change of the same size in the same step of the
            // file system's clock would not show, so the file is read once more when it is older.
            let mut unsettled = young(&seen);
            loop {
                cx.background_executor().timer(SETTINGS_LOOK).await;
                let now = stamps(theme.as_deref());
                // Read now: a stamp changed, or the time that was young is young no longer.
                let read_now = now != seen || (unsettled && !young(&now));
                if !read_now {
                    continue;
                }
                let read = Config::load(&path);
                // The settings may name another theme now: its file is the one to follow. Its stamp
                // is taken before the window reads it, so a change after this look is seen by the next.
                if let Ok(config) = &read {
                    theme = file_of(config.theme.name.as_deref());
                }
                seen = [now[0], theme.as_deref().and_then(stamp)];
                unsettled = young(&seen);
                let alive = this.update(cx, |this, cx| match read {
                    Ok(config) => this.use_settings(config, cx),
                    Err(error) => log::warn!("settings: the changed file is not used: {error:#}"),
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Puts settings that were read again to use (see [`Workspace::follow_settings`]).
    ///
    /// A theme that is not there, or whose file is wrong, changes no color:
    /// the colors in use stay, and the log says why.
    fn use_settings(&mut self, config: Config, cx: &mut Context<'_, Self>) {
        let (theme, pictures_at) = config::theme(&config, self.themes.as_deref()).unwrap_or_else(|error| {
            log::warn!("theme: the colors in use stay: {error:#}");
            (self.settings.theme.clone(), self.pictures_at.clone())
        });
        let picture = self.pictures.of(pictures_at.picture.as_ref());
        let wallpaper = self.wallpapers.of(pictures_at.wallpaper.as_ref());
        let same = |new: &Option<Arc<gpui::Image>>, old: &Option<Arc<gpui::Image>>| match (new, old) {
            (Some(new), Some(old)) => Arc::ptr_eq(new, old),
            (None, None) => true,
            _ => false,
        };
        let same_pictures = same(&picture, &self.picture) && same(&wallpaper, &self.wallpaper);
        if config == self.settings.config && theme == self.settings.theme && same_pictures {
            return;
        }
        let backdrop = if wallpaper.is_some() { Backdrop::Wallpaper } else { Backdrop::Color };
        (self.pictures_at, self.picture, self.wallpaper) = (pictures_at, picture, wallpaper);
        // Other keys in the settings: they are the window's keys from now on.
        if config.keys != self.settings.config.keys {
            cx.clear_key_bindings();
            cx.bind_keys(key_bindings(&config.keys));
        }
        // The setting changed: it is what holds again, not what the key asked before.
        if config.window.tab_bar != self.settings.config.window.tab_bar {
            self.bar.asked = None;
        }
        self.settings = Arc::new(Settings { config, theme, backdrop });
        for tab in &self.tabs {
            let settings = Arc::clone(&self.settings);
            tab.view.update(cx, |view, cx| view.use_settings(settings, cx));
        }
        cx.notify();
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
        // `ut setting`, run in this tab, changes the file this window reads.
        match &self.named_settings {
            Some(file) => command.env(unifyterm_config::FILE_VARIABLE, file),
            None => command.env_remove(unifyterm_config::FILE_VARIABLE),
        }
        let name = command.get_argv().first().map(|program| tab_name(&program.to_string_lossy())).unwrap_or_default();
        Ok((TerminalView::start(command, &self.settings)?, name))
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(tab) = self.tabs.get(index) else { return };
        self.active = index;
        window.focus(&tab.view.read(cx).focus);
        window.set_window_title(&self.title(index, cx));
        self.show_bar_a_moment(window, cx);
        cx.notify();
    }

    /// Whether the row of tabs keeps out of the way now: the settings, or
    /// the key, ask for it. (On macOS the system's three buttons in the row
    /// go with it: see [`lights`].)
    fn bar_keeps_away(&self) -> bool {
        self.bar_mode() == config::TabBar::AutoHide
    }

    /// When the row of tabs shows, in this window: what its key last asked,
    /// else what the settings say.
    fn bar_mode(&self) -> config::TabBar {
        self.bar.asked.unwrap_or(self.settings.config.window.tab_bar)
    }

    /// The key for the row of tabs (Ctrl-Shift-B): a row that is always
    /// there keeps out of the way from now on, and one that keeps out of the
    /// way is always there. For this window, until the setting is changed.
    fn toggle_tab_bar(&mut self, cx: &mut Context<'_, Self>) {
        self.bar.asked = Some(match self.bar_mode() {
            config::TabBar::Always => config::TabBar::AutoHide,
            config::TabBar::AutoHide => config::TabBar::Always,
        });
        // Out of the way at once: the key said so, and no pointer is at the top edge.
        self.bar.shown = false;
        self.bar.kept_until = None;
        cx.notify();
    }

    /// A tab was opened, closed or gone to: a row that keeps out of the way
    /// shows which tab is the one in use now, for a moment.
    fn show_bar_a_moment(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.bar_keeps_away() {
            return;
        }
        self.bar.shown = true;
        self.bar.kept_until = Some(std::time::Instant::now() + BAR_PEEK);
        self.bar.peeks += 1;
        let peek = self.bar.peeks;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(BAR_PEEK).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // The pointer on the row keeps it: it goes when the pointer leaves (see `pointer_moved`).
                let on_the_row = Pointer::at(window.mouse_position(), window.viewport_size()) == Pointer::OnTheRow;
                if this.bar.peeks == peek && !on_the_row {
                    this.bar.shown = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The pointer moved, to `at` in the window: a row of tabs that keeps
    /// out of the way comes when the pointer rests at the window's top
    /// edge, and goes when the pointer is well below it.
    fn pointer_moved(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.bar_keeps_away() {
            return;
        }
        let pointer = Pointer::at(at, window.viewport_size());
        if !self.bar.shown {
            if pointer != Pointer::AtTheEdge || self.bar.coming {
                return;
            }
            // A pointer on its way out of the window crosses the edge too: only one that stays there calls the row.
            self.bar.coming = true;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(BAR_DWELL).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.bar.coming = false;
                    if Pointer::at(window.mouse_position(), window.viewport_size()) == Pointer::AtTheEdge {
                        this.bar.shown = true;
                        this.bar.kept_until = None;
                        cx.notify();
                    }
                });
            })
            .detach();
            return;
        }
        let kept = self.bar.kept_until.is_some_and(|until| std::time::Instant::now() < until);
        if pointer == Pointer::Elsewhere && !kept {
            self.bar.shown = false;
            cx.notify();
        }
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
    fn sidebar(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = &self.settings.theme;
        let (ink, muted, rule, well) =
            (solid(theme.foreground), solid(theme.muted()), solid(theme.rule()), solid(theme.well()));
        let mono = SharedString::from(self.settings.config.font.family.clone());
        let listed = match &self.sidebar.agents {
            Some(agents::Agents::Listed(agents)) => agents.clone(),
            _ => Vec::new(),
        };
        let notice =
            self.sidebar.agents.as_ref().map_or_else(|| agents::Agents::Asking.notice(), agents::Agents::notice);
        let count = if listed.is_empty() { String::new() } else { listed.len().to_string() };
        let cards = listed.into_iter().enumerate().map(|(index, agent)| {
            let (title, place, figures) = (agent.title(), agent.place(), agent.figures());
            div()
                .id(("agent", index))
                .p_3()
                .rounded(px(theme.rounding))
                .border_1()
                .border_color(rule)
                .bg(solid(theme.panel()))
                .hover(move |card| card.bg(well))
                .cursor_pointer()
                .flex()
                .flex_col()
                .gap_1()
                .on_click(cx.listener(move |this, _, _, cx| this.go_to(&agent, cx)))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(title)))
                .child(
                    div()
                        .font_family(mono.clone())
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(SharedString::from(place)),
                )
                .child(div().text_size(px(12.)).child(SharedString::from(figures)))
        });
        let close = sign_button("hide-agents", TAB_CLOSE, "×", theme)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_agents(cx)));
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(SIDEBAR_HEAD))
            .px_4()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Agents"))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(SharedString::from(count)),
                    ),
            )
            .child(close);
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(seen_through(theme.bar(), theme.opacity))
            .border_l_1()
            .border_color(rule)
            .text_color(ink)
            .child(head)
            .child(
                div()
                    .id("agents")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(notice.map(|notice| div().text_color(muted).child(SharedString::from(notice))))
                    .children(cards)
                    .children(
                        self.sidebar.note.clone().map(|note| div().text_color(muted).child(SharedString::from(note))),
                    ),
            )
            .children(self.sidebar_picture(window))
    }

    /// The look's picture, at the sidebar's foot: the sidebar's own color
    /// lies over its upper part and fades out. `None` for a look with no
    /// picture, and in a window too low for it.
    ///
    /// The picture fills the strip from side to side and is cut at the
    /// strip's edges: one wider than the strip is tall for it goes past both
    /// sides, and must not be drawn there, over the text beside the sidebar.
    fn sidebar_picture(&self, window: &Window) -> Option<gpui::Div> {
        let picture = self.picture.clone()?;
        if window.viewport_size().height < px(SIDEBAR_PICTURE_FROM) {
            return None;
        }
        let bar = solid(self.settings.theme.bar());
        let fade = gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(bar, 0.),
            gpui::linear_color_stop(Hsla { a: 0., ..bar }, SIDEBAR_PICTURE_FADE),
        );
        Some(
            div()
                .relative()
                .flex_none()
                .w_full()
                .h(px(SIDEBAR_PICTURE))
                .overflow_hidden()
                .child(gpui::img(picture).absolute().size_full().object_fit(gpui::ObjectFit::Cover))
                .child(div().absolute().size_full().bg(fade)),
        )
    }

    /// One tab of the row: its sign, its name, its close button; the one in
    /// use has the well's color behind it and a short line under it.
    fn tab(&self, index: usize, cx: &mut Context<'_, Self>) -> impl IntoElement + use<> {
        let theme = &self.settings.theme;
        let (ink, muted, well) = (solid(theme.foreground), solid(theme.muted()), solid(theme.well()));
        let active = index == self.active;
        let close =
            sign_button(("close", index), TAB_CLOSE, "×", theme).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close(index, window, cx);
            }));
        // Inside the tab's lower edge: one pixel lower, the tab's own line is drawn over half of it.
        let mark = div()
            .absolute()
            .left_3()
            .bottom(px(0.))
            .w(px(TAB_MARK.0))
            .h(px(TAB_MARK.1))
            .rounded(px(1.))
            .bg(solid(theme.accent()));
        div()
            .id(("tab", index))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(TAB_HEIGHT))
            .min_w(px(TAB_LEAST_WIDTH))
            .pl_3()
            .pr_1()
            .rounded(px(theme.rounding))
            .border_1()
            .border_color(if active { solid(theme.rule()) } else { gpui::transparent_black() })
            .font_family(SharedString::from(self.settings.config.font.family.clone()))
            .text_size(px(12.))
            .text_color(if active { ink } else { muted })
            .when(active, |tab| tab.bg(well))
            .when(!active, |tab| tab.hover(move |tab| tab.bg(well)))
            .on_click(cx.listener(move |this, _, window, cx| this.activate(index, window, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| this.close(index, window, cx)))
            .child(div().text_color(if active { solid(theme.symbol()) } else { muted }).child(TAB_SIGN))
            .child(div().flex_1().child(SharedString::from(tab_title(&self.title(index, cx)))))
            .child(close)
            .when(active, |tab| tab.child(mark))
    }

    /// The row of tabs: the tabs, the new-tab button, the part that moves
    /// the window, the notice of a newer version, the window's buttons.
    fn tab_bar(&self, place: BarPlace, window: &Window, cx: &mut Context<'_, Self>) -> gpui::Div {
        let theme = &self.settings.theme;
        let ink = solid(theme.foreground);
        let tabs: Vec<_> = (0..self.tabs.len()).map(|index| self.tab(index, cx)).collect();
        let add = sign_button("new-tab", TAB_ADD, "+", theme)
            .on_click(cx.listener(|this, _, window, cx| this.open(None, window, cx)));
        let newer = self.newer.as_ref().map(|newer| {
            div()
                .id("update")
                .flex_none()
                .h_full()
                .px_3()
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(solid(theme.ansi[3]))
                .hover(move |notice| notice.text_color(ink))
                .on_click(cx.listener(|this, _, _, cx| this.open_update(cx)))
                .child(SharedString::from(newer.text()))
        });
        let own_bar = self.title_bar == TitleBar::Tabs;
        // Full screen has no title bar: the tabs do not move the window then.
        let handle = window_handle(if window.is_fullscreen() { TitleBar::System } else { self.title_bar });
        // Full screen has no frame to minimize or restore (macOS draws its own buttons there).
        let buttons =
            (own_bar && cfg!(target_os = "windows") && !window.is_fullscreen()).then(|| window_buttons(window, theme));
        // macOS draws its three buttons at the row's left, but not in full screen and not while
        // they are hidden with the row; elsewhere the tabs begin a little in from the edge.
        let lights = own_bar && cfg!(target_os = "macos") && !window.is_fullscreen();
        let lead = if lights { TRAFFIC_LIGHTS_WIDTH } else { 12. };
        // Seen through like the text's background when it is part of the window; whole when it lies over the text.
        let lies_over = place == BarPlace::OverTheText;
        let behind = if lies_over { 1.0 } else { theme.opacity };
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(TAB_BAR_HEIGHT))
            .bg(seen_through(theme.bar(), behind))
            .border_b_1()
            .border_color(solid(theme.rule()))
            .when(lies_over, |bar| bar.absolute().top_0().left_0().right_0().occlude().shadow_md())
            .child(div().flex_none().w(px(lead)))
            .child(div().flex().flex_none().items_center().gap_1().children(tabs).child(add))
            .child(handle)
            .children(newer)
            .children(buttons)
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

/// Where the row of tabs is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarPlace {
    /// Above the text, as a part of the window.
    InItsRow,
    /// Over the text, when it keeps out of the way and shows for a while: it
    /// takes the pointer from what is under it.
    OverTheText,
}

/// A small button that is one sign (a tab's or the sidebar's ×, the + of a
/// new tab): `size` is its width and height. What a click does is added by
/// who makes it.
fn sign_button(
    id: impl Into<gpui::ElementId>,
    size: (f32, f32),
    sign: &'static str,
    theme: &theme::Theme,
) -> gpui::Stateful<gpui::Div> {
    let (ink, well) = (solid(theme.foreground), solid(theme.well()));
    div()
        .id(id)
        .w(px(size.0))
        .h(px(size.1))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(theme.button_rounding()))
        .text_size(px(17.))
        .text_color(solid(theme.muted()))
        .hover(move |button| button.bg(well).text_color(ink))
        .child(sign)
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
    let (foreground, dim, lit) = (theme.foreground, theme.muted(), theme.well());
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
        // The system is told when the settings begin or stop asking for a window that is seen through.
        let behind = Behind::of(&self.settings.theme);
        if behind != self.behind {
            window.set_background_appearance(behind.appearance());
            self.behind = behind;
        }
        // macOS: the system's three buttons are in the row of tabs when it is the title bar. They
        // go away with a row that keeps out of the way, and come back with it; in full screen the
        // system hides them itself.
        let own_row = self.title_bar == TitleBar::Tabs && !window.is_fullscreen();
        let lights = if own_row && self.bar_keeps_away() && !self.bar.shown { Lights::Hidden } else { Lights::Shown };
        if lights != self.lights {
            lights::set(window, lights);
            self.lights = lights;
        }
        // And of the window's corners, when the look names them (or stops naming them).
        let corners = self.settings.theme.corners;
        if self.corners_told != CornersTold::These(corners) {
            corners::set(window, corners);
            self.corners_told = CornersTold::These(corners);
        }
        // A row that keeps out of the way lies over the text when it shows: the text keeps its place and size.
        let keeps_away = self.bar_keeps_away();
        let in_its_place = (!keeps_away).then(|| self.tab_bar(BarPlace::InItsRow, window, cx));
        let over_the_text = (keeps_away && self.bar.shown).then(|| self.tab_bar(BarPlace::OverTheText, window, cx));
        // The wallpaper, under everything: the color behind the text first (the tabs do not draw
        // it then), and the picture over it as large as the window, what goes past the window's
        // sides cut off, showing as much as the look says.
        let theme = &self.settings.theme;
        let wallpaper = self.wallpaper.clone().map(|picture| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .overflow_hidden()
                .bg(seen_through(theme.background, theme.opacity))
                .child(
                    gpui::img(picture)
                        .absolute()
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover)
                        .opacity(theme.wallpaper_opacity),
                )
        });
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .children(wallpaper)
            .text_size(px(13.))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.open(None, window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close(this.active, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step(Step::Next, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.step(Step::Previous, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleAgents, _, cx| this.toggle_agents(cx)))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &ToggleTabBar, _, cx| this.toggle_tab_bar(cx)))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.pointer_moved(event.position, window, cx);
            }))
            .children(in_its_place)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(
                        self.tabs.get(self.active).map(|tab| div().flex_1().min_w_0().h_full().child(tab.view.clone())),
                    )
                    .when(self.sidebar.shown, |area| area.child(self.sidebar(window, cx))),
            )
            .children(over_the_text)
    }
}

/// The keys for the window's own commands, as the settings' `[keys]` give
/// them (their defaults are `unifyterm_config::DEFAULT_KEYS`). A command
/// with no key is done by none.
fn key_bindings(keys: &config::KeysConfig) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for (name, chords) in keys.all() {
        for chord in chords {
            // The settings checked the key; GPUI reads it once more its own way, and a key it
            // cannot read must not end the program.
            if gpui::Keystroke::parse(chord).is_err() {
                log::warn!("keys.{name}: \"{chord}\" is not a key this window can take: left out");
                continue;
            }
            bindings.push(match name {
                "new_tab" => KeyBinding::new(chord, NewTab, None),
                "close_tab" => KeyBinding::new(chord, CloseTab, None),
                "next_tab" => KeyBinding::new(chord, NextTab, None),
                "previous_tab" => KeyBinding::new(chord, PreviousTab, None),
                "copy" => KeyBinding::new(chord, Copy, None),
                "paste" => KeyBinding::new(chord, Paste, None),
                "agents" => KeyBinding::new(chord, ToggleAgents, None),
                "tab_bar" => KeyBinding::new(chord, ToggleTabBar, None),
                "full_screen" => KeyBinding::new(chord, ToggleFullScreen, None),
                "explain" => KeyBinding::new(chord, Explain, None),
                _ => continue,
            });
        }
    }
    bindings
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
    let asked = if args.config.is_some() { Asked::WithConfig } else { Asked::Default };
    let path = args.config.or_else(Config::default_path);
    // The file as the tabs' programs name it, whatever directory they are in.
    let file = path
        .as_deref()
        .map(|path| SettingsFile { path: std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()), asked });
    let (config, mut notices) = match path.as_deref().map(Config::load) {
        Some(Err(error)) => (Config::default(), vec![format!("settings not used: {error:#}")]),
        Some(Ok(config)) => (config, Vec::new()),
        None => (Config::default(), Vec::new()),
    };
    let themes = file.as_ref().map(|file| unifyterm_config::themes::directory(&file.path));
    let (theme, pictures) = config::theme(&config, themes.as_deref()).unwrap_or_else(|error| {
        notices.push(format!("theme not used: {error:#}"));
        (config::own_colors(&config), config::Pictures::default())
    });
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
    // What is behind the text is settled when the window opens: it reads the wallpaper then.
    let settings = Arc::new(Settings { config, theme, backdrop: Backdrop::Color });
    let helper = Arc::new(helper);
    let start = Start {
        settings,
        pictures,
        commands,
        recorder,
        hooks,
        directory,
        file,
        helper,
        command: args.command,
        notices,
    };

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings(&start.settings.config.keys));
        let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
        let own_bar = TitleBar::choose(&start.settings.config.window) == TitleBar::Tabs;
        let options = WindowOptions {
            window_background: Behind::of(&start.settings.theme).appearance(),
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

    /// The keys bound to `action` in `bindings`, as GPUI writes them.
    fn keys_of(bindings: &[KeyBinding], action: &dyn gpui::Action) -> Vec<String> {
        bindings
            .iter()
            .filter(|binding| binding.action().partial_eq(action))
            .map(|binding| binding.keystrokes().iter().map(gpui::KeybindingKeystroke::unparse).collect())
            .collect()
    }

    #[test]
    fn full_screen_has_its_keys() {
        let bindings = key_bindings(&config::KeysConfig::default());
        let wanted: &[&str] =
            if cfg!(target_os = "macos") { &["cmd-enter", "ctrl-cmd-f"] } else { &["f11", "alt-enter"] };
        assert_eq!(keys_of(&bindings, &ToggleFullScreen), wanted);
    }

    #[test]
    fn a_pointer_calls_or_keeps_the_row_of_tabs_only_from_inside_the_window() {
        let window = size(px(960.), px(620.));
        let at = |x: f32, y: f32| Pointer::at(point(px(x), px(y)), window);
        assert_eq!(at(300., 0.), Pointer::AtTheEdge);
        assert_eq!(at(300., 10.), Pointer::AtTheEdge);
        assert_eq!(at(300., 11.), Pointer::OnTheRow);
        assert_eq!(at(300., 58.), Pointer::OnTheRow, "the row and 12 pixels under it");
        assert_eq!(at(300., 59.), Pointer::Elsewhere);
        assert_eq!(at(300., 400.), Pointer::Elsewhere);
        // Outside the window, above it too: the row is neither called nor kept.
        for (x, y) in [(300., -1.), (300., -400.), (-5., 5.), (960., 5.), (300., 620.), (2000., 30.)] {
            assert_eq!(at(x, y), Pointer::Elsewhere, "({x}, {y})");
        }
    }

    #[test]
    fn every_default_key_of_every_system_is_one_the_window_takes() {
        // Each command of the settings has an action here, on both tables of defaults.
        for (name, elsewhere, mac) in unifyterm_config::DEFAULT_KEYS {
            for chord in elsewhere.iter().chain(mac) {
                assert!(gpui::Keystroke::parse(chord).is_ok(), "keys.{name}: {chord}");
            }
        }
        let defaults = config::KeysConfig::default();
        let bound = key_bindings(&defaults);
        let given: usize = defaults.all().iter().map(|(_, chords)| chords.len()).sum();
        assert_eq!(bound.len(), given, "a key table entry for every key the settings give");
    }

    #[test]
    fn the_settings_keys_are_the_windows_keys() {
        let settings = Config::parse("[keys]\nagents = [\"ctrl-alt-g\", \"f9\"]\nexplain = []\ntab_bar = [\"ctrl--\"]")
            .expect("settings");
        let bound = key_bindings(&settings.keys);
        assert_eq!(keys_of(&bound, &ToggleAgents), ["ctrl-alt-g", "f9"], "the settings' keys, not the default one");
        assert!(keys_of(&bound, &Explain).is_empty(), "no key: no key does it");
        assert_eq!(keys_of(&bound, &ToggleTabBar), ["ctrl--"], "the minus key");
        assert!(!keys_of(&bound, &NewTab).is_empty(), "the others keep theirs");
        // Every form the settings keep a key in is one GPUI reads.
        for typed in ["ctrl-alt-escape", "cmd-shift-pagedown", "alt-[", "ctrl-+", "ctrl-shift-space", "f35", "ctrl-1"] {
            let kept = unifyterm_config::chord::chord(typed).expect(typed);
            assert!(gpui::Keystroke::parse(&kept).is_ok(), "{kept}");
        }
    }

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
