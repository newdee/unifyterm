# Writes the scoop and winget manifests for a release, from the files just
# built (no download): the zip's and the MSI's SHA-256, the MSI's ProductCode.
# The release workflow runs it, then commits packaging/scoop and
# packaging/windows/winget back to main, so that `scoop install <raw url>`
# always installs the latest release.
#
#   pwsh tools/package-manifests.ps1 -Version 0.1.0 -ZipPath dist\unifyterm-v0.1.0-windows-x86_64.zip -MsiPath dist\unifyterm-0.1.0-x64.msi
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$ZipPath,
    [Parameter(Mandatory)][string]$MsiPath
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$repo = 'https://github.com/newdee/unifyterm'
$tag = "v$Version"
$zipName = "unifyterm-$tag-windows-x86_64"
$sha = { param($path) (Get-FileHash $path -Algorithm SHA256).Hash.ToLower() }

# scoop: the zip, unpacked; unifyterm.exe and ut.exe on PATH, UnifyTerm in the Start menu.
$scoopPath = Join-Path $root 'packaging/scoop/unifyterm.json'
$scoop = Get-Content -Raw $scoopPath | ConvertFrom-Json
$scoop.version = $Version
$scoop.architecture.'64bit'.url = "$repo/releases/download/$tag/$zipName.zip"
$scoop.architecture.'64bit'.hash = & $sha $ZipPath
$scoop.architecture.'64bit'.extract_dir = $zipName
[IO.File]::WriteAllText($scoopPath, ($scoop | ConvertTo-Json -Depth 10) + "`n", [Text.UTF8Encoding]::new($false))

# winget: the MSI, with the ProductCode it was built with.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @((Resolve-Path $MsiPath).Path, 0))
$view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @("SELECT Value FROM Property WHERE Property = 'ProductCode'"))
$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
$record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
$productCode = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
$msiName = Split-Path -Leaf $MsiPath
foreach ($file in Get-ChildItem (Join-Path $root 'packaging/windows/winget') -Filter *.yaml) {
    $text = [IO.File]::ReadAllText($file.FullName)
    $text = [regex]::Replace($text, '(?m)^PackageVersion: .*$', "PackageVersion: $Version")
    $text = [regex]::Replace($text, '(?m)^ProductCode: .*$', "ProductCode: '$productCode'")
    $text = [regex]::Replace($text, '(?m)^(\s+InstallerUrl: ).*$', "`${1}$repo/releases/download/$tag/$msiName")
    $text = [regex]::Replace($text, '(?m)^(\s+InstallerSha256: ).*$', "`${1}$((& $sha $MsiPath).ToUpper())")
    [IO.File]::WriteAllText($file.FullName, $text, [Text.UTF8Encoding]::new($false))
}
"manifests for ${Version}: scoop $(& $sha $ZipPath); winget $msiName $productCode"
