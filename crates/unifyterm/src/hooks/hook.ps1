# Written by unifyterm each time it starts: changes here are lost.
# PowerShell runs this after your profile, in unifyterm's tabs only.
# `[history] enabled = false` in unifyterm's settings turns it off.
#
# It tells unifyterm, in standard terminal marks, where the prompt starts
# and ends (OSC 133;A and B), what the last command was and how it ended
# (OSC 633;E and 133;D) and which directory this is (OSC 633;P;Cwd). Your
# prompt itself is kept as it is: this wraps it. With prompt.enabled in
# unifyterm's settings, ut prompt writes the prompt in its place.
# Adapted from keepane's PowerShell hook (MIT, same author).

# A mark's text: \ is written \\, ; is \x3b, a control character \xHH.
function global:__unifyterm_escape([string]$text) {
    $out = [System.Text.StringBuilder]::new()
    foreach ($c in $text.ToCharArray()) {
        if ($c -eq '\') { [void]$out.Append('\\') }
        elseif ($c -eq ';') { [void]$out.Append('\x3b') }
        elseif ([int]$c -lt 32) { [void]$out.Append(('\x{0:x2}' -f [int]$c)) }
        else { [void]$out.Append($c) }
    }
    $out.ToString()
}

# The history entry last reported: a prompt with no new entry reports no command.
# Not known yet: the line that started this shell (-Command ...) enters the
# history only when it ends, so the first prompt takes note and reports nothing.
$global:__unifyterm_seen = $null
$global:__unifyterm_first = $true
$global:__unifyterm_prompt = $function:prompt

function global:prompt {
    # First, before anything changes them: how the last command ended.
    $ok = $?
    $code = $global:LASTEXITCODE
    $e = [char]27
    $marks = ''
    $last = Get-History -Count 1
    $ran = $false
    if ($global:__unifyterm_first) {
        $global:__unifyterm_first = $false
        $global:__unifyterm_seen = $last.Id
    }
    elseif ($last -and $last.Id -ne $global:__unifyterm_seen) {
        $global:__unifyterm_seen = $last.Id
        $exit = if ($ok) { 0 } elseif ($code) { $code } else { 1 }
        $ran = $true
        $marks += "$e]633;E;$(__unifyterm_escape $last.CommandLine)$e\$e]133;D;$exit$e\"
    }
    $location = $executionContext.SessionState.Path.CurrentLocation
    if ($location.Provider.Name -eq 'FileSystem') {
        $marks += "$e]633;P;Cwd=$(__unifyterm_escape $location.ProviderPath)$e\"
    }
    # UnifyTerm writes the prompt when its settings say prompt.enabled: ut prompt, told how the
    # last command went. If it says nothing, the prompt is the shell's own, as below.
    $text = $null
    if ($env:UNIFYTERM_PROMPT) {
        $arguments = @('prompt', '--shell', 'pwsh', '--width', $Host.UI.RawUI.WindowSize.Width)
        if ($location.Provider.Name -eq 'FileSystem') { $arguments += @('--cwd', $location.ProviderPath) }
        if ($ran) {
            $took = [long]($last.EndExecutionTime - $last.StartExecutionTime).TotalMilliseconds
            $arguments += @('--status', $exit, '--took', $took)
        }
        $text = (& $env:UNIFYTERM_PROMPT @arguments 2>$null) -join "`n"
    }
    # The shell's own prompt, called once, when UnifyTerm writes none (or ut said nothing).
    if ($null -eq $text -or ($env:UNIFYTERM_PROMPT -and -not $text)) {
        $text = if ($global:__unifyterm_prompt) { & $global:__unifyterm_prompt } else { "PS $location$('>' * ($nestedPromptLevel + 1)) " }
    }
    # The prompt function must not change how the last command ended for whoever asks next.
    $global:LASTEXITCODE = $code
    "$marks$e]133;A$e\$text$e]133;B$e\"
}

[Console]::Write("$([char]27)]633;P;Shell=$(if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' })$([char]27)\")
