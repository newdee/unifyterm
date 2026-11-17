# One acceptance round: every check there is, with its numbers. The exit
# code is how many checks failed.
#
#   pwsh tools/round.ps1 [-Profile debug|release] [-SkipLinux]
#
# What it runs:
#   1. cargo fmt --check, cargo clippy with warnings denied, cargo test
#   2. every one of ut's commands started once (--help), directly and by its link
#   3. tools/verify.ps1: the built uni-term driven through its own window
#   4. tools/linux-test.sh in WSL: ut's tests on Linux against the same record,
#      and its grep against GNU grep
param(
    [ValidateSet('debug', 'release')][string]$Profile = 'debug',
    [switch]$SkipLinux
)
$root = Split-Path $PSScriptRoot
Set-Location $root
$script:failed = 0
function Report([string]$name, [bool]$ok, [string]$detail) {
    if (-not $ok) { $script:failed++ }
    "{0}  {1,-34} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail
}
# rustc has crashed on this machine with 32 jobs at once (STATUS_ACCESS_VIOLATION, twice); eight has not.
$jobs = 8
# @( ) around the whole: one value alone would otherwise be a string, and splatting a string passes its characters.
$flag = @(if ($Profile -eq 'release') { '--release' })

# Nothing in a round may write to the user's own PowerShell history: the tests, the live
# checks and the Linux run are all inside this measurement.
$history = (Get-PSReadLineOption).HistorySavePath
$historyBefore = if (Test-Path $history) { (Get-Item $history).Length } else { 0 }

"=== 1. format, lints, unit tests ($Profile) ==="
cargo fmt --all --check *> $null
Report 'cargo fmt --check' ($LASTEXITCODE -eq 0) "exit $LASTEXITCODE"
$clippy = cargo clippy --workspace --all-targets --all-features -j $jobs -- -D warnings 2>&1
$warnings = @($clippy | Select-String '^(warning|error)' | Where-Object { $_ -notmatch 'future version|could not compile|build failed' }).Count
Report 'cargo clippy -D warnings' ($LASTEXITCODE -eq 0) "exit $LASTEXITCODE, $warnings warnings or errors"
$build = cargo build --workspace -j $jobs @flag 2>&1
Report "cargo build ($Profile)" ($LASTEXITCODE -eq 0) "exit $LASTEXITCODE"
if ($LASTEXITCODE -ne 0) {
    $build | Select-String '^error|STATUS_|could not compile' | Select-Object -First 5 | ForEach-Object { "      $($_.Line)" }
    "nothing was built: the checks that need the programs are not run"
    exit 1
}
$tests = cargo test --workspace -j $jobs 2>&1 | Select-String 'test result'
$passed = ($tests | ForEach-Object { [int]([regex]::Match($_, '(\d+) passed').Groups[1].Value) } | Measure-Object -Sum).Sum
$notPassed = ($tests | ForEach-Object { [int]([regex]::Match($_, '(\d+) failed').Groups[1].Value) } | Measure-Object -Sum).Sum
Report 'cargo test' ($LASTEXITCODE -eq 0 -and $notPassed -eq 0 -and $passed -gt 0) "$passed passed, $notPassed failed, in $($tests.Count) test programs"

"=== 1b. history: a search among 100 000 commands (release) ==="
$speed = cargo test --release -p uni-history --test speed -j $jobs -- --ignored --nocapture 2>&1
$line = ($speed | Select-String 'commands;' | Select-Object -First 1)
Report 'history search p95 < 5 ms' ($LASTEXITCODE -eq 0 -and $line) "$line"

"=== 2. every command starts ==="
$ut = Join-Path $root "target\$Profile\ut.exe"
$names = @(& $ut --list)
# Three end otherwise by design: false always fails, test takes --help as a word, [ needs its ].
$special = 'false', 'test', '['
$broken = @(foreach ($name in $names) {
    $text = (& $ut $name --help 2>&1 | Out-String)
    $ok = ($LASTEXITCODE -eq 0 -and $text.Trim().Length -ge 20) -or ($name -in $special)
    if (-not $ok -or $text -match 'panicked|could not load its messages') { $name }
})
Report 'ut COMMAND --help' ($names.Count -gt 0 -and $broken.Count -eq 0) "$($names.Count - $broken.Count) of $($names.Count) start and print help$(if ($broken) { '; not: ' + ($broken -join ', ') })"

"=== 3. uni-term through its own window ==="
$exe = Join-Path $root "target\$Profile\uni-term.exe"
# Everything it says is kept, its errors too: a run that stops half way must not pass as
# fewer checks. It passed only if it got to its last line ("0 failed") through every section.
$live = @(pwsh -NoProfile -File "$PSScriptRoot\verify.ps1" -Section all -Exe $exe 2>&1 | ForEach-Object { "$_" })
$kept = Join-Path ([IO.Path]::GetTempPath()) "uni-term-round-verify-$Profile.txt"
$live | Set-Content -Path $kept -Encoding utf8
$livePass = @($live | Where-Object { $_ -match '^PASS' }).Count
$liveFail = @($live | Where-Object { $_ -match '^FAIL' })
$sections = 'input', 'settings', 'edges', 'commands', 'history', 'ai'
$missing = @($sections | Where-Object { $live -notcontains "--- $_ ---" })
$finished = ($live | Where-Object { $_.Trim() } | Select-Object -Last 1) -eq '0 failed'
Report 'tools/verify.ps1' ($liveFail.Count -eq 0 -and $livePass -gt 0 -and $finished -and $missing.Count -eq 0) "$livePass passed, $($liveFail.Count) failed; ran to its end: $finished; sections missing: $($missing.Count) $($missing -join ' ') (all of it: $kept)"
$liveFail | ForEach-Object { "      $_" }
if (-not $finished) { $live | Select-Object -Last 8 | ForEach-Object { "      | $_" } }
# The numbers that say a mechanism is alive, for comparing rounds.
$live | Where-Object { $_ -match 'pixels differ|cols=|lines are numbered|same picture' } | ForEach-Object { "      " + ($_ -replace '^PASS\s+', '') }

if (-not $SkipLinux) {
    "=== 4. ut on Linux (WSL) ==="
    $env:WSL_UTF8 = 1
    $linux = wsl -- bash -l "/mnt/$($root.Substring(0,1).ToLower())$($root.Substring(2) -replace '\\', '/')/tools/linux-test.sh" 2>&1 | Where-Object { $_ -notmatch 'localhost' }
    $linuxExit = $LASTEXITCODE
    $results = @($linux | Where-Object { $_ -match '^test result' })
    $linuxFailed = ($results | ForEach-Object { [int]([regex]::Match($_, '(\d+) failed').Groups[1].Value) } | Measure-Object -Sum).Sum
    $linuxPassed = ($results | ForEach-Object { [int]([regex]::Match($_, '(\d+) passed').Groups[1].Value) } | Measure-Object -Sum).Sum
    Report 'cargo test -p ut on Linux' ($results.Count -gt 0 -and $linuxFailed -eq 0) "$linuxPassed passed, $linuxFailed failed; $(($linux | Where-Object { $_ -match 'cases;' } | Select-Object -First 1))"
    $hooks = ($linux | Where-Object { $_ -match '^shell hooks:' } | Select-Object -Last 1)
    Report 'bash, zsh, fish hooks print their marks' ($hooks -match ', 0 missing' -and $hooks -notmatch ': 0 complete') "$hooks"
    $linux | Where-Object { $_ -match '^MISSING|not installed' } | ForEach-Object { "      $_" }
    $gnu = ($linux | Where-Object { $_ -match 'against GNU grep' } | Select-Object -Last 1)
    Report 'grep against GNU grep' ($linuxExit -eq 0 -and $gnu -match ', 0 differ') "$gnu"
    $linux | Where-Object { $_ -match '^DIFFERS' } | ForEach-Object { "      $_" }
}

$historyAfter = if (Test-Path $history) { (Get-Item $history).Length } else { 0 }
Report 'the user''s history is not written' ($historyAfter -eq $historyBefore) "the history file: $historyBefore bytes before the round, $historyAfter after"

"$script:failed failed"
exit $script:failed
