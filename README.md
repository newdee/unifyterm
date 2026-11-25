<p align="center">
  <img src="docs/brand/unifyterm-wordmark-light.png" alt="UnifyTerm" width="420">
</p>

# UnifyTerm

One terminal for Windows, macOS and Linux. The page that introduces it: [dfine.tech/unifyterm](https://dfine.tech/unifyterm/).

![UnifyTerm: the same Unix commands on Windows, history as you type, pictures and markdown, a model's explanation, agents beside the terminal](docs/media/unifyterm-promo.gif)

- **The same Unix commands everywhere.** `cat`, `ls`, `grep`, `jq`, `find`, `sed` and the rest of 99 commands (114 on Linux) come with it (`ut`): on Windows too, and before PowerShell's aliases. Paths are printed with `/`.
- **A command history you can search as you type.** Every command a shell runs is kept (without secrets) with its directory and exit code; a list of matches shows while you type.
- **Help with commands.** A corrected command when one is not found, guessed on your machine from the commands it knows. With a model you name and agree to once: better corrections, what each part of a command means (Ctrl-Shift-E), the rest of a command in grey, and a command for what you say in words (`# the five largest files here`, then Enter). The model is a service, a model server on your machine, or an agent's command line you already have (Claude Code, Codex, pi).
- **Pictures, video and markdown in the terminal.** `ut view photo.png`, `ut view clip.mp4`, `ut md README.md`.
- **For agents.** A sidebar with the agents keepane watches (Ctrl-Shift-A); `ut mcp`, an MCP server with the history and the commands' help; a skill file.

It runs your own shell: PowerShell, cmd, bash, zsh or fish.

## Install

**macOS and Linux** (Apple silicon, Intel; Linux x86_64), with [Homebrew](https://brew.sh):

```bash
brew install newdee/tap/unifyterm
```

Homebrew 6 and later load a formula of a tap that is not their own only once it is trusted. The full name above installs it; before `brew upgrade unifyterm` or any other use of the short name, trust it once (`--formula` trusts this one, not the whole tap):

```bash
brew trust --formula newdee/tap/unifyterm   # Homebrew 6 and later
brew upgrade unifyterm
```

Or with one line, for you alone (no root): it puts `unifyterm` and `ut` in `~/.local/bin`, UnifyTerm.app in `~/Applications` on macOS, a menu entry on Linux.

```bash
curl -fsSL https://raw.githubusercontent.com/newdee/unifyterm/main/install.sh | sh
```

**Windows** 10 1809 or later, with [Scoop](https://scoop.sh): add the bucket once, then install by name.

```powershell
scoop bucket add newdee https://github.com/newdee/scoop-bucket
scoop install unifyterm
```

Installed earlier from the manifest's address, it keeps updating from there: `scoop update unifyterm`.

Or with one line, for you alone (no administrator rights): it puts UnifyTerm in `%LOCALAPPDATA%\Programs\unifyterm`, on your `PATH` and in the Start menu.

```powershell
irm https://raw.githubusercontent.com/newdee/unifyterm/main/install.ps1 | iex
```

**Packages** on the [releases page](https://github.com/newdee/unifyterm/releases), each with a `.sha256` beside it:

| System | File |
|---|---|
| Windows | `unifyterm-VERSION-x64.msi` (into Program Files, for every user, with a Start menu item and a desktop icon; `msiexec /i unifyterm-VERSION-x64.msi DESKTOPSHORTCUT=0` makes no desktop icon), or `unifyterm-vVERSION-windows-x86_64.zip` |
| macOS | `unifyterm-VERSION-macos-aarch64.dmg` or `-macos-x86_64.dmg`, or the `.tar.gz` |
| Debian, Ubuntu | `unifyterm_VERSION-1_amd64.deb`: `sudo apt install ./unifyterm_VERSION-1_amd64.deb` |
| Other Linux | `unifyterm-VERSION-x86_64.AppImage`, or the `.tar.gz` |

On Linux the window needs Vulkan and xkbcommon (on Debian and Ubuntu: `mesa-vulkan-drivers libxkbcommon-x11-0`). Updating: `brew upgrade unifyterm`, `scoop update unifyterm`, or the same one line again.

The builds are not signed yet. Windows SmartScreen may ask once ("More info", then "Run anyway"); macOS asks once (right-click the app, then "Open").

Uninstalling keeps your settings, history and log (see [Where things are](#where-things-are)).

From source: `cargo build --release` with Rust 1.93 or later; on Windows run `pwsh tools/fetch-conpty.ps1` first. On Linux GPUI needs `libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libvulkan-dev libfontconfig-dev`.

## Keys

On macOS the window's own keys use Cmd, as the second column shows; the keys at the prompt are the same everywhere. The first ten, down to "Explain", are the keys until you give others: each is a setting in `[keys]` (below), and a change is the window's key at once. The keys of the list and the popups are fixed.

| Key | macOS | What it does |
|---|---|---|
| Ctrl-Shift-T | Cmd-T | A new tab |
| Ctrl-Shift-W | Cmd-W | Close the tab |
| Ctrl-Tab | Ctrl-Tab | The next tab |
| Ctrl-Shift-Tab | Ctrl-Shift-Tab | The tab before |
| Ctrl-Shift-C | Cmd-C | Copy the selection |
| Ctrl-Shift-V, Shift-Insert | Cmd-V | Paste |
| Ctrl-Shift-A | Cmd-Shift-A | Show or hide the agents' sidebar |
| Ctrl-Shift-B | Cmd-Shift-B | The row of tabs out of the way (`window.tab_bar = "auto-hide"`), or always there again: for this window, until the setting is changed |
| F11, Alt-Enter | Cmd-Enter, Ctrl-Cmd-F | Full screen, over the taskbar; again to leave it. A program in the terminal no longer gets Alt-Enter |
| Ctrl-Shift-E | Ctrl-Shift-E | Explain the command on the line |
| Ctrl-N, Down | Ctrl-N, Down | The next command in the list while typing |
| Ctrl-P, Up | Ctrl-P, Up | The command before in the list |
| Tab, Right | Tab, Right | Put the chosen command on the line; Right with none chosen takes the grey suggestion |
| Enter | Enter | With a command chosen: put it on the line (it does not run) |
| Esc | Esc | Close the list or a popup |

In a popup, Enter takes what it offers (a corrected command goes on the line, not run) and Esc closes it. A click on a tab goes to it; a middle click closes it. A click on an agent's card goes to its pane, or copies where the pane is when that fails.

## Settings

The settings file is `config.toml` in `%APPDATA%\unifyterm` (Windows), `~/Library/Application Support/unifyterm` (macOS) or `~/.config/unifyterm` (Linux); `unifyterm --config FILE` names another. Every key is optional. A key that is misspelt, or a value out of range, is named in a line at the top of the first tab, and the defaults are used. The same file works on every system: paths may be written with `/`.

`ut setting`, in a tab, shows every setting and changes it from the keyboard. ↑↓ move; Enter changes the one the cursor is on: a switch turns, and a font, a shell, the fallback fonts, the commands to leave to the system and the model's provider open a list to pick from (type to narrow it; Space marks several, in order). ←→ make a number smaller or larger, `e` types any value (a font the list lacks), `d` puts the default back, `u` takes the last change back, `/` narrows the settings, `q` leaves. A value UnifyTerm would refuse is not written, and the line says why. The comments in your file stay. For a script: `ut setting list`, `ut setting get font.size`, `ut setting set font.size 16`, `ut setting reset font.size`, `ut setting path`.

UnifyTerm reads the file again whenever it changes, by `ut setting` or by an editor. The font, the theme and everything in `[theme]`, the keys in `[keys]`, the padding, `window.tab_bar` and the list of earlier commands change at once. `shell.*` and `terminal.scrollback` are used by the next tab; `agents.command` and `agents.poll_ms` when the sidebar is shown again. The rest (`commands.*`, `history.enabled`, `ai.*`, `window.system_title_bar`, `agents.sidebar`, `updates.check`) is set up when UnifyTerm starts. A changed file that is wrong changes nothing: the settings in use stay, and the log says why.

| Key | Default | What it is |
|---|---|---|
| `font.family` | Cascadia Code (Windows), Menlo (macOS), DejaVu Sans Mono (Linux) | The font |
| `font.size` | `14` | In pixels, 6 to 72 |
| `font.line_height` | `1.35` | A line's height as a multiple of the size, 1 to 2 |
| `font.ligatures` | `true` | `->`, `!=` and the like as one shape, where the font has them |
| `font.fallbacks` | Microsoft YaHei UI, Segoe UI Emoji, Segoe UI Symbol (Windows); PingFang SC, Apple Color Emoji (macOS); Noto Sans CJK SC, Noto Color Emoji (Linux) | Fonts tried, in order, for characters the font lacks |
| `shell.program` | PowerShell (Windows), `$SHELL` (elsewhere) | The program a new tab runs |
| `shell.args` | `[]` | Its arguments. With arguments of your own the shell gets nothing from UnifyTerm: no history hook, no login start |
| `terminal.scrollback` | `10000` | Lines kept for each tab, at most 1000000 |
| `terminal.padding` | `6` | Pixels between the window's edge and the text, 0 to 64 |
| `window.system_title_bar` | `false` | The system's title bar above the tabs. Without it the row of tabs is the title bar: drag its empty part to move the window. On Linux the system's bar is always there |
| `window.tab_bar` | `"always"` | `"always"`: the row of tabs is always there. `"auto-hide"`: it keeps out of the way until the pointer rests at the window's top edge, or a tab is opened, closed or gone to; it then lies over the text, and the terminal keeps its size. Ctrl-Shift-B changes it for the window it is pressed in |

| `keys.new_tab` | as in Keys, above | The keys for a new tab: a list, as `["ctrl-shift-t"]`. A key is its name with the keys held with it before it, `-` between: `ctrl-shift-t`, `cmd-enter`, `alt-[`, `f11`. `[]`: no key |
| `keys.close_tab` | as above | The keys that close the tab |
| `keys.next_tab` | as above | The keys for the next tab |
| `keys.previous_tab` | as above | The keys for the tab before |
| `keys.copy` | as above | The keys that copy the selection |
| `keys.paste` | as above | The keys that paste |
| `keys.agents` | as above | The keys that show or hide the agents' sidebar |
| `keys.tab_bar` | as above | The keys that put the row of tabs out of the way, or back |
| `keys.full_screen` | as above | The keys for full screen, and out of it |
| `keys.explain` | as above | The keys that explain the command on the line |
| `theme.name` | | The theme: one of UnifyTerm's or one of your own (below). Not set: `tokyo-night` |
| `theme.foreground` | the theme's | The color of text, `#rrggbb` |
| `theme.background` | the theme's | The color behind it |
| `theme.cursor` | the theme's | The cursor's color. It also marks the tab in use and the chosen row of a list |
| `theme.selection` | the theme's | The color behind selected text |
| `theme.bar` | the theme's | The color behind the row of tabs and the sidebar |
| `theme.panel` | the theme's | The color behind a list or a box over the text, and behind an agent's card |
| `theme.well` | the theme's | The color behind the tab in use, and behind what the pointer is over |
| `theme.rule` | the theme's | The color of a line between two parts |
| `theme.edge` | the theme's | The color of the line around a list or a box |
| `theme.muted` | the theme's | The color of words that matter less |
| `theme.symbol` | the theme's | The color of the small signs, as the one in the tab in use |
| `theme.ansi` | the theme's | The 16 colors: black, red, green, yellow, blue, magenta, cyan, white, then the bright eight |
| `theme.opacity` | the theme's, else `1` | How much the window hides what is behind it, 0.1 to 1. The color behind the text, the row of tabs and the sidebar are seen through; text, lists and boxes are not |
| `theme.blur` | the theme's, else `false` | What shows through the window is blurred, as through frosted glass. It needs `opacity` under 1 |
| `theme.picture` | the theme's | A picture's file, shown at the foot of the sidebar: beside the file that names it, or one of UnifyTerm's own (`aurora.png`). `""`: none |
| `theme.rounding` | the theme's, else `7` | How round the corners of the tabs, the cards, the lists and the boxes are, in pixels, 0 to 18. 0 is square |
| `theme.corners` | the theme's | The window's own corners: `"round"`, `"small"` or `"square"`. Windows 11 only; not set, they are as the system makes them |
| `commands.builtin` | `true` | Put UnifyTerm's commands before the system's in each shell's `PATH` |
| `commands.system` | `[]` | Commands that stay the system's although `builtin` is on, as `["find", "sort"]` |
| `history.enabled` | `true` | Keep the commands shells run. Off: no shell hook, no history file |
| `history.candidates` | `true` | Show the list while typing |
| `history.limit` | `8` | The most commands the list shows, 1 to 20 |
| `ai.provider` | `"none"` | Where the model is: see the table below |
| `ai.model` | | The model's name, as the service knows it (`"claude-opus-5-5"`). An agent's command line uses its own when none is named |
| `ai.base_url` | | Where the service is: needed for `openai-compatible`; else only for another address than the provider's own |
| `ai.api_key_env` | | The environment variable that holds the key (the key itself is never in this file) |
| `ai.command` | `[]` | For the provider `command`: the program that answers, with its arguments |
| `ai.suggest` | `true` | The rest of the command in grey while typing (not from an agent's command line) |
| `ai.fix` | `true` | A corrected command when one is not found |
| `ai.explain` | `true` | What each part of a command means (Ctrl-Shift-E) |
| `ai.ask` | `true` | A line that begins with `#` is a sentence: Enter turns it into a command |
| `ai.ask_enter` | `"manual"` | The command made from a sentence: `"manual"` puts it on the line; `"auto"` also runs it when it only reads |
| `agents.sidebar` | `false` | Show the agents' sidebar when the window opens |
| `agents.poll_ms` | `2000` | How often the agents are asked for while the sidebar shows, 500 to 60000 |
| `agents.command` | `["keepane", "list-agents", "-J"]` | The program that prints the agents as JSON |
| `agents.focus` | `["keepane", "focus-pane", "{pane}"]` | The program that brings an agent's pane to the front; `{pane}` is its number |
| `updates.check` | `true` | Once a day, ask GitHub whether a newer UnifyTerm is out, and name it in the row of tabs |

**Keys.** A key in `[keys]` may be written as you would say it (`Ctrl+Shift+G`): `ut setting` keeps it as `ctrl-shift-g`. The keys held are `ctrl`, `alt`, `shift` and `cmd` (the Windows key on a PC). A letter, a digit or a sign needs Ctrl, Alt or Cmd with it, and so do Enter, Tab, Esc and the arrows: alone, or with Shift only, they are what the shell is typed to with, and are refused. A function key and Insert may stand alone. A key cannot be given to two commands. A key another program has taken for the whole system (a screenshot key) never reaches UnifyTerm: give the command another one, as `ut setting set keys.agents ctrl-alt-a`.

**Themes.** A theme is a set of colors under a name. UnifyTerm has nine: `tokyo-night` (in use until you choose another), `aurora`, `eclipse`, `phosphor`, `night` and `paper-dark`, which are dark, and `riso`, `day` and `paper`, which are light. `paper-dark` and `paper` are keepane's own two looks, with its colors. `ut theme` lists them with a sample of each one's colors; `ut theme NAME` uses one. Every open window changes at once, and the next time UnifyTerm opens it has that theme: the name is the setting `theme.name`. `demo/appearance.html` shows six of them in a browser, and the window's colors are the ones on that page.

A theme of your own is a file `NAME.toml` in the `themes` folder beside the settings file (`ut theme directory` prints it), with a table `[theme]` and the keys of the table above, all but `name`. It must give `foreground`, `background` and `ansi`. It may give `cursor`, `selection` and the colors around the text (`bar`, `panel`, `well`, `rule`, `edge`, `muted`, `symbol`): one it leaves out is worked out from the foreground and the background (the cursor is the text's color). It may also give what is not a color: `opacity`, `blur`, `rounding`, `corners`, and `picture`, a picture's file beside the theme's own (PNG, JPEG, WebP, GIF, BMP, SVG or TIFF, at most 16 MB) that is shown at the foot of the sidebar. UnifyTerm's own six pictures are found by their names from any folder, so a copy of a built-in theme keeps its picture. A key this version does not know is passed over, and `ut theme check` names it: a theme written for a later UnifyTerm still works in this one. `ut theme new NAME` begins one as a copy of the theme in use, and `ut theme show NAME` prints any theme's file. A window that uses your theme follows its file as you save it. A name is small letters, digits and `-`; your file with a built-in theme's name is used in that theme's place.

A theme that is not there, or whose file is wrong, is never put to use: `ut theme NAME` says what is wrong and writes nothing, and a window keeps the colors it has. What the settings file gives itself in `[theme]` stays over any theme: `theme.background = "#000000"`, or `theme.opacity = 0.85` for every theme you use. On a light theme, white text on the theme's own background is drawn in the theme's text color: programs that take the screen for a dark one write in white (PowerShell does, for what you type), and a light theme's white is as light as its background.

Nothing is sent to a model until `ai.provider` names one **and** you agree in the window (once). What is sent, as the window says when it asks: the command line you type, what you write after `#`, the directory (your home as `~`), your last five commands, and for a command that was not found the last 40 lines it printed. Anything that looks like a password, token or key is replaced first. With a sentence (`#`) the command lists go along too (below): one line for each program that has one, and the commands of the programs the sentence or your last commands name.

**Where the model is** (`ai.provider`):

| Provider | What it is | What else to set |
|---|---|---|
| `anthropic` | Claude, Anthropic's service | `ai.model`, `ai.api_key_env` |
| `openai` | OpenAI's service | `ai.model`, `ai.api_key_env` |
| `openai-compatible` | Any service with OpenAI's chat-completions shape: llama.cpp's server, LocalAI, Jan, OpenRouter, DeepSeek | `ai.model`, `ai.base_url` (as `http://localhost:8080/v1`); `ai.api_key_env` if it wants a key |
| `ollama` | Ollama on this machine, at `http://localhost:11434/v1` | `ai.model` (as `qwen2.5-coder:1.5b`) |
| `lm-studio` | LM Studio's server, at `http://localhost:1234/v1` | `ai.model` |
| `vllm` | vLLM, at `http://localhost:8000/v1` | `ai.model` |
| `claude-code` | Claude Code's command line (`claude`), as you are logged in to it | nothing; `ai.model` to name a model |
| `codex` | Codex's command line (`codex`) | nothing; `ai.model` to name a model |
| `pi` | pi's command line (`pi`) | nothing; `ai.model` to name a model |
| `command` | A program of your own | `ai.command`, as `["my-agent", "--quiet"]` |

UnifyTerm brings no model and downloads none. A service is asked for JSON that fits the answer's shape (`response_format: json_schema`); one that refuses that is asked for plain JSON, then without `response_format`, and the way that worked is kept while the window is open.

An agent's command line is started once for each question, with its tools off (Claude Code `--tools ""`, Codex `--sandbox read-only`, pi `--no-tools`), no session kept, in an empty directory of its own under UnifyTerm's data: it can answer, and nothing else. It takes 10 to 40 seconds and counts against your plan with that agent, so it gives corrections, explanations and commands for sentences, but no grey suggestions. It gets 90 seconds; then it is stopped. A program of your own (`command`) gets the question on its standard input, or in place of `{prompt}` in an argument, and prints the answer; what it may do is its own business. Cursor's command line is not among these: its documentation says that print mode writes files whatever the flags, so it cannot be made to only answer.

**A command for a sentence.** With a model set, type `#` and what you want at the prompt, then Enter: `# the five largest files here`. UnifyTerm asks the model and puts its command on the input line in place of the sentence. It is not run: read it, change it, press Enter. Ctrl-Shift-E explains it. With `ai.ask_enter = "auto"` a command that only reads is run at once; "only reads" means every program on the line is one of a short list (`ls`, `cat`, `grep`, `find` without `-delete` or `-exec`, `git status`, PowerShell's `Get-…` and the like) and the line writes nothing (`>`), runs nothing inside itself (`$(…)`, `{…}`) and nothing in the background. Anything else waits for your Enter, and so does every command in cmd.exe. With no model set, or `ai.ask = false`, such a line goes to the shell as before, which takes it as a comment.

**Command lists.** A program can say what its commands, options and arguments are, in a file in the [OpenCLI](https://opencli.org) format (JSON). `ut program add FILE` keeps such a file (`PROGRAM ... | ut program add -` reads it from the program itself). An open window reads it as you type on (it looks at the folder at most every two seconds): no restart. UnifyTerm runs no program to get one, and nothing in a list is ever run. With a program's list, and with no model:

- The list under the input line also has the program's own words for the word you are typing, after the commands from the history, at most 6: its commands (`git st` gives `stash` and `status` among them), its options once you type `-`, and the values an option or an argument takes when the list names them. Beside each is the first sentence of what the list says of it. `history.candidates = false` turns the whole list off.
- When a command of the program fails and one of its words is no command or `--option` the list has, but is one or two keystrokes from one that is, the corrected line is offered (`git comit -m x` gives `git commit -m x`). Enter puts it on the input line, not run. `ai.fix = false` turns it off.
- Ctrl-Shift-E tells what each word of the line is, from the list, at once; nothing is sent. This is for a line that is one command (no pipe, redirection or variable); any other line goes to the model as before, with what the list says of the command used.
- A command the model wrote for a sentence is checked against the list: when it uses a command or `--option` the list does not have, the box says so, and the command is not run even with `ai.ask_enter = "auto"`.

A list is as good as the program made it. A word the list does not have may still be right (a git alias, a plug-in); it is named, never refused. Names in a list that hold a blank or a character a shell reads are left out when the file is read. The OpenCLI project has lists for git, curl, ffmpeg and dotnet among its [examples](https://github.com/spectreconsole/open-cli/tree/main/examples).

A tab starts in your home directory, however UnifyTerm was started: from the Dock, the Start menu, a launcher or another shell. `unifyterm --cwd DIRECTORY` starts the tabs in another one (`unifyterm --cwd .` for the directory you are in).

On macOS a tab's shell is a login shell, as in Terminal.app: zsh and fish get `-l`, PowerShell `-Login`, and bash reads `/etc/profile` and your `~/.bash_profile` (or `~/.bash_login`, `~/.profile`). A program started from the Finder has only the system's short `PATH`; the profile files are where Homebrew's and your own directories come from. On Linux and Windows the shell is started plainly, as other terminals there do.

After a shell's start files have run, UnifyTerm's commands are put first in `PATH` again (bash, zsh, fish, PowerShell): your files may add directories, but `cat` stays UnifyTerm's. `commands.system` names the commands you want from the system instead.

## Environment variables

| Variable | What it does |
|---|---|
| `UNIFYTERM_CONFIG` | The settings file `ut setting` changes, when it is not the default one. UnifyTerm sets it in its tabs when it was started with `--config` |
| `UNIFYTERM_DATA_DIR` | Keep UnifyTerm's data (history, log, command links, shell hooks) in this directory instead of the system's |
| `UNIFYTERM_LOG` | `info` or `debug`: the log file (`unifyterm.log` in the data directory) also gets what is not a warning, as what the system was told about the window. For looking into a problem |
| `UNIFYTERM_RAW_LOG` | A file to append every byte the first tab's program prints to (later tabs: `.2`, `.3`); `unifyterm --replay FILE` shows the screen they draw |
| `UT_NATIVE_PATHS` | On Windows: `ut`'s commands print paths with Windows' own `\` |
| `UT_NO_GLOB` | On Windows: `ut`'s commands keep every wildcard as typed instead of expanding it |

The variable named by `ai.api_key_env` holds the model service's key. `ut view` and `ut jq` follow `NO_COLOR` (no colors when it is set); `ut jq` takes its colors from `JQ_COLORS`, as jq does.

## The commands: `ut`

`ut --list` names them (99 on Windows, 114 on Linux): coreutils, findutils, `grep`, `sed`, `jq`, `diff`, `which` and others. In a UnifyTerm shell they are on `PATH` under their own names; elsewhere run them as `ut cat FILE`, or put them in every shell with `ut shell enable`.

**In every shell** (keepane's panes, another terminal, an agent's shell): `ut shell enable` puts a line at the end of each shell's start file (PowerShell's profile, `.zshrc`, `.bashrc`, fish's `config.fish`) between two marked lines, and on Windows the commands' directory first in your `PATH`. The line loads a script in UnifyTerm's data directory that puts the directory first in `PATH`; in PowerShell it also takes `cat`, `ls` and the other aliases of the same names out of the way, once in each shell. A shell that is open keeps what it has: open a new one (a new keepane pane). `ut shell disable` takes out only what enable put in; `ut shell status` says where it is enabled. After changing `commands.system`, run `ut shell enable` again.

| Command | What it does |
|---|---|
| `ut history recent [-n N] [--cwd DIR] [--long]` | The commands last run |
| `ut history search TEXT` | Commands that begin with or contain TEXT |
| `ut history forget TEXT` | Remove a command from the history |
| `ut view FILE [--mode M] [--width N]` | A picture, or a video with ffmpeg (no sound; `q` stops it) |
| `ut md [FILE]` | Markdown as styled text |
| `ut mcp` | An MCP server on standard input and output (below) |
| `ut skill [--install DIR]` | The agents' skill file, printed or written to `DIR/unifyterm/SKILL.md` |
| `ut shell enable\|disable\|status` | These commands in every shell, not only in UnifyTerm's tabs (above) |
| `ut program` | The programs whose command lists UnifyTerm has: name, version, how many commands and options |
| `ut program add FILE\|-` | Check a command list and keep it (`-` reads standard input) |
| `ut program check FILE\|-` | Check it only: for who writes one |
| `ut program show NAME [WORD...]` | A program's commands, or one command with its options and arguments |
| `ut program remove NAME` | Forget a program's list |
| `ut program directory` | Print the folder the lists are kept in |
| `ut theme` | The themes there are, with a sample of each one's colors; `*` marks the one in use |
| `ut theme NAME` | Use the theme NAME: it is read first, and one that is wrong is not written |
| `ut theme show NAME` | Print a theme's file |
| `ut theme new NAME [--from NAME]` | Begin a theme of your own: a copy of the theme in use, or of the one named |
| `ut theme check FILE` | Check a theme file only: for who writes one |
| `ut theme directory` | Print the folder your own themes are in |

## For agents

- **The sidebar** (Ctrl-Shift-A) shows the agents in keepane's panes (keepane is a terminal multiplexer): model, cost, context, CPU, memory. It asks keepane only while it shows.
- **`ut mcp`** gives an agent four tools: `history_search`, `history_recent`, `list_commands`, `command_help`. It reads the history and runs nothing. Register it with Claude Code: `claude mcp add unifyterm -- ut mcp`.
- **The skill** (`skills/unifyterm/SKILL.md`, or `ut skill`) tells an agent how to work in UnifyTerm.

## Where things are

| What | Windows | macOS | Linux |
|---|---|---|---|
| Settings | `%APPDATA%\unifyterm\config.toml` | `~/Library/Application Support/unifyterm/config.toml` | `~/.config/unifyterm/config.toml` |
| Your own themes | `%APPDATA%\unifyterm\themes` | `~/Library/Application Support/unifyterm/themes` | `~/.config/unifyterm/themes` |
| History, log, links, command lists (`programs/`) | `%LOCALAPPDATA%\unifyterm` | `~/Library/Application Support/unifyterm` | `~/.local/share/unifyterm` |

The history is one SQLite file, `history.db`; a copy works on every system. `update.json` beside it keeps what GitHub last said about the latest version (`updates.check`): once a day the window asks `api.github.com` for it, telling it only `unifyterm/VERSION`.

## Known limits

- No sound with video; ffmpeg must be installed for video.
- UnifyTerm draws kitty's graphics protocol; sixel and iTerm2 images are for `ut view` in other terminals.
- PowerShell's Tab completion still writes paths with `\`; `ls -R` and `find -exec` print uutils' own `\` paths on Windows.
- The shell hooks (history, the list while typing) need PowerShell 7 or 5.1, cmd, bash, zsh or fish.
- macOS: CI builds it, runs the tests and starts it, and it is checked on one Mac; it has been used little there by hand. 0.1.0 did not start a login shell there (`PATH` lacked Homebrew) and started tabs in `/`: both fixed in 0.1.1.
- On macOS with `history.enabled = false`, zsh, bash and fish have no start file of UnifyTerm's, so the system's directories come before UnifyTerm's commands in `PATH`.
- The agent sidebar finds `keepane` in UnifyTerm's own `PATH` or, on macOS and Linux, where your login shell finds it (`$SHELL -l -c 'command -v keepane'`, asked once). A `PATH` that only your `.zshrc` sets is not seen: then give `agents.command` keepane's full path.
- On Windows the command links are hard links to `ut.exe`: after an update they lead to the old one until UnifyTerm opens a window or `ut shell enable` runs again.
- Where the model is: Claude Code and Codex were tried with their real answers (a command with a fitting program for 30 of 30 sentences with Claude Code, 10 of 10 with Codex). The services (`openai`, `ollama`, `lm-studio`, `vllm`) are written to their documents and tried against a stand-in on this machine, not yet against the real thing; `pi` is written to its documents. No model is right every time: read a command before you run it.
- A correction from an agent's command line comes 10 to 40 seconds after the command that was not found, and each one starts the agent once: `ai.fix = false` turns that off.
- Command lists: a list tells the commands a program has, not what is on this machine now, so there are no session names, branch names or file names in what is offered. OpenCLI is a draft (version 0.1); UnifyTerm reads the root shape of 2026-04 and the one before it. The lists tried: git, curl, ffmpeg and dotnet from the OpenCLI project's examples, and small ones written for the tests.
- `window.tab_bar = "auto-hide"`: while the row is out of the way, the window's buttons and the part that moves the window are out of the way with it; the pointer at the top edge brings them back. On macOS the system's three buttons are hidden with the row and shown with it (0.6.0; before, the row kept out of the way in full screen only).
- `theme.opacity`, `theme.blur` and `theme.corners` were measured on Windows 11, on the screen itself (behind a half-opaque window, an even mix of the two colors; stripes behind it blurred; the window's corner cut by 1, 2 or 3 pixels). Opacity and blur have not been looked at on macOS or Linux: on Linux they need a compositor, and X11 has no blur. `theme.corners` does nothing but on Windows 11.
- The picture at the foot of the sidebar is read again when the settings or the theme's file change, not when only the picture's own file is written anew. It is not shown in a window less than 480 pixels high.
- The row of tabs, the lists, the boxes and the sidebar were redrawn in 0.5.0 and looked at on Windows only: macOS and Linux build them and run the tests.
- The white-text rule for light themes covers white and bright white on the theme's background. A program that writes another color the theme has made light is left as it wrote it.
- The builds are not signed.

## License

MIT (`LICENSE`). The licenses of everything built in are in `THIRD-PARTY-NOTICES`.
