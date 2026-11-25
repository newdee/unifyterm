# Checks the built unifyterm by running it: types into its window, reads back
# what its programs printed (UNIFYTERM_RAW_LOG, replayed to text) and compares
# pictures of the window. Each check prints PASS or FAIL with the numbers it
# measured; the exit code is how many failed.
#
#   pwsh tools/verify.ps1 [-Section all|input|settings|themes|edges|commands|history|ai|programs|graphics|agents] [-Exe path]
#
# It needs a desktop session (a window opens for each check, a few seconds
# each) but not the foreground: input and pictures go through the window's
# own handle.
param(
    [ValidateSet('all', 'input', 'settings', 'themes', 'edges', 'commands', 'history', 'ai', 'programs', 'graphics', 'agents')][string]$Section = 'all',
    [string]$Exe = "$PSScriptRoot\..\target\debug\unifyterm.exe"
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$Exe = (Resolve-Path $Exe).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "unifyterm-verify-$PID"
# The user's own history file, as it is before the checks: there or not, how long, when written.
# (This machine's owner may use UnifyTerm: the file being there says nothing; its changing would.)
$realHistory = Join-Path $env:LOCALAPPDATA 'unifyterm\history.db'
$StampOf = { param($file) if (Test-Path $file) { $i = Get-Item $file; "$($i.Length) bytes, written $($i.LastWriteTimeUtc.ToString('o'))" } else { 'not there' } }
$realHistoryBefore = & $StampOf $realHistory
# unifyterm's data (command links, start scripts, history, log) for these checks only:
# never the user's own. Every unifyterm started below inherits it.
$env:UNIFYTERM_DATA_DIR = Join-Path $work 'data'
New-Item -ItemType Directory -Force $work | Out-Null
$env:NO_COLOR = $null   # PowerShell drops colors from its output when this is set.
$script:failed = 0

# The text of the screen that a raw log draws. The program is waited for and
# its output read from a file: PowerShell does not wait for a window program
# (a release build on Windows) or read its pipe.
function Replay([string]$log, [string]$size) {
    $out = Join-Path $work 'replay.out'
    $p = Start-Process $Exe -ArgumentList "--replay `"$log`" --size $size" -RedirectStandardOutput $out -Wait -PassThru -NoNewWindow
    if ($p.ExitCode -ne 0) { return '' }
    (Get-Content -Raw -Encoding utf8 $out) ?? ''
}

function Report([string]$name, [bool]$ok, [string]$detail) {
    if (-not $ok) { $script:failed++ }
    "{0}  {1,-44} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail
}

# Typed first in every PowerShell these checks start. PowerShell would write each typed
# line to the user's own history file; after this line it writes none. The line itself is
# not written either: PSReadLine keeps a line with the word "password" in it out of the file.
# The screen is cleared, so what follows starts in the first row as it did without this line.
$NoHistory = "Set-PSReadLineOption -HistorySaveStyle SaveNothing; cls # password: a test's lines stay out of your history{ENTER}{WAIT:700}"
# The same without clearing the screen, for a check that reads what unifyterm said before the shell started.
$NoHistoryKeep = $NoHistory.Replace('; cls ', ' ')
# After one typed line: the same for a shell that did not get the line above (a second tab, a shell fallen back to).
$Unsaved = ' # password: not for your history'

# Runs unifyterm with a settings file's text, types, and saves a picture.
# Returns the text its programs' output draws (replayed at a size big
# enough that nothing scrolls away), or with -Screen at the window's size.
function Run([string]$name, [string]$type, [string]$toml = '', [string]$more = '', [double]$after = 1, [string]$screen = '', [switch]$keep) {
    $config = Join-Path $work "$name.toml"; $log = Join-Path $work "$name.raw"
    # Every PowerShell (the default shell, or pwsh / powershell named in the settings) is told
    # first; another program (cmd.exe, nvim) keeps no PowerShell history: nothing to say to it.
    $named = [regex]::Match($toml, '(?m)^program\s*=\s*"([^"]*)"').Groups[1].Value
    $powershell = ($named -eq '' -or $named -match '(^|[\\/])(pwsh|powershell)(\.exe)?$') -and $more -notmatch '(^|\s)-e\s'
    if ($powershell) { $type = $(if ($keep) { $NoHistoryKeep } else { $NoHistory }) + $type }
    Set-Content $config $toml -Encoding utf8
    Remove-Item "$log*" -ErrorAction SilentlyContinue
    $env:UNIFYTERM_RAW_LOG = $log
    try {
        & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work "$name.png") -Type $type -Wait 4 -After $after -App:" --config `"$config`" $more" | Out-Null
    } finally { $env:UNIFYTERM_RAW_LOG = $null }
    if (-not (Test-Path $log)) { return '' }
    $size = if ($screen) { $screen } else { '250x2000' }
    # Each tab has its own log (name, name.2, name.3): each is its own screen.
    $logs = @($log) + @(Get-ChildItem $work | Where-Object { $_.Name -match "^$([regex]::Escape($name))\.raw\.\d+$" } | Sort-Object Name | ForEach-Object FullName)
    ($logs | ForEach-Object { Replay $_ $size }) -join "`n"
}

# The most a color channel may differ for two pixels to count as the same.
# The GPU's blending of a glyph's edge rounds one way or the other from run
# to run (measured: 1 of 255, in 2 pixels, in 4 of 11 runs whose output
# bytes were identical), so exact equality would fail by chance.
$ChannelTolerance = 2

# How many pixels differ between two pictures inside a rectangle.
function PixelDiff([string]$a, [string]$b, [int]$x, [int]$y, [int]$w, [int]$h) {
    $ia = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$a.png")); $ib = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$b.png")); $n = 0
    try {
        for ($j = $y; $j -lt $y + $h; $j++) {
            for ($i = $x; $i -lt $x + $w; $i++) {
                $p = $ia.GetPixel($i, $j); $q = $ib.GetPixel($i, $j)
                if ([Math]::Abs($p.R - $q.R) -gt $ChannelTolerance -or [Math]::Abs($p.G - $q.G) -gt $ChannelTolerance -or [Math]::Abs($p.B - $q.B) -gt $ChannelTolerance) { $n++ }
            }
        }
    } finally { $ia.Dispose(); $ib.Dispose() }
    $n
}
# How many pixels of a picture's rectangle are a color, within the tolerance.
function CountColor([string]$a, [string]$hex, [int]$x, [int]$y, [int]$w, [int]$h) {
    $want = [Convert]::ToInt32($hex.TrimStart('#'), 16); $r = ($want -shr 16) -band 255; $g = ($want -shr 8) -band 255; $b = $want -band 255
    $i = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$a.png")); $n = 0
    try {
        # Within the picture: a smaller window gives a smaller one.
        for ($j = $y; $j -lt [Math]::Min($y + $h, $i.Height); $j++) {
            for ($k = $x; $k -lt [Math]::Min($x + $w, $i.Width); $k++) {
                $p = $i.GetPixel($k, $j)
                if ([Math]::Abs($p.R - $r) -le $ChannelTolerance -and [Math]::Abs($p.G - $g) -le $ChannelTolerance -and [Math]::Abs($p.B - $b) -le $ChannelTolerance) { $n++ }
            }
        }
    } finally { $i.Dispose() }
    $n
}
function PixelAt([string]$a, [int]$x, [int]$y) {
    $i = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$a.png"))
    try { '#{0:x6}' -f ($i.GetPixel($x, $y).ToArgb() -band 0xFFFFFF) } finally { $i.Dispose() }
}
function Count([string]$text, [string]$needle) { ([regex]::Matches($text, [regex]::Escape($needle))).Count }
# unifyterm processes and the console hosts started from its folder.
function Leftovers { @(Get-Process unifyterm, OpenConsole -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith((Split-Path $Exe)) }).Count }

# The first text row of the window, in window pixels (title bar and tab bar above it).
$Row = @{ Y = 78; H = 22 }

if ($Section -in 'all', 'input') {
    "--- input ---"
    $t = Run 'typed' "echo marker-123{ENTER}{WAIT:600}echo 中文输入-ok{ENTER}"
    Report 'typed text reaches the shell' ((Count $t 'marker-123') -eq 2) "'marker-123' seen $(Count $t 'marker-123') times (the command and its output)"
    Report 'Chinese text reaches the shell' ((Count $t '中文输入-ok') -eq 2) "'中文输入-ok' seen $(Count $t '中文输入-ok') times"

    $t = Run 'ctrlc' "Start-Sleep 60; 'slept-through'{ENTER}{WAIT:1200}{KEY:CTRL+C}{WAIT:800}echo after-interrupt{ENTER}"
    Report 'Ctrl-C interrupts the running command' ((Count $t 'slept-through') -eq 1 -and (Count $t 'after-interrupt') -eq 2) "'slept-through' $(Count $t 'slept-through') (typed only), 'after-interrupt' $(Count $t 'after-interrupt')"

    $t = Run 'tabs' "echo tab-one{ENTER}{WAIT:500}{KEY:CTRL+SHIFT+T}{WAIT:2500}echo tab-two$Unsaved{ENTER}{WAIT:600}{KEY:CTRL+TAB}{WAIT:500}echo back-in-one{ENTER}{WAIT:500}{KEY:CTRL+SHIFT+W}{WAIT:700}echo left-in-two$Unsaved{ENTER}"
    $all = @('tab-one', 'tab-two', 'back-in-one', 'left-in-two' | ForEach-Object { Count $t $_ })
    Report 'tabs: open, switch, close by keys' (($all | Where-Object { $_ -ne 2 }).Count -eq 0) "each of 4 markers seen: $($all -join ', ') times"

    $t = Run 'resize' "{SIZE:700,420}{WAIT:900}echo `"cols=`$(`$Host.UI.RawUI.WindowSize.Width) rows=`$(`$Host.UI.RawUI.WindowSize.Height)`"{ENTER}"
    $m = [regex]::Match($t, '(?m)^cols=(\d+) rows=(\d+)')
    Report 'a smaller window gives the shell a smaller screen' ($m.Success -and [int]$m.Groups[1].Value -lt 100 -and [int]$m.Groups[2].Value -lt 30) "the shell says $($m.Value) (the window starts at 115x30)"

    $saved = Get-Clipboard -Raw
    try {
        # The clipboard is the whole machine's. When another program writes it while a check
        # runs, the check says nothing about unifyterm: it runs again, three times at most,
        # and the detail says which try it was (more than one: something else used the clipboard).
        $try = 0
        do {
            $try++
            $t = Run 'select' "cls; echo select-me-please{ENTER}{WAIT:600}{DRAG:7,62,134,62}{WAIT:300}{RCLICK:300,300}{WAIT:300}"
            $copied = Get-Clipboard -Raw
        } while ($copied -ne 'select-me-please' -and $try -lt 3)
        Report 'drag selects, right click copies' ($copied -eq 'select-me-please') "the clipboard holds '$copied' (try $try)"
        $try = 0
        do {
            $try++
            Set-Clipboard 'echo pasted-by-click'
            $t = Run 'paste' "{RCLICK:300,300}{WAIT:400}{ENTER}"
            $taken = (Get-Clipboard -Raw) -ne 'echo pasted-by-click'
        } while ((Count $t 'pasted-by-click') -ne 2 -and $taken -and $try -lt 3)
        Report 'right click pastes' ((Count $t 'pasted-by-click') -eq 2) "'pasted-by-click' seen $(Count $t 'pasted-by-click') times (try $try)"
    } finally { if ($null -ne $saved) { Set-Clipboard -Value $saved } }

    $forty = Join-Path $work 'forty.txt'; 1..40 | ForEach-Object { "line $_" } | Set-Content $forty
    $t = Run 'nvim' "{WAIT:500}:set mouse=a{ENTER}{WAIT:300}{CLICK:200,160}{WAIT:300}:echo 'at' line('.'){ENTER}{WAIT:400}{WHEEL:200,160,-1}{WAIT:400}:echo 'top' line('w0'){ENTER}{WAIT:400}" '' "-e nvim --clean `"$forty`"" 0.5 '115x30'
    $last = ($t -split "`n" | Where-Object { $_.Trim() } | Select-Object -Last 1)
    Report 'nvim: full screen, mouse, wheel' ($last -match '^top 4\b' -and $t -match '(?m)^line 4$') "last line: '$last' (one notch of wheel is nvim's 3 lines: the top line goes from 1 to 4)"

    $out = (& pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'exit.png') -Type ($NoHistory + "exit{ENTER}") -Wait 4 -After 3) -join ' '
    Report "'exit' in the last tab ends unifyterm" ($out -match 'exited: 0') "the runner saw: $(if ($out -match 'exited: (-?\d+)') { "ended with code $($Matches[1])" } else { 'still running after 3 s' })"    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) unifyterm or console-host processes still running"
}

if ($Section -in 'all', 'settings') {
    "--- settings ---"
    $line = "cls; echo '-> => != >= <= === 中文字符'{ENTER}"
    Run 'base' $line | Out-Null; Run 'base2' $line | Out-Null
    Run 'nolig' $line "[font]`nligatures = false" | Out-Null
    Run 'kaiti' $line "[font]`nfallbacks = [`"KaiTi`"]" | Out-Null
    $noise = PixelDiff base base2 8 $Row.Y 500 $Row.H
    Report 'the same settings draw the same picture' ($noise -eq 0) "$noise pixels differ by more than $ChannelTolerance/255 between two runs"
    $lig = PixelDiff base nolig 8 $Row.Y 160 $Row.H; $ligCjk = PixelDiff base nolig 172 $Row.Y 70 $Row.H
    Report 'font.ligatures changes -> => != and nothing else' ($lig -gt 100 -and $ligCjk -eq 0) "$lig pixels differ in the symbols, $ligCjk in the Chinese text"
    $fb = PixelDiff base kaiti 172 $Row.Y 70 $Row.H
    Report 'font.fallbacks changes the Chinese text' ($fb -gt 50) "$fb pixels differ with KaiTi as the fallback"

    $rows = "cls; 1..12 | % { `"row `$_`" }{ENTER}"
    Run 'tall' $rows "[font]`nline_height = 1.9" | Out-Null; Run 'short' $rows "[font]`nline_height = 1.0" | Out-Null
    $lh = PixelDiff tall short 8 $Row.Y 200 300
    Report 'font.line_height changes the rows' ($lh -gt 500) "$lh pixels differ between 1.9 and 1.0"

    $t18 = Run 'big' "echo `"cols=`$(`$Host.UI.RawUI.WindowSize.Width)`"{ENTER}" "[font]`nsize = 18"
    $cols = [regex]::Match($t18, '(?m)^cols=(\d+)').Groups[1].Value
    Report 'font.size changes how many columns fit' ($cols -and [int]$cols -lt 100) "cols=$cols at size 18 (115 at size 14)"

    $long = "1..60 | % { `"line `$_`" }{ENTER}{WAIT:1200}{WHEEL:400,300,5}{WAIT:500}"
    Run 'sb' $long | Out-Null; Run 'sb5' $long "[terminal]`nscrollback = 5" | Out-Null
    $sb = PixelDiff sb sb5 8 $Row.Y 200 500
    Report 'terminal.scrollback limits how far back the wheel goes' ($sb -gt 500) "$sb pixels differ between the default and 5 lines"

    Run 'theme' 'cls{ENTER}' "[theme]`nbackground = `"#0b2a3a`"`n[terminal]`npadding = 30" | Out-Null
    $bg = PixelAt theme 400 400
    Report 'theme.background is the window''s color' ($bg -eq '#0b2a3a') "the pixel at (400,400) is $bg"
    Run 'consolas' $line "[font]`nfamily = `"Consolas`"" | Out-Null
    $family = PixelDiff base consolas 8 $Row.Y 160 $Row.H
    Report 'font.family changes the letters' ($family -gt 100) "$family pixels differ with Consolas"

    Run 'pad' $line "[terminal]`npadding = 30" | Out-Null
    # With 30 pixels of padding nothing is drawn where the text starts by default.
    $strip = PixelDiff base pad 8 $Row.Y 22 $Row.H
    Report 'terminal.padding moves the text' ($strip -gt 20) "$strip pixels differ in the first 22 columns of pixels of the first row"

    # Colors that are in no theme, so a pixel of one can only come from the setting.
    $marks = "[theme]`ncursor = `"#ff0001`"`nselection = `"#00ff01`"`nansi = [`"#000000`", `"#123456`", `"#00aa00`", `"#aaaa00`", `"#0000aa`", `"#aa00aa`", `"#00aaaa`", `"#aaaaaa`", `"#555555`", `"#ff5555`", `"#55ff55`", `"#ffff55`", `"#5555ff`", `"#ff55ff`", `"#55ffff`", `"#ffffff`"]"
    Run 'marks' ("cls; echo `"`$([char]27)[41m          `$([char]27)[0m select-me`"{ENTER}{WAIT:700}{DRAG:100,50,170,50}{WAIT:400}") $marks | Out-Null
    $cursor = CountColor marks '#ff0001' 0 60 600 80; $selection = CountColor marks '#00ff01' 0 60 600 30; $ansi = CountColor marks '#123456' 0 60 600 30
    Run 'fore' $line "[theme]`nforeground = `"#ff0002`"" | Out-Null
    $fore = CountColor fore '#ff0002' 0 60 900 80
    Report 'theme.foreground is the text''s color' ($fore -gt 200) "$fore pixels of #ff0002 in the first rows"
    Report 'theme.cursor is the cursor''s color' ($cursor -gt 50) "$cursor pixels of #ff0001"
    Report 'theme.selection is the selection''s color' ($selection -gt 100) "$selection pixels of #00ff01 after a drag"
    Report 'theme.ansi gives the 16 colors' ($ansi -gt 500) "$ansi pixels of #123456 behind ten cells of ESC[41m"
    Report 'the built-in theme has none of those colors' (((CountColor base '#ff0001' 0 60 600 80) + (CountColor base '#00ff01' 0 60 600 30) + (CountColor base '#123456' 0 60 600 30)) -eq 0) "pixels of the three marker colors without the settings: $((CountColor base '#ff0001' 0 60 600 80) + (CountColor base '#00ff01' 0 60 600 30) + (CountColor base '#123456' 0 60 600 30))"

    $t = Run 'shell' "{WAIT:500}" "[shell]`nprogram = `"cmd.exe`"`nargs = [`"/k`", `"echo from-cmd-shell`"]"
    Report 'shell.program and shell.args start that program' ((Count $t 'from-cmd-shell') -ge 1) "'from-cmd-shell' seen $(Count $t 'from-cmd-shell') times"

    # ut setting, run in a tab, changes the file this window reads, and the window takes
    # the change at once: a larger font leaves fewer columns, a new background is drawn.
    $t = Run 'live' "echo `"file=`$(Split-Path -Leaf `$env:UNIFYTERM_CONFIG) w1=`$([Console]::WindowWidth)`"$Unsaved{ENTER}{WAIT:700}ut setting set font.size 24$Unsaved{ENTER}{WAIT:2500}echo `"w2=`$([Console]::WindowWidth) size=`$(ut setting get font.size)`"$Unsaved{ENTER}{WAIT:700}ut setting set theme.background 102030$Unsaved{ENTER}{WAIT:2500}ut setting set font.size 500$Unsaved{ENTER}{WAIT:800}" "# kept`n[font]`nsize = 14" -after 1.5
    $flat = $t -replace "`n", ' '
    $w1 = [int][regex]::Match($flat, '(?<!")w1=(\d+)').Groups[1].Value; $w2 = [int][regex]::Match($flat, '(?<!")w2=(\d+)').Groups[1].Value
    Report 'a tab''s ut setting changes the window''s own file' ($flat -match 'file=live\.toml' -and $flat -match 'size=24\.0') "the file the tab was told: $([regex]::Match($flat, 'file=[\w.-]+\.toml').Value); font.size read back: $([regex]::Match($flat, 'size=[\d.]+').Value)"
    Report 'a changed font size is used at once' ($w1 -ge 80 -and $w2 -gt 0 -and $w2 -lt $w1 * 0.7) "$w1 columns at 14 pixels, $w2 at 24"
    $ground = CountColor 'live' '#102030' 300 200 400 300
    Report 'a changed background is used at once' ($ground -ge 100000) "$ground of 120000 pixels in the middle of the window are #102030"
    $file = Get-Content (Join-Path $work 'live.toml') -Raw
    Report 'a refused value is not written, and the file keeps its comment' ($flat -match 'font\.size is 500: it must be from 6 to 72' -and $file -match '(?m)^# kept' -and $file -match 'size = 24\.0' -and $file -notmatch '500') "said why: $($flat -match 'it must be from 6 to 72'); comment kept: $($file -match '(?m)^# kept'); size in the file: $([regex]::Match($file, 'size = [\d.]+').Value)"
}

if ($Section -in 'all', 'themes') {
    "--- themes ---"
    # The preview page's colors (demo/appearance.html) as #rrggbb, theme by theme: behind the row of
    # tabs (--bar), behind the tab in use (--well), the mark under it (--amber), behind the text
    # (--screen). The page gives night and day in oklch: converted once, they are these.
    $page = [ordered]@{
        aurora   = @{ bar = '#10252d'; well = '#203c47'; accent = '#a6ffce'; screen = '#0c1c24' }
        eclipse  = @{ bar = '#2b1a21'; well = '#432b32'; accent = '#ffc178'; screen = '#21151b' }
        phosphor = @{ bar = '#152015'; well = '#243324'; accent = '#d9ff8f'; screen = '#101810' }
        riso     = @{ bar = '#f3ead7'; well = '#ebdcc3'; accent = '#9c301d'; screen = '#fbf5e7' }
        night    = @{ bar = '#121b26'; well = '#1c2938'; accent = '#f0be6e'; screen = '#0e1620' }
        day      = @{ bar = '#e8eff6'; well = '#dbe6f3'; accent = '#774000'; screen = '#f4f9fe' }
    }
    # The four the page writes as #rrggbb are read from the page here, so the table cannot drift from it.
    $html = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\demo\appearance.html'))
    $drift = foreach ($name in 'aurora', 'eclipse', 'phosphor', 'riso') {
        $at = $html.IndexOf(":root[data-theme=`"$name`"]"); $block = $html.Substring($at, $html.IndexOf('}', $at) - $at)
        foreach ($pair in @{ bar = 'bar'; well = 'well'; accent = 'amber'; screen = 'screen' }.GetEnumerator()) {
            if ($block -notmatch "--$($pair.Value): $([regex]::Escape($page[$name][$pair.Key]));") { "$name.$($pair.Value)" }
        }
    }
    Report 'the colors checked are the preview page''s' (@($drift).Count -eq 0) "of 16 colors of 4 themes, not as the page has them: $(@($drift).Count) $($drift -join ' ')"
    function SameColor([string]$a, [string]$b) {
        $x = [Convert]::ToInt32($a.TrimStart('#'), 16); $y = [Convert]::ToInt32($b.TrimStart('#'), 16)
        (0, 8, 16 | ForEach-Object { [Math]::Abs((($x -shr $_) -band 255) - (($y -shr $_) -band 255)) } | Measure-Object -Maximum).Maximum -le $ChannelTolerance
    }
    # In a picture of the window: an empty part of the row of tabs, above where the first row of
    # text is when the row of tabs is away; a point of the first tab over its name; a point of the
    # text's background. (The row of tabs is rows 31 to 76 of the picture, the first tab 36 to 71.)
    $InBar = 500, 34; $InTab = 95, 42; $InText = 500, 400
    $quiet = "[updates]`ncheck = false"
    foreach ($name in $page.Keys) {
        Run "theme-$name" '{WAIT:500}' "[theme]`nname = `"$name`"`n$quiet" | Out-Null
        $seen = [ordered]@{ bar = (PixelAt "theme-$name" @InBar); well = (PixelAt "theme-$name" @InTab); screen = (PixelAt "theme-$name" @InText) }
        $mark = CountColor "theme-$name" $page[$name].accent 30 60 40 12
        $ok = @($seen.Keys | Where-Object { -not (SameColor $seen[$_] $page[$name][$_]) }).Count -eq 0 -and $mark -ge 40
        Report "${name}: the window has the page's colors" $ok "bar $($seen.bar) (page $($page[$name].bar)), tab in use $($seen.well) ($($page[$name].well)), text's background $($seen.screen) ($($page[$name].screen)), mark: $mark pixels of $($page[$name].accent)"
    }
    # keepane's own two looks (its themes/paper-dark.conf and paper.conf): the panes' background,
    # the bar a shade darker, and cinnabar under the tab in use.
    $keepane = [ordered]@{
        'paper-dark' = @{ bar = '#121212'; accent = '#dd625e'; screen = '#171716' }
        'paper'      = @{ bar = '#e3d8bf'; accent = '#a53736'; screen = '#f0e7d2' }
    }
    foreach ($name in $keepane.Keys) {
        Run "theme-$name" '{WAIT:500}' "[theme]`nname = `"$name`"`n$quiet" | Out-Null
        $seen = [ordered]@{ bar = (PixelAt "theme-$name" @InBar); screen = (PixelAt "theme-$name" @InText) }
        $mark = CountColor "theme-$name" $keepane[$name].accent 30 60 40 12
        $ok = @($seen.Keys | Where-Object { -not (SameColor $seen[$_] $keepane[$name][$_]) }).Count -eq 0 -and $mark -ge 40
        Report "${name}: the window has keepane's colors" $ok "bar $($seen.bar) (keepane $($keepane[$name].bar)), text's background $($seen.screen) ($($keepane[$name].screen)), mark: $mark pixels of $($keepane[$name].accent)"
    }
    # The default theme gives no colors for what is around the text: they are worked out.
    Run 'theme-default' '{WAIT:500}' $quiet | Out-Null
    Report 'no theme named: Tokyo Night, its bar worked out' ((SameColor (PixelAt 'theme-default' @InText) '#16161e') -and (SameColor (PixelAt 'theme-default' @InBar) '#1c1c26')) "text's background $(PixelAt 'theme-default' @InText) (#16161e), bar $(PixelAt 'theme-default' @InBar) (#1c1c26)"

    # ut theme NAME, in a tab: the window changes at once, the settings file has the name, and a
    # theme that is not there changes neither.
    $t = Run 'theme-switch' "ut theme aurora{ENTER}{WAIT:2500}{SHOT:theme-switched}ut theme nosuch{ENTER}{WAIT:1500}" "# kept`n$quiet" -after 1.5
    $file = Get-Content (Join-Path $work 'theme-switch.toml') -Raw
    Report 'ut theme NAME changes the open window at once' ((SameColor (PixelAt 'theme-switched' @InText) $page.aurora.screen) -and (SameColor (PixelAt 'theme-switched' @InBar) $page.aurora.bar)) "after 'ut theme aurora': text's background $(PixelAt 'theme-switched' @InText), bar $(PixelAt 'theme-switched' @InBar)"
    Report 'the name is kept in the settings file' ($file -match '(?m)^name = "aurora"' -and $file -match '(?m)^# kept') "in the file: $([regex]::Match($file, '(?m)^name = .*').Value.Trim()); its comment kept: $($file -match '(?m)^# kept')"
    Report 'a theme that is not there changes nothing' (($t -replace "`n", ' ') -match 'no theme is called nosuch' -and $file -notmatch 'nosuch' -and (SameColor (PixelAt 'theme-switch' @InText) $page.aurora.screen)) "said so: $(($t -replace "`n", ' ') -match 'no theme is called nosuch'); the window after it: $(PixelAt 'theme-switch' @InText)"
    # Opened again with the file as it was left: the same theme.
    Run 'theme-again' '{WAIT:500}' $file | Out-Null
    Report 'opened again, the window has the theme chosen before' (SameColor (PixelAt 'theme-again' @InText) $page.aurora.screen) "text's background $(PixelAt 'theme-again' @InText) (aurora's is $($page.aurora.screen))"

    # A theme of one's own: begun from another, chosen, and its file changed while the window is open.
    Remove-Item (Join-Path $work 'themes') -Recurse -Force -ErrorAction SilentlyContinue
    $mine = (Join-Path $work 'themes\mine.toml') -replace '\\', '/'
    Run 'theme-own' "ut theme new mine --from night{ENTER}{WAIT:1200}ut theme mine{ENTER}{WAIT:2500}{SHOT:theme-own-chosen}(Get-Content -Raw '$mine').Replace('#0e1620', '#405060') | Set-Content '$mine'{ENTER}{WAIT:4000}" $quiet -after 1.5 | Out-Null
    Report 'a theme of one''s own is begun, chosen, and followed as its file changes' ((Test-Path $mine) -and (SameColor (PixelAt 'theme-own-chosen' @InText) $page.night.screen) -and (SameColor (PixelAt 'theme-own' @InText) '#405060')) "the file is there: $(Test-Path $mine); chosen: $(PixelAt 'theme-own-chosen' @InText) (night's $($page.night.screen)); after the file's background became #405060: $(PixelAt 'theme-own' @InText)"

    # A light theme: white text on the theme's background is drawn in the theme's text color.
    Run 'theme-white' "Write-Host `"``e[37m████████████████████ ``e[97m████████████████████``e[0m`"{ENTER}{WAIT:900}" "[theme]`nname = `"day`"`n$quiet" | Out-Null
    # Under the command's own row: the command may take two rows, and PowerShell leaves one empty.
    $inked = CountColor 'theme-white' '#14253a' 0 ($Row.Y + $Row.H) 500 (4 * $Row.H)
    Report 'a light theme: white words can be read' ($inked -ge 4000) "$inked pixels of the theme's text color (#14253a) in the rows where 40 full blocks were printed in white and bright white (a block is about 8 by 19 pixels)"

    # The picture a theme names: at the foot of the sidebar, cut at the sidebar's sides, and nowhere
    # else. In a picture of the window the sidebar is columns 700 to 967 and the strip rows 471 to
    # 650; the sidebar's own color fades out over the strip's upper half, so the lower part is read.
    $side = "[agents]`nsidebar = true`ncommand = ['unifyterm-no-such-program']`n$quiet"
    $strip = 710, 580, 248, 65; $beside = 560, 471, 138, 175
    function Unlike([string]$name, [string]$hex, [int[]]$at) { $at[2] * $at[3] - (CountColor $name $hex @at) }
    Run 'picture-aurora' '{WAIT:800}' "[theme]`nname = `"aurora`"`n$side" | Out-Null
    $inStrip = Unlike 'picture-aurora' $page.aurora.bar $strip; $left = Unlike 'picture-aurora' $page.aurora.screen $beside
    Report 'a theme''s picture is at the foot of the sidebar, and only there' ($inStrip -ge 10000 -and $left -eq 0) "of $($strip[2] * $strip[3]) pixels of the strip, not the sidebar's own color: $inStrip; of $($beside[2] * $beside[3]) pixels of the text area beside it, not the text's background: $left"
    Run 'picture-none' '{WAIT:800}' $side | Out-Null
    $plainStrip = Unlike 'picture-none' '#1c1c26' $strip
    Run 'picture-off' '{WAIT:800}' "[theme]`nname = `"aurora`"`npicture = `"`"`n$side" | Out-Null
    $offStrip = Unlike 'picture-off' $page.aurora.bar $strip
    Report 'no picture: a theme without one, and one turned off in the settings' ($plainStrip -eq 0 -and $offStrip -eq 0) "pixels of the strip that are not the sidebar's own color: the default theme $plainStrip (a pixel of it: $(PixelAt 'picture-none' 800 600)); aurora with picture = `"`" $offStrip"
    # A theme of one's own names a picture of its own, beside its file: one flat color, made here.
    New-Item -ItemType Directory -Force (Join-Path $work 'themes') | Out-Null
    $flat = [System.Drawing.Bitmap]::new(64, 64); $fg = [System.Drawing.Graphics]::FromImage($flat); $fg.Clear([System.Drawing.Color]::FromArgb(255, 255, 0, 255)); $fg.Dispose()
    $flat.Save((Join-Path $work 'themes\flat.png'), [System.Drawing.Imaging.ImageFormat]::Png); $flat.Dispose()
    $nightFile = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '..\demo\themes\night.toml')).Replace('picture = "night.png"', 'picture = "flat.png"')
    [IO.File]::WriteAllText((Join-Path $work 'themes\pictured.toml'), $nightFile)
    Run 'picture-own' '{WAIT:800}' "[theme]`nname = `"pictured`"`n$side" | Out-Null
    $ownStrip = CountColor 'picture-own' '#ff00ff' @strip
    Report 'a theme of one''s own shows the picture its file names' ($ownStrip -ge 15000) "pixels of the picture's color (#ff00ff) in the strip: $ownStrip of $($strip[2] * $strip[3])"

    # theme.wallpaper: a picture behind the text, as large as the window. The picture made here is
    # magenta on its left half and green on its right: whole (opacity 1) the text area shows the
    # two colors, to the window's far corner; at 0.5 on a black background each is halved; and in
    # a window made wider the two still meet in the middle. The row of tabs keeps its own color.
    $wall = [System.Drawing.Bitmap]::new(400, 200); $wg = [System.Drawing.Graphics]::FromImage($wall)
    $wg.Clear([System.Drawing.Color]::FromArgb(255, 255, 0, 255)); $wg.FillRectangle([System.Drawing.Brushes]::Lime, 200, 0, 200, 200); $wg.Dispose()
    $wall.Save((Join-Path $work 'wall.png'), [System.Drawing.Imaging.ImageFormat]::Png); $wall.Dispose()
    $walled = "[theme]`nbackground = `"#000000`"`nwallpaper = `"wall.png`""
    Run 'wall-whole' '{WAIT:1000}' "$walled`nwallpaper_opacity = 1`n$quiet" | Out-Null
    Run 'wall-half' '{WAIT:1000}' "$walled`nwallpaper_opacity = 0.5`n$quiet" | Out-Null
    Run 'wall-wide' '{WAIT:1000}{SIZE:1400,500}{WAIT:1200}' "$walled`n$quiet" | Out-Null
    $whole = @((PixelAt 'wall-whole' 244 395), (PixelAt 'wall-whole' 732 395), (PixelAt 'wall-whole' 956 639))
    $half = @((PixelAt 'wall-half' 244 395), (PixelAt 'wall-half' 732 395))
    $wide = @((PixelAt 'wall-wide' 350 318), (PixelAt 'wall-wide' 1050 318))
    $okWhole = (SameColor $whole[0] '#ff00ff') -and (SameColor $whole[1] '#00ff00') -and (SameColor $whole[2] '#00ff00')
    $okHalf = (SameColor $half[0] '#7f007f') -and (SameColor $half[1] '#007f00')
    $okWide = (SameColor $wide[0] '#400040') -and (SameColor $wide[1] '#004000')
    $barKept = SameColor (PixelAt 'wall-whole' @InBar) (PixelAt 'wall-half' @InBar)
    Report 'theme.wallpaper: a picture behind the text, as large as the window' ($okWhole -and $okHalf -and $okWide -and $barKept -and -not (SameColor (PixelAt 'wall-whole' @InBar) '#ff00ff')) "whole: left $($whole[0]), right $($whole[1]), far corner $($whole[2]); at 0.5 on black: $($half -join ', '); 1400 wide at the default 0.25: left $($wide[0]), right $($wide[1]); the row of tabs: $(PixelAt 'wall-whole' @InBar)"

    # theme.rounding: the first tab's own corner (its outline is columns 20 on, rows 36 to 71 of the
    # picture). Round, the corner's pixel is the row's color; square, it is the tab's line.
    Run 'round-7' '{WAIT:500}' "[theme]`nname = `"night`"`n$quiet" | Out-Null
    Run 'round-0' '{WAIT:500}' "[theme]`nname = `"night`"`nrounding = 0`n$quiet" | Out-Null
    Report 'theme.rounding makes the tabs'' corners round or square' ((SameColor (PixelAt 'round-7' 20 36) $page.night.bar) -and (SameColor (PixelAt 'round-0' 20 36) '#2b3644')) "the tab's corner pixel: $(PixelAt 'round-7' 20 36) as the theme has it (the row's color is $($page.night.bar)); $(PixelAt 'round-0' 20 36) with rounding = 0 (the tab's line is #2b3644)"

    # Ctrl-Shift-B: the row keeps out of the way, and is there again, for this window.
    Run 'bar-key' '{WAIT:500}{KEY:CTRL+SHIFT+B}{WAIT:700}{SHOT:bar-key-away}{KEY:CTRL+SHIFT+B}{WAIT:700}' "[theme]`nname = `"night`"`n$quiet" | Out-Null
    $keyed = @('bar-key-away', 'bar-key' | ForEach-Object { if (SameColor (PixelAt $_ @InBar) $page.night.bar) { 'there' } elseif (SameColor (PixelAt $_ @InBar) $page.night.screen) { 'away' } else { PixelAt $_ @InBar } })
    Report 'Ctrl-Shift-B puts the row of tabs away, and back' (($keyed -join ' ') -eq 'away there') "after the key, and after it again: $($keyed -join ', ')"

    # window.tab_bar = "auto-hide": the row is out of the way; it comes when the pointer rests at
    # the top edge and when a tab opens, goes when the pointer leaves or after a moment, and
    # does not come for a pointer that only crosses the edge. The terminal keeps its rows.
    $t = Run 'bar' "echo `"h1=`$([Console]::WindowHeight)`"{ENTER}{WAIT:2500}{SHOT:bar-1}{MOVE:300,6}{WAIT:700}{SHOT:bar-2}echo `"h2=`$([Console]::WindowHeight)`"{ENTER}{WAIT:600}{MOVE:300,200}{WAIT:500}{SHOT:bar-3}{KEY:CTRL+SHIFT+T}{WAIT:700}{SHOT:bar-4}{WAIT:2200}{SHOT:bar-5}{MOVE:300,6}{MOVE:300,300}{WAIT:700}" "[theme]`nname = `"night`"`n[window]`ntab_bar = `"auto-hide`"`n$quiet"
    $steps = @('bar-1', 'bar-2', 'bar-3', 'bar-4', 'bar-5', 'bar' | ForEach-Object { if (SameColor (PixelAt $_ @InBar) $page.night.bar) { 'shown' } elseif (SameColor (PixelAt $_ @InBar) $page.night.screen) { 'away' } else { PixelAt $_ @InBar } })
    Report 'auto-hide: the row comes and goes' (($steps -join ' ') -eq 'away shown away shown away away') "at rest, pointer at the top edge, pointer away, a new tab, 2 s later, pointer across the edge: $($steps -join ', ')"
    $flat = $t -replace "`n", ' '
    $h1 = [regex]::Match($flat, '(?<!")h1=(\d+)').Groups[1].Value; $h2 = [regex]::Match($flat, '(?<!")h2=(\d+)').Groups[1].Value
    Report 'auto-hide: the terminal keeps its rows when the row shows' ($h1 -ne '' -and $h1 -eq $h2) "rows with the row away: $h1; with it over the text: $h2"

    # theme.opacity: read from the screen itself. A window with a red background, a window with a
    # blue one right over it: whole, the blue one hides the red; at 0.5 the screen shows an even mix.
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Screen2 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, uint attr, out RECT r, int size);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
    [Screen2]::SetProcessDPIAware() | Out-Null
    function SeenThrough([double]$opacity) {
        $plain = "$quiet`n[history]`nenabled = false`n[commands]`nbuiltin = false"
        Set-Content (Join-Path $work 'under.toml') "[theme]`nbackground = `"#ff0000`"`n$plain" -Encoding utf8
        Set-Content (Join-Path $work 'over.toml') "[theme]`nbackground = `"#0000ff`"`nopacity = $opacity`n$plain" -Encoding utf8
        $opened = @()
        try {
            foreach ($which in 'under', 'over') {
                $p = Start-Process -FilePath $Exe -ArgumentList "--config `"$(Join-Path $work "$which.toml")`" -e cmd /k cls" -PassThru
                $deadline = (Get-Date).AddSeconds(15)
                while ($p.MainWindowHandle -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200; $p.Refresh() }
                Start-Sleep -Seconds 2
                # Topmost, at one place and size.
                [Screen2]::SetWindowPos($p.MainWindowHandle, [IntPtr](-1), 200, 200, 900, 600, 0x0040) | Out-Null
                $opened += $p
            }
            # The red one right behind the blue one, whatever came to the front since: no move, no size, no activation.
            [Screen2]::SetWindowPos($opened[0].MainWindowHandle, $opened[1].MainWindowHandle, 0, 0, 0, 0, 0x0013) | Out-Null
            Start-Sleep -Seconds 2
            $r = [Screen2+RECT]::new(); [Screen2]::GetWindowRect($opened[1].MainWindowHandle, [ref]$r) | Out-Null
            $shot = [System.Drawing.Bitmap]::new(1, 1); $g = [System.Drawing.Graphics]::FromImage($shot)
            # A point of the text area, well below the rows of tabs and what cmd printed.
            $g.CopyFromScreen([int](($r.L + $r.R) / 2), [int]($r.T + ($r.B - $r.T) * 0.7), 0, 0, $shot.Size); $g.Dispose()
            $c = $shot.GetPixel(0, 0); $shot.Dispose()
            '#{0:x2}{1:x2}{2:x2}' -f $c.R, $c.G, $c.B
        } finally { $opened | ForEach-Object { if (-not $_.HasExited) { $_.Kill() } }; Start-Sleep -Milliseconds 300 }
    }
    # theme.blur: stripes of red and white behind a half-opaque blue window. Sharp, a line of the
    # screen has the two mixes and no other color; blurred, it has many.
    function ThroughStripes([string]$blur) {
        $plain = "$quiet`n[history]`nenabled = false`n[commands]`nbuiltin = false"
        Set-Content (Join-Path $work 'under.toml') $plain -Encoding utf8
        Set-Content (Join-Path $work 'over.toml') "[theme]`nbackground = `"#0000ff`"`nopacity = 0.5`nblur = $blur`n$plain" -Encoding utf8
        $stripes = Join-Path $work 'stripes.ps1'
        Set-Content $stripes "`$e = [char]27`n`$line = (`"`$e[48;2;255;0;0m    `$e[48;2;255;255;255m    `" * 40) + `"`$e[0m`"`n1..60 | ForEach-Object { [Console]::Out.WriteLine(`$line) }`nStart-Sleep -Seconds 60" -Encoding utf8
        $opened = @()
        try {
            foreach ($which in @{ file = 'under'; run = "-e pwsh -NoProfile -File `"$stripes`"" }, @{ file = 'over'; run = '-e cmd /k cls' }) {
                $p = Start-Process -FilePath $Exe -ArgumentList "--config `"$(Join-Path $work "$($which.file).toml")`" $($which.run)" -PassThru
                $deadline = (Get-Date).AddSeconds(15)
                while ($p.MainWindowHandle -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200; $p.Refresh() }
                Start-Sleep -Seconds 3
                [Screen2]::SetWindowPos($p.MainWindowHandle, [IntPtr](-1), 200, 200, 900, 600, 0x0040) | Out-Null
                $opened += $p
            }
            [Screen2]::SetWindowPos($opened[0].MainWindowHandle, $opened[1].MainWindowHandle, 0, 0, 0, 0, 0x0013) | Out-Null
            Start-Sleep -Seconds 2
            $r = [Screen2+RECT]::new(); [Screen2]::GetWindowRect($opened[1].MainWindowHandle, [ref]$r) | Out-Null
            $shot = [System.Drawing.Bitmap]::new(300, 1); $g = [System.Drawing.Graphics]::FromImage($shot)
            $g.CopyFromScreen($r.L + 250, [int]($r.T + ($r.B - $r.T) * 0.7), 0, 0, $shot.Size); $g.Dispose()
            $seen = @{}
            for ($x = 0; $x -lt 300; $x++) { $c = $shot.GetPixel($x, 0); $seen['#{0:x2}{1:x2}{2:x2}' -f $c.R, $c.G, $c.B] = 1 }
            $shot.Dispose()
            $seen.Count
        } finally { $opened | ForEach-Object { if (-not $_.HasExited) { $_.Kill() } }; Start-Sleep -Milliseconds 300 }
    }

    # theme.corners: the window's own corners, read from the screen (Windows 11 cuts them). A red
    # window behind, larger all round; the window in front has a blue row of tabs. Down the
    # diagonal from the corner of its outline, the pixels before its own blue: one (the system's
    # line) for a square corner, more where the corner is cut round.
    function CornerCut([string]$corners) {
        $plain = "$quiet`n[history]`nenabled = false`n[commands]`nbuiltin = false"
        Set-Content (Join-Path $work 'under.toml') "[theme]`nbackground = `"#ff0000`"`nbar = `"#ff0000`"`n$plain" -Encoding utf8
        Set-Content (Join-Path $work 'over.toml') "[theme]`nbackground = `"#0000ff`"`nbar = `"#0000ff`"`nrule = `"#0000ff`"`ncorners = `"$corners`"`n$plain" -Encoding utf8
        $opened = @()
        try {
            foreach ($which in @{ file = 'under'; grow = 60 }, @{ file = 'over'; grow = 0 }) {
                $p = Start-Process -FilePath $Exe -ArgumentList "--config `"$(Join-Path $work "$($which.file).toml")`" -e cmd /k cls" -PassThru
                $deadline = (Get-Date).AddSeconds(15)
                while ($p.MainWindowHandle -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200; $p.Refresh() }
                Start-Sleep -Seconds 3
                [Screen2]::SetWindowPos($p.MainWindowHandle, [IntPtr](-1), 300 - $which.grow, 300 - $which.grow, 800 + 2 * $which.grow, 500 + 2 * $which.grow, 0x0040) | Out-Null
                $opened += $p
            }
            [Screen2]::SetWindowPos($opened[0].MainWindowHandle, $opened[1].MainWindowHandle, 0, 0, 0, 0, 0x0013) | Out-Null
            Start-Sleep -Seconds 2
            # The outline the system draws, without the unseen border for sizing (DWMWA_EXTENDED_FRAME_BOUNDS).
            $r = [Screen2+RECT]::new(); [Screen2]::DwmGetWindowAttribute($opened[1].MainWindowHandle, 9, [ref]$r, 16) | Out-Null
            $shot = [System.Drawing.Bitmap]::new(12, 12); $g = [System.Drawing.Graphics]::FromImage($shot)
            $g.CopyFromScreen($r.L, $r.T, 0, 0, $shot.Size); $g.Dispose()
            $cut = 0; for ($d = 0; $d -lt 12; $d++) { $c = $shot.GetPixel($d, $d); if (-not ($c.B -gt 200 -and $c.R -lt 60)) { $cut++ } else { break } }
            $shot.Dispose()
            $cut
        } finally { $opened | ForEach-Object { if (-not $_.HasExited) { $_.Kill() } }; Start-Sleep -Milliseconds 300 }
    }
    # What is read from the screen itself is only as good as the screen: a layer another program
    # puts over it (a screenshot tool's dimming) changes every color. So an opaque blue window is
    # read first and last: it must be blue both times, or the readings between are thrown away
    # and taken again a little later, three times at most. (Seen once: opaque blue read as
    # #00008d and the even mix as #470046, both 0.55 of what they are; read again, both right.)
    $canChoose = [Environment]::OSVersion.Version.Build -ge 22000
    $clear = $false
    for ($try = 1; $try -le 3 -and -not $clear; $try++) {
        if ($try -gt 1) { Start-Sleep -Seconds 20 }
        $whole = SeenThrough 1
        if ($whole -ne '#0000ff') { continue }
        $half = SeenThrough 0.5
        $sharp = ThroughStripes 'false'; $blurred = ThroughStripes 'true'
        if ($canChoose) { $square = CornerCut 'square'; $small = CornerCut 'small'; $round = CornerCut 'round' }
        $clear = (SeenThrough 1) -eq '#0000ff'
    }
    $tries = $try - 1
    if (-not $clear) {
        Report 'the screen shows a window as it is' $false "an opaque blue window, read from the screen, was $whole and not #0000ff in $tries tries: something lies over the screen, and what shows through a window cannot be read"
    } else {
        $read = "read from the screen at try $tries of 3"
        Report 'theme.blur: what shows through is blurred' ($sharp -le 4 -and $blurred -ge 8) "colors on a line of 300 pixels, stripes behind: $sharp without blur (the two mixes), $blurred with it; $read"
        if ($canChoose) {
            Report 'theme.corners: the window''s own corners are square, small or round' ($square -le 1 -and $small -gt $square -and $round -gt $small) "pixels cut off down the corner's diagonal: square $square, small $small, round $round; $read"
        } else {
            "SKIP  theme.corners: this Windows (build $([Environment]::OSVersion.Version.Build)) does not let a program choose its window's corners"
        }
        $mixed = $half -match '^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$' -and [Math]::Abs([Convert]::ToInt32($Matches[1], 16) - 0x80) -le 8 -and [Convert]::ToInt32($Matches[2], 16) -le 8 -and [Math]::Abs([Convert]::ToInt32($Matches[3], 16) - 0x80) -le 8
        Report 'theme.opacity: what is behind shows through, by that much' $mixed "a blue window over a red one: opacity 1 gives $whole, before and after; 0.5 gives $half (an even mix is #80007f); $read"
    }
}
if ($Section -in 'all', 'edges') {
    "--- edges ---"
    $t = Run 'badkey' "echo still-works{ENTER}" "[font]`nsise = 12" -keep
    Report 'a misspelt setting: opens, names it' ((Count $t 'sise') -ge 1 -and (Count $t 'still-works') -eq 2) "'sise' seen $(Count $t 'sise') times, 'still-works' $(Count $t 'still-works')"

    $garbage = Join-Path $work 'garbage.toml'; [IO.File]::WriteAllBytes($garbage, [byte[]](0..255))
    $log = Join-Path $work 'garbage.raw'; $env:UNIFYTERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'garbage.png') -Type ($NoHistoryKeep + "echo alive{ENTER}") -Wait 4 -After 1 -App:" --config `"$garbage`"" | Out-Null } finally { $env:UNIFYTERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a settings file of random bytes: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $dir = Join-Path $work 'a-directory'; New-Item -ItemType Directory -Force $dir | Out-Null
    $log = Join-Path $work 'dir.raw'; $env:UNIFYTERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'dir.png') -Type ($NoHistoryKeep + "echo alive{ENTER}") -Wait 4 -After 1 -App:" --config `"$dir`"" | Out-Null } finally { $env:UNIFYTERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a directory as the settings file: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $t = Run 'badshell' "echo fell-back$Unsaved{ENTER}" "[shell]`nprogram = `"no-such-shell-xyz`""
    Report 'a shell that does not exist: the default one, says so' ((Count $t 'shell.program did not start') -eq 1 -and (Count $t 'fell-back') -eq 2) "'shell.program did not start' $(Count $t 'shell.program did not start'), 'fell-back' $(Count $t 'fell-back')"

    $p = Start-Process $Exe -ArgumentList '-e', 'no-such-program-xyz' -PassThru
    $gone = $p.WaitForExit(8000)
    Report 'a program that does not exist: ends, with failure' ($gone -and $p.ExitCode -ne 0) "ended: $gone, exit code $(if ($gone) { $p.ExitCode } else { 'none (still running)' })"
    if (-not $gone) { Stop-Process -Id $p.Id -Force }

    # Where a tab starts. A launcher leaves the program in a directory nobody chose
    # (here: Windows' own): the tab is in the home directory. --cwd names another.
    $where = @{}
    foreach ($case in @(@('home', @('-e', 'cmd', '/c', 'cd')), @('asked', @('--cwd', $work, '-e', 'cmd', '/c', 'cd')))) {
        $log = Join-Path $work "cwd-$($case[0]).raw"; $env:UNIFYTERM_RAW_LOG = $log
        try {
            $p = Start-Process $Exe -ArgumentList $case[1] -WorkingDirectory $env:SystemRoot -PassThru
            if (-not $p.WaitForExit(8000)) { Stop-Process -Id $p.Id -Force }
        } finally { $env:UNIFYTERM_RAW_LOG = $null }
        $where[$case[0]] = if (Test-Path $log) { (Replay $log '250x30') -replace "`n", '' } else { '' }
    }
    $userHome = [Environment]::GetFolderPath('UserProfile')
    Report 'started by a launcher: the tab is in the home directory' ($where.home.Contains($userHome) -and -not $where.home.Contains($env:SystemRoot)) "the home directory printed: $($where.home.Contains($userHome)); the directory it was started in printed: $($where.home.Contains($env:SystemRoot))"
    Report '--cwd DIRECTORY: the tab is there' ($where.asked.Contains($work)) "the directory asked for printed: $($where.asked.Contains($work))"

    # The program file carries its icon: the window, the taskbar and Explorer take it
    # from there. Counted: pixels of the logo's dark ground and of its three strands.
    Add-Type -AssemblyName System.Drawing
    $icon = [System.Drawing.Icon]::ExtractAssociatedIcon($Exe).ToBitmap()
    $ground = 0; $strands = 0
    for ($y = 0; $y -lt $icon.Height; $y++) { for ($x = 0; $x -lt $icon.Width; $x++) {
        $c = $icon.GetPixel($x, $y); if ($c.A -lt 200) { continue }
        if ($c.R -lt 0x30 -and $c.G -lt 0x30 -and $c.B -ge 0x20 -and $c.B -lt 0x40) { $ground++ }
        elseif ($c.B -ge 0xE0 -and $c.G -ge 0x80) { $strands++ }
    } }
    Report 'the program file carries its icon' ($ground -ge 300 -and $strands -ge 20) "of $($icon.Width * $icon.Height) pixels: $ground of the logo's dark ground, $strands of its strands"

    # The row of tabs is the title bar. Each part says what it is when Windows asks
    # (the pointer is moved there first, as a hand moves it), and the three buttons act.
    Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Bar {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
"@
    $barConfig = Join-Path $work 'bar.toml'; "[history]`nenabled = false" | Set-Content $barConfig
    $p = Start-Process $Exe -ArgumentList "--cwd `"$($work -replace '\\', '/')`" --config `"$barConfig`"" -PassThru
    $bar = [ordered]@{}; $acts = [ordered]@{}
    try {
        $h = [IntPtr]::Zero
        for ($i = 0; $i -lt 60 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 250; $p.Refresh(); $h = $p.MainWindowHandle }
        Start-Sleep 2
        $scale = [Bar]::GetDpiForWindow($h) / 96.0
        $at = { param([int]$x, [int]$y) [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF)) }
        # A point of the bar, counted from the window's right edge (or its left): 8 is the unseen border Windows adds.
        $hit = { param([int]$fromRight, [int]$fromLeft)
            $r = New-Object Bar+RECT; [void][Bar]::GetWindowRect($h, [ref]$r)
            $x = if ($fromRight) { $r.R - [int]((8 + $fromRight) * $scale) } else { $r.L + [int]((8 + $fromLeft) * $scale) }
            $y = $r.T + [int](25 * $scale)
            $c = New-Object Bar+POINT; $c.X = $x; $c.Y = $y; [void][Bar]::ScreenToClient($h, [ref]$c)
            [void][Bar]::PostMessage($h, 0x0200, [IntPtr]::Zero, (& $at $c.X $c.Y)); Start-Sleep -Milliseconds 300
            [pscustomobject]@{ Code = [Bar]::SendMessage($h, 0x0084, [IntPtr]::Zero, (& $at $x $y)).ToInt64(); L = (& $at $x $y) }
        }
        $press = { param($where) [void][Bar]::PostMessage($h, 0x00A1, [IntPtr]$where.Code, $where.L); Start-Sleep -Milliseconds 120; [void][Bar]::PostMessage($h, 0x00A2, [IntPtr]$where.Code, $where.L); Start-Sleep -Milliseconds 900 }
        $bar.tab = (& $hit 0 40).Code; $bar.empty = (& $hit 0 400).Code; $bar.minimize = (& $hit 115 0).Code; $bar.maximize = (& $hit 69 0).Code; $bar.close = (& $hit 23 0).Code
        & $press (& $hit 69 0); $acts.maximized = [Bar]::IsZoomed($h)
        & $press (& $hit 69 0); $acts.restored = -not [Bar]::IsZoomed($h)
        & $press (& $hit 115 0); $acts.minimized = [Bar]::IsIconic($h)
        [void][Bar]::ShowWindow($h, 9); Start-Sleep 1
        & $press (& $hit 23 0); $acts.closed = $p.WaitForExit(5000)
    } finally { if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force } }
    # Windows' names for the parts: 1 the window's content, 2 the title bar, 8 minimize, 9 maximize, 20 close.
    Report 'the row of tabs is the title bar: each part is what it is' ($bar.tab -eq 1 -and $bar.empty -eq 2 -and $bar.minimize -eq 8 -and $bar.maximize -eq 9 -and $bar.close -eq 20) "a tab $($bar.tab) (1), the empty part $($bar.empty) (2), the buttons $($bar.minimize) $($bar.maximize) $($bar.close) (8 9 20)"
    Report 'its three buttons act' (@($acts.Values | Where-Object { $_ }).Count -eq 4) "maximized $($acts.maximized), restored $($acts.restored), minimized $($acts.minimized), closed $($acts.closed)"

    $t = Run 'tiny' "{SIZE:160,120}{WAIT:900}echo tiny-ok{ENTER}{WAIT:500}{SIZE:900,600}{WAIT:900}echo big-again{ENTER}"
    Report 'a tiny window and back: still works' ((Count ($t -replace "`n", '') 'tiny-ok') -ge 2 -and (Count $t 'big-again') -eq 2) "'tiny-ok' $(Count ($t -replace "`n", '') 'tiny-ok') (wrapped in the tiny screen), 'big-again' $(Count $t 'big-again')"

    $watch = [Diagnostics.Stopwatch]::StartNew()
    $t = Run 'flood' "1..20000 | % { `"flood line `$_`" }; echo flood-done{ENTER}{WAIT:9000}" '' '' 1
    # The replay's screen keeps the last 2000 lines: the typed command has scrolled out of it.
    $numbered = ([regex]::Matches($t, '(?m)^flood line \d+$')).Count
    Report '20 000 lines of output: all arrive' ($t -match '(?m)^flood line 20000$' -and $t -match '(?m)^flood-done$' -and $numbered -ge 1990) "the last line 'flood line 20000' is there: $($t -match '(?m)^flood line 20000$'), then 'flood-done': $($t -match '(?m)^flood-done$'); $numbered of the screen's 2000 lines are numbered ones; $([int]$watch.Elapsed.TotalSeconds) s with startup"

    $wide = 'x' * 600
    $t = Run 'wide' "echo $wide{ENTER}"
    $joined = ($t -replace "`n", '')
    Report 'a 600-character line: wraps, nothing lost' ($joined -match "x{600}") "longest run of x in the output: $(([regex]::Matches($joined, 'x+') | ForEach-Object { $_.Length } | Measure-Object -Maximum).Maximum)"

    $t = Run 'zero' "1..40 | % { `"z `$_`" }{ENTER}{WAIT:800}{WHEEL:400,300,5}{WAIT:300}echo zero-ok{ENTER}" "[terminal]`nscrollback = 0"
    Report 'no scrollback at all: the wheel does no harm' ((Count $t 'zero-ok') -eq 2) "'zero-ok' seen $(Count $t 'zero-ok') times"

    # Typed in a terminal (unifyterm itself), with nothing redirected: a release
    # build is a window program and must still print there. It does not hold
    # the shell, so its text lands after the next prompt; nothing more is
    # typed in the same run, or the shell's line editor would draw over it.
    $t = Run 'cli-version' "& '$Exe' --version{ENTER}{WAIT:1500}"
    $version = [regex]::Match($t, 'UnifyTerm \d+\.\d+\.\d+').Value
    Report '--version prints in a terminal' ([bool]$version) "printed '$version'"
    $t = Run 'cli-wrong' "& '$Exe' --nope{ENTER}{WAIT:1500}"
    Report 'a wrong argument is named in a terminal' ((Count $t 'unknown argument: --nope') -eq 1 -and (Count $t 'usage: unifyterm') -eq 1) "'unknown argument: --nope' $(Count $t 'unknown argument: --nope'), 'usage: unifyterm' $(Count $t 'usage: unifyterm')"
    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) unifyterm or console-host processes still running"
}

if ($Section -in 'all', 'commands') {
    "--- commands ---"
    $json = Join-Path $work 'data.json'; [IO.File]::WriteAllText($json, '{"items":[{"name":"alpha"},{"name":"beta"}]}')
    $kind = 'echo "cat=$((Get-Command cat).CommandType) ls=$((Get-Command ls).CommandType)"{ENTER}{WAIT:500}'
    $t = Run 'cmd-on' ($kind + 'echo "from=$((Get-Command cat).Source)"{ENTER}{WAIT:500}' + "cat '$json' | jq -r '.items[].name'{ENTER}{WAIT:900}ls --version{ENTER}{WAIT:700}")
    Report 'cat and ls are unifyterm''s programs' ((Count $t 'cat=Application ls=Application') -eq 1) "the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"
    Report 'they come from unifyterm''s command directory' ($t -match ('from=' + [regex]::Escape("$env:UNIFYTERM_DATA_DIR\bin\cat.exe"))) "$([regex]::Match(($t -replace "`n", ''), 'from=[A-Z]:\\\S+').Value)"
    Report 'cat | jq works in PowerShell' ($t -match '(?m)^alpha$' -and $t -match '(?m)^beta$') "'alpha' and 'beta' on lines of their own: $($t -match '(?m)^alpha$'), $($t -match '(?m)^beta$')"
    Report 'ls is the uutils one' ((Count $t 'uutils') -ge 1) "'uutils' in ls --version: $(Count $t 'uutils')"

    $made = Join-Path $work 'made'
    $t = Run 'cmd-mkdir' "mkdir -p '$made\a\b\c'; ls -1 '$made\a\b'{ENTER}{WAIT:900}echo `"中文 pipe`" | grep -c 中{ENTER}{WAIT:900}echo `"page=`$([Console]::OutputEncoding.CodePage)`"{ENTER}"
    Report 'mkdir -p makes the whole path' ((Test-Path "$made\a\b\c") -and $t -match '(?m)^c$') "the directory exists: $(Test-Path "$made\a\b\c"); ls -1 printed 'c': $($t -match '(?m)^c$')"
    Report 'Chinese text passes through a pipe to grep' ($t -match '(?m)^1$') "grep -c printed '1' on a line: $($t -match '(?m)^1$')"
    Report 'the console is UTF-8' ((Count $t 'page=65001') -eq 1) "$([regex]::Match($t, 'page=\d+').Value)"

    # Paths with / on Windows too: the prompt, pwd, find. Backslashes are typed, so a slashed path can only be printed.
    $t = Run 'cmd-slash' "cd '$made\a'{ENTER}{WAIT:700}pwd{ENTER}{WAIT:900}find . -name c{ENTER}{WAIT:900}`$env:UT_NATIVE_PATHS = 1{ENTER}{WAIT:500}pwd{ENTER}{WAIT:900}"
    $flat = $t -replace "`n", ''; $slashed = ($made -replace '\\', '/') + '/a'
    $prompts = Count $flat "PS $slashed>"; $printed = (Count $flat $slashed) - $prompts
    Report 'the prompt shows the folder with /' ($prompts -ge 3) "'PS $slashed>' seen $prompts times"
    Report 'pwd prints the folder with /' ($printed -eq 1) "the slashed folder outside a prompt: $printed time(s)"
    Report 'find prints what it found with /' ($t -match '(?m)^\./b/c$') "'./b/c' on a line of its own: $($t -match '(?m)^\./b/c$')"
    $nativePrompts = Count $flat "PS $made\a>"; $nativePrinted = (Count $flat "$made\a") - $nativePrompts - (Count $flat "'$made\a'")
    Report 'UT_NATIVE_PATHS gives Windows'' own \ back' ($nativePrompts -ge 1 -and $nativePrinted -eq 1) "after it is set: $nativePrompts prompt(s) and $nativePrinted pwd with \"

    # Wildcards: replaced by the commands as a Unix shell does it; a quoted one stays (the start script marks it).
    # The lines are typed, so this is also what says that a typed `cat` is unifyterm's program and no other.
    'one' | Set-Content "$made\a\x1.txt"; 'two' | Set-Content "$made\a\x2.txt"; 'deep' | Set-Content "$made\a\b\x3.txt"
    $t = Run 'cmd-wild' "cd '$made\a'{ENTER}{WAIT:700}echo W1 x*.txt{ENTER}{WAIT:900}echo W2 `"x*.txt`" 'x?.txt'{ENTER}{WAIT:900}find . -name `"x*.txt`" | sort | tr '\n' ' '{ENTER}{WAIT:1200}cat x*.txt | wc -l{ENTER}{WAIT:1200}Get-History | ForEach-Object { 'H' + [int][bool](`$_.CommandLine -match [char]0xE000) } | Select-Object -Last 1{ENTER}{WAIT:900}"
    Report 'a wildcard becomes the names it matches' ($t -match '(?m)^W1 x1\.txt x2\.txt$') "'echo W1 x*.txt' printed: $([regex]::Match($t, '(?m)^W1 .*$').Value)"
    Report 'a quoted wildcard stays as typed' ($t -match '(?m)^W2 x\*\.txt x\?\.txt$') "'echo W2 `"x*.txt`" ''x?.txt''' printed: $([regex]::Match($t, '(?m)^W2 .*$').Value)"
    Report 'find gets its quoted pattern whole' ($t -match '(?m)^\./b/x3\.txt \./x1\.txt \./x2\.txt\s*$') "find . -name `"x*.txt`" found: $([regex]::Match($t, '(?m)^\./b/x3.*$').Value)"
    Report 'cat x*.txt reads both files' ($t -match '(?m)^2$') "wc -l printed '2' on a line: $($t -match '(?m)^2$')"

    # The same without a profile, in both PowerShells: the start script alone must install the marking.
    # The start script is the one unifyterm wrote for the default settings; it is given by hand, as unifyterm would.
    $start = (Join-Path $env:UNIFYTERM_DATA_DIR 'shell\bin.ps1').Replace("'", "''")
    foreach ($shell in 'pwsh.exe', 'powershell.exe') {
        $bare = "[shell]`nprogram = `"$shell`"`nargs = [`"-NoProfile`", `"-NoExit`", `"-Command`", `". '$($start.Replace('\', '\\'))'`"]"
        $t = Run "cmd-bare-$shell" ($NoHistory + "cd '$made\a'{ENTER}{WAIT:700}echo B1 x*.txt `"x*.txt`"{ENTER}{WAIT:1200}") $bare
        Report "no profile, $($shell): wildcards as in a shell" ($t -match '(?m)^B1 x1\.txt x2\.txt x\*\.txt\s*$') "printed: $([regex]::Match($t, '(?m)^B1 .*$').Value)"
    }

    $t = Run 'cmd-off' $kind "[commands]`nbuiltin = false"
    Report 'commands.builtin = false: PowerShell''s own again' ((Count $t 'cat=Alias ls=Alias') -eq 1) "the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"

    $t = Run 'cmd-some' $kind "[commands]`nsystem = [`"cat`", `"nosuch`"]" -keep
    Report 'commands.system keeps one command the system''s' ((Count $t 'cat=Alias ls=Application') -eq 1) "the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"
    Report 'a name in commands.system that is no command is named' ((Count $t "commands.system names 'nosuch'") -eq 1) "'commands.system names 'nosuch'' seen $(Count $t "commands.system names 'nosuch'") times"

    $t = Run 'cmd-cmd' "where cat{ENTER}{WAIT:900}cat --version{ENTER}" "[shell]`nprogram = `"cmd.exe`""
    $first = ($t -split "`n" | Where-Object { $_ -match 'cat\.exe' } | Select-Object -First 1)
    Report 'cmd.exe finds unifyterm''s cat first' ($first -match [regex]::Escape("$env:UNIFYTERM_DATA_DIR\bin\cat.exe") -and (Count $t 'uutils') -ge 1) "first match of 'where cat': $first"

    # unifyterm without ut beside it: it says so and runs without its commands.
    $alone = Join-Path $work 'alone'; New-Item -ItemType Directory -Force $alone | Out-Null
    Get-ChildItem (Split-Path $Exe) -File | Where-Object { $_.Name -in 'unifyterm.exe', 'conpty.dll', 'OpenConsole.exe' } | Copy-Item -Destination $alone
    $kept = $Exe; $Exe = Join-Path $alone 'unifyterm.exe'
    try { $t = Run 'cmd-alone' $kind -keep } finally { $Exe = $kept }
    Report 'without ut beside it: says so, still a terminal' ((Count $t "UnifyTerm's commands are not in use") -eq 1 -and (Count $t 'cat=Alias ls=Alias') -eq 1) "'commands are not in use' $(Count $t "UnifyTerm's commands are not in use"); the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"

    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) unifyterm or console-host processes still running"
}

if ($Section -in 'all', 'history') {
    "--- history ---"
    # A data directory of this section's own: the history starts empty, whatever ran before.
    $keptData = $env:UNIFYTERM_DATA_DIR
    $env:UNIFYTERM_DATA_DIR = Join-Path $work 'data-history'
    $ut = Join-Path (Split-Path $Exe) 'ut.exe'
    function Read-UtHistory([string[]]$words) { @(& $ut history @words 2>&1 | ForEach-Object { "$_" }) }
    # The list is drawn over the text, not in it: it is seen in pictures. The same input
    # with the list turned off is the picture without it. Below the first line, after cls.
    $noList = "[history]`ncandidates = false"
    function ListPixels([string]$a, [string]$b) { PixelDiff $a $b 0 ($Row.Y + $Row.H) 900 220 }
    try {
        Run 'hist-basic' "echo h-one{ENTER}{WAIT:600}echo h-two{ENTER}{WAIT:600}echo h-three{ENTER}{WAIT:800}" | Out-Null
        $recent = Read-UtHistory 'recent'
        Report 'commands that ran are kept, newest first' (($recent -join '|') -eq 'echo h-three|echo h-two|echo h-one') "ut history recent: $($recent -join ' | ')"

        $t = Run 'hist-fail' "ls no-such-dir-x{ENTER}{WAIT:800}cd '$work'{ENTER}{WAIT:600}echo in-work{ENTER}{WAIT:700}echo x --password hunter2{ENTER}{WAIT:700}"
        $failedLine = (Read-UtHistory 'search', 'no-such-dir-x', '--long') -join ''
        Report 'a failed command is kept with its exit code' ($failedLine -match '^\s+1\s+2\s+\S+\s+ls no-such-dir-x$') "ut history search --long: '$failedLine'"
        $here = Read-UtHistory 'recent', '--cwd', ($work -replace '\\', '/')
        Report 'the directory is kept, with /' (($here -join '|') -eq 'echo in-work') "commands run in $($work -replace '\\', '/'): $($here -join ' | ')"
        $secret = Read-UtHistory 'search', 'hunter2'
        Report 'a command with a password is not kept' ($secret.Count -eq 0) "ut history search hunter2: $($secret.Count) found"

        Run 'hist-list' "echo h-{WAIT:1000}" | Out-Null
        Run 'hist-nolist' "echo h-{WAIT:1000}" $noList | Out-Null
        $shown = ListPixels 'hist-list' 'hist-nolist'
        Report 'typing at the prompt shows the list' ($shown -gt 500) "$shown pixels differ below the input line from the same input with the list off"
        # Three commands begin with "echo h-": with limit = 1 the second and third rows are not drawn.
        Run 'hist-limit' "echo h-{WAIT:1000}" "[history]`nlimit = 1" | Out-Null
        $rowsGone = PixelDiff 'hist-list' 'hist-limit' 0 ($Row.Y + 2 * $Row.H) 900 (2 * $Row.H)
        $firstKept = PixelDiff 'hist-list' 'hist-limit' 0 ($Row.Y + $Row.H) 900 $Row.H
        Report 'history.limit is how many rows the list has' ($rowsGone -gt 300 -and $firstKept -lt 300) "rows 2-3: $rowsGone pixels differ from limit = 1; row 1: $firstKept"

        $t = Run 'hist-take' "echo h-t{WAIT:1000}{KEY:CTRL+N}{WAIT:300}{TAB}{WAIT:900}"
        $lines = @($t -split "`n" | Where-Object { $_.Trim() })
        Report 'Ctrl-N and Tab put the command on the line, not run it' ($lines[-1] -match '> echo h-three$' -and (Count $t 'h-three') -eq 1) "last line: '$($lines[-1])'; 'h-three' seen $(Count $t 'h-three') time(s)"
        $t = Run 'hist-run' "echo h-t{WAIT:1000}{DOWN}{WAIT:300}{ENTER}{WAIT:700}{ENTER}{WAIT:900}"
        Report 'Enter on a chosen row fills; a second Enter runs it' ((Count $t 'h-three') -eq 2) "'h-three' seen $(Count $t 'h-three') times (the line, then its output)"

        Run 'hist-esc' "echo h-{WAIT:1000}{ESC}{WAIT:500}" | Out-Null
        $closed = ListPixels 'hist-esc' 'hist-nolist'
        Report 'Esc closes the list and leaves the line' ($closed -lt 50) "$closed pixels differ from the same input with the list off"

        Run 'hist-nvim' "nvim --clean{ENTER}{WAIT:1500}iec{WAIT:1000}" | Out-Null
        Run 'hist-nvim-off' "nvim --clean{ENTER}{WAIT:1500}iec{WAIT:1000}" $noList | Out-Null
        $inNvim = PixelDiff 'hist-nvim' 'hist-nvim-off' 0 0 900 500
        Report 'no list in a program that has the whole screen' ($inNvim -lt 50) "$inNvim pixels differ in nvim between the list on and off"

        $before = (Read-UtHistory 'recent', '-n', '100').Count
        Run 'hist-off' "echo off-test{ENTER}{WAIT:700}echo h-{WAIT:1000}" "[history]`nenabled = false" | Out-Null
        $off = Read-UtHistory 'search', 'off-test'
        Run 'hist-off-nolist' "echo off-test{ENTER}{WAIT:700}echo h-{WAIT:1000}" $noList | Out-Null
        $offList = ListPixels 'hist-off' 'hist-off-nolist'
        Report 'history.enabled = false: nothing kept, no list' ($off.Count -eq 0 -and $offList -lt 50) "found: $($off.Count); commands before: $before; list pixels: $offList"

        foreach ($shell in 'cmd.exe', 'powershell.exe') {
            $name = $shell -replace '\.exe$', ''
            Run "hist-$name" "echo $name-one{ENTER}{WAIT:900}" "[shell]`nprogram = `"$shell`"" | Out-Null
            $keptLine = (Read-UtHistory 'search', "$name-one", '--long') -join ''
            Run "hist-$name-list" "echo $name-{WAIT:1100}" "[shell]`nprogram = `"$shell`"" | Out-Null
            Run "hist-$name-nolist" "echo $name-{WAIT:1100}" "[shell]`nprogram = `"$shell`"`n$noList" | Out-Null
            $listed = ListPixels "hist-$name-list" "hist-$name-nolist"
            Report "$($shell): kept, and the list shows" ($keptLine -match "echo $name-one" -and $listed -gt 300) "kept: '$($keptLine.Trim())'; list pixels: $listed"
        }
    } finally { $env:UNIFYTERM_DATA_DIR = $keptData }
    $realHistoryAfter = & $StampOf $realHistory
    Report "the user's own history file is not touched" ($realHistoryAfter -ceq $realHistoryBefore) "before the checks: $realHistoryBefore; after: $realHistoryAfter$(if ($realHistoryAfter -cne $realHistoryBefore) { ' (your own UnifyTerm, used meanwhile, writes it too)' })"
}
if ($Section -in 'all', 'ai') {
    "--- ai ---"
    $keptData = $env:UNIFYTERM_DATA_DIR
    $env:UNIFYTERM_DATA_DIR = Join-Path $work 'data-ai'
    # A stand-in for the model's service on this machine (tools/fake-model.ps1); nothing leaves it.
    $port = 8765
    $requests = Join-Path $work 'model-requests.jsonl'
    function Start-Model([int]$delay, [string]$takes = 'schema') {
        Start-Process pwsh -ArgumentList "-NoProfile -File `"$PSScriptRoot\fake-model.ps1`" -Port $port -Log `"$requests`" -DelaySeconds $delay -Takes $takes" -PassThru -WindowStyle Hidden
    }
    # The last line of a screen's text that is not empty: the input line, when the shell waits at its prompt.
    function LastLine([string]$text) { @($text -split "`n" | Where-Object { $_.Trim() })[-1] }
    # The questions about a sentence among the requests from number $from on.
    function Asked([int]$from) { @(Sent | Select-Object -Skip $from | Where-Object { $_ -match 'Request: ' }).Count }
    function Sent { @(Get-Content $requests -ErrorAction Ignore) }
    $env:UNIFYTERM_TEST_KEY = 'test-key'
    # The history's own list is off throughout: the pictures are of the model's help alone.
    $quiet = "[history]`ncandidates = false"
    $model = "$quiet`n[ai]`nprovider = `"anthropic`"`nmodel = `"claude-haiku-5-5`"`nbase_url = `"http://localhost:$port`"`napi_key_env = `"UNIFYTERM_TEST_KEY`""
    $noFix = "$quiet`n[ai]`nfix = false"
    function Below([string]$a, [string]$b) { PixelDiff $a $b 0 ($Row.Y + $Row.H) 900 220 }
    $server = Start-Model 0
    Start-Sleep 2
    try {
        # No model named: the fix for a misspelt name still comes, from the names the shell can find.
        Run 'ai-local' "gti status{ENTER}{WAIT:1500}" $quiet | Out-Null
        Run 'ai-local-off' "gti status{ENTER}{WAIT:1500}" $noFix | Out-Null
        $local = Below 'ai-local' 'ai-local-off'
        $t = Run 'ai-local-take' "gti status{ENTER}{WAIT:1500}{ENTER}{WAIT:800}" $quiet
        $last = @($t -split "`n" | Where-Object { $_.Trim() })[-1]
        Report 'no model: a misspelt name is put right, not run' ($local -gt 500 -and $last -match '> git status$') "the fix's box: $local pixels; last line after Enter: '$last'"
        Report 'no model: nothing is sent' ((Sent).Count -eq 0) "requests the model's service got: $((Sent).Count)"

        # A model named: first the question whether it may be asked; Esc is no.
        Run 'ai-ask' "echo hel{WAIT:1500}" $model | Out-Null
        Run 'ai-ask-off' "echo hel{WAIT:1500}" $quiet | Out-Null
        $asked = Below 'ai-ask' 'ai-ask-off'
        Run 'ai-no' "echo hel{WAIT:1500}{ESC}{WAIT:300}echo more{WAIT:1500}" $model | Out-Null
        Report 'first the user is asked; after Esc nothing is sent' ($asked -gt 500 -and (Sent).Count -eq 0) "the question's box: $asked pixels; requests after Esc: $((Sent).Count)"

        # Enter is yes: the suggestion comes, Right takes it.
        $t = Run 'ai-yes' "echo hel{WAIT:1500}{ENTER}{WAIT:2000}{RIGHT}{WAIT:800}" $model
        $last = @($t -split "`n" | Where-Object { $_.Trim() })[-1]
        $agreed = Join-Path $env:UNIFYTERM_DATA_DIR 'ai-consent'
        Report 'after yes, the suggestion is shown and Right takes it' ($last -match '> echo hel --from-model$' -and (Test-Path $agreed)) "last line: '$last'; the answer kept: $(Test-Path $agreed)"

        $t = Run 'ai-fix' "zzqqxx-nothing{ENTER}{WAIT:3000}{ENTER}{WAIT:800}" $model
        $lines = @($t -split "`n" | Where-Object { $_.Trim() })
        Report "the model's fix is put on the line, not run" ($lines[-1] -match '> echo fixed-by-model$' -and (Count $t 'fixed-by-model') -eq 1) "last line: '$($lines[-1])'; 'fixed-by-model' seen $(Count $t 'fixed-by-model') time(s)"

        # Each feature has its own switch: off, its question is never sent.
        $before = (Sent).Count
        Run 'ai-suggest-off' "echo sug{WAIT:2000}" "$model`nsuggest = false" | Out-Null
        Run 'ai-suggest-on' "echo sug{WAIT:2000}" $model | Out-Null
        $mid = (Sent).Count
        $t = Run 'ai-explain-switched' "tar -czf a.tgz src{WAIT:400}{KEY:CTRL+SHIFT+E}{WAIT:2000}" "$model`nexplain = false"
        $offAsks = @(Sent | Select-Object -Skip $mid | Where-Object { $_ -match '"parts"' }).Count
        Report 'ai.suggest and ai.explain turn their feature off' ($mid - $before -eq 1 -and $offAsks -eq 0) "suggestions asked with suggest off, then on: $($mid - $before) (1 = only the second); explanations asked with explain off: $offAsks"

        $before = (Sent).Count
        Run 'ai-explain' "tar -czf a.tgz src{WAIT:400}{KEY:CTRL+SHIFT+E}{WAIT:3000}" $model | Out-Null
        Run 'ai-explain-off' "tar -czf a.tgz src{WAIT:400}{WAIT:3000}" $quiet | Out-Null
        $explained = Below 'ai-explain' 'ai-explain-off'
        $asks = @(Sent | Select-Object -Skip $before | Where-Object { $_ -match '"parts"' }).Count
        Report 'Ctrl-Shift-E explains the line in a box' ($explained -gt 1000 -and $asks -ge 1) "the box: $explained pixels; questions about parts: $asks"        # The first time ever (nothing agreed to yet): the question whether the model may be asked, and,
        # after the yes, the explanation itself: the same picture as an explanation asked for later.
        $keptAi = $env:UNIFYTERM_DATA_DIR; $env:UNIFYTERM_DATA_DIR = Join-Path $work 'data-ai-first'
        Run 'ai-explain-first' "tar -czf a.tgz src{WAIT:400}{KEY:CTRL+SHIFT+E}{WAIT:1200}{ENTER}{WAIT:3000}" $model | Out-Null
        $env:UNIFYTERM_DATA_DIR = $keptAi
        $first = Below 'ai-explain-first' 'ai-explain'
        Report 'the first explanation comes right after the yes' ($first -lt 500) "pixels that differ from an explanation asked for later: $first (the box alone is $explained)"

        Run 'ai-secret' "echo --token abc123secretvalue{WAIT:2000}" $model | Out-Null
        $all = (Sent) -join "`n"
        Report 'a secret on the line is not sent' ($all -notmatch 'abc123secretvalue' -and $all -match 'redacted') "requests: $((Sent).Count); the secret in them: $($all -match 'abc123secretvalue'); '‹redacted›' in them: $($all -match 'redacted')"

        # A wrong key: no suggestion, and the log says why.
        $env:UNIFYTERM_TEST_KEY = 'bad-key'
        Run 'ai-badkey' "echo bad{WAIT:2000}" $model | Out-Null
        $env:UNIFYTERM_TEST_KEY = 'test-key'
        $logged = Get-Content (Join-Path $env:UNIFYTERM_DATA_DIR 'unifyterm.log') -Raw -ErrorAction Ignore
        Report 'a refused key is written to the log' ($logged -match 'suggestion: .*401.*invalid x-api-key') "the log has the 401: $($logged -match 'suggestion: .*401')"

        # A sentence after # at the prompt. With no model named the line is the shell's, a comment, as it was.
        $before = (Sent).Count
        $t = Run 'ask-none' "# list the files{ENTER}{WAIT:1200}echo after{ENTER}{WAIT:600}" $quiet
        Report 'no model: a line with # is the shell''s comment' ((Sent).Count -eq $before -and (Count $t '# list the files') -eq 1 -and (Count $t 'after') -eq 2) "requests: $((Sent).Count - $before); the line on the screen: $(Count $t '# list the files') time(s); 'after' seen $(Count $t 'after') times (the line, then its output)"

        # With a model the sentence becomes a command on the input line: not run until Enter.
        $t = Run 'ask-put' "# list the files{ENTER}{WAIT:2500}" $model
        $words = ((Sent | Select-Object -Skip $before) -join "`n") -match 'Request: list the files'
        Report 'a sentence becomes a command on the line, not run' ((LastLine $t) -match '> echo asked-by-model$' -and (Count $t 'asked-by-model') -eq 1 -and $words) "last line: '$(LastLine $t)'; 'asked-by-model' seen $(Count $t 'asked-by-model') time(s); the words were sent: $words"
        $t = Run 'ask-run' "# list the files{ENTER}{WAIT:2500}{ENTER}{WAIT:800}" $model
        Report 'Enter then runs it' ((Count $t 'asked-by-model') -eq 2) "'asked-by-model' seen $(Count $t 'asked-by-model') times (the line, then its output)"
        $t = Run 'ask-chinese' "# 列出文件{ENTER}{WAIT:2500}" $model
        $words = ((Sent) -join "`n") -match 'Request: 列出文件'
        Report 'a sentence in Chinese is sent as written' ((LastLine $t) -match '> echo asked-by-model$' -and $words) "last line: '$(LastLine $t)'; the words were sent: $words"

        $before = (Sent).Count
        $t = Run 'ask-off' "# list the files{ENTER}{WAIT:1500}" "$model`nask = false"
        Report 'ai.ask = false leaves the line to the shell' ((Asked $before) -eq 0 -and (Count $t 'asked-by-model') -eq 0) "questions about a sentence: $(Asked $before)"

        $t = Run 'ask-lines' "# two lines please{ENTER}{WAIT:2500}" $model
        Report 'a command of two lines never reaches the line' ((LastLine $t) -match '> # two lines please$' -and (Count $t 'echo one') -eq 0) "last line: '$(LastLine $t)'; 'echo one' seen $(Count $t 'echo one') time(s)"
        $t = Run 'ask-nothing' "# nothing to do{ENTER}{WAIT:2500}" $model
        Report 'no command for the sentence: the line stays' ((LastLine $t) -match '> # nothing to do$') "last line: '$(LastLine $t)'"

        # ask_enter = "auto": what only reads is run at once, anything else waits for Enter.
        $auto = "$model`nask_enter = `"auto`""
        $t = Run 'ask-auto' "# list the files{ENTER}{WAIT:3000}" $auto
        Report 'auto: a command that only reads is run' ((Count $t 'asked-by-model') -eq 2) "'asked-by-model' seen $(Count $t 'asked-by-model') times (the line, then its output)"
        $t = Run 'ask-auto-held' "# delete the build{ENTER}{WAIT:3000}" $auto
        Report 'auto: one that may change something waits' ((LastLine $t) -match '> rm -rf zz-not-there$' -and (Count $t 'zz-not-there') -eq 1) "last line: '$(LastLine $t)'; 'zz-not-there' seen $(Count $t 'zz-not-there') time(s) (2 would be the command's own error)"

        # Not at the prompt (a program reads the keyboard): the line is the program's, and nothing is asked.
        $before = (Sent).Count
        $t = Run 'ask-program' "`$x = Read-Host{ENTER}{WAIT:800}# list the files{ENTER}{WAIT:1500}echo got=`$x{ENTER}{WAIT:600}" $model
        Report 'a program that reads the keyboard gets the line' ((Count $t 'got=# list the files') -eq 1 -and (Asked $before) -eq 0) "the program got it: $((Count $t 'got=# list the files') -eq 1); questions about a sentence: $(Asked $before)"

        # The answer takes 3 s and the line is changed meanwhile: the command is not put over it.
        Stop-Process -Id $server.Id -Force; $server = Start-Model 3; Start-Sleep 2
        $t = Run 'ask-changed' "# list the files{ENTER}{WAIT:600}x{WAIT:4500}" $model
        Report 'a line changed meanwhile is left as it is' ((LastLine $t) -match '> # list the filesx$' -and (Count $t 'asked-by-model') -eq 0) "last line: '$(LastLine $t)'"

        # A service that takes 20 s: the terminal goes on, nothing waits for it.
        Stop-Process -Id $server.Id -Force; $server = Start-Model 20; Start-Sleep 2
        $t = Run 'ai-slow' "zzqqyy-nothing{ENTER}{WAIT:500}echo still-alive{ENTER}{WAIT:1200}" $model
        Report 'a slow model holds nothing up' ((Count $t 'still-alive') -eq 2) "'still-alive' seen $(Count $t 'still-alive') times (the line, then its output)"

        $t = Run 'ai-nokey' "echo x{ENTER}" ($model -replace 'UNIFYTERM_TEST_KEY', 'UNIFYTERM_NO_SUCH_KEY_7Q') -keep
        Report 'a key that is not set is named, the terminal works' ((Count $t 'UNIFYTERM_NO_SUCH_KEY_7Q') -ge 1 -and (Count $t 'x') -ge 1) "the notice names the variable: $((Count $t 'UNIFYTERM_NO_SUCH_KEY_7Q') -ge 1)"
        # A service with OpenAI's shape that knows only JSON mode: asked the strictest way, then the way it takes.
        # Another model than the one agreed to: the first Enter brings the question, the second says yes.
        Stop-Process -Id $server.Id -Force; $server = Start-Model 0 'object'; Start-Sleep 2
        $chat = "$quiet`n[ai]`nprovider = `"openai-compatible`"`nmodel = `"local`"`nbase_url = `"http://localhost:$port/v1`""
        $before = (Sent).Count
        $t = Run 'ask-chat' "# list the files{ENTER}{WAIT:1200}{ENTER}{WAIT:3000}" $chat
        $formats = @(Sent | Select-Object -Skip $before | ForEach-Object { ($_ | ConvertFrom-Json).response_format.type }) -join ', '
        Report 'an OpenAI-shaped service is asked the way it takes' ((LastLine $t) -match '> echo asked-by-model$' -and $formats -eq 'json_schema, json_object') "last line: '$(LastLine $t)'; response_format of its requests: $formats"
        $before = (Sent).Count
        $t = Run 'ask-chat-again' "# list the files{ENTER}{WAIT:2500}" $chat
        $again = @(Sent | Select-Object -Skip $before | ForEach-Object { ($_ | ConvertFrom-Json).response_format.type }) -join ', '
        Report 'a new window starts from the strictest way again' ((LastLine $t) -match '> echo asked-by-model$' -and $again -eq 'json_schema, json_object') "response_format of its requests: $again"

        # A program on this machine answers: the question on its standard input, in a directory of its own.
        $questions = Join-Path $work 'agent-questions.txt'
        $script = "$PSScriptRoot\fake-agent.ps1" -replace '\\', '/'
        $agent = "$quiet`n[ai]`nprovider = `"command`"`ncommand = [`"pwsh`", `"-NoProfile`", `"-File`", `"$script`", `"-Log`", `"$($questions -replace '\\', '/')`"]"
        $before = (Sent).Count
        $t = Run 'ask-agent' "# list the files{ENTER}{WAIT:1200}{ENTER}{WAIT:6000}" $agent
        $got = (Get-Content $questions -Raw -ErrorAction Ignore) -match 'Request: list the files'
        $own = Join-Path $env:UNIFYTERM_DATA_DIR 'agent-cwd'
        $empty = (Test-Path $own) -and @(Get-ChildItem $own -Force).Count -eq 0
        Report 'a program on this machine answers a sentence' ((LastLine $t) -match '> echo asked-by-agent$' -and $got -and $empty -and (Sent).Count -eq $before) "last line: '$(LastLine $t)'; it read the question: $got; its directory is there and empty: $empty; requests to a service: $((Sent).Count - $before)"
        $t = Run 'ask-agent-missing' "# list the files{ENTER}{WAIT:1200}{ENTER}{WAIT:2500}" "$quiet`n[ai]`nprovider = `"command`"`ncommand = [`"unifyterm-no-such-agent-71b3`"]"
        Report 'a program that is not there: the line stays' ((LastLine $t) -match '> # list the files$') "last line: '$(LastLine $t)'"
    } finally {
        Stop-Process -Id $server.Id -Force -ErrorAction Ignore
        $env:UNIFYTERM_DATA_DIR = $keptData
        Remove-Item Env:UNIFYTERM_TEST_KEY -ErrorAction Ignore
    }
}
if ($Section -in 'all', 'programs') {
    "--- programs ---"
    # What a program says of its own commands (ut program), used with no model: in the list
    # under the input line, to put a mistyped command right, to tell what a line means.
    # A data directory of this section's own with one description in it, and one with none:
    # the same keys in both show what the description adds.
    $keptData = $env:UNIFYTERM_DATA_DIR
    $with = Join-Path $work 'data-programs'; $without = Join-Path $work 'data-programs-none'
    $env:UNIFYTERM_DATA_DIR = $with
    $ut = Join-Path (Split-Path $Exe) 'ut.exe'
    # A small part of git, written here: git is on this machine (the ai section's first check
    # needs it too) and refuses a command it does not know with exit code 1.
    $description = Join-Path $work 'git.opencli.json'
    Set-Content $description -Encoding utf8 -Value @"
{"opencli": "0.1", "info": {"title": "git", "summary": "A small part of git, for these checks.", "version": "0.0.1"},
 "command": {"name": "git", "commands": [
   {"name": "status", "description": "Show the working tree status.",
    "options": [{"name": "--short", "aliases": ["-s"], "description": "Give the output in the short format."}]},
   {"name": "stash", "description": "Stash the changes away."},
   {"name": "commit", "description": "Record changes.", "options": [{"name": "--amend", "description": "Replace the last commit."}]}]}}
"@
    $noList = "[history]`ncandidates = false"
    $lastOf = { param([string]$text) @($text -split "`n" | Where-Object { $_.Trim() })[-1] }
    function BelowLine([string]$a, [string]$b) { PixelDiff $a $b 0 ($Row.Y + $Row.H) 900 220 }
    # PowerShell's own prediction (grey text after the cursor, from the user's PowerShell history:
    # on the owner's machine 'git sta' has one) is turned off first: the text read back is then
    # only what was typed and what UnifyTerm put on the line. Kept out of both histories as above.
    $plain = "Set-PSReadLineOption -PredictionSource None; cls # password: a test's line{ENTER}{WAIT:700}"
    $port = 8766
    $requests = Join-Path $work 'programs-requests.jsonl'
    $server = $null
    try {
        $added = (& $ut program add $description 2>&1 | Out-String).Trim()
        $known = (& $ut program 2>&1 | Out-String).Trim()
        Report 'ut program add keeps a description' ($added -eq 'added git 0.0.1: 3 commands, 2 options' -and $known -eq 'git 0.0.1: 3 commands, 2 options') "add: '$added'; ut program: '$known'"

        # The history is empty here: every row of the list is a word of the description.
        Run 'prog-list' "${plain}git sta{WAIT:1200}" | Out-Null
        Run 'prog-nolist' "${plain}git sta{WAIT:1200}" $noList | Out-Null
        $env:UNIFYTERM_DATA_DIR = $without
        Run 'prog-none' "${plain}git sta{WAIT:1200}" | Out-Null
        $env:UNIFYTERM_DATA_DIR = $with
        $shown = BelowLine 'prog-list' 'prog-nolist'; $bare = BelowLine 'prog-none' 'prog-nolist'
        Report "a described program's commands are in the list" ($shown -gt 300 -and $bare -lt 50) "$shown pixels differ below the input line from the list off; with no description: $bare"
        $t = Run 'prog-take' "${plain}git sta{WAIT:1200}{KEY:CTRL+N}{WAIT:300}{TAB}{WAIT:900}"
        Report 'Ctrl-N and Tab put the first command on the line' ((& $lastOf $t) -match '> git status$') "last line: '$(& $lastOf $t)' (the description's first command that begins with 'sta')"
        $t = Run 'prog-option' "${plain}git commit --{WAIT:1200}{KEY:CTRL+N}{WAIT:300}{TAB}{WAIT:900}"
        Report 'after a dash, the options of the command' ((& $lastOf $t) -match '> git commit --amend$') "last line: '$(& $lastOf $t)'"

        # git does not know 'statsu' and ends with 1: the description has 'status' one keystroke away.
        $t = Run 'prog-fix' "${plain}git statsu{ENTER}{WAIT:2000}{ENTER}{WAIT:800}" $noList
        $refused = Count $t "is not a git command"
        Report 'a mistyped command of a described program is put right' ((& $lastOf $t) -match '> git status$' -and $refused -ge 1) "last line after Enter: '$(& $lastOf $t)'; git refused it: $($refused -ge 1)"
        $t = Run 'prog-fix-off' "${plain}git statsu{ENTER}{WAIT:2000}{ENTER}{WAIT:800}" "$noList`n[ai]`nfix = false"
        $env:UNIFYTERM_DATA_DIR = $without
        $u = Run 'prog-fix-none' "${plain}git statsu{ENTER}{WAIT:2000}{ENTER}{WAIT:800}" $noList
        $env:UNIFYTERM_DATA_DIR = $with
        Report 'no fix with ai.fix off, none with no description' ((& $lastOf $t) -notmatch 'git status$' -and (& $lastOf $u) -notmatch 'git status$') "last lines: '$(& $lastOf $t)' | '$(& $lastOf $u)'"

        # No model is named. With the description the box tells each word (6 lines); without, it
        # says that no model is set (2 lines): the difference is in the box's lines 3 to 5.
        Run 'prog-explain' "${plain}git commit --amend{WAIT:500}{KEY:CTRL+SHIFT+E}{WAIT:1500}" $noList | Out-Null
        $env:UNIFYTERM_DATA_DIR = $without
        Run 'prog-explain-none' "${plain}git commit --amend{WAIT:500}{KEY:CTRL+SHIFT+E}{WAIT:1500}" $noList | Out-Null
        $env:UNIFYTERM_DATA_DIR = $with
        $told = PixelDiff 'prog-explain' 'prog-explain-none' 0 ($Row.Y + 3 * $Row.H) 900 (3 * $Row.H)
        Report 'Ctrl-Shift-E tells a described command with no model' ($told -gt 300) "$told pixels differ in the box's lines 3 to 5 from the same keys with no description"

        # With a model (tools/fake-model.ps1 on this machine): the description goes along with a
        # sentence that names the program, and the model's command is checked against it.
        $server = Start-Process pwsh -ArgumentList "-NoProfile -File `"$PSScriptRoot\fake-model.ps1`" -Port $port -Log `"$requests`"" -PassThru -WindowStyle Hidden
        Start-Sleep 2
        $env:UNIFYTERM_TEST_KEY = 'test-key'
        $model = "$noList`n[ai]`nprovider = `"anthropic`"`nmodel = `"claude-haiku-5-5`"`nbase_url = `"http://localhost:$port`"`napi_key_env = `"UNIFYTERM_TEST_KEY`""
        $auto = "$model`nask_enter = `"auto`""
        $sent = { @(Get-Content $requests -ErrorAction Ignore) }
        # Nothing agreed to yet in this data directory: the first Enter brings the question, the second says yes.
        # The request that carries a sentence, among all the service got (a suggestion may be asked for after it).
        $about = { param([string]$words) "$(@(& $sent | Where-Object { $_ -match "Request: $words" })[-1])" }
        $t = Run 'prog-ask' "${plain}# git status in short form{ENTER}{WAIT:1200}{ENTER}{WAIT:3000}" $model
        $request = & $about 'git status in short form'
        $named = $request -match '- git 0\.0\.1: A small part of git'; $table = $request -match 'The commands of git'; $options = $request -match 'status \[--short\]'
        Report 'the description goes with a sentence that names the program' ((& $lastOf $t) -match '> git status --short$' -and $named -and $table -and $options) "last line: '$(& $lastOf $t)'; in the request: the program's line $named, its commands $table, their options $options ($((& $sent).Count) requests, this one $($request.Length) characters)"
        $t = Run 'prog-ask-other' "${plain}# list the files{ENTER}{WAIT:3000}" $model
        $request = & $about 'list the files'
        $named = $request -match '- git 0\.0\.1: A small part of git'; $table = $request -match 'The commands of git'
        Report 'a sentence that does not name it gets one line of it' ($named -and -not $table) "the program's line: $named; its commands: $table"
        # ask_enter = "auto": 'git status --short' only reads and the description has it: run at once.
        # '--frobnicate' is no option of the description: left on the line, whatever the mode.
        $before = (& $sent).Count
        $t = Run 'prog-ask-auto' "${plain}# git status in short form{ENTER}{WAIT:3500}" $auto
        $ran = (& $lastOf $t) -notmatch 'git status --short$' -and (Count $t 'git status --short') -ge 1
        # How often the model was asked says, when this fails, whether the window took the line for a sentence at all.
        $askedNow = @(& $sent | Select-Object -Skip $before | Where-Object { $_ -match 'Request: git status in short form' }).Count
        Report 'auto: a command the description has is run' $ran "last line: '$(& $lastOf $t)' (the prompt again: it ran); the command on the screen $(Count $t 'git status --short') time(s); the model asked about the sentence $askedNow time(s)"
        $t = Run 'prog-ask-doubt' "${plain}# git status frob{ENTER}{WAIT:3500}" $auto
        Report 'auto: an option the description does not have is not run' ((& $lastOf $t) -match '> git status --frobnicate$' -and (Count $t 'frobnicate') -eq 1) "last line: '$(& $lastOf $t)'; 'frobnicate' seen $(Count $t 'frobnicate') time(s) (2 would be git's own error)"

        $removed = (& $ut program remove git 2>&1 | Out-String).Trim()
        $left = (& $ut program 2>&1 | Out-String).Trim()
        Report 'ut program remove forgets it' ($removed -eq 'removed git' -and $left -match '^no program') "remove: '$removed'; ut program: '$left'"
    } finally {
        if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction Ignore }
        $env:UNIFYTERM_DATA_DIR = $keptData
        Remove-Item Env:UNIFYTERM_TEST_KEY -ErrorAction Ignore
    }
}
if ($Section -in 'all', 'graphics') {
    "--- graphics ---"
    # The test picture: four colored quarters (and a white diagonal), drawn here, so its colors are known exactly.
    # In a fixed order: what is printed must be the same from run to run.
    $quarters = [ordered]@{ red = '#dc2828'; green = '#28c83c'; blue = '#2850dc'; yellow = '#e6d228' }
    $png = Join-Path $work 'picture.png'
    $bitmap = [System.Drawing.Bitmap]::new(64, 64)
    try {
        for ($y = 0; $y -lt 64; $y++) {
            for ($x = 0; $x -lt 64; $x++) {
                $hex = if ($x -eq $y -or $x + 1 -eq $y) { '#ffffff' } elseif ($y -lt 32) { if ($x -lt 32) { $quarters.red } else { $quarters.green } } else { if ($x -lt 32) { $quarters.blue } else { $quarters.yellow } }
                $bitmap.SetPixel($x, $y, [System.Drawing.ColorTranslator]::FromHtml($hex))
            }
        }
        $bitmap.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    $md = Join-Path $work 'doc.md'
    [IO.File]::WriteAllText($md, "# Title`n`n- one`n- two`n`n| a | b |`n|---|---|`n| 1 | 2 |`n")
    function Quarters([string]$name) { @($quarters.Keys | ForEach-Object { CountColor $name $quarters[$_] 0 40 900 560 }) }
    $show = "ut view '$png' --width 20{ENTER}{WAIT:1800}"

    Run 'gfx-view' $show | Out-Null
    $counts = Quarters 'gfx-view'
    Report 'ut view draws the picture in the window' (@($counts | Where-Object { $_ -gt 1000 }).Count -eq 4) "pixels of red, green, blue, yellow: $($counts -join ', ')"
    $t = Run 'gfx-pipe' "ut view '$png' --width 20 | cat{ENTER}{WAIT:1500}"
    Report 'through a pipe it is ASCII, not a picture' ((Count $t '====') -ge 5 -and (@(Quarters 'gfx-pipe' | Where-Object { $_ -gt 0 }).Count -eq 0)) "rows with '====': $(Count $t '===='); picture pixels: $((Quarters 'gfx-pipe') -join ', ')"
    $t = Run 'gfx-half' "ut view '$png' --mode half --width 20{ENTER}{WAIT:1500}"
    Report 'ut view --mode half draws half blocks' ((Count $t '▀') -ge 100) "'▀' on the screen: $(Count $t '▀')"

    Run 'gfx-scroll' ($show + "1..45 | % { `"l `$_`" }{ENTER}{WAIT:1500}") | Out-Null
    $scrolled = Quarters 'gfx-scroll'
    Report 'the picture scrolls away with the text' (@($scrolled | Where-Object { $_ -gt 0 }).Count -eq 0) "pixels of the quarters after 45 lines: $($scrolled -join ', ')"
    # On a full screen, 3 lines printed under the picture move it up 4 rows (the 3 and the next prompt).
    function RedTop([string]$name) {
        $i = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$name.png"))
        try { for ($y = 40; $y -lt $i.Height; $y++) { for ($x = 0; $x -lt 300; $x++) { $p = $i.GetPixel($x, $y); if ([Math]::Abs($p.R - 0xdc) -le $ChannelTolerance -and [Math]::Abs($p.G - 0x28) -le $ChannelTolerance -and [Math]::Abs($p.B - 0x28) -le $ChannelTolerance) { return $y } } }; -1 } finally { $i.Dispose() }
    }
    Run 'gfx-move' ("1..40 | % { `"f `$_`" }{ENTER}{WAIT:800}" + $show + "{SHOT:gfx-move-a}1..3 | % { `"l `$_`" }{ENTER}{WAIT:1000}") | Out-Null
    $before = RedTop 'gfx-move-a'; $after = RedTop 'gfx-move'; $moved = $before - $after
    $whole = (CountColor 'gfx-move-a' $quarters.red 0 40 900 560) -eq (CountColor 'gfx-move' $quarters.red 0 40 900 560)
    Report 'the picture moves up with its text, row for row' ($before -gt 0 -and $after -gt 0 -and $moved -ge 4 * 18 -and $moved -le 4 * 20 -and $whole) "top of the picture: $before px, then $after px: $moved px for 4 rows of about 19 px; still whole: $whole"
    Run 'gfx-clear' ($show + "clear{ENTER}{WAIT:1000}") | Out-Null
    $cleared = Quarters 'gfx-clear'
    Report 'clearing the screen takes the picture away' (@($cleared | Where-Object { $_ -gt 0 }).Count -eq 0) "pixels after clear: $($cleared -join ', ')"
    Run 'gfx-resize' ($show + "{SIZE:760,460}{WAIT:1200}") | Out-Null
    $resized = Quarters 'gfx-resize'
    Report 'a smaller window keeps the picture where its text is' (@($resized | Where-Object { $_ -gt 500 }).Count -eq 4) "pixels of red, green, blue, yellow after the resize: $($resized -join ', ')"
    $t = Run 'gfx-md' "ut md '$md'{ENTER}{WAIT:1200}"
    Report 'ut md draws headings, lists and tables' ((Count $t 'Title') -ge 1 -and (Count $t '•') -eq 2 -and (Count $t '┌') -ge 1) "'Title' $(Count $t 'Title'), '•' $(Count $t '•'), '┌' $(Count $t '┌')"

    # Windows Terminal has sixel: ut view chooses it there by itself.
    if (Get-Command wt -ErrorAction SilentlyContinue) {
        $said = Join-Path $work 'wt-choice.txt'
        $utExe = Join-Path (Split-Path $Exe) 'ut.exe'
        # These checks may be run from a tab of UnifyTerm: what it tells its own shells (TERM_PROGRAM)
        # would go on to the program in Windows Terminal, which would take that for the terminal it is in.
        Start-Process wt -ArgumentList @('-w', 'new', '--title', 'unifyterm-verify', 'cmd', '/c', "set TERM_PROGRAM=& set KITTY_WINDOW_ID=& `"$utExe`" view `"$png`" --width 8 --verbose 2> `"$said`" & ping -n 2 127.0.0.1 >nul")
        for ($i = 0; $i -lt 50 -and -not ((Test-Path $said) -and (Get-Item $said).Length -gt 0); $i++) { Start-Sleep -Milliseconds 200 }
        $choice = if (Test-Path $said) { (Get-Content $said -Raw).Trim() } else { '(nothing said)' }
        Report 'in Windows Terminal, ut view chooses sixel by itself' ($choice -match '^ut view: sixel') $choice
    } else {
        "SKIP  in Windows Terminal, ut view chooses sixel by itself: Windows Terminal is not installed here"
    }

    # A video: 4 s, 64 × 64, red for 2 s, then blue (made here with ffmpeg, so its colors are known).
    if (Get-Command ffmpeg -ErrorAction SilentlyContinue) {
        $video = Join-Path $work 'video.mp4'
        ffmpeg -v error -y -f lavfi -i "color=c=$($quarters.red.TrimStart('#')):s=64x64:d=2:r=10" -f lavfi -i "color=c=$($quarters.blue.TrimStart('#')):s=64x64:d=2:r=10" -filter_complex '[0][1]concat=n=2:v=1[v]' -map '[v]' -pix_fmt yuv420p $video
        # yuv420p keeps a color within a few steps of 255: 6 here.
        # A point inside the picture: the command takes two rows of the window, the picture begins under them.
        $InVideo = $Row.Y + 2 * $Row.H + 20
        function Near([string]$name, [string]$hex) { $p = PixelAt $name 40 $InVideo; $a = [Convert]::ToInt32($p.TrimStart('#'), 16); $b = [Convert]::ToInt32($hex.TrimStart('#'), 16); (0, 8, 16 | ForEach-Object { [Math]::Abs((($a -shr $_) -band 255) - (($b -shr $_) -band 255)) } | Measure-Object -Maximum).Maximum -le 6 }
        Run 'video' "ut view '$video' --width 8{ENTER}{WAIT:1000}{SHOT:video-1s}{WAIT:2000}{SHOT:video-3s}{WAIT:2500}" | Out-Null
        Report 'a video plays in the window, at its own pace' ((Near 'video-1s' $quarters.red) -and (Near 'video-3s' $quarters.blue)) "at 1 s: $(PixelAt 'video-1s' 40 $InVideo) (red $($quarters.red)); at 3 s: $(PixelAt 'video-3s' 40 $InVideo) (blue $($quarters.blue))"
        $long = Join-Path $work 'video-long.mp4'
        ffmpeg -v error -y -f lavfi -i 'testsrc=s=64x64:d=10:r=10' -pix_fmt yuv420p $long
        $t = Run 'video-q' "ut view '$long' --width 8{ENTER}{WAIT:1500}q{WAIT:900}echo stopped-early{ENTER}{WAIT:700}"
        $raw = Get-Content (Join-Path $work 'video-q.raw') -Raw
        $shown = $raw.LastIndexOf("`e[?25h") -gt $raw.LastIndexOf("`e[?25l")
        $left = @(Get-Process ffmpeg -ErrorAction SilentlyContinue).Count
        Report 'q stops a video: the prompt is back, the cursor shown, no ffmpeg left' ((Count $t 'stopped-early') -ge 2 -and $shown -and $left -eq 0) "'stopped-early' on the screen: $(Count $t 'stopped-early') (typed 0.9 s after q); cursor shown: $shown; ffmpeg processes: $left"
        $saved = $env:PATH; $env:PATH = Join-Path $env:SystemRoot 'System32'
        try { $said = (& (Join-Path (Split-Path $Exe) 'ut.exe') view $video 2>&1 | Out-String); $code = $LASTEXITCODE } finally { $env:PATH = $saved }
        Report 'no ffmpeg on PATH: ut view says how to get it' ($code -eq 1 -and $said -match 'needs ffmpeg' -and $said -match 'winget install') "exit $code; '$(($said -split "`n")[0].Trim())'"
    } else {
        "SKIP  videos: ffmpeg is not installed here"
    }

    # Thirty pictures of 2048 × 2048 (16 MiB each once decoded): the terminal keeps at most 256 MiB of them.
    $big = 1..30 | ForEach-Object {
        $file = Join-Path $work "big-$_.png"
        $image = [System.Drawing.Bitmap]::new(2048, 2048)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($image)
            $graphics.Clear([System.Drawing.Color]::FromArgb(255, $_ * 8, 255 - $_ * 8, 128)); $graphics.Dispose()
            $image.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $image.Dispose() }
        $file
    }
    $many = ($big | ForEach-Object { "ut view '$_' --width 4{ENTER}{WAIT:700}" }) -join ''
    Run 'gfx-many' ($many + "{WAIT:1500}{MEM:gfx-many}") | Out-Null
    Run 'gfx-none' ("echo none{ENTER}{WAIT:1500}{MEM:gfx-none}") | Out-Null
    $grown = ([long](Get-Content (Join-Path $work 'gfx-many.mem')) - [long](Get-Content (Join-Path $work 'gfx-none.mem'))) / 1MB
    Report 'thirty 16 MiB pictures: memory stays under the limit' ($grown -lt 356) ("{0:N0} MiB more than with no picture (limit: 256 MiB of pictures + 100)" -f $grown)
}
if ($Section -in 'all', 'agents') {
    "--- agents ---"
    $utExe = Join-Path (Split-Path $Exe) 'ut.exe'
    # Made-up agents in keepane's shape (crates/unifyterm/tests/fixtures/agents.json): two of them.
    $fixture = Join-Path $work 'agents.json'
    Copy-Item (Join-Path $PSScriptRoot '..\crates\unifyterm\tests\fixtures\agents.json') $fixture
    $list = "command = ['$utExe', 'cat', '$fixture']"
    $logFile = Join-Path $env:UNIFYTERM_DATA_DIR 'unifyterm.log'
    function Logged { if (Test-Path $logFile) { Get-Content $logFile -Raw } else { '' } }
    # The sidebar's cards: runs of the card color, at least 40 px tall, down a column inside them.
    function Cards([string]$name) {
        $i = [System.Drawing.Bitmap]::FromFile((Join-Path $work "$name.png"))
        try {
            # The sidebar is 268 px wide; its cards begin 16 px in. Down a column 4 px inside a card: no text there.
            $right = $i.Width - 8; $x = $right - 268 + 16 + 4; $back = $i.GetPixel($right - 268 + 4, $i.Height - 30)
            $runs = 0; $run = 0
            for ($y = 95; $y -lt $i.Height - 10; $y++) {
                $p = $i.GetPixel($x, $y)
                $differs = [Math]::Abs($p.R - $back.R) + [Math]::Abs($p.G - $back.G) + [Math]::Abs($p.B - $back.B) -gt 6
                if ($differs) { $run++ } else { if ($run -ge 40) { $runs++ }; $run = 0 }
            }
            $runs
        } finally { $i.Dispose() }
    }

    Run 'agents-shown' "{WAIT:1500}" "[agents]`nsidebar = true`n$list" | Out-Null
    Report 'agents.sidebar = true: a card for each agent' ((Cards 'agents-shown') -eq 2) "cards: $(Cards 'agents-shown') (the list has 2)"
    # keepane gives null for a figure it does not know (the cost of an agent it has no prices for), and may
    # one day give an entry this version cannot read: the other cards are shown, and the log names that entry.
    $odd = Join-Path $work 'agents-odd.json'
    $more = ',{"pane":"%14","target":"work:3.0","agent":"other","model":null,"cost":null,"context_pct":null,"cpu":1.0,"mem":1048576,"turns":2},{"pane":"%15","mem":"lots"}]'
    [IO.File]::WriteAllText($odd, (Get-Content $fixture -Raw).Replace('"cost":3.25', '"cost":null').TrimEnd().TrimEnd(']') + $more)
    Run 'agents-odd' "{WAIT:1500}" "[agents]`nsidebar = true`ncommand = ['$utExe', 'cat', '$odd']" | Out-Null
    $unread = ([regex]::Matches((Logged), "agents: 1 of the 4 entries in keepane's answer are not an agent's: entry 4: ")).Count
    Report 'a cost that is null, and an entry that cannot be read: the other cards are shown' ((Cards 'agents-odd') -eq 3 -and $unread -eq 1) "cards: $(Cards 'agents-odd') (4 entries: 2 with a null cost, 1 that is not an agent's); log lines that name entry 4: $unread"
    Run 'agents-hidden' "{WAIT:1500}" "[agents]`n$list" | Out-Null
    Report 'by default the sidebar is hidden' ((Cards 'agents-hidden') -eq 0) "cards: $(Cards 'agents-hidden')"
    Run 'agents-key' "{KEY:CTRL+SHIFT+A}{WAIT:1500}" "[agents]`n$list" | Out-Null
    Run 'agents-key-twice' "{KEY:CTRL+SHIFT+A}{WAIT:1500}{KEY:CTRL+SHIFT+A}{WAIT:800}" "[agents]`n$list" | Out-Null
    Report 'Ctrl-Shift-A shows the sidebar, and hides it again' ((Cards 'agents-key') -eq 2 -and (Cards 'agents-key-twice') -eq 0) "cards: $(Cards 'agents-key'), then $(Cards 'agents-key-twice')"
    # [keys]: the sidebar's key is the one the settings give, and the default one does nothing then.
    Run 'keys-old' "{KEY:CTRL+SHIFT+A}{WAIT:1500}" "[keys]`nagents = [`"ctrl-alt-g`"]`n[agents]`n$list" | Out-Null
    Run 'keys-new' "{KEY:CTRL+ALT+G}{WAIT:1500}" "[keys]`nagents = [`"ctrl-alt-g`"]`n[agents]`n$list" | Out-Null
    Report 'keys.agents: the settings'' key shows the sidebar, the old one does not' ((Cards 'keys-old') -eq 0 -and (Cards 'keys-new') -eq 2) "cards after Ctrl-Shift-A: $(Cards 'keys-old'); after Ctrl-Alt-G: $(Cards 'keys-new')"
    # Changed by ut setting in a tab: the new key is the window's key at once, written in the one form.
    $t = Run 'keys-live' "ut setting set keys.agents Ctrl+Alt+J{ENTER}{WAIT:2500}ut setting get keys.agents{ENTER}{WAIT:700}{KEY:CTRL+ALT+J}{WAIT:1500}" "[agents]`n$list" -after 1.5
    Report 'a key changed in a tab is the window''s key at once' ((Cards 'keys-live') -eq 2 -and ($t -replace "`n", ' ') -match '(?<!set keys\.agents )ctrl-alt-j') "cards after the new key: $(Cards 'keys-live'); kept as: $([regex]::Match(($t -replace "`n", ' '), 'ctrl-alt-j').Value)"
    $t = Run 'keys-wrong' "ut setting set keys.agents ctrl-shift-t{ENTER}{WAIT:1200}ut setting set keys.agents g{ENTER}{WAIT:1200}" "[agents]`n$list"
    $flat = $t -replace "`n", ' '
    Report 'a key another command has, and a key the shell needs, are refused' ($flat -match 'keys\.new_tab and keys\.agents both have' -and $flat -match 'the shell needs that key') "said of the taken key: $($flat -match 'both have'); of the bare letter: $($flat -match 'the shell needs that key')"

    # A click on the first card runs agents.focus with its pane; when that fails, the place is copied.
    $went = Join-Path $work 'went'
    New-Item -ItemType Directory -Force $went | Out-Null
    $t = Run 'agents-go' "{WAIT:1500}{CLICK:800,140}{WAIT:1500}echo typed-after-click{ENTER}{WAIT:600}" "[agents]`nsidebar = true`n$list`nfocus = ['$utExe', 'touch', '$went\{pane}']"
    $made = @(Get-ChildItem $went -Name)
    Report 'a click on a card runs agents.focus with its pane' ($made.Count -eq 1 -and $made[0] -eq '%12') "made: $($made -join ', ') (the first card's pane is %12)"
    Report 'after a click on a card, the keys still go to the terminal' ((Count $t 'typed-after-click') -ge 2) "'typed-after-click' on the screen: $(Count $t 'typed-after-click') (the line and its output)"
    $saved = Get-Clipboard -Raw
    try {
        Set-Clipboard 'nothing yet'
        Run 'agents-copy' "{WAIT:1500}{CLICK:800,140}{WAIT:1500}" "[agents]`nsidebar = true`n$list`nfocus = ['unifyterm-no-such-program', '{pane}']" | Out-Null
        $copied = Get-Clipboard -Raw
        Report 'when going there fails, the place is copied' ($copied -eq 'work:1.0' -and (Logged) -match 'going to %12') "clipboard: '$copied'; logged: $((Logged) -match 'going to %12')"
    } finally { if ($null -ne $saved) { Set-Clipboard -Value $saved } }

    # keepane's troubles: a line in the sidebar and one in the log, and the terminal works on.
    $t = Run 'agents-noserver' "{WAIT:2500}echo still-fine{ENTER}{WAIT:600}" "[agents]`nsidebar = true`npoll_ms = 500`ncommand = ['keepane', '-L', 'unifyterm-test-none', 'list-agents', '-J']"
    $noServer = ([regex]::Matches((Logged), 'agents: keepane: no server running')).Count
    Report 'no keepane server: said once, and the terminal works on' ($noServer -eq 1 -and (Count $t 'still-fine') -ge 2 -and (Cards 'agents-noserver') -eq 0) "log lines: $noServer (asked every 500 ms for 3 s: about 6 times); 'still-fine' on the screen: $(Count $t 'still-fine')"
    $t = Run 'agents-noprogram' "{WAIT:2500}echo still-fine{ENTER}{WAIT:600}" "[agents]`nsidebar = true`ncommand = ['unifyterm-no-such-program']"
    Report 'no keepane program: said, and the terminal works on' ((Logged) -match 'agents: unifyterm-no-such-program was not found' -and (Count $t 'still-fine') -ge 2) "logged: $((Logged) -match 'was not found'); 'still-fine' on the screen: $(Count $t 'still-fine')"
    if (Get-Command keepane -ErrorAction SilentlyContinue) {
        # A test server of our own (socket unifyterm-test), with two panes and no agent in them.
        # keepane saves a server's sessions when it stops, and keeps a copy of PowerShell's history
        # for each pane: each session is stopped and forgotten first, then the socket's folder goes.
        function Clear-TestKeepane {
            foreach ($name in @(keepane -L unifyterm-test list-saved 2>$null | ForEach-Object { if ($_ -match '^([^:]+): saved') { $Matches[1] } })) {
                keepane -L unifyterm-test kill-session -t $name 2>&1 | Out-Null
                keepane -L unifyterm-test delete-saved $name 2>&1 | Out-Null
            }
            keepane -L unifyterm-test kill-server 2>&1 | Out-Null
            Remove-Item (Join-Path $env:LOCALAPPDATA 'keepane\sessions-unifyterm-test') -Recurse -Force -ErrorAction SilentlyContinue
        }
        Clear-TestKeepane
        keepane -L unifyterm-test new-session -d -s verify -x 160 -y 50 2>&1 | Out-Null
        keepane -L unifyterm-test split-window -t verify 2>&1 | Out-Null
        $panes = @(keepane -L unifyterm-test list-panes -a 2>$null).Count
        try {
            Run 'agents-keepane' "{WAIT:2500}" "[agents]`nsidebar = true`ncommand = ['keepane', '-L', 'unifyterm-test', 'list-agents', '-J']" | Out-Null
        } finally { Clear-TestKeepane }
        $left = Test-Path (Join-Path $env:LOCALAPPDATA 'keepane\sessions-unifyterm-test')
        Report 'a real keepane with no agents: no cards, nothing logged' ($panes -eq 2 -and (Cards 'agents-keepane') -eq 0 -and (Logged) -notmatch 'unifyterm-test[^-]' -and -not $left) "panes in the test server: $panes (2 made); cards: $(Cards 'agents-keepane'); its folder left after: $left"
    } else {
        "SKIP  a real keepane with no agents: keepane is not installed here"
    }

    # Hidden, nothing runs; shown, the list is asked for every poll_ms; hidden again, it stops.
    $count = Join-Path $work 'asked.txt'
    $counting = "command = ['cmd', '/c', 'echo x>>$count& type $fixture']`npoll_ms = 500"
    Run 'agents-idle' "{WAIT:6000}" "[agents]`n$counting" | Out-Null
    $idle = if (Test-Path $count) { @(Get-Content $count).Count } else { 0 }
    Run 'agents-polling' "{KEY:CTRL+SHIFT+A}{WAIT:3000}{KEY:CTRL+SHIFT+A}{WAIT:500}{SHOT:agents-polled}{WAIT:3000}" "[agents]`n$counting" | Out-Null
    $polled = if (Test-Path $count) { @(Get-Content $count).Count } else { 0 }
    Report 'hidden: the list is never asked for; shown: every poll_ms' ($idle -eq 0 -and $polled -ge 4 -and $polled -le 8) "runs in 6 s hidden: $idle; in 3 s shown at 500 ms, then 3 s hidden: $polled (6 expected, 4 to 8 allowed)"

    # ut mcp, as an agent's client: what a shell in unifyterm ran is found.
    Run 'agents-record' "echo mcp-probe-4217{ENTER}{WAIT:1200}" "" | Out-Null
    function Ask-Mcp([string[]]$lines, [string]$data) {
        $saved = $env:UNIFYTERM_DATA_DIR; $env:UNIFYTERM_DATA_DIR = $data
        try { $lines | & $utExe mcp 2>$null | ForEach-Object { $_ | ConvertFrom-Json } } finally { $env:UNIFYTERM_DATA_DIR = $saved }
    }
    $answers = @(Ask-Mcp @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":2,"method":"tools/list"}',
        '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"history_search","arguments":{"query":"mcp-probe"}}}'
    ) $env:UNIFYTERM_DATA_DIR)
    $found = if ($answers.Count -eq 3) { @($answers[2].result.content[0].text | ConvertFrom-Json) } else { @() }
    Report 'ut mcp: a client finds the command a shell ran' ($answers.Count -eq 3 -and $answers[1].result.tools.Count -eq 4 -and $found.Count -ge 1 -and $found[0].command -eq 'echo mcp-probe-4217') "answers: $($answers.Count); tools: $($answers[1].result.tools.Count); found: $($found | ForEach-Object command)"
    $empty = Join-Path $work 'no-data'
    New-Item -ItemType Directory -Force $empty | Out-Null
    $none = @(Ask-Mcp @('{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"history_recent","arguments":{}}}') $empty)
    $left = @(Get-ChildItem $empty -Force).Count
    Report 'ut mcp with no history: an empty list, and no file made' ($none.Count -eq 1 -and $none[0].result.content[0].text -eq '[]' -and $left -eq 0) "answer: $($none[0].result.content[0].text); files made: $left"
}
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
"$script:failed failed"
exit $script:failed
