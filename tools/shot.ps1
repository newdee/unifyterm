# Starts unifyterm, optionally types and clicks in its window, saves pictures of the window.
#
#   pwsh tools/shot.ps1 -Out shot.png [-Type 'echo hi{ENTER}'] [-Wait 4] [-After 2] [-Keep] [-App '<unifyterm args>']
#
# Input goes to unifyterm's own window handle (PostMessage) and the picture
# is taken from the window (PrintWindow): neither needs the window to be in
# front, and nothing is ever typed into another program.
#
# -Type: plain characters are sent as text (WM_CHAR, the path an input
# method's finished text takes, so Chinese works). In braces:
#   {ENTER} {ESC} {TAB} {BACKSPACE} {UP} {DOWN} {LEFT} {RIGHT} {HOME} {END}
#   {DELETE} {PGUP} {PGDN} {F1}..{F12}      a key
#   {WAIT:500}                               pause, in milliseconds
#   {CLICK:x,y} {RCLICK:x,y} {DCLICK:x,y}    a click at a point of the window's content
#   {DRAG:x1,y1,x2,y2}                       press, move, release with the left button
#   {WHEEL:x,y,notches}                      the wheel; positive is up
#   {SIZE:w,h}                               resize the window
#   {KEY:CTRL+C} {KEY:CTRL+SHIFT+T}          a key with Ctrl, Shift or Alt held
#   {SHOT:name}                              a picture now, saved beside -Out as <name>.png
#   {MEM:name}                               unifyterm's working set now, in bytes, beside -Out as <name>.mem

param(
    [Parameter(Mandatory)][string]$Out,
    [string]$Exe = "$PSScriptRoot\..\target\debug\unifyterm.exe",
    [string]$Type = '',
    [double]$Wait = 4,
    [double]$After = 2,
    [switch]$Keep,
    # unifyterm's own arguments, as one string: -App '-e nvim file.txt'
    [string]$App = ''
)

# unifyterm's data for tests, unless the caller chose a directory: never the user's own.
if (-not $env:UNIFYTERM_DATA_DIR) { $env:UNIFYTERM_DATA_DIR = Join-Path ([IO.Path]::GetTempPath()) 'unifyterm-test-data' }

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
    const uint WM_KEYDOWN = 0x0100, WM_KEYUP = 0x0101, WM_CHAR = 0x0102;
    const uint WM_MOUSEMOVE = 0x0200, WM_LDOWN = 0x0201, WM_LUP = 0x0202, WM_RDOWN = 0x0204, WM_RUP = 0x0205, WM_WHEEL = 0x020A;
    static IntPtr At(int x, int y) { return (IntPtr)((y << 16) | (x & 0xFFFF)); }
    public static void Char(IntPtr h, char c) { PostMessageW(h, WM_CHAR, (IntPtr)c, (IntPtr)1); }
    public static void Key(IntPtr h, int vk) {
        PostMessageW(h, WM_KEYDOWN, (IntPtr)vk, (IntPtr)1);
        PostMessageW(h, WM_KEYUP, (IntPtr)vk, unchecked((IntPtr)(long)0xC0000001));
    }
    public static void Move(IntPtr h, int x, int y, bool held) { PostMessageW(h, WM_MOUSEMOVE, (IntPtr)(held ? 1 : 0), At(x, y)); }
    public static void Left(IntPtr h, int x, int y, bool down) { PostMessageW(h, down ? WM_LDOWN : WM_LUP, (IntPtr)(down ? 1 : 0), At(x, y)); }
    public static void Right(IntPtr h, int x, int y, bool down) { PostMessageW(h, down ? WM_RDOWN : WM_RUP, (IntPtr)(down ? 2 : 0), At(x, y)); }
    // The wheel's position is in screen coordinates.
    public static void Wheel(IntPtr h, int x, int y, int notches) {
        var p = new POINT { X = x, Y = y }; ClientToScreen(h, ref p);
        PostMessageW(h, WM_WHEEL, (IntPtr)((notches * 120) << 16), At(p.X, p.Y));
    }
    public static void Size(IntPtr h, int w, int ht) { SetWindowPos(h, IntPtr.Zero, 0, 0, w, ht, 0x0002 | 0x0004 | 0x0010); }

    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("user32.dll")] static extern bool GetKeyboardState(byte[] state);
    [DllImport("user32.dll")] static extern bool SetKeyboardState(byte[] state);
    // A key with modifiers held. The window reads modifiers from its
    // thread's keyboard state, so this thread shares that state for a
    // moment and marks the modifiers as down in it: still no real key is
    // pressed and no other window sees anything.
    public static void Combo(IntPtr h, bool ctrl, bool shift, bool alt, int vk) {
        uint me = GetCurrentThreadId(), them = GetWindowThreadProcessId(h, IntPtr.Zero);
        AttachThreadInput(me, them, true);
        var before = new byte[256]; GetKeyboardState(before);
        var held = (byte[])before.Clone();
        if (ctrl) held[0x11] = 0x80;
        if (shift) held[0x10] = 0x80;
        if (alt) held[0x12] = 0x80;
        SetKeyboardState(held);
        PostMessageW(h, WM_KEYDOWN, (IntPtr)vk, (IntPtr)1);
        System.Threading.Thread.Sleep(120);
        PostMessageW(h, WM_KEYUP, (IntPtr)vk, unchecked((IntPtr)(long)0xC0000001));
        System.Threading.Thread.Sleep(40);
        SetKeyboardState(before);
        AttachThreadInput(me, them, false);
    }
}
'@
[void][Win]::SetProcessDPIAware()

$vk = @{ ENTER = 0x0D; ESC = 0x1B; TAB = 0x09; BACKSPACE = 0x08; UP = 0x26; DOWN = 0x28; LEFT = 0x25; RIGHT = 0x27
         HOME = 0x24; END = 0x23; DELETE = 0x2E; PGUP = 0x21; PGDN = 0x22 }
1..12 | ForEach-Object { $vk["F$_"] = 0x6F + $_ }

function Save-Shot([IntPtr]$h, [string]$path) {
    $r = New-Object Win+RECT
    [void][Win]::GetWindowRect($h, [ref]$r)
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [Win]::PrintWindow($h, $hdc, 2)   # 2: the whole window as the compositor draws it
    $g.ReleaseHdc($hdc)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    "saved ${w}x${ht} (PrintWindow: $ok) -> $path"
}

# UnifyTerm starts its tabs in the home directory. What is typed here must not
# land there: the tab starts where this script was started. (/ for \: a path
# that ends in \ would take the closing quote with it.)
$here = (Get-Location).Path.Replace('\', '/')
$p = Start-Process -FilePath $Exe -ArgumentList "--cwd `"$here`" $App" -PassThru
$deadline = (Get-Date).AddSeconds(20)
while ((Get-Date) -lt $deadline) {
    $p.Refresh()
    if ($p.HasExited) { "exited early: $($p.ExitCode)"; exit 1 }
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 200
}
$h = $p.MainWindowHandle
"window handle: $h  title: '$($p.MainWindowTitle)'"
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; exit 1 }
Start-Sleep -Seconds $Wait

foreach ($part in [regex]::Split($Type, '(\{[A-Z0-9]+(?::[^}]*)?\})')) {
    $p.Refresh()
    if ($p.HasExited) { break }
    if ($part -match '^\{([A-Z0-9]+)(?::([^}]*))?\}$') {
        $name = $Matches[1]; $arg = $Matches[2]; $n = @(); if ($arg) { $n = $arg -split ',' }
        switch ($name) {
            'WAIT'   { Start-Sleep -Milliseconds ([int]$arg) }
            'SHOT'   { Save-Shot $h (Join-Path (Split-Path $Out) "$arg.png") }
            'MEM'    { $p.Refresh(); [IO.File]::WriteAllText((Join-Path (Split-Path $Out) "$arg.mem"), [string]$p.WorkingSet64) }
            'CLICK'  { [Win]::Move($h, $n[0], $n[1], $false); [Win]::Left($h, $n[0], $n[1], $true); [Win]::Left($h, $n[0], $n[1], $false) }
            'DCLICK' { 1..2 | ForEach-Object { [Win]::Left($h, $n[0], $n[1], $true); [Win]::Left($h, $n[0], $n[1], $false) } }
            'RCLICK' { [Win]::Move($h, $n[0], $n[1], $false); [Win]::Right($h, $n[0], $n[1], $true); [Win]::Right($h, $n[0], $n[1], $false) }
            'DRAG'   {
                [Win]::Move($h, $n[0], $n[1], $false); [Win]::Left($h, $n[0], $n[1], $true); Start-Sleep -Milliseconds 60
                [Win]::Move($h, ([int]$n[0] + [int]$n[2]) / 2, ([int]$n[1] + [int]$n[3]) / 2, $true); Start-Sleep -Milliseconds 60
                [Win]::Move($h, $n[2], $n[3], $true); Start-Sleep -Milliseconds 60; [Win]::Left($h, $n[2], $n[3], $false)
            }
            'WHEEL'  { [Win]::Wheel($h, $n[0], $n[1], $n[2]) }
            'SIZE'   { [Win]::Size($h, $n[0], $n[1]) }
            'KEY'    {
                $keys = $arg -split '\+'; $last = $keys[-1]
                $code = if ($vk.ContainsKey($last)) { $vk[$last] } else { [int][char]$last.ToUpper() }
                [Win]::Combo($h, $keys -contains 'CTRL', $keys -contains 'SHIFT', $keys -contains 'ALT', $code)
            }
            default  { if ($vk.ContainsKey($name)) { [Win]::Key($h, $vk[$name]) } else { "unknown: {$name}" } }
        }
        Start-Sleep -Milliseconds 40
    } else {
        foreach ($c in $part.ToCharArray()) { [Win]::Char($h, $c); Start-Sleep -Milliseconds 8 }
    }
}
Start-Sleep -Seconds $After

$p.Refresh()
if ($p.HasExited) { "exited: $($p.ExitCode)"; exit 0 }
"title now: '$($p.MainWindowTitle)'"
Save-Shot $h $Out
if (-not $Keep) { Stop-Process -Id $p.Id -Force }
