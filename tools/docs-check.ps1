# README.md against the code: every setting, key and environment variable the
# code has is in the README's tables, and the README names none the code lacks.
#
#   pwsh tools/docs-check.ps1        prints the counts; exit 1 on a difference
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$readme = [IO.File]::ReadAllText((Join-Path $root 'README.md'))
$read = { param($path) [IO.File]::ReadAllText((Join-Path $root $path)) }
$failed = 0
function Compare-Sets([string]$what, [string[]]$code, [string[]]$documented) {
    $missing = @($code | Where-Object { $_ -notin $documented })
    $extra = @($documented | Where-Object { $_ -notin $code })
    $ok = $missing.Count -eq 0 -and $extra.Count -eq 0
    if (-not $ok) { $script:failed++ }
    "{0}  {1,-22} code {2}, README {3}{4}{5}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $what, $code.Count, $documented.Count,
        $(if ($missing) { "; not in the README: $($missing -join ', ')" }), $(if ($extra) { "; not in the code: $($extra -join ', ')" })
}

# Settings: each `pub(crate) key:` of each `...Config` section in config.rs.
$config = & $read 'crates/config/src/lib.rs'
$settings = foreach ($section in [regex]::Matches($config, '(?s)pub struct (\w+)Config \{(.*?)\n\}')) {
    if ($section.Groups[1].Value -eq '') { continue }
    foreach ($key in [regex]::Matches($section.Groups[2].Value, 'pub (\w+):')) { "$($section.Groups[1].Value.ToLower()).$($key.Groups[1].Value)" }
}
$settingsTable = ($readme -split '## Settings')[1].Split('## Environment')[0]
$documentedSettings = [regex]::Matches($settingsTable, '(?m)^\| `([a-z_]+\.[a-z_]+)` \|') | ForEach-Object { $_.Groups[1].Value }
Compare-Sets 'settings' @($settings) @($documentedSettings)

# Keys: the window's (key_bindings in main.rs) and the terminal view's (terminal.rs).
$main = & $read 'crates/unifyterm/src/main.rs'
$bindings = ($main -split 'fn key_bindings')[1].Split('/// Lets a release build')[0]
$spelled = { param($key) ($key -split '-' | ForEach-Object { switch ($_) { 'ctrl' { 'Ctrl' } 'shift' { 'Shift' } 'cmd' { 'Cmd' } 'alt' { 'Alt' } 'insert' { 'Insert' } 'tab' { 'Tab' } 'enter' { 'Enter' } default { $_.ToUpper() } } }) -join '-' }
$windowKeys = [regex]::Matches($bindings, '\("([a-z0-9-]+)", \d\)') | ForEach-Object { & $spelled $_.Groups[1].Value } | Sort-Object -Unique
$terminal = & $read 'crates/unifyterm/src/terminal.rs'
$viewKeys = @()
if ($terminal -match 'key\.key == "e" && held\.control && held\.shift') { $viewKeys += 'Ctrl-Shift-E' }
foreach ($m in [regex]::Matches($terminal, '"([np])" if held\.control')) { $viewKeys += "Ctrl-$($m.Groups[1].Value.ToUpper())" }
foreach ($m in [regex]::Matches($terminal, '"(down|up|tab|right|enter|escape)"(?: \| "(right)")? if plain')) { foreach ($g in 1, 2) { if ($m.Groups[$g].Success) { $viewKeys += @{ down = 'Down'; up = 'Up'; tab = 'Tab'; right = 'Right'; enter = 'Enter'; escape = 'Esc' }[$m.Groups[$g].Value] } } }
$keysTable = ($readme -split '## Keys')[1].Split('## Settings')[0]
$codeKeys = @($windowKeys) + @($viewKeys | Sort-Object -Unique)
$documentedKeys = $codeKeys | Where-Object { $keysTable -match "(^|[\s|,])$([regex]::Escape($_))([\s|,]|$)" }
# The other way: every key named in the table's first two columns is one the code has.
$named = [regex]::Matches($keysTable, '(?m)^\| ([^|]+) \| ([^|]+) \|') | Where-Object { $_.Groups[1].Value -ne 'Key' -and $_.Groups[1].Value -notmatch '^-+$' } | ForEach-Object { ($_.Groups[1].Value + ',' + $_.Groups[2].Value) -split ',\s*' } | ForEach-Object { $_.Trim() } | Where-Object { $_ -and $_ -notmatch '^-+$' } | Sort-Object -Unique
$documentedAll = @($documentedKeys) + @($named | Where-Object { $_ -notin $codeKeys })
Compare-Sets 'keys' @($codeKeys | Sort-Object -Unique) @($documentedAll | Sort-Object -Unique)

# Environment variables the code reads, less the ones that are not unifyterm's own.
# Not unifyterm's own: the system's, an internal one, and conventions other programs share (named in a sentence below the table).
$notOwn = 'PATH', 'PATHEXT', 'PROMPT', 'SHELL', 'ZDOTDIR', 'UNIFYTERM_COMMANDS', 'UNIFYTERM_LOGIN'
$conventions = 'NO_COLOR', 'JQ_COLORS'
$sources = Get-ChildItem (Join-Path $root 'crates') -Recurse -Include *.rs | Where-Object { $_.FullName -notmatch '[\\/]tests[\\/]' -and $_.Name -ne 'build.rs' } | ForEach-Object { [IO.File]::ReadAllText($_.FullName) }
$variables = foreach ($text in $sources) {
    foreach ($m in [regex]::Matches($text, 'env::var(?:_os)?\("([A-Z_]+)"\)|const [A-Z_]+: &str = "((?:UNIFYTERM|UT)_[A-Z_]+)"|get\("([A-Z_]+)"\)')) { ($m.Groups[1..2] | Where-Object Success | Select-Object -First 1).Value }
}
# `get(...)` in ut view reads how the terminal names itself (TERM_PROGRAM and the like): the terminal's, not ours.
$terminalSays = 'TERM', 'TERM_PROGRAM', 'COLORTERM', 'WT_SESSION', 'KITTY_WINDOW_ID', 'LANG', 'LC_ALL', 'LC_CTYPE', 'WEZTERM_EXECUTABLE', 'GHOSTTY_RESOURCES_DIR'
$ownVariables = @($variables | Sort-Object -Unique | Where-Object { $_ -notin $notOwn -and $_ -notin $conventions -and $_ -notin $terminalSays })
$envTable = ($readme -split '## Environment variables')[1].Split('## The commands')[0]
$documentedVariables = [regex]::Matches($envTable, '(?m)^\| `([A-Z_]+)` \|') | ForEach-Object { $_.Groups[1].Value }
Compare-Sets 'environment variables' $ownVariables @($documentedVariables)
$readConventions = @($conventions | Where-Object { $_ -in $variables })
$said = @($readConventions | Where-Object { $envTable.Contains('`' + $_ + '`') })
Compare-Sets 'shared conventions' $readConventions $said

if ($failed) { "$failed failed"; exit 1 }
'0 failed'
