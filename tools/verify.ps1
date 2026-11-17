# Checks the built uni-term by running it: types into its window, reads back
# what its programs printed (UNI_TERM_RAW_LOG, replayed to text) and compares
# pictures of the window. Each check prints PASS or FAIL with the numbers it
# measured; the exit code is how many failed.
#
#   pwsh tools/verify.ps1 [-Section all|input|settings|edges|commands] [-Exe path]
#
# It needs a desktop session (a window opens for each check, a few seconds
# each) but not the foreground: input and pictures go through the window's
# own handle.
param(
    [ValidateSet('all', 'input', 'settings', 'edges', 'commands')][string]$Section = 'all',
    [string]$Exe = "$PSScriptRoot\..\target\debug\uni-term.exe"
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$Exe = (Resolve-Path $Exe).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "uni-term-verify-$PID"
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
    # Another program than the default shell (cmd.exe, nvim) is no PowerShell: nothing to say to it.
    if ($toml -notmatch '(?m)^program\s*=' -and $more -notmatch '(^|\s)-e\s') { $type = $(if ($keep) { $NoHistoryKeep } else { $NoHistory }) + $type }
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
        for ($j = $y; $j -lt $y + $h; $j++) {
            for ($k = $x; $k -lt $x + $w; $k++) {
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
    Report 'they come from uni-term''s command directory' ($t -match 'from=[A-Z]:.*\\uni-term\\bin\\cat\.exe') "$([regex]::Match(($t -replace "`n", ''), 'from=[A-Z]:\\\S+').Value)"
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
    $start = (Join-Path $env:LOCALAPPDATA 'uni-term\shell\bin.ps1').Replace("'", "''")
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
    Report 'cmd.exe finds uni-term''s cat first' ($first -match '\\uni-term\\bin\\cat\.exe' -and (Count $t 'uutils') -ge 1) "first match of 'where cat': $first"

    # uni-term without ut beside it: it says so and runs without its commands.
    $alone = Join-Path $work 'alone'; New-Item -ItemType Directory -Force $alone | Out-Null
    Get-ChildItem (Split-Path $Exe) -File | Where-Object { $_.Name -in 'uni-term.exe', 'conpty.dll', 'OpenConsole.exe' } | Copy-Item -Destination $alone
    $kept = $Exe; $Exe = Join-Path $alone 'uni-term.exe'
    try { $t = Run 'cmd-alone' $kind -keep } finally { $Exe = $kept }
    Report 'without ut beside it: says so, still a terminal' ((Count $t "uni-term's commands are not in use") -eq 1 -and (Count $t 'cat=Alias ls=Alias') -eq 1) "'commands are not in use' $(Count $t "uni-term's commands are not in use"); the shell says: $([regex]::Match($t, 'cat=\w+ ls=\w+').Value)"

    Start-Sleep 1
    Report 'no process is left behind' ((Leftovers) -eq 0) "$(Leftovers) uni-term or console-host processes still running"
}

Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
"$script:failed failed"
exit $script:failed
