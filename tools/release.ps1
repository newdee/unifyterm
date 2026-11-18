# Fills the winget manifests and the Homebrew cask with a release's addresses
# and SHA-256s, from the packages CI built (downloaded into one folder). The
# templates in packaging/ stay as they are; the filled copies go to OUT.
#
#   pwsh tools/release.ps1 -Owner NAME -Packages DIR [-Out target/release-manifests]
#
# It changes no remote and publishes nothing: docs/release.md says what to do with the files.
param(
    [Parameter(Mandatory)][string]$Owner,
    [Parameter(Mandatory)][string]$Packages,
    [string]$Out = (Join-Path (Split-Path $PSScriptRoot) 'target/release-manifests')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$version = (cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $root 'Cargo.toml') | ConvertFrom-Json).packages |
    Where-Object name -eq 'uni-term' | ForEach-Object version
function Get-Sha([string]$pattern) {
    $file = Get-ChildItem $Packages -Recurse -File -Filter $pattern | Select-Object -First 1
    if (-not $file) { throw "no $pattern in $Packages" }
    (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLower()
}
New-Item -ItemType Directory -Force $Out | Out-Null
$msi = "uni-term-$version-x64.msi"
# The MSI's ProductCode, which winget wants, is in the package's Property table.
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @((Get-ChildItem $Packages -Recurse -Filter $msi | Select-Object -First 1).FullName, 0))
$view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @("SELECT Value FROM Property WHERE Property = 'ProductCode'"))
$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
$record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
$productCode = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
foreach ($file in Get-ChildItem (Join-Path $root 'packaging/windows/winget') -Filter *.yaml) {
    $text = [IO.File]::ReadAllText($file.FullName).Replace('0.1.0', $version).Replace('OWNER', $Owner)
    $text = $text.Replace('SHA256-OF-THE-MSI', (Get-Sha $msi).ToUpper()).Replace('{PRODUCT-CODE}', $productCode)
    [IO.File]::WriteAllText((Join-Path $Out $file.Name), $text, [Text.UTF8Encoding]::new($false))
}
$cask = [IO.File]::ReadAllText((Join-Path $root 'packaging/macos/uni-term.rb')).Replace('"0.1.0"', "`"$version`"").Replace('OWNER', $Owner)
$cask = $cask.Replace('SHA256-OF-THE-ARM64-DMG', (Get-Sha "uni-term-$version-macos-arm64.dmg")).Replace('SHA256-OF-THE-X86_64-DMG', (Get-Sha "uni-term-$version-macos-x86_64.dmg"))
[IO.File]::WriteAllText((Join-Path $Out 'uni-term.rb'), $cask, [Text.UTF8Encoding]::new($false))
"filled for $version into $Out"
Get-ChildItem $Out | ForEach-Object { "  $($_.Name)" }
