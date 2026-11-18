# Installs UnifyTerm on Windows (x86_64) from its latest GitHub release, for
# you alone (no administrator rights):
#
#   irm https://raw.githubusercontent.com/newdee/unifyterm/main/install.ps1 | iex
#
# What it does: downloads unifyterm-<tag>-windows-x86_64.zip and its .sha256,
# checks the checksum, unpacks it into %LOCALAPPDATA%\Programs\unifyterm,
# puts that folder on your PATH and UnifyTerm in the Start menu. Run it again
# to update (close UnifyTerm first).
#
# Settings, by environment variable:
#   UNIFYTERM_VERSION      a version (0.1.0) instead of the latest
#   UNIFYTERM_DIR          where it goes (default %LOCALAPPDATA%\Programs\unifyterm)
#   UNIFYTERM_BASE         where the releases are (default the GitHub releases)
#   UNIFYTERM_NO_PATH      set: leave PATH alone
#   UNIFYTERM_NO_SHORTCUT  set: no Start menu item
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'newdee/unifyterm'
$dir = if ($env:UNIFYTERM_DIR) { $env:UNIFYTERM_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\unifyterm' }
$base = if ($env:UNIFYTERM_BASE) { $env:UNIFYTERM_BASE } else { "https://github.com/$repo/releases/download" }
function Say([string]$text) { Write-Host "unifyterm install: $text" }

if (-not [Environment]::Is64BitOperatingSystem) { throw 'unifyterm install: there is no 32-bit build' }
$tag = if ($env:UNIFYTERM_VERSION) { 'v' + $env:UNIFYTERM_VERSION.TrimStart('v') } else {
    (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'unifyterm-install' }).tag_name
}
$name = "unifyterm-$tag-windows-x86_64"
$tmp = Join-Path ([IO.Path]::GetTempPath()) "unifyterm-install-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    Say "downloading $name"
    Invoke-WebRequest "$base/$tag/$name.zip" -OutFile "$tmp\$name.zip" -UseBasicParsing
    Invoke-WebRequest "$base/$tag/$name.zip.sha256" -OutFile "$tmp\$name.zip.sha256" -UseBasicParsing
    # The .sha256 file is "<hash>  <file name>".
    $want = ((Get-Content -Raw "$tmp\$name.zip.sha256").Trim() -split '\s+')[0].ToLower()
    $got = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if ($want -ne $got) { throw "unifyterm install: the checksum does not match: expected $want, got $got" }
    Expand-Archive "$tmp\$name.zip" $tmp -Force
    if (Get-Process unifyterm -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($dir) }) {
        throw "unifyterm install: UnifyTerm is running from ${dir}: close it first"
    }
    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item "$tmp\$name\*" $dir -Recurse -Force
} finally { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }

if (-not $env:UNIFYTERM_NO_PATH) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($userPath -split ';' | Where-Object { $_ })
    if ($parts -notcontains $dir) {
        [Environment]::SetEnvironmentVariable('Path', (($parts + $dir) -join ';'), 'User')
        Say "added $dir to your PATH (new terminals see it)"
    }
}
if (-not $env:UNIFYTERM_NO_SHORTCUT) {
    $link = Join-Path ([Environment]::GetFolderPath('Programs')) 'UnifyTerm.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($link)
    $shortcut.TargetPath = Join-Path $dir 'unifyterm.exe'
    $shortcut.WorkingDirectory = $env:USERPROFILE
    $shortcut.Save()
    Say 'UnifyTerm is in the Start menu'
}
Say "installed ${tag}: $dir\unifyterm.exe and $dir\ut.exe"
