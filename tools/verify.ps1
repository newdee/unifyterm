# Checks the built uni-term by running it: types into its window, reads back
# what its programs printed (UNI_TERM_RAW_LOG, replayed to text) and compares
# pictures of the window. Each check prints PASS or FAIL with the numbers it
# measured; the exit code is how many failed.
#
#   pwsh tools/verify.ps1 [-Section all|input|settings|edges|commands|history|ai|graphics] [-Exe path]
#
# It needs a desktop session (a window opens for each check, a few seconds
# each) but not the foreground: input and pictures go through the window's
# own handle.
param(
    [ValidateSet('all', 'input', 'settings', 'edges', 'commands', 'history', 'ai', 'graphics')][string]$Section = 'all',
    [string]$Exe = "$PSScriptRoot\..\target\debug\uni-term.exe"
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$Exe = (Resolve-Path $Exe).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "uni-term-verify-$PID"
# uni-term's data (command links, start scripts, history, log) for these checks only:
# never the user's own. Every uni-term started below inherits it.
$env:UNI_TERM_DATA_DIR = Join-Path $work 'data'
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
# The same without clearing the screen, for a check that reads what uni-term said before the shell started.
$NoHistoryKeep = $NoHistory.Replace('; cls ', ' ')
# After one typed line: the same for a shell that did not get the line above (a second tab, a shell fallen back to).
$Unsaved = ' # password: not for your history'

# Runs uni-term with a settings file's text, types, and saves a picture.
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
    $env:UNI_TERM_RAW_LOG = $log
    try {
        & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work "$name.png") -Type $type -Wait 4 -After $after -App:" --config `"$config`" $more" | Out-Null
    } finally { $env:UNI_TERM_RAW_LOG = $null }
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
# uni-term processes and the console hosts started from its folder.
function Leftovers { @(Get-Process uni-term, OpenConsole -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith((Split-Path $Exe)) }).Count }

# The first text row of the window, in window pixels (title bar and tab bar above it).
$Row = @{ Y = 66; H = 22 }

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
        # runs, the check says nothing about uni-term: it runs again, three times at most,
        # and the detail says which try it was (more than one: something else used the clipboard).
        $try = 0
        do {
            $try++
            $t = Run 'select' "cls; echo select-me-please{ENTER}{WAIT:600}{DRAG:7,50,134,50}{WAIT:300}{RCLICK:300,300}{WAIT:300}"
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
    Report "'exit' in the last tab ends uni-term" ($out -match 'exited: 0') "the runner saw: $(if ($out -match 'exited: (-?\d+)') { "ended with code $($Matches[1])" } else { 'still running after 3 s' })"    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) uni-term or console-host processes still running"
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
}

if ($Section -in 'all', 'edges') {
    "--- edges ---"
    $t = Run 'badkey' "echo still-works{ENTER}" "[font]`nsise = 12" -keep
    Report 'a misspelt setting: opens, names it' ((Count $t 'sise') -ge 1 -and (Count $t 'still-works') -eq 2) "'sise' seen $(Count $t 'sise') times, 'still-works' $(Count $t 'still-works')"

    $garbage = Join-Path $work 'garbage.toml'; [IO.File]::WriteAllBytes($garbage, [byte[]](0..255))
    $log = Join-Path $work 'garbage.raw'; $env:UNI_TERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'garbage.png') -Type ($NoHistoryKeep + "echo alive{ENTER}") -Wait 4 -After 1 -App:" --config `"$garbage`"" | Out-Null } finally { $env:UNI_TERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a settings file of random bytes: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $dir = Join-Path $work 'a-directory'; New-Item -ItemType Directory -Force $dir | Out-Null
    $log = Join-Path $work 'dir.raw'; $env:UNI_TERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'dir.png') -Type ($NoHistoryKeep + "echo alive{ENTER}") -Wait 4 -After 1 -App:" --config `"$dir`"" | Out-Null } finally { $env:UNI_TERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a directory as the settings file: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $t = Run 'badshell' "echo fell-back$Unsaved{ENTER}" "[shell]`nprogram = `"no-such-shell-xyz`""
    Report 'a shell that does not exist: the default one, says so' ((Count $t 'shell.program did not start') -eq 1 -and (Count $t 'fell-back') -eq 2) "'shell.program did not start' $(Count $t 'shell.program did not start'), 'fell-back' $(Count $t 'fell-back')"

    $p = Start-Process $Exe -ArgumentList '-e', 'no-such-program-xyz' -PassThru
    $gone = $p.WaitForExit(8000)
    Report 'a program that does not exist: ends, with failure' ($gone -and $p.ExitCode -ne 0) "ended: $gone, exit code $(if ($gone) { $p.ExitCode } else { 'none (still running)' })"
    if (-not $gone) { Stop-Process -Id $p.Id -Force }

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

    # Typed in a terminal (uni-term itself), with nothing redirected: a release
    # build is a window program and must still print there. It does not hold
    # the shell, so its text lands after the next prompt; nothing more is
    # typed in the same run, or the shell's line editor would draw over it.
    $t = Run 'cli-version' "& '$Exe' --version{ENTER}{WAIT:1500}"
    $version = [regex]::Match($t, 'uni-term \d+\.\d+\.\d+').Value
    Report '--version prints in a terminal' ([bool]$version) "printed '$version'"
    $t = Run 'cli-wrong' "& '$Exe' --nope{ENTER}{WAIT:1500}"
    Report 'a wrong argument is named in a terminal' ((Count $t 'unknown argument: --nope') -eq 1 -and (Count $t 'usage: uni-term') -eq 1) "'unknown argument: --nope' $(Count $t 'unknown argument: --nope'), 'usage: uni-term' $(Count $t 'usage: uni-term')"
    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) uni-term or console-host processes still running"
}

if ($Section -in 'all', 'commands') {
    "--- commands ---"
    $json = Join-Path $work 'data.json'; [IO.File]::WriteAllText($json, '{"items":[{"name":"alpha"},{"name":"beta"}]}')
    $kind = 'echo "cat=$((Get-Command cat).CommandType) ls=$((Get-Command ls).CommandType)"{ENTER}{WAIT:500}'
    $t = Run 'cmd-on' ($kind + 'echo "from=$((Get-Command cat).Source)"{ENTER}{WAIT:500}' + "cat '$json' | jq -r '.items[].name'{ENTER}{WAIT:900}ls --version{ENTER}{WAIT:700}")
    Report 'cat and ls are uni-term''s programs' ((Count $t 'cat=Application ls=Application') -eq 1) "the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"
    Report 'they come from uni-term''s command directory' ($t -match ('from=' + [regex]::Escape("$env:UNI_TERM_DATA_DIR\bin\cat.exe"))) "$([regex]::Match(($t -replace "`n", ''), 'from=[A-Z]:\\\S+').Value)"
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
    # The lines are typed, so this is also what says that a typed `cat` is uni-term's program and no other.
    'one' | Set-Content "$made\a\x1.txt"; 'two' | Set-Content "$made\a\x2.txt"; 'deep' | Set-Content "$made\a\b\x3.txt"
    $t = Run 'cmd-wild' "cd '$made\a'{ENTER}{WAIT:700}echo W1 x*.txt{ENTER}{WAIT:900}echo W2 `"x*.txt`" 'x?.txt'{ENTER}{WAIT:900}find . -name `"x*.txt`" | sort | tr '\n' ' '{ENTER}{WAIT:1200}cat x*.txt | wc -l{ENTER}{WAIT:1200}Get-History | ForEach-Object { 'H' + [int][bool](`$_.CommandLine -match [char]0xE000) } | Select-Object -Last 1{ENTER}{WAIT:900}"
    Report 'a wildcard becomes the names it matches' ($t -match '(?m)^W1 x1\.txt x2\.txt$') "'echo W1 x*.txt' printed: $([regex]::Match($t, '(?m)^W1 .*$').Value)"
    Report 'a quoted wildcard stays as typed' ($t -match '(?m)^W2 x\*\.txt x\?\.txt$') "'echo W2 `"x*.txt`" ''x?.txt''' printed: $([regex]::Match($t, '(?m)^W2 .*$').Value)"
    Report 'find gets its quoted pattern whole' ($t -match '(?m)^\./b/x3\.txt \./x1\.txt \./x2\.txt\s*$') "find . -name `"x*.txt`" found: $([regex]::Match($t, '(?m)^\./b/x3.*$').Value)"
    Report 'cat x*.txt reads both files' ($t -match '(?m)^2$') "wc -l printed '2' on a line: $($t -match '(?m)^2$')"

    # The same without a profile, in both PowerShells: the start script alone must install the marking.
    # The start script is the one uni-term wrote for the default settings; it is given by hand, as uni-term would.
    $start = (Join-Path $env:UNI_TERM_DATA_DIR 'shell\bin.ps1').Replace("'", "''")
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
    Report 'cmd.exe finds uni-term''s cat first' ($first -match [regex]::Escape("$env:UNI_TERM_DATA_DIR\bin\cat.exe") -and (Count $t 'uutils') -ge 1) "first match of 'where cat': $first"

    # uni-term without ut beside it: it says so and runs without its commands.
    $alone = Join-Path $work 'alone'; New-Item -ItemType Directory -Force $alone | Out-Null
    Get-ChildItem (Split-Path $Exe) -File | Where-Object { $_.Name -in 'uni-term.exe', 'conpty.dll', 'OpenConsole.exe' } | Copy-Item -Destination $alone
    $kept = $Exe; $Exe = Join-Path $alone 'uni-term.exe'
    try { $t = Run 'cmd-alone' $kind -keep } finally { $Exe = $kept }
    Report 'without ut beside it: says so, still a terminal' ((Count $t "uni-term's commands are not in use") -eq 1 -and (Count $t 'cat=Alias ls=Alias') -eq 1) "'commands are not in use' $(Count $t "uni-term's commands are not in use"); the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"

    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) uni-term or console-host processes still running"
}

if ($Section -in 'all', 'history') {
    "--- history ---"
    # A data directory of this section's own: the history starts empty, whatever ran before.
    $keptData = $env:UNI_TERM_DATA_DIR
    $env:UNI_TERM_DATA_DIR = Join-Path $work 'data-history'
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
    } finally { $env:UNI_TERM_DATA_DIR = $keptData }
    $real = Join-Path $env:LOCALAPPDATA 'uni-term\history.db'
    Report "the user's own history file is not touched" (-not (Test-Path $real)) "$real exists: $(Test-Path $real)"
}
if ($Section -in 'all', 'ai') {
    "--- ai ---"
    $keptData = $env:UNI_TERM_DATA_DIR
    $env:UNI_TERM_DATA_DIR = Join-Path $work 'data-ai'
    # A stand-in for the model's service on this machine (tools/fake-model.ps1); nothing leaves it.
    $port = 8765
    $requests = Join-Path $work 'model-requests.jsonl'
    function Start-Model([int]$delay) {
        Start-Process pwsh -ArgumentList "-NoProfile -File `"$PSScriptRoot\fake-model.ps1`" -Port $port -Log `"$requests`" -DelaySeconds $delay" -PassThru -WindowStyle Hidden
    }
    function Sent { @(Get-Content $requests -ErrorAction Ignore) }
    $env:UNI_TERM_TEST_KEY = 'test-key'
    # The history's own list is off throughout: the pictures are of the model's help alone.
    $quiet = "[history]`ncandidates = false"
    $model = "$quiet`n[ai]`nprovider = `"anthropic`"`nmodel = `"claude-haiku-5-5`"`nbase_url = `"http://localhost:$port`"`napi_key_env = `"UNI_TERM_TEST_KEY`""
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
        $agreed = Join-Path $env:UNI_TERM_DATA_DIR 'ai-consent'
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
        Report 'Ctrl-Shift-E explains the line in a box' ($explained -gt 1000 -and $asks -ge 1) "the box: $explained pixels; questions about parts: $asks"

        Run 'ai-secret' "echo --token abc123secretvalue{WAIT:2000}" $model | Out-Null
        $all = (Sent) -join "`n"
        Report 'a secret on the line is not sent' ($all -notmatch 'abc123secretvalue' -and $all -match 'redacted') "requests: $((Sent).Count); the secret in them: $($all -match 'abc123secretvalue'); '‹redacted›' in them: $($all -match 'redacted')"

        # A wrong key: no suggestion, and the log says why.
        $env:UNI_TERM_TEST_KEY = 'bad-key'
        Run 'ai-badkey' "echo bad{WAIT:2000}" $model | Out-Null
        $env:UNI_TERM_TEST_KEY = 'test-key'
        $logged = Get-Content (Join-Path $env:UNI_TERM_DATA_DIR 'uni-term.log') -Raw -ErrorAction Ignore
        Report 'a refused key is written to the log' ($logged -match 'suggestion: .*401.*invalid x-api-key') "the log has the 401: $($logged -match 'suggestion: .*401')"

        # A service that takes 20 s: the terminal goes on, nothing waits for it.
        Stop-Process -Id $server.Id -Force; $server = Start-Model 20; Start-Sleep 2
        $t = Run 'ai-slow' "zzqqyy-nothing{ENTER}{WAIT:500}echo still-alive{ENTER}{WAIT:1200}" $model
        Report 'a slow model holds nothing up' ((Count $t 'still-alive') -eq 2) "'still-alive' seen $(Count $t 'still-alive') times (the line, then its output)"

        $t = Run 'ai-nokey' "echo x{ENTER}" ($model -replace 'UNI_TERM_TEST_KEY', 'UNI_TERM_NO_SUCH_KEY_7Q') -keep
        Report 'a key that is not set is named, the terminal works' ((Count $t 'UNI_TERM_NO_SUCH_KEY_7Q') -ge 1 -and (Count $t 'x') -ge 1) "the notice names the variable: $((Count $t 'UNI_TERM_NO_SUCH_KEY_7Q') -ge 1)"
    } finally {
        Stop-Process -Id $server.Id -Force -ErrorAction Ignore
        $env:UNI_TERM_DATA_DIR = $keptData
        Remove-Item Env:UNI_TERM_TEST_KEY -ErrorAction Ignore
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
        Start-Process wt -ArgumentList @('-w', 'new', '--title', 'uni-term-verify', 'cmd', '/c', "`"$utExe`" view `"$png`" --width 8 --verbose 2> `"$said`" & ping -n 2 127.0.0.1 >nul")
        for ($i = 0; $i -lt 50 -and -not ((Test-Path $said) -and (Get-Item $said).Length -gt 0); $i++) { Start-Sleep -Milliseconds 200 }
        $choice = if (Test-Path $said) { (Get-Content $said -Raw).Trim() } else { '(nothing said)' }
        Report 'in Windows Terminal, ut view chooses sixel by itself' ($choice -match '^ut view: sixel') $choice
    } else {
        "SKIP  in Windows Terminal, ut view chooses sixel by itself: Windows Terminal is not installed here"
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
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
"$script:failed failed"
exit $script:failed
