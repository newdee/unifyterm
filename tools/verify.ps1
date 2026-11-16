# Checks the built uni-term by running it: types into its window, reads back
# what its programs printed (UNI_TERM_RAW_LOG, replayed to text) and compares
# pictures of the window. Each check prints PASS or FAIL with the numbers it
# measured; the exit code is how many failed.
#
#   pwsh tools/verify.ps1 [-Section all|input|settings|edges] [-Exe path]
#
# It needs a desktop session (a window opens for each check, a few seconds
# each) but not the foreground: input and pictures go through the window's
# own handle.
param(
    [ValidateSet('all', 'input', 'settings', 'edges')][string]$Section = 'all',
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

# Runs uni-term with a settings file's text, types, and saves a picture.
# Returns the text its programs' output draws (replayed at a size big
# enough that nothing scrolls away), or with -Screen at the window's size.
function Run([string]$name, [string]$type, [string]$toml = '', [string]$more = '', [double]$after = 1, [string]$screen = '') {
    $config = Join-Path $work "$name.toml"; $log = Join-Path $work "$name.raw"
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

    $t = Run 'tabs' "echo tab-one{ENTER}{WAIT:500}{KEY:CTRL+SHIFT+T}{WAIT:2500}echo tab-two{ENTER}{WAIT:600}{KEY:CTRL+TAB}{WAIT:500}echo back-in-one{ENTER}{WAIT:500}{KEY:CTRL+SHIFT+W}{WAIT:700}echo left-in-two{ENTER}"
    $all = @('tab-one', 'tab-two', 'back-in-one', 'left-in-two' | ForEach-Object { Count $t $_ })
    Report 'tabs: open, switch, close by keys' (($all | Where-Object { $_ -ne 2 }).Count -eq 0) "each of 4 markers seen: $($all -join ', ') times"

    $t = Run 'resize' "{SIZE:700,420}{WAIT:900}echo `"cols=`$(`$Host.UI.RawUI.WindowSize.Width) rows=`$(`$Host.UI.RawUI.WindowSize.Height)`"{ENTER}"
    $m = [regex]::Match($t, '(?m)^cols=(\d+) rows=(\d+)')
    Report 'a smaller window gives the shell a smaller screen' ($m.Success -and [int]$m.Groups[1].Value -lt 100 -and [int]$m.Groups[2].Value -lt 30) "the shell says $($m.Value) (the window starts at 115x30)"

    $saved = Get-Clipboard -Raw
    try {
        $t = Run 'select' "cls; echo select-me-please{ENTER}{WAIT:600}{DRAG:7,50,134,50}{WAIT:300}{RCLICK:300,300}{WAIT:300}"
        $copied = Get-Clipboard -Raw
        Report 'drag selects, right click copies' ($copied -eq 'select-me-please') "the clipboard holds '$copied'"
        Set-Clipboard 'echo pasted-by-click'
        $t = Run 'paste' "{RCLICK:300,300}{WAIT:400}{ENTER}"
        Report 'right click pastes' ((Count $t 'pasted-by-click') -eq 2) "'pasted-by-click' seen $(Count $t 'pasted-by-click') times"
    } finally { if ($null -ne $saved) { Set-Clipboard -Value $saved } }

    $forty = Join-Path $work 'forty.txt'; 1..40 | ForEach-Object { "line $_" } | Set-Content $forty
    $t = Run 'nvim' "{WAIT:500}:set mouse=a{ENTER}{WAIT:300}{CLICK:200,160}{WAIT:300}:echo 'at' line('.'){ENTER}{WAIT:400}{WHEEL:200,160,-1}{WAIT:400}:echo 'top' line('w0'){ENTER}{WAIT:400}" '' "-e nvim --clean `"$forty`"" 0.5 '115x30'
    $last = ($t -split "`n" | Where-Object { $_.Trim() } | Select-Object -Last 1)
    Report 'nvim: full screen, mouse, wheel' ($last -match '^top 4\b' -and $t -match '(?m)^line 4$') "last line: '$last' (one notch of wheel is nvim's 3 lines: the top line goes from 1 to 4)"

    $out = (& pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'exit.png') -Type "exit{ENTER}" -Wait 4 -After 3) -join ' '
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
    $t = Run 'shell' "{WAIT:500}" "[shell]`nprogram = `"cmd.exe`"`nargs = [`"/k`", `"echo from-cmd-shell`"]"
    Report 'shell.program and shell.args start that program' ((Count $t 'from-cmd-shell') -ge 1) "'from-cmd-shell' seen $(Count $t 'from-cmd-shell') times"
}

if ($Section -in 'all', 'edges') {
    "--- edges ---"
    $t = Run 'badkey' "echo still-works{ENTER}" "[font]`nsise = 12"
    Report 'a misspelt setting: opens, names it' ((Count $t 'sise') -ge 1 -and (Count $t 'still-works') -eq 2) "'sise' seen $(Count $t 'sise') times, 'still-works' $(Count $t 'still-works')"

    $garbage = Join-Path $work 'garbage.toml'; [IO.File]::WriteAllBytes($garbage, [byte[]](0..255))
    $log = Join-Path $work 'garbage.raw'; $env:UNI_TERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'garbage.png') -Type "echo alive{ENTER}" -Wait 4 -After 1 -App:" --config `"$garbage`"" | Out-Null } finally { $env:UNI_TERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a settings file of random bytes: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $dir = Join-Path $work 'a-directory'; New-Item -ItemType Directory -Force $dir | Out-Null
    $log = Join-Path $work 'dir.raw'; $env:UNI_TERM_RAW_LOG = $log
    try { & pwsh -NoProfile -File "$PSScriptRoot\shot.ps1" -Exe $Exe -Out (Join-Path $work 'dir.png') -Type "echo alive{ENTER}" -Wait 4 -After 1 -App:" --config `"$dir`"" | Out-Null } finally { $env:UNI_TERM_RAW_LOG = $null }
    $t = if (Test-Path $log) { Replay $log '250x2000' } else { '' }
    Report 'a directory as the settings file: opens, says so' ((Count $t 'settings not used') -ge 1 -and (Count $t 'alive') -eq 2) "'settings not used' $(Count $t 'settings not used'), 'alive' $(Count $t 'alive')"

    $t = Run 'badshell' "echo fell-back{ENTER}" "[shell]`nprogram = `"no-such-shell-xyz`""
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

Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
"$script:failed failed"
exit $script:failed
