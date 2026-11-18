# site/ (the product page) against the repository: what the page says and
# what it loads must be true of the code beside it.
#
#   pwsh tools/site-check.ps1 [-Ut PATH]     prints one line per check; exit 1 on a failure
#
# -Ut is the built `ut` to ask for its command names; without it the newest of
# target/release and target/debug is used, and the check fails if neither is there.
param([string]$Ut)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$site = Join-Path $root 'site'
$html = [IO.File]::ReadAllText((Join-Path $site 'index.html'))
$css = [IO.File]::ReadAllText((Join-Path $site 'style.css'))
$js = [IO.File]::ReadAllText((Join-Path $site 'site.js'))
$readme = [IO.File]::ReadAllText((Join-Path $root 'README.md'))
$failed = 0
function Report([string]$what, [bool]$ok, [string]$detail) {
    if (-not $ok) { $script:failed++ }
    "{0}  {1,-34} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $what, $detail
}

# 1. Each marked element has its Chinese, and the dictionary holds nothing the page lacks.
$marked = @([regex]::Matches($html, 'data-i18n="([^"]+)"') | ForEach-Object { $_.Groups[1].Value })
$marked += [regex]::Matches($html, 'data-i18n-attr="([^"]+)"') | ForEach-Object { $_.Groups[1].Value -split ';' | ForEach-Object { ($_ -split ':')[1] } }
# The band's notes and its "Copied" are set by the script, not marked in the page.
$marked += 'mrz.note.scoop', 'mrz.note.curl', 'mrz.note.irm', 'mrz.copied'
$marked = @($marked | Sort-Object -Unique)
$dictionary = ($js -split 'var zh = \{')[1].Split("`n  };")[0]
$translated = @([regex]::Matches($dictionary, '(?m)^\s+"?([a-z0-9.]+)"?:') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
$missing = @($marked | Where-Object { $_ -notin $translated })
$unused = @($translated | Where-Object { $_ -notin $marked })
Report 'Chinese for every marked text' ($missing.Count -eq 0 -and $unused.Count -eq 0) ("page {0}, dictionary {1}{2}{3}" -f $marked.Count, $translated.Count,
    $(if ($missing) { "; no Chinese: $($missing -join ', ')" }), $(if ($unused) { "; not in the page: $($unused -join ', ')" }))

# 2. Every file the page names is in site/.
$named = @([regex]::Matches($html, '(?:src|poster|href)="([^"#:]+)"') | ForEach-Object { $_.Groups[1].Value })
$named += [regex]::Matches($css, 'url\("([^"]+)"\)') | ForEach-Object { $_.Groups[1].Value }
$named = @($named | Sort-Object -Unique)
$absent = @($named | Where-Object { -not (Test-Path (Join-Path $site $_)) })
Report 'files the page names exist' ($absent.Count -eq 0) ("{0} files{1}" -f $named.Count, $(if ($absent) { "; absent: $($absent -join ', ')" }))

# 3. Nothing is loaded from another host: only links (<a href>) leave the page.
$loaded = @([regex]::Matches($html, '<(?!a\s)[a-z]+\s[^>]*?(?:src|href|poster)="(https?:[^"]+)"') | ForEach-Object { $_.Groups[1].Value })
$loaded += [regex]::Matches($css + $js, 'url\(["'']?(https?:[^)"'']+)') | ForEach-Object { $_.Groups[1].Value }
Report 'nothing loaded from another host' ($loaded.Count -eq 0) ("{0} found{1}" -f $loaded.Count, $(if ($loaded) { ": $($loaded -join ', ')" }))

# 4. The command names in the page are the ones `ut --list` prints on Windows, and the count it states is theirs.
if (-not $Ut) {
    $Ut = @('release', 'debug') | ForEach-Object { Join-Path $root "target/$_/ut.exe" } | Where-Object { Test-Path $_ } |
        Sort-Object { (Get-Item $_).LastWriteTime } -Descending | Select-Object -First 1
}
$pageNames = @([System.Net.WebUtility]::HtmlDecode([regex]::Match($html, '<p class="names__list">([^<]*)</p>').Groups[1].Value) -split '\s+' | Where-Object { $_ })
if ($Ut -and (Test-Path $Ut)) {
    $utNames = @(& $Ut --list)
    $different = @(Compare-Object $pageNames $utNames -CaseSensitive)
    $stated = [regex]::Matches($html, '(\d+) (?:commands )?on Windows') | ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
    Report 'command names are ut --list' ($different.Count -eq 0 -and "$stated" -ceq "$($utNames.Count)") ("page {0}, ut {1}, stated {2}" -f $pageNames.Count, $utNames.Count, ($stated -join '/'))
} else {
    Report 'command names are ut --list' $false 'no built ut.exe: build first, or pass -Ut'
}

# 5. Each install command in the page is in the README, letter for letter.
$commands = @([regex]::Matches($html, 'data-command="([^"]+)"') | ForEach-Object { [System.Net.WebUtility]::HtmlDecode($_.Groups[1].Value) })
# Only the install page's blocks: the other pages show commands to run, not to install with.
$installPage = ($html -split '<section[^>]*id="install"')[1].Split('</section>')[0]
$commands += [regex]::Matches($installPage, '<pre><code>([^<]+)</code></pre>') | ForEach-Object { [System.Net.WebUtility]::HtmlDecode($_.Groups[1].Value) }
$commands = @($commands | Sort-Object -Unique)
$unknown = @($commands | Where-Object { -not $readme.Contains($_) })
Report 'install commands are the README''s' ($commands.Count -eq 4 -and $unknown.Count -eq 0) ("{0} commands{1}" -f $commands.Count, $(if ($unknown) { "; not in the README: $($unknown -join ' | ')" }))

# 6. Nothing of the machine it was made on: no home folder, no mail address.
$text = Get-ChildItem $site -Recurse -File -Include *.html, *.css, *.js, *.svg, *.md | ForEach-Object { [IO.File]::ReadAllText($_.FullName) }
$private = @([regex]::Matches(($text -join "`n"), '(?i)[a-z]:[\\/]users[\\/]\w+|/home/\w+|/Users/\w+|[\w.+-]+@[\w-]+\.[a-z]{2,}') | ForEach-Object { $_.Value } | Sort-Object -Unique)
Report 'no home folder, no mail address' ($private.Count -eq 0) ("{0} found{1}" -f $private.Count, $(if ($private) { ": $($private -join ', ')" }))

# 7. The version the page states is the workspace's.
$version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), '(?m)^version = "([^"]+)"').Groups[1].Value
$onPage = @([regex]::Matches($html + $js, '\b\d+\.\d+\.\d+\b') | ForEach-Object { $_.Value } | Where-Object { $_ -notmatch '^(127\.0\.0|0\.0\.0)' } | Sort-Object -Unique)
Report 'the version is the workspace''s' ($onPage.Count -eq 1 -and $onPage[0] -ceq $version) ("Cargo.toml {0}, page {1}" -f $version, ($onPage -join ', '))

# 8. The page stays light: what a first visit downloads before the video is asked for.
$first = Get-ChildItem $site -Recurse -File | Where-Object { $_.Extension -ne '.mp4' -and $_.Name -ne 'LICENSES.md' }
$bytes = ($first | Measure-Object Length -Sum).Sum
$limit = 1500KB
Report 'first visit under 1500 KB' ($bytes -le $limit) ("{0} files, {1:N0} KB" -f @($first).Count, ($bytes / 1KB))

"$failed failed"
exit $(if ($failed) { 1 } else { 0 })
