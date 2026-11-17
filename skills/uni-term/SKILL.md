---
name: uni-term
description: Working in uni-term, the terminal with the same Unix commands on Windows, macOS and Linux and a searchable command history. Use when the shell you run in is uni-term's (TERM_PROGRAM=uni-term), or when asked to look up commands run before.
---

# Working in uni-term

uni-term is a terminal. Its shells are the system's own (PowerShell, cmd, bash, zsh, fish); what it adds is below.

## The same commands everywhere

On Windows too, these run as on Linux: `cat`, `ls`, `cp`, `mv`, `rm`, `mkdir`, `head`, `tail`, `sort`, `uniq`, `wc`, `cut`, `tr`, `sed`, `grep`, `find`, `xargs`, `diff`, `jq`, `which` and the rest of coreutils (99 commands on Windows, 114 on Linux; `ut --list` names them).

- In PowerShell, `cat`, `ls`, `rm` and the like are these programs, not PowerShell's aliases.
- Paths are printed with `/` (`C:/Users/me/x`). Windows reads them as well as `\`.
- A wildcard (`*.txt`) becomes the names it matches, as a Unix shell does it. In quotes (`"*.txt"`) it stays as typed: quote a pattern meant for `find -name` or `grep`.
- There is no `awk` and no `rg`. Use `grep`, `sed`, `jq` or `cut`.
- `ut view FILE` shows a picture; `ut md FILE` shows markdown.

## The command history

Every command a shell in uni-term ran is kept: its text, directory, exit code and how often it ran. Secrets (passwords, tokens, keys) are not kept.

- From a shell: `ut history search TEXT`, `ut history recent [--cwd DIR] [--long]`.
- From an agent: the MCP server `ut mcp` (register it once: `claude mcp add uni-term -- ut mcp`).

| tool | use it for |
|------|------------|
| `history_search` | how something was done before: "the command that built the release", "how did I run the tests" (`query`, and `cwd` to rank that directory's commands first) |
| `history_recent` | what was just run, here (`cwd`) or anywhere |
| `list_commands` | whether a command is there before using it |
| `command_help` | how one of these commands is used, without running it |

The MCP server runs no commands: use your own shell for that.

## Do not

- Do not assume `awk`, `rg`, `bat` or other programs beyond the list: check `list_commands` or `which NAME`.
- Do not rewrite a path with `/` into `\` for these commands: both work, `/` is the house style.
