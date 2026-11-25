# Makes the promotional video and GIF from the real program: each scene is
# UnifyTerm itself, driven through its window by tools/shot.ps1 (a frame for
# every typed character), set on a 1280×720 card with a caption, then joined
# with ffmpeg. Nothing is mocked up, and nothing touches the user's own data:
# every scene has a throwaway data directory and settings file.
#
#   pwsh tools/promo.ps1 [-Cut first|second] [-Exe target/release/unifyterm.exe] [-Out docs/media] [-Stills site/img] [-Narrate] [-Only s2,s3]
#
# Two videos: `first` (0.2.4: the commands, the history, pictures, a model's explanation, the agents,
# the settings) and `second` (0.3.0 to 0.6.2: themes, a command for a sentence, the model's sources,
# command lists, the row of tabs, keys of your own).
# Writes OUT/unifyterm-promo.mp4 and OUT/unifyterm-promo.gif (no sound); the second video's files are
# named unifyterm-promo-2. With -Narrate also OUT/unifyterm-promo-zh.mp4 and -en.mp4 (-2-zh, -2-en). With -Stills (first video), also one
# picture of the window for each scene (the site's pictures, cut to the rows that have
# something in them): hero.png, commands.png, history.png, media.png, explain.png, agents.png, settings.png. Needs ffmpeg and Microsoft Edge (for the cards).
# -Only records the scenes named and joins them alone, without the cards: for trying a scene out.
param(
    [ValidateSet('first', 'second')][string]$Cut = 'first',
    [string]$Exe = (Join-Path (Split-Path $PSScriptRoot) 'target/release/unifyterm.exe'),
    [string]$Out = (Join-Path (Split-Path $PSScriptRoot) 'docs/media'),
    [string]$Stills = '',
    # Speak each part's line in each language (see $narration), and write a video for each.
    [switch]$Narrate,
    [string[]]$Only = @()
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
# The words on the cards, one line a scene (written by an agent from the README, checked against it).
$caption = if ($Cut -eq 'first') { @{
    subtitle = 'One terminal for Windows, macOS and Linux'
    points   = '99+ Unix commands &middot; Searchable history &middot; Media previews &middot; Agent sidebar'
    s2       = 'Run 99 bundled Unix commands in Windows PowerShell'
    s3       = 'Select with <b>Ctrl-N</b> and insert with <b>Enter</b>'
    s4       = 'View images, videos and Markdown in the terminal'
    s5       = 'Press <b>Ctrl-Shift-E</b> to explain each command part'
    s6       = 'Check agent models, costs and context usage'
    s7       = 'Change the terminal font immediately with <b>ut setting</b>'
    s8       = 'Enable commands in new shells with <b>ut shell enable</b>'
    s9       = 'Enter full screen with <b>F11</b> (<b>Cmd-Enter</b> on macOS)'
    s10      = 'Tab bar shows new versions and how to upgrade'
    install  = 'Install it from the UnifyTerm website'
} } else { @{
    # The second video's words: written by ChatGPT from the README and the scenes, checked against the README.
    subtitle = 'A cross-platform GUI terminal for Windows, macOS and Linux'
    points   = 'What is new from 0.3.0 to 0.6.2'
    themes   = 'Nine built-in themes, switched from the command line'
    look     = 'Custom themes with wallpaper and a redesigned interface'
    ask      = 'Turn natural language into commands you can review'
    sources  = 'Connect hosted models, local servers, or agent CLIs'
    lists    = 'OpenCLI command lists provide help without a model'
    fix      = 'Correct mistyped commands using local command lists'
    explain  = 'Press <b>Ctrl-Shift-E</b>: explained without calling a model'
    tabbar   = 'Hide the tab bar with <b>Ctrl-Shift-B</b>, show it at the top edge'
    shortcuts = 'Customize shortcuts for ten window actions'
    install  = 'Install UnifyTerm: website and GitHub'
} }
# What is said over each part, one line a part, in each language (written by an agent from the README).
# With -Narrate each language's lines are spoken (edge-tts: the words go to Microsoft's speech service)
# and each part lasts until its longest line is said; OUT/unifyterm-promo-<language>.mp4 has that voice.
$narration = if ($Cut -eq 'second') { [ordered]@{
    zh = @{
        voice   = 'zh-CN-XiaoxiaoNeural'
        title   = 'UnifyTerm 是一款支持 Windows、macOS 和 Linux 的图形终端。'
        themes  = '内置九套主题，可以查看色样并通过命令即时切换。'
        look    = '主题文件可设置壁纸、侧栏图片、透明度、毛玻璃和圆角。'
        ask     = '输入自然语言，模型生成命令并放到输入行，检查后再执行。'
        sources = '模型可以来自在线服务、本机模型服务器或已有的 Agent 命令行。'
        lists   = '登记命令清单后，输入 git st 即可查看子命令和说明，无需模型。'
        fix     = '命令拼错时，程序根据命令清单提供修正建议。'
        explain = '按下快捷键，程序根据命令清单逐词解释，无需调用模型。'
        tabbar  = '按快捷键隐藏标签栏，鼠标移到窗口顶边即可重新显示。'
        shortcuts = '可以在设置中修改十个窗口操作的快捷键。'
        end     = 'UnifyTerm 0.6.2，安装方式和项目源码请访问官网及 GitHub。'
    }
    en = @{
        voice   = 'en-US-AriaNeural'
        title   = 'UnifyTerm is a graphical terminal for Windows, macOS, and Linux.'
        themes  = 'Choose from nine built-in themes and switch between them from the command line.'
        look    = 'Theme files support wallpaper, sidebar pictures, opacity, blur, and rounded corners.'
        ask     = 'Describe what you need, and the model puts a command on the input line for review.'
        sources = 'Connect a hosted model, a local model server, or an existing agent CLI.'
        lists   = 'Register an OpenCLI list to see Git subcommands and descriptions without a model.'
        fix     = 'Mistyped commands can be corrected using the registered command list.'
        explain = 'Explain each word of a listed command locally, without calling a model.'
        tabbar  = 'Hide the tab bar with a shortcut, then move the pointer to the top edge to show it again.'
        shortcuts = 'Customize the keyboard shortcuts for ten window actions in settings.'
        end     = 'Get UnifyTerm 0.6.2. Visit the website for installation and GitHub for the source code.'
    }
} } else { [ordered]@{
    zh = @{
        voice = 'zh-CN-XiaoxiaoNeural'
        title = '这个终端在 Windows、macOS 和 Linux 上都能用。'
        s2    = '这里是在 Windows 上，直接用 grep 查文本。'
        s3    = '打几个字，选中以前用过的命令，回车就填好了。'
        s4    = '图片可以直接看，文档也能按原来的排版读。'
        s5    = '配好模型并确认后，就能逐段查看命令的含义。'
        s6    = '打开侧栏，就能看每个助手的模型、费用和上下文用量。'
        s7    = '用方向键选个本机字体，窗口马上就换好了。'
        s8    = '启用后，新开的窗格和助手的终端也能用这些命令。'
        s9    = '切到全屏后，终端会盖住任务栏。'
        s10   = '有新版时，看标签栏就知道怎么升级。'
        end   = '安装方法在官网，网址见视频结尾和简介。'
    }
    en = @{
        voice = 'en-US-AriaNeural'
        title = 'UnifyTerm runs on Windows, macOS and Linux.'
        s2    = 'Here on Windows, you can run Unix commands like grep directly in PowerShell.'
        s3    = 'Start typing, pick a command from your history, then press Enter to fill it in.'
        s4    = 'You can view a picture here, then read a Markdown file with its formatting.'
        s5    = 'Once you configure a model and give permission, it can explain the command piece by piece.'
        s6    = "Open the sidebar to see each agent's model, cost and context usage."
        s7    = 'Pick an installed font with the arrow keys. The window changes as soon as you apply it.'
        s8    = 'Once enabled, the commands also work in new keepane panes and in new shells opened by agents.'
        s9    = 'Switch to full screen and the terminal covers the taskbar.'
        s10   = 'When a new version is available, the tab bar tells you how to update.'
        end   = "The website has installation instructions. You'll find the link on screen and in the description."
    }
} }
# Each part's least length in seconds: its longest spoken line, a little before it and after it. None without -Narrate.
$least = @{}
if ($Narrate) {
    foreach ($language in $narration.Keys) {
        $lines = $narration[$language]
        foreach ($part in $lines.Keys | Where-Object { $_ -ne 'voice' }) {
            $mp3 = Join-Path $work "say-$language-$part.mp3"
            # The speech service is over the network: a file cut short has no length ffprobe can read; ask again.
            $spoken = 0
            for ($try = 1; $try -le 3 -and $spoken -le 0; $try++) {
                Remove-Item $mp3 -ErrorAction SilentlyContinue
                uvx edge-tts --voice $lines.voice --text $lines[$part] --write-media $mp3 2>&1 | Out-Null
                if (Test-Path $mp3) { $spoken = [double](ffprobe -v error -show_entries format=duration -of csv=p=0 $mp3 2>$null) }
            }
            if ($spoken -le 0) { throw "promo: edge-tts said nothing for $language $part" }
            $seconds = $spoken + 0.9
            $least[$part] = [math]::Max($seconds, ($least[$part] ?? 0))
        }
    }
}
# PowerShell writes every typed line to the user's history file unless told not to, and shows lines from that
# file in grey as you type (its predictions): neither, so no command of the user's is seen. The screen is cleared after.
$quiet = "Set-PSReadLineOption -HistorySaveStyle SaveNothing -PredictionSource None; cls # password: not for your history{ENTER}{WAIT:800}"
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
# Runs UnifyTerm with these settings and keys; its pictures land in $work. The settings file
# is $config when given (on the demo drive: a scene that shows its name shows no path of this machine).
# No scene asks GitHub for a newer version unless its settings say so: the pictures do not depend on the network.
function Record([string]$scene, [string]$toml, [string]$keys, [hashtable]$data = @{}, [string]$config = '') {
    $dir = Join-Path $work "data-$scene"; New-Item -ItemType Directory -Force $dir | Out-Null
    foreach ($file in $data.Keys) { [IO.File]::WriteAllText((Join-Path $dir $file), $data[$file]) }
    if (-not $toml.Contains('[updates]')) { $toml += "`n[updates]`ncheck = false`n" }
    if (-not $config) { $config = Join-Path $work "$scene.toml" }
    [IO.File]::WriteAllText($config, $big + $toml)
    $env:UNIFYTERM_DATA_DIR = $dir
    Push-Location "${drive}:\"
    try { & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'shot.ps1') -Exe $Exe -Out (Join-Path $work "$scene-end.png") -Type ($quiet + $keys) -Wait 5 -After 0.5 -App:" --config `"$config`"" | Out-Null }
    finally { Pop-Location; Remove-Item Env:UNIFYTERM_DATA_DIR -ErrorAction SilentlyContinue }
}
# One scene as video: its frames on its backdrop, the last held for $hold seconds.
function Join-Scene([string]$scene, [string]$backdrop, [double]$hold) {
    $frames = @(Get-ChildItem $work -Filter "$scene-0*.png" | Sort-Object Name)
    if (-not $frames) { throw "promo: no pictures for $scene" }
    $hold = [math]::Max($hold, ($least[$scene] ?? 0) - $frames.Count / $fps)
    $list = Join-Path $work "$scene.txt"
    $lines = foreach ($f in $frames) { "file '$($f.FullName -replace '\\', '/')'"; "duration $([math]::Round(1 / $fps, 4))" }
    $lines += "file '$($frames[-1].FullName -replace '\\', '/')'"; $lines += "duration $hold"; $lines += "file '$($frames[-1].FullName -replace '\\', '/')'"
    [IO.File]::WriteAllLines($list, $lines)
    $clip = Join-Path $work "$scene.mp4"
    ffmpeg -v error -y -i $backdrop -f concat -safe 0 -i $list -filter_complex "[1:v]crop=960:620:8:31,fps=$fps[w];[0:v][w]overlay=160:72,format=yuv420p,fade=t=in:st=0:d=0.3[v]" -map '[v]' -r $fps $clip
    $clip
}
# The window's own picture at one moment of a scene (the last frame unless told which), without the frame around it.
function Save-Still([string]$scene, [string]$name, [int]$height, [int]$frame = 0) {
    if (-not $Stills) { return }
    $frames = @(Get-ChildItem $work -Filter "$scene-0*.png" | Sort-Object Name)
    $pick = if ($frame -gt 0) { $frames[$frame - 1] } else { $frames[-1] }
    New-Item -ItemType Directory -Force $Stills | Out-Null
    ffmpeg -v error -y -i $pick.FullName -vf "crop=960:${height}:8:31" -frames:v 1 (Join-Path $Stills "$name.png")
}
# A scene whose window changes size (full screen): each picture fitted into the place a window has,
# a window's picture cut to its content first as in Join-Scene, a full screen's scaled down whole.
function Join-Fitted([string]$scene, [string]$backdrop, [double]$hold) {
    $frames = @(Get-ChildItem $work -Filter "$scene-0*.png" | Sort-Object Name)
    if (-not $frames) { throw "promo: no pictures for $scene" }
    $hold = [math]::Max($hold, ($least[$scene] ?? 0) - $frames.Count / $fps)
    $fitted = foreach ($f in $frames) {
        # A PNG's width: four bytes, high first, at byte 16.
        $head = [IO.File]::ReadAllBytes($f.FullName)[16..19]
        $width = ([int]$head[0] -shl 24) -bor ([int]$head[1] -shl 16) -bor ([int]$head[2] -shl 8) -bor [int]$head[3]
        $cut = if ($width -le 1000) { 'crop=960:620:8:31' } else { 'scale=960:620:force_original_aspect_ratio=decrease,pad=960:620:(ow-iw)/2:(oh-ih)/2:color=0x11131c' }
        $out = Join-Path $work "fit-$($f.Name)"
        ffmpeg -v error -y -i $f.FullName -vf $cut $out
        $out
    }
    $list = Join-Path $work "$scene.txt"
    $lines = foreach ($f in $fitted) { "file '$($f -replace '\\', '/')'"; "duration $([math]::Round(1 / $fps, 4))" }
    $lines += "file '$($fitted[-1] -replace '\\', '/')'"; $lines += "duration $hold"; $lines += "file '$($fitted[-1] -replace '\\', '/')'"
    [IO.File]::WriteAllLines($list, $lines)
    $clip = Join-Path $work "$scene.mp4"
    ffmpeg -v error -y -i $backdrop -f concat -safe 0 -i $list -filter_complex "[1:v]fps=$fps[w];[0:v][w]overlay=160:72,format=yuv420p,fade=t=in:st=0:d=0.3[v]" -map '[v]' -r $fps $clip
    $clip
}
function Join-Card([string]$card, [double]$seconds, [string]$name, [string]$part) {
    $seconds = [math]::Max($seconds, ($least[$part] ?? 0))
    $clip = Join-Path $work "$name.mp4"
    ffmpeg -v error -y -loop 1 -t $seconds -i $card -vf "format=yuv420p,fade=t=in:st=0:d=0.4,fade=t=out:st=$($seconds - 0.4):d=0.4" -r $fps $clip
    $clip
}

$clips = @()
# 1. The name and what it is.
if (-not $Only) {
    $title = Save-Card 'title' "<div style='height:720px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:26px'><div style='width:200px;height:200px'>$mark</div><div style='font-size:76px;font-weight:700;letter-spacing:-2px;color:#e6e9f5'>Unify<b>Term</b></div><div style='font-size:30px;opacity:.9'>$($caption.subtitle)</div><div style='font-size:21px;opacity:.6'>$($caption.points)</div></div>"
    $clips += Join-Card $title 3.2 'c-title' 'title'
}

if ($Cut -eq 'first') {

# 2. The same commands on Windows.
$script:frame = 0
$keys = (Typed 's2' "cat Cargo.toml | grep -n version") + "{ENTER}" + (Shots 's2' 10) + (Typed 's2' "echo '[1,2,3]' | jq add") + "{ENTER}" + (Shots 's2' 10) + (Typed 's2' "ls | wc -l") + "{ENTER}" + (Shots 's2' 8)
Record 's2' "" $keys
$clips += Join-Scene 's2' (Save-Backdrop 'b2' $caption.s2) 1.6
Save-Still 's2' 'commands' 310

# 3. The history, as you type.
$script:frame = 0
$seed = "git --version{ENTER}{WAIT:900}git log --oneline -3{ENTER}{WAIT:900}git show --stat HEAD{ENTER}{WAIT:900}cls{ENTER}{WAIT:700}"
$keys = $seed + (Typed 's3' "git") + (Shots 's3' 6) + "{KEY:CTRL+N}" + (Shots 's3' 4) + "{KEY:CTRL+N}" + (Shots 's3' 6) + "{ENTER}" + (Shots 's3' 4) + "{ENTER}" + (Shots 's3' 10)
Record 's3' "" $keys
$clips += Join-Scene 's3' (Save-Backdrop 'b3' $caption.s3) 1.6
# Frame 19: "git" typed, the list open, the second command chosen (3 + 6 + 4 + 6 pictures in).
Save-Still 's3' 'history' 300 19

# 4. Pictures and markdown.
$script:frame = 0
$keys = (Typed 's4' "ut view unifyterm.png --width 12") + "{ENTER}" + (Shots 's4' 12) + (Typed 's4' "ut md notes.md") + "{ENTER}" + (Shots 's4' 10)
Record 's4' "" $keys
$clips += Join-Scene 's4' (Save-Backdrop 'b4' $caption.s4) 1.8
Save-Still 's4' 'media' 620

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
$clips += Join-Scene 's5' (Save-Backdrop 'b5' $caption.s5) 1.6
Save-Still 's5' 'explain' 300

# 6. The agents beside the terminal (made-up agents, from the tests' fixture).
$script:frame = 0
$fixture = (Join-Path $root 'crates/unifyterm/tests/fixtures/agents.json') -replace '\\', '/'
$ut = (Join-Path (Split-Path $Exe) 'ut.exe') -replace '\\', '/'
$agents = "[agents]`nsidebar = true`ncommand = ['$ut', 'cat', '$fixture']"
$keys = (Shots 's6' 4 200) + (Typed 's6' "cat notes.md | head -4") + "{ENTER}" + (Shots 's6' 8)
Record 's6' $agents $keys
$clips += Join-Scene 's6' (Save-Backdrop 'b6' $caption.s6) 1.6
Save-Still 's6' 'agents' 330

# 7. The settings screen: the font chosen from this system's, the window changes at once. The
# settings file is on the demo drive (the screen names it); `ut setting` changes the one the window reads.
$script:frame = 0
$noColor = $env:NO_COLOR; Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue
try {
    $keys = (Typed 's7' "ut setting") + "{ENTER}" + (Shots 's7' 8 150) + "{ENTER}" + (Shots 's7' 6 150) + (Typed 's7' "courier") + (Shots 's7' 4 150) + "{ENTER}" + (Shots 's7' 12 150) + "q" + (Shots 's7' 4 150)
    Record 's7' "" $keys @{} "${drive}:\settings.toml"
} finally { if ($null -ne $noColor) { $env:NO_COLOR = $noColor }; Remove-Item "${drive}:\settings.toml" -ErrorAction SilentlyContinue }
$clips += Join-Scene 's7' (Save-Backdrop 'b7' $caption.s7) 1.6

# 8. The commands in every shell, and the skill file that tells an agent about them. `ut shell enable`
# itself is not run here: it would change this machine's profiles and PATH.
$script:frame = 0
$keys = (Typed 's8' "ut shell --help | head -3") + "{ENTER}" + (Shots 's8' 10) + (Typed 's8' "ut skill | head -4") + "{ENTER}" + (Shots 's8' 12)
Record 's8' "" $keys
$clips += Join-Scene 's8' (Save-Backdrop 'b8' $caption.s8) 1.8

# 9. Full screen, and back.
$script:frame = 0
$keys = (Typed 's9' "ls") + "{ENTER}" + (Shots 's9' 4) + "{F11}" + (Shots 's9' 24 150) + "{F11}" + (Shots 's9' 8 150)
Record 's9' "" $keys
$clips += Join-Fitted 's9' (Save-Backdrop 'b9' $caption.s9) 1.0

# 10. A newer version named in the row of tabs: what the check learned today is kept (update.json), so
# nothing is asked of GitHub here. The version is one for the picture, after this one.
$script:frame = 0
$today = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$keys = (Shots 's10' 6 200) + (Typed 's10' "ut --version") + "{ENTER}" + (Shots 's10' 10)
Record 's10' "[updates]`ncheck = true`n" $keys @{ 'update.json' = "{`"checked`": $today, `"latest`": `"0.3.0`", `"dismissed`": null}" }
$clips += Join-Scene 's10' (Save-Backdrop 'b10' $caption.s10) 1.8

# For the site alone: one picture with two commands and a picture in the terminal.
if ($Stills) {
    $script:frame = 0
    $keys = "cat Cargo.toml | grep -n version{ENTER}{WAIT:700}echo '[1,2,3]' | jq add{ENTER}{WAIT:700}ut view unifyterm.png --width 10{ENTER}{WAIT:1500}" + (Shots 'sh' 2 300)
    Record 'sh' "" $keys
    Save-Still 'sh' 'hero' 430

    # The settings screen, on a file of the demo folder's own (the picture names it: no path of this machine).
    # With colors: this script's own shell may ask for none (NO_COLOR), the picture is of what a person sees.
    $script:frame = 0
    [IO.File]::WriteAllText("${drive}:\config.toml", "[font]`nsize = 16`n`n[theme]`nbackground = `"#16161e`"`n")
    $noColor = $env:NO_COLOR; Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue
    try { Record 'ss' "" ("ut setting --config config.toml{ENTER}{WAIT:1500}{DOWN}{WAIT:400}" + (Shots 'ss' 2 300) + "q{WAIT:500}") }
    finally { if ($null -ne $noColor) { $env:NO_COLOR = $noColor }; Remove-Item "${drive}:\config.toml" -ErrorAction SilentlyContinue }
    Save-Still 'ss' 'settings' 620
}
} else {
# The second video. Each scene reads a settings file on the demo drive, so what `ut theme` and
# `ut setting` print names no path of this machine; and with colors, for the themes' samples.
function Wanted([string]$scene) { -not $Only -or $Only -contains $scene }
function Record-Second([string]$scene, [string]$toml, [string]$keys, [hashtable]$data = @{}) {
    $config = "${drive}:\settings.toml"
    $noColor = $env:NO_COLOR; Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue
    try { Record $scene $toml $keys $data $config }
    finally { if ($null -ne $noColor) { $env:NO_COLOR = $noColor }; Remove-Item $config -ErrorAction SilentlyContinue }
}
function Theme([string]$name) { "[theme]`nname = `"$name`"`n" }
$fixture = (Join-Path $root 'crates/unifyterm/tests/fixtures/agents.json') -replace '\\', '/'
$ut = (Join-Path (Split-Path $Exe) 'ut.exe') -replace '\\', '/'
$agents = "[agents]`ncommand = ['$ut', 'cat', '$fixture']`n"

# 2. The themes: the list with each one's colors, then two of them put to use.
if (Wanted 'themes') {
    $script:frame = 0
    $keys = (Typed 'themes' 'ut theme') + '{ENTER}' + (Shots 'themes' 14) + (Typed 'themes' 'ut theme aurora') + '{ENTER}' + (Shots 'themes' 16) + (Typed 'themes' 'ut theme paper') + '{ENTER}' + (Shots 'themes' 18)
    Record-Second 'themes' '' $keys
    $clips += Join-Scene 'themes' (Save-Backdrop 'b-themes' $caption.themes) 1.2
}

# 3. A wallpaper, the sidebar with the theme's picture at its foot, the list as you type.
if (Wanted 'look') {
    $script:frame = 0
    $seed = '{WAIT:300}git --version{ENTER}{WAIT:900}git log --oneline -3{ENTER}{WAIT:900}git status --short{ENTER}{WAIT:900}cls{ENTER}{WAIT:700}'
    $keys = $seed + (Shots 'look' 4 200) + (Typed 'look' 'git') + (Shots 'look' 6) + '{KEY:CTRL+N}' + (Shots 'look' 8) + '{ESC}{BACKSPACE}{BACKSPACE}{BACKSPACE}' + (Shots 'look' 2) + (Typed 'look' 'cat notes.md | head -6') + '{ENTER}' + (Shots 'look' 12)
    Record-Second 'look' ((Theme 'aurora') + "wallpaper = `"aurora.png`"`nwallpaper_opacity = 0.35`n" + $agents + "sidebar = true`n") $keys
    $clips += Join-Scene 'look' (Save-Backdrop 'b-look' $caption.look) 1.4
}

# 4. A command for a sentence (a model on this machine: tools/fake-model.ps1 with the command it gives).
if (Wanted 'ask') {
    $script:frame = 0
    $port = 8767
    $asked = Join-Path $work 'asked.json'
    [IO.File]::WriteAllText($asked, (@{ 'the five largest files here' = @{ command = 'ls -lhS | head -n 6'; note = 'ls -S puts the largest first; head keeps the first six lines' } } | ConvertTo-Json))
    $model = Start-Process pwsh -ArgumentList "-NoProfile -File `"$PSScriptRoot\fake-model.ps1`" -Port $port -Log `"$(Join-Path $work 'model-ask.jsonl')`" -DelaySeconds 1 -Asked `"$asked`"" -PassThru -WindowStyle Hidden
    Start-Sleep 2
    try {
        $env:UNIFYTERM_PROMO_KEY = 'promo'
        $ai = (Theme 'eclipse') + "[history]`ncandidates = false`n[ai]`nprovider = `"anthropic`"`nmodel = `"claude-haiku-5-5`"`nbase_url = `"http://localhost:$port`"`napi_key_env = `"UNIFYTERM_PROMO_KEY`"`nsuggest = false`n"
        $keys = (Typed 'ask' '# the five largest files here') + (Shots 'ask' 4) + '{ENTER}' + (Shots 'ask' 18 100) + '{ENTER}' + (Shots 'ask' 14)
        Record-Second 'ask' $ai $keys @{ 'ai-consent' = "Anthropic claude-haiku-5-5 http://localhost:$port" }
    } finally { Stop-Process -Id $model.Id -Force -ErrorAction SilentlyContinue; Remove-Item Env:UNIFYTERM_PROMO_KEY -ErrorAction SilentlyContinue }
    $clips += Join-Scene 'ask' (Save-Backdrop 'b-ask' $caption.ask) 1.4
}

# 5. Where the model is: the list `ut setting` offers for ai.provider.
if (Wanted 'sources') {
    $script:frame = 0
    $keys = (Typed 'sources' 'ut setting') + '{ENTER}' + (Shots 'sources' 6 150) + '/' + (Typed 'sources' 'provider') + (Shots 'sources' 4 150) + '{ENTER}' + (Shots 'sources' 4 150) + '{ENTER}' + (Shots 'sources' 6 150) + '{DOWN}' + (Shots 'sources' 3 150) + '{DOWN}' + (Shots 'sources' 3 150) + '{DOWN}' + (Shots 'sources' 6 150)
    Record-Second 'sources' (Theme 'day') $keys
    $clips += Join-Scene 'sources' (Save-Backdrop 'b-sources' $caption.sources) 1.2
}

# 6 to 8. A command list (git's, from the OpenCLI project's examples): its words as you type, a
# correction, an explanation, all in one window and without a model.
if ((Wanted 'lists') -or (Wanted 'fix') -or (Wanted 'explain')) {
    Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/spectreconsole/open-cli/main/examples/git.json' -OutFile "${drive}:\git.json"
    $script:frame = 0
    $keys = (Typed 'lists' 'ut program add git.json') + '{ENTER}' + (Shots 'lists' 8) + (Typed 'lists' 'git st') + (Shots 'lists' 16) + '{ESC}' + ('{BACKSPACE}' * 6) +
        (Typed 'fix' 'git comit -m x') + '{ENTER}' + (Shots 'fix' 20 100) + '{ESC}{WAIT:300}' +
        (Typed 'explain' 'git log --oneline -n 3') + (Shots 'explain' 3) + '{KEY:CTRL+SHIFT+E}' + (Shots 'explain' 20 100)
    try { Record-Second 'lists' (Theme 'phosphor') $keys }
    finally { Remove-Item "${drive}:\git.json" -ErrorAction SilentlyContinue }
    foreach ($scene in 'lists', 'fix', 'explain') { if (Wanted $scene) { $clips += Join-Scene $scene (Save-Backdrop "b-$scene" $caption[$scene]) 1.2 } }
}

# 9. The row of tabs out of the way, and back with the pointer at the top edge.
if (Wanted 'tabbar') {
    $script:frame = 0
    $keys = (Typed 'tabbar' 'ls') + '{ENTER}' + (Shots 'tabbar' 6) + '{KEY:CTRL+SHIFT+B}' + (Shots 'tabbar' 18 120) + '{MOVE:480,2}' + (Shots 'tabbar' 22 150)
    Record-Second 'tabbar' (Theme 'night') $keys
    $clips += Join-Scene 'tabbar' (Save-Backdrop 'b-tabbar' $caption.tabbar) 1.2
}

# 10. A key of your own for the sidebar, used at once.
if (Wanted 'shortcuts') {
    $script:frame = 0
    $keys = (Typed 'shortcuts' 'ut setting set keys.agents ctrl-alt-a') + '{ENTER}' + (Shots 'shortcuts' 8) + '{KEY:CTRL+ALT+A}' + (Shots 'shortcuts' 16 150)
    Record-Second 'shortcuts' ((Theme 'paper-dark') + $agents) $keys
    $clips += Join-Scene 'shortcuts' (Save-Backdrop 'b-shortcuts' $caption.shortcuts) 1.4
}
}

# 7. How to get it.
if (-not $Only) {
    $end = Save-Card 'end' "<div style='height:720px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:20px'><div style='display:flex;align-items:center;gap:18px'><div style='width:84px;height:84px'>$mark</div><div style='font-size:56px;font-weight:700;letter-spacing:-1.5px;color:#e6e9f5'>Unify<b>Term</b></div></div><div style='font-size:28px'>$($caption.install)</div><div style='font-size:26px;line-height:1.7;text-align:center'><b>https://dfine.tech/unifyterm/</b><br>https://github.com/newdee/unifyterm</div></div>"
    $clips += Join-Card $end 3.4 'c-end' 'end'
}

# All of it, then the GIF.
$name = if ($Only) { 'unifyterm-promo-try' } elseif ($Cut -eq 'second') { 'unifyterm-promo-2' } else { 'unifyterm-promo' }
$list = Join-Path $work 'all.txt'
[IO.File]::WriteAllLines($list, @($clips | ForEach-Object { "file '$($_ -replace '\\', '/')'" }))
$mp4 = Join-Path $Out "$name.mp4"
ffmpeg -v error -y -f concat -safe 0 -i $list -c:v libx264 -preset slow -crf 20 -pix_fmt yuv420p -movflags +faststart $mp4
$gif = Join-Path $Out "$name.gif"
ffmpeg -v error -y -i $mp4 -vf "fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" $gif
# Each language's voice over the same pictures: a part's line starts 0.3 s into its clip, silence fills the rest.
if ($Narrate) {
    foreach ($language in $narration.Keys) {
        $pieces = foreach ($clip in $clips) {
            $part = [IO.Path]::GetFileNameWithoutExtension($clip) -replace '^c-', ''
            $span = [double](ffprobe -v error -show_entries format=duration -of csv=p=0 $clip)
            $piece = Join-Path $work "voice-$language-$part.wav"
            ffmpeg -v error -y -i (Join-Path $work "say-$language-$part.mp3") -af "adelay=300|300,apad=whole_dur=$span,atrim=0:$span" -ar 48000 -ac 2 $piece
            "file '$($piece -replace '\\', '/')'"
        }
        $voiceList = Join-Path $work "voice-$language.txt"; [IO.File]::WriteAllLines($voiceList, @($pieces))
        $voice = Join-Path $work "voice-$language.wav"
        ffmpeg -v error -y -f concat -safe 0 -i $voiceList -c copy $voice
        $spoken = Join-Path $Out "$name-$language.mp4"
        ffmpeg -v error -y -i $mp4 -i $voice -map 0:v -map 1:a -c:v copy -c:a aac -b:a 160k -shortest -movflags +faststart $spoken
        "promo: {0} ({1:N1} s)" -f $spoken, [double](ffprobe -v error -show_entries format=duration -of csv=p=0 $spoken)
    }
}
$length = [double](ffprobe -v error -show_entries format=duration -of csv=p=0 $mp4)
"promo: {0} ({1:N1} s, {2:N1} MB), {3} ({4:N1} MB)" -f $mp4, $length, ((Get-Item $mp4).Length / 1MB), $gif, ((Get-Item $gif).Length / 1MB)
subst "${drive}:" /d
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
