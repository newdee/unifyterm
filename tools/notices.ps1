# Writes THIRD-PARTY-NOTICES: every license of what goes into unifyterm and ut.
#
#   pwsh tools/notices.ps1          write it
#   pwsh tools/notices.ps1 -Check   fail when the file in the repository is not what it would write
#
# Needs cargo-about 0.9 (cargo install cargo-about --locked --features cli --root target/tools).
# A dependency under a license not in about.toml's list makes it fail.
param([switch]$Check)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$tools = Join-Path $root 'target\tools\bin'
if (Test-Path $tools) { $env:PATH = "$tools$([IO.Path]::PathSeparator)$env:PATH" }
Push-Location $root
try {
    # To a file: cargo-about refuses PowerShell's redirection (it would change the encoding).
    $raw = Join-Path ([IO.Path]::GetTempPath()) "unifyterm-notices-$PID.txt"
    cargo about generate --locked --workspace --output-file $raw about.hbs
    if ($LASTEXITCODE -ne 0) { throw "cargo about failed (exit $LASTEXITCODE)" }
    $generated = [IO.File]::ReadAllText($raw).Replace("`r", ''); Remove-Item $raw
    $conpty = [IO.File]::ReadAllText((Join-Path $root 'vendor/conpty/LICENSE')).Replace("`r", '').TrimEnd()
    # cargo-about writes HTML entities for some characters; the file is plain text.
    $text = (($generated).Replace('CONPTY-LICENSE-HERE', $conpty)).Replace('&quot;', '"').Replace('&#x27;', "'").Replace('&lt;', '<').Replace('&gt;', '>').Replace('&#x3D;', '=').Replace('&#x60;', '`').Replace('&amp;', '&').TrimEnd() + "`n"
    $file = Join-Path $root 'THIRD-PARTY-NOTICES'
    if ($Check) {
        $kept = if (Test-Path $file) { [IO.File]::ReadAllText($file).Replace("`r", '') } else { '' }
        if ($kept -ne $text) { Write-Output 'THIRD-PARTY-NOTICES is out of date: run tools/notices.ps1'; exit 1 }
        Write-Output 'THIRD-PARTY-NOTICES is up to date'
    } else {
        [IO.File]::WriteAllText($file, $text, [Text.UTF8Encoding]::new($false))
        $crates = ([regex]::Matches($text, '(?m)^  \S+ \d+\.\d+')).Count
        $licenses = ([regex]::Matches($text, '(?m)^Used by:$')).Count
        Write-Output "THIRD-PARTY-NOTICES: $licenses license texts, $crates crate uses"
    }
} finally { Pop-Location }