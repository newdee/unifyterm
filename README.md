<p align="center">
  <img src="docs/brand/unifyterm-wordmark-light.png" alt="UnifyTerm" width="420">
</p>

# UnifyTerm

One terminal for Windows, macOS and Linux. The page that introduces it: [dfine.tech/unifyterm](https://dfine.tech/unifyterm/).

![UnifyTerm: the same Unix commands on Windows, history as you type, pictures and markdown, a model's explanation, agents beside the terminal](docs/media/unifyterm-promo.gif)

- **The same Unix commands everywhere.** `cat`, `ls`, `grep`, `jq`, `find`, `sed` and the rest of 99 commands (114 on Linux) come with it (`ut`): on Windows too, and before PowerShell's aliases. Paths are printed with `/`.
- **A command history you can search as you type.** Every command a shell runs is kept (without secrets) with its directory and exit code; a list of matches shows while you type.
- **Help with commands.** A corrected command when one is not found, guessed on your machine from the commands it knows. With a model you name and agree to once: better corrections, what each part of a command means (Ctrl-Shift-E), and the rest of a command in grey.
- **Pictures, video and markdown in the terminal.** `ut view photo.png`, `ut view clip.mp4`, `ut md README.md`.
- **For agents.** A sidebar with the agents keepane watches (Ctrl-Shift-A); `ut mcp`, an MCP server with the history and the commands' help; a skill file.

It runs your own shell: PowerShell, cmd, bash, zsh or fish.

## Install

**macOS and Linux** (Apple silicon, Intel; Linux x86_64), with [Homebrew](https://brew.sh):

```bash
brew install newdee/tap/unifyterm
```

Or with one line, for you alone (no root): it puts `unifyterm` and `ut` in `~/.local/bin`, UnifyTerm.app in `~/Applications` on macOS, a menu entry on Linux.

```bash
curl -fsSL https://raw.githubusercontent.com/newdee/unifyterm/main/install.sh | sh
```

**Windows** 10 1809 or later, with [Scoop](https://scoop.sh):

```powershell
scoop install https://raw.githubusercontent.com/newdee/unifyterm/main/packaging/scoop/unifyterm.json
```

Or with one line, for you alone (no administrator rights): it puts UnifyTerm in `%LOCALAPPDATA%\Programs\unifyterm`, on your `PATH` and in the Start menu.

```powershell
irm https://raw.githubusercontent.com/newdee/unifyterm/main/install.ps1 | iex
```

**Packages** on the [releases page](https://github.com/newdee/unifyterm/releases), each with a `.sha256` beside it:

| System | File |
|---|---|
| Windows | `unifyterm-VERSION-x64.msi` (into Program Files, for every user), or `unifyterm-vVERSION-windows-x86_64.zip` |
| macOS | `unifyterm-VERSION-macos-aarch64.dmg` or `-macos-x86_64.dmg`, or the `.tar.gz` |
| Debian, Ubuntu | `unifyterm_VERSION-1_amd64.deb`: `sudo apt install ./unifyterm_VERSION-1_amd64.deb` |
| Other Linux | `unifyterm-VERSION-x86_64.AppImage`, or the `.tar.gz` |

On Linux the window needs Vulkan and xkbcommon (on Debian and Ubuntu: `mesa-vulkan-drivers libxkbcommon-x11-0`). Updating: `brew upgrade unifyterm`, `scoop update unifyterm`, or the same one line again.

The builds are not signed yet. Windows SmartScreen may ask once ("More info", then "Run anyway"); macOS asks once (right-click the app, then "Open").

Uninstalling keeps your settings, history and log (see [Where things are](#where-things-are)).

From source: `cargo build --release` with Rust 1.93 or later; on Windows run `pwsh tools/fetch-conpty.ps1` first. On Linux GPUI needs `libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libvulkan-dev libfontconfig-dev`.

## Keys

On macOS the window's own keys use Cmd, as the second column shows; the keys at the prompt are the same everywhere.

| Key | macOS | What it does |
|---|---|---|
| Ctrl-Shift-T | Cmd-T | A new tab |
| Ctrl-Shift-W | Cmd-W | Close the tab |
| Ctrl-Tab | Ctrl-Tab | The next tab |
| Ctrl-Shift-Tab | Ctrl-Shift-Tab | The tab before |
| Ctrl-Shift-C | Cmd-C | Copy the selection |
| Ctrl-Shift-V, Shift-Insert | Cmd-V | Paste |
| Ctrl-Shift-A | Cmd-Shift-A | Show or hide the agents' sidebar |
| Ctrl-Shift-E | Ctrl-Shift-E | Explain the command on the line |
| Ctrl-N, Down | Ctrl-N, Down | The next command in the list while typing |
| Ctrl-P, Up | Ctrl-P, Up | The command before in the list |
| Tab, Right | Tab, Right | Put the chosen command on the line; Right with none chosen takes the grey suggestion |
| Enter | Enter | With a command chosen: put it on the line (it does not run) |
| Esc | Esc | Close the list or a popup |

In a popup, Enter takes what it offers (a corrected command goes on the line, not run) and Esc closes it. A click on a tab goes to it; a middle click closes it. A click on an agent's card goes to its pane, or copies where the pane is when that fails.

## Settings

The settings file is `config.toml` in `%APPDATA%\unifyterm` (Windows), `~/Library/Application Support/unifyterm` (macOS) or `~/.config/unifyterm` (Linux); `unifyterm --config FILE` names another. Every key is optional. A key that is misspelt, or a value out of range, is named in a line at the top of the first tab, and the defaults are used. The same file works on every system: paths may be written with `/`.

| Key | Default | What it is |
|---|---|---|
| `font.family` | Cascadia Code (Windows), Menlo (macOS), DejaVu Sans Mono (Linux) | The font |
| `font.size` | `14` | In pixels, 6 to 72 |
| `font.line_height` | `1.35` | A line's height as a multiple of the size, 1 to 2 |
| `font.ligatures` | `true` | `->`, `!=` and the like as one shape, where the font has them |
| `font.fallbacks` | Microsoft YaHei UI, Segoe UI Emoji, Segoe UI Symbol (Windows); PingFang SC, Apple Color Emoji (macOS); Noto Sans CJK SC, Noto Color Emoji (Linux) | Fonts tried, in order, for characters the font lacks |
| `shell.program` | PowerShell (Windows), `$SHELL` (elsewhere) | The program a new tab runs |
| `shell.args` | `[]` | Its arguments |
| `terminal.scrollback` | `10000` | Lines kept for each tab, at most 1000000 |
| `terminal.padding` | `6` | Pixels between the window's edge and the text, 0 to 64 |
| `theme.foreground` | built-in | `#rrggbb` |
| `theme.background` | built-in | `#rrggbb` |
| `theme.cursor` | built-in | `#rrggbb` |
| `theme.selection` | built-in | `#rrggbb` |
| `theme.ansi` | built-in | The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight |
| `commands.builtin` | `true` | Put UnifyTerm's commands before the system's in each shell's `PATH` |
| `commands.system` | `[]` | Commands that stay the system's although `builtin` is on, as `["find", "sort"]` |
| `history.enabled` | `true` | Keep the commands shells run. Off: no shell hook, no history file |
| `history.candidates` | `true` | Show the list while typing |
| `history.limit` | `8` | The most commands the list shows, 1 to 20 |
| `ai.provider` | `"none"` | `"anthropic"` (Claude) or `"openai-compatible"` (a local model, for one) |
| `ai.model` | | The model's name, as the service knows it (`"claude-opus-5-5"`) |
| `ai.base_url` | | Where the service is: needed for `openai-compatible`; for `anthropic`, only through a proxy |
| `ai.api_key_env` | | The environment variable that holds the key (the key itself is never in this file) |
| `ai.suggest` | `true` | The rest of the command in grey while typing |
| `ai.fix` | `true` | A corrected command when one is not found |
| `ai.explain` | `true` | What each part of a command means (Ctrl-Shift-E) |
| `agents.sidebar` | `false` | Show the agents' sidebar when the window opens |
| `agents.poll_ms` | `2000` | How often the agents are asked for while the sidebar shows, 500 to 60000 |
| `agents.command` | `["keepane", "list-agents", "-J"]` | The program that prints the agents as JSON |
| `agents.focus` | `["keepane", "focus-pane", "{pane}"]` | The program that brings an agent's pane to the front; `{pane}` is its number |

Nothing is sent to a model until `ai.provider` names one **and** you agree in the window (once). What is sent, as the window says when it asks: the command line you type, the directory (your home as `~`), your last five commands, and for a command that was not found the last 40 lines it printed. Anything that looks like a password, token or key is replaced first.

## Environment variables

| Variable | What it does |
|---|---|
| `UNIFYTERM_DATA_DIR` | Keep UnifyTerm's data (history, log, command links, shell hooks) in this directory instead of the system's |
| `UNIFYTERM_RAW_LOG` | A file to append every byte the first tab's program prints to (later tabs: `.2`, `.3`); `unifyterm --replay FILE` shows the screen they draw |
| `UT_NATIVE_PATHS` | On Windows: `ut`'s commands print paths with Windows' own `\` |
| `UT_NO_GLOB` | On Windows: `ut`'s commands keep every wildcard as typed instead of expanding it |

The variable named by `ai.api_key_env` holds the model service's key. `ut view` and `ut jq` follow `NO_COLOR` (no colors when it is set); `ut jq` takes its colors from `JQ_COLORS`, as jq does.

## The commands: `ut`

`ut --list` names them (99 on Windows, 114 on Linux): coreutils, findutils, `grep`, `sed`, `jq`, `diff`, `which` and others. In a UnifyTerm shell they are on `PATH` under their own names; elsewhere run them as `ut cat FILE`.

| Command | What it does |
|---|---|
| `ut history recent [-n N] [--cwd DIR] [--long]` | The commands last run |
| `ut history search TEXT` | Commands that begin with or contain TEXT |
| `ut history forget TEXT` | Remove a command from the history |
| `ut view FILE [--mode M] [--width N]` | A picture, or a video with ffmpeg (no sound; `q` stops it) |
| `ut md [FILE]` | Markdown as styled text |
| `ut mcp` | An MCP server on standard input and output (below) |
| `ut skill [--install DIR]` | The agents' skill file, printed or written to `DIR/unifyterm/SKILL.md` |

## For agents

- **The sidebar** (Ctrl-Shift-A) shows the agents in keepane's panes (keepane is a terminal multiplexer): model, cost, context, CPU, memory. It asks keepane only while it shows.
- **`ut mcp`** gives an agent four tools: `history_search`, `history_recent`, `list_commands`, `command_help`. It reads the history and runs nothing. Register it with Claude Code: `claude mcp add unifyterm -- ut mcp`.
- **The skill** (`skills/unifyterm/SKILL.md`, or `ut skill`) tells an agent how to work in UnifyTerm.

## Where things are

| What | Windows | macOS | Linux |
|---|---|---|---|
| Settings | `%APPDATA%\unifyterm\config.toml` | `~/Library/Application Support/unifyterm/config.toml` | `~/.config/unifyterm/config.toml` |
| History, log, links | `%LOCALAPPDATA%\unifyterm` | `~/Library/Application Support/unifyterm` | `~/.local/share/unifyterm` |

The history is one SQLite file, `history.db`; a copy works on every system.

## Known limits

- No sound with video; ffmpeg must be installed for video.
- UnifyTerm draws kitty's graphics protocol; sixel and iTerm2 images are for `ut view` in other terminals.
- PowerShell's Tab completion still writes paths with `\`; `ls -R` and `find -exec` print uutils' own `\` paths on Windows.
- The shell hooks (history, the list while typing) need PowerShell 7 or 5.1, cmd, bash, zsh or fish.
- macOS has not been used by hand: CI builds it, runs the tests and starts it.
- The builds are not signed.

## License

MIT (`LICENSE`). The licenses of everything built in are in `THIRD-PARTY-NOTICES`.
