# Makes the promotional video and GIF from the real program: each scene is
# UnifyTerm itself, driven through its window by tools/shot.ps1 (a frame for
# every typed character), set on a 1280×720 card with a caption, then joined
# with ffmpeg. Nothing is mocked up, and nothing touches the user's own data:
# every scene has a throwaway data directory and settings file.
#
#   pwsh tools/promo.ps1 [-Exe target/release/unifyterm.exe] [-Out docs/media]
#
# Writes OUT/unifyterm-promo.mp4 and OUT/unifyterm-promo.gif. Needs ffmpeg and
# Microsoft Edge (for the cards).
param(
    [string]$Exe = (Join-Path (Split-Path $PSScriptRoot) 'target/release/unifyterm.exe'),
    [string]$Out = (Join-Path (Split-Path $PSScriptRoot) 'docs/media')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) "unifyterm-promo-$PID"
New-Item -ItemType Directory -Force $work, $Out | Out-Null
$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'promo: Microsoft Edge renders the cards, and it is not here' }
$fps = 12
# The scenes run in a folder of their own, mounted as a drive of its own: the prompt shows only
# that drive, no user's name or folders. A small git repository in it (author "demo").
$demo = Join-Path $work 'demo'; New-Item -ItemType Directory -Force $demo | Out-Null
Copy-Item (Join-Path $root 'Cargo.toml'), (Join-Path $root 'README.md') $demo
Copy-Item (Join-Path $root 'docs/brand/unifyterm-512.png') (Join-Path $demo 'unifyterm.png')
[IO.File]::WriteAllText((Join-Path $demo 'notes.md'), "# Release notes`n`n- **Same** commands everywhere`n- History you can *search*`n`n| system | shell |`n|---|---|`n| Windows | pwsh |`n| macOS | zsh |`n")
foreach ($message in 'Start the notes', 'Add the release table', 'Say which shells') {
    Add-Content (Join-Path $demo 'log.txt') $message
    git -C $demo -c init.defaultBranch=main init -q 2>$null
    git -C $demo add -A; git -C $demo -c user.name=demo -c user.email=demo@example.com commit -qm $message
}
$drive = @('U', 'V', 'W', 'X', 'Y', 'Z') | Where-Object { -not (Test-Path "${_}:\") } | Select-Object -First 1
if (-not $drive) { throw 'promo: no free drive letter (U to Z) for the demo folder' }
subst "${drive}:" $demo
# Larger letters than the default: the window is shown smaller in the video and the GIF.
$big = "[font]`nsize = 20`n"
# PowerShell writes every typed line to the user's history file unless told not to; the screen is cleared after.
$quiet = "Set-PSReadLineOption -HistorySaveStyle SaveNothing; cls # password: not for your history{ENTER}{WAIT:800}"
$mark = [IO.File]::ReadAllText((Join-Path $root 'docs/brand/unifyterm-mark.svg'))
$font = '"Segoe UI Variable Display","Segoe UI","SF Pro Display","Helvetica Neue",Arial,sans-serif'
$mono = '"Cascadia Code","JetBrains Mono",Consolas,monospace'

function Save-Card([string]$name, [string]$body) {
    $page = Join-Path $work "$name.html"
    $html = "<!doctype html><html><head><meta charset='utf-8'><style>html,body{margin:0;width:1280px;height:720px;overflow:hidden;background:radial-gradient(ellipse at 30% 0%,#252a40 0%,#11131c 70%);color:#c0caf5;font-family:$font}b{color:#7aa2f7}code{font-family:$mono;color:#e0af68}</style></head><body>$body</body></html>"
    [IO.File]::WriteAllText($page, $html, [Text.UTF8Encoding]::new($false))
    $png = Join-Path $work "$name.png"
    & $edge --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --window-size=1280,720 --screenshot="$png" "file:///$($page -replace '\\', '/')" 2>&1 | Out-Null
    for ($i = 0; $i -lt 30 -and -not (Test-Path $png); $i++) { Start-Sleep -Milliseconds 200 }
    $png
}
# A scene's background: the caption across the top, room for the window below.
function Save-Backdrop([string]$name, [string]$caption) {
    Save-Card $name "<div style='height:52px;display:flex;align-items:center;justify-content:center;gap:12px;font-size:26px;font-weight:600'><div style='width:30px;height:30px'>$mark</div>$caption</div>"
}
# Types text one character at a time, a picture after each.
$script:frame = 0
function Typed([string]$scene, [string]$text) {
    ($text.ToCharArray() | ForEach-Object { $script:frame++; "$_{WAIT:40}{SHOT:$scene-$('{0:d4}' -f $script:frame)}" }) -join ''
}
function Shots([string]$scene, [int]$count, [int]$every = 80) {
    (1..$count | ForEach-Object { $script:frame++; "{WAIT:$every}{SHOT:$scene-$('{0:d4}' -f $script:frame)}" }) -join ''
}
# Runs UnifyTerm with these settings and keys; its pictures land in $work.
function Record([string]$scene, [string]$toml, [string]$keys, [hashtable]$data = @{}) {
    $dir = Join-Path $work "data-$scene"; New-Item -ItemType Directory -Force $dir | Out-Null
    foreach ($file in $data.Keys) { [IO.File]::WriteAllText((Join-Path $dir $file), $data[$file]) }
    $config = Join-Path $work "$scene.toml"; [IO.File]::WriteAllText($config, $big + $toml)
    $env:UNIFYTERM_DATA_DIR = $dir
    Push-Location "${drive}:\"
    try { & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'shot.ps1') -Exe $Exe -Out (Join-Path $work "$scene-end.png") -Type ($quiet + $keys) -Wait 5 -After 0.5 -App:" --config `"$config`"" | Out-Null }
    finally { Pop-Location; Remove-Item Env:UNIFYTERM_DATA_DIR -ErrorAction SilentlyContinue }
}
# One scene as video: its frames on its backdrop, the last held for $hold seconds.
function Join-Scene([string]$scene, [string]$backdrop, [double]$hold) {
    $frames = @(Get-ChildItem $work -Filter "$scene-0*.png" | Sort-Object Name)
    if (-not $frames) { throw "promo: no pictures for $scene" }
    $list = Join-Path $work "$scene.txt"
    $lines = foreach ($f in $frames) { "file '$($f.FullName -replace '\\', '/')'"; "duration $([math]::Round(1 / $fps, 4))" }
    $lines += "file '$($frames[-1].FullName -replace '\\', '/')'"; $lines += "duration $hold"; $lines += "file '$($frames[-1].FullName -replace '\\', '/')'"
    [IO.File]::WriteAllLines($list, $lines)
    $clip = Join-Path $work "$scene.mp4"
    ffmpeg -v error -y -i $backdrop -f concat -safe 0 -i $list -filter_complex "[1:v]crop=960:620:8:31,fps=$fps[w];[0:v][w]overlay=160:72,format=yuv420p,fade=t=in:st=0:d=0.3[v]" -map '[v]' -r $fps $clip
    $clip
}
function Join-Card([string]$card, [double]$seconds, [string]$name) {
    $clip = Join-Path $work "$name.mp4"
    ffmpeg -v error -y -loop 1 -t $seconds -i $card -vf "format=yuv420p,fade=t=in:st=0:d=0.4,fade=t=out:st=$($seconds - 0.4):d=0.4" -r $fps $clip
    $clip
}

$clips = @()
# 1. The name and what it is.
$title = Save-Card 'title' "<div style='height:720px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:26px'><div style='width:200px;height:200px'>$mark</div><div style='font-size:76px;font-weight:700;letter-spacing:-2px;color:#e6e9f5'>Unify<b>Term</b></div><div style='font-size:30px;opacity:.9'>One terminal for Windows, macOS and Linux</div><div style='font-size:21px;opacity:.6'>Same Unix commands &middot; History as you type &middot; Pictures, video, markdown &middot; Built for agents</div></div>"
$clips += Join-Card $title 3.2 'c-title'

# 2. The same commands on Windows.
$script:frame = 0
$keys = (Typed 's2' "cat Cargo.toml | grep -n version") + "{ENTER}" + (Shots 's2' 10) + (Typed 's2' "echo '[1,2,3]' | jq add") + "{ENTER}" + (Shots 's2' 10) + (Typed 's2' "ls | wc -l") + "{ENTER}" + (Shots 's2' 8)
Record 's2' "" $keys
$clips += Join-Scene 's2' (Save-Backdrop 'b2' 'The same Unix commands, on <b>Windows</b> too') 1.6

# 3. The history, as you type.
$script:frame = 0
$seed = "git --version{ENTER}{WAIT:900}git log --oneline -3{ENTER}{WAIT:900}git show --stat HEAD{ENTER}{WAIT:900}cls{ENTER}{WAIT:700}"
$keys = $seed + (Typed 's3' "git") + (Shots 's3' 6) + "{KEY:CTRL+N}" + (Shots 's3' 4) + "{KEY:CTRL+N}" + (Shots 's3' 6) + "{ENTER}" + (Shots 's3' 4) + "{ENTER}" + (Shots 's3' 10)
Record 's3' "" $keys
$clips += Join-Scene 's3' (Save-Backdrop 'b3' 'Your <b>history</b>, as you type: Ctrl-N, Enter') 1.6

# 4. Pictures and markdown.
$script:frame = 0
$keys = (Typed 's4' "ut view unifyterm.png --width 12") + "{ENTER}" + (Shots 's4' 12) + (Typed 's4' "ut md notes.md") + "{ENTER}" + (Shots 's4' 10)
Record 's4' "" $keys
$clips += Join-Scene 's4' (Save-Backdrop 'b4' '<b>Pictures</b>, video and <b>markdown</b>, in the terminal') 1.8

# 5. What a command means (a model on this machine: tools/fake-model.ps1).
$script:frame = 0
$port = 8766
$meanings = Join-Path $work 'meanings.json'
[IO.File]::WriteAllText($meanings, (@{
    'tar' = 'the tape archiver: packs files into one archive'
    '-czf' = 'c: create an archive, z: compress it with gzip, f: into the file named next'
    'site.tgz' = 'the archive it writes'
    'public' = 'the folder that goes into it'
} | ConvertTo-Json))
$model = Start-Process pwsh -ArgumentList "-NoProfile -File `"$PSScriptRoot\fake-model.ps1`" -Port $port -Log `"$(Join-Path $work 'model.jsonl')`" -DelaySeconds 0 -Meanings `"$meanings`"" -PassThru -WindowStyle Hidden
Start-Sleep 2
try {
    $env:UNIFYTERM_PROMO_KEY = 'promo'
    $ai = "[history]`ncandidates = false`n[ai]`nprovider = `"anthropic`"`nmodel = `"claude-haiku-5-5`"`nbase_url = `"http://localhost:$port`"`napi_key_env = `"UNIFYTERM_PROMO_KEY`"`nsuggest = false"
    $keys = (Typed 's5' "tar -czf site.tgz public") + (Shots 's5' 4) + "{KEY:CTRL+SHIFT+E}" + (Shots 's5' 18 100)
    Record 's5' $ai $keys @{ 'ai-consent' = "Anthropic claude-haiku-5-5 http://localhost:$port" }
} finally { Stop-Process -Id $model.Id -Force -ErrorAction SilentlyContinue; Remove-Item Env:UNIFYTERM_PROMO_KEY -ErrorAction SilentlyContinue }
$clips += Join-Scene 's5' (Save-Backdrop 'b5' 'Ask what a command means: <b>Ctrl-Shift-E</b>') 1.6

# 6. The agents beside the terminal (made-up agents, from the tests' fixture).
$script:frame = 0
$fixture = (Join-Path $root 'crates/unifyterm/tests/fixtures/agents.json') -replace '\\', '/'
$ut = (Join-Path (Split-Path $Exe) 'ut.exe') -replace '\\', '/'
$agents = "[agents]`nsidebar = true`ncommand = ['$ut', 'cat', '$fixture']"
$keys = (Shots 's6' 4 200) + (Typed 's6' "cat notes.md | head -4") + "{ENTER}" + (Shots 's6' 8)
Record 's6' $agents $keys
$clips += Join-Scene 's6' (Save-Backdrop 'b6' 'Your <b>agents</b>, beside the terminal: Ctrl-Shift-A') 1.6

# 7. How to get it.
$end = Save-Card 'end' "<div style='height:720px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:22px'><div style='display:flex;align-items:center;gap:18px'><div style='width:84px;height:84px'>$mark</div><div style='font-size:56px;font-weight:700;letter-spacing:-1.5px;color:#e6e9f5'>Unify<b>Term</b></div></div><div style='font-size:24px;line-height:2;text-align:left'><code>brew install newdee/tap/unifyterm</code><br><code>scoop install https://…/packaging/scoop/unifyterm.json</code><br><code>curl -fsSL https://…/install.sh | sh</code></div><div style='font-size:28px;margin-top:8px'>github.com/<b>newdee/unifyterm</b> &middot; MIT</div></div>"
$clips += Join-Card $end 3.4 'c-end'

# All of it, then the GIF.
$list = Join-Path $work 'all.txt'
[IO.File]::WriteAllLines($list, @($clips | ForEach-Object { "file '$($_ -replace '\\', '/')'" }))
$mp4 = Join-Path $Out 'unifyterm-promo.mp4'
ffmpeg -v error -y -f concat -safe 0 -i $list -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -movflags +faststart $mp4
$gif = Join-Path $Out 'unifyterm-promo.gif'
ffmpeg -v error -y -i $mp4 -vf "fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" $gif
$length = [double](ffprobe -v error -show_entries format=duration -of csv=p=0 $mp4)
"promo: {0} ({1:N1} s, {2:N1} MB), {3} ({4:N1} MB)" -f $mp4, $length, ((Get-Item $mp4).Length / 1MB), $gif, ((Get-Item $gif).Length / 1MB)
subst "${drive}:" /d
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
