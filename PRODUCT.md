# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Static HTML, CSS and a little JavaScript, no framework and no build step: the same shape as keepane's site (the user asked for "a page like keepane's"). Fonts and pictures ship with the page. Published with GitHub Pages from the repository's `site/` folder to `dfine.tech/unifyterm/` (confirmed 2026-11-18).

## Users

Developers who work on two or three of Windows, macOS and Linux (confirmed 2026-11-18 as the primary reader). Their situation: they change machines and the commands, the paths and the shell history change with them. They read English first; a Chinese version of the same content must be one click away.

## Product Purpose

UnifyTerm is a GUI terminal that behaves the same on Windows, macOS and Linux. It runs the system's own shell (PowerShell, cmd, bash, zsh, fish) and adds what differs between systems. Success for a visitor: they understand what is the same everywhere, see it working in real pictures, and install it with one command.

## Positioning

The same Unix commands on every system, Windows included, without WSL or Git Bash: `cat`, `grep`, `jq`, `find`, `sed` and the rest (99 on Windows, 114 on Linux) come with the terminal as one program, `ut`, and sit in front of PowerShell's aliases. Paths are printed with `/` everywhere.

## Operating Context

- Installed with `brew install newdee/tap/unifyterm`, `scoop install <manifest url>`, one line of `curl ... | sh` or `irm ... | iex`, or a package (MSI, .deb, AppImage, .dmg).
- Used beside keepane (the same author's terminal multiplexer), which handles panes, agents, messages between panes and observability; UnifyTerm does not repeat those.
- Source: https://github.com/newdee/unifyterm, MIT.

## Capabilities and Constraints

Confirmed, each with checks in `docs/verification-log.md`:

- `ut`: the Unix commands as one program; wildcards expand as in a Unix shell; paths with `/`.
- A command history in SQLite (directory, exit code, duration; secrets are not kept) and a list of matches while typing (Ctrl-N/P, arrows, Tab, Enter fills without running).
- Help with commands: a local fix for a misspelt command always; with a model the user names and agrees to once, a fix, an explanation of each part (Ctrl-Shift-E) and a grey suggestion.
- Pictures (kitty graphics protocol), video through ffmpeg (no sound), markdown: `ut view`, `ut md`.
- For agents: a sidebar of the agents keepane watches (Ctrl-Shift-A), an MCP server (`ut mcp`, four read-only tools), a skill file.
- Written in Rust on GPUI.

Limits that must be said, not hidden: the builds are not signed; macOS has only been built and started in CI, not used by hand; no sound with video; PowerShell's Tab completion still writes `\` paths.

Not part of the product: an own shell (a separate project, UnifyShell, not started), communication and observability (keepane's).

## Brand Commitments

- Name: UnifyTerm where people read it, `unifyterm` where machines do. The command is `ut`.
- Logo: a U on its side, open to the left (UnifyTerm's U, and a prompt's `>`), in cream #cecabe, then a red underline cursor #dd625e, on warm black #171716: the colours of paper-dark, the default theme. Files in `docs/brand/`; `mark-small.svg` (thicker, to the edge) is drawn for 32 px and down.
- Voice (user, 2026-11-18): plain, the way a person says it; no marketing tone, nothing that reads as machine-written; well ordered.
- Author and copyright holder: newdee.

## Evidence on Hand

- Real pictures of the program: `tools/promo.ps1` drives UnifyTerm itself and records it (`docs/media/unifyterm-promo.mp4`, `.gif`); `tools/shot.ps1` takes single pictures.
- Numbers that are measured and may be quoted: 99 commands on Windows, 114 on Linux; a history search among 100 000 commands at p95 under 5 ms; 207 tests; 110 checks that drive the real window.
- There are no users' quotes, no download counts, no stars to cite: none may be invented.

## Product Principles

1. The same on every system: a difference between systems is a bug to remove, not a feature to document.
2. The system's own shell stays: UnifyTerm adds to it and replaces nothing the user chose.
3. Nothing leaves the machine unless the user named where it goes and agreed.
4. Say what is not done.

## Accessibility & Inclusion

Text contrast of 4.5:1 or more in both themes; the page works without JavaScript (the language and theme switches are the only things that need it); reduced motion is honoured.
