# Written by uni-term each time it starts: changes here are lost.
# PowerShell runs this after your profile, in uni-term's tabs only.
# `[commands] builtin = false` in uni-term's settings turns all of it off.

# 1. uni-term's commands (cat, ls, ...) are programs in PATH. PowerShell finds
# an alias or a function of the same name first, so its own ones are removed.
# An alias or function that is no longer PowerShell's own (yours) is kept.
[Console]::OutputEncoding = [Console]::InputEncoding = $OutputEncoding = [System.Text.UTF8Encoding]::new($false)
foreach ($pair in @(@ALIASES@)) {
    $alias = Get-Alias -Name $pair[0] -ErrorAction Ignore
    if ($alias -and $alias.Definition -eq $pair[1]) { Remove-Item -Path "Alias:\$($pair[0])" -Force -ErrorAction Ignore }
}
foreach ($pair in @(@FUNCTIONS@)) {
    $function = Get-Item -Path "Function:\$($pair[0])" -ErrorAction Ignore
    if ($function -and $function.ScriptBlock.ToString().Contains($pair[1])) { Remove-Item -Path "Function:\$($pair[0])" -Force -ErrorAction Ignore }
}

# 2. The prompt shows the folder with / as on every other system, while the
# prompt is still PowerShell's own. Set UT_NATIVE_PATHS to see Windows' own \.
$function = Get-Item -Path 'Function:\prompt' -ErrorAction Ignore
if ($function -and $function.ScriptBlock.ToString().Contains('PS $($executionContext.SessionState.Path.CurrentLocation)$(')) {
    function global:prompt {
        $location = [string]$executionContext.SessionState.Path.CurrentLocation
        if (-not $env:UT_NATIVE_PATHS) { $location = $location.Replace('\', '/') }
        "PS $location$('>' * ($nestedPromptLevel + 1)) "
    }
}
Remove-Variable pair, alias, function -ErrorAction Ignore

# The commands of uni-term's that are in use. The directory they are in is
# $env:UNI_TERM_COMMANDS, which uni-term sets for every shell it starts.
$global:__uni_term_names = [System.Collections.Generic.HashSet[string]]::new([string[]]@(@NAMES@), [System.StringComparer]::OrdinalIgnoreCase)

# 3. "Coreutils for Windows" puts a reader of each typed line in the profile,
# which runs its own cat, ls, ... whatever PATH says. The names of uni-term's
# commands in use are taken out of its list, for this tab; the others stay its.
if ($global:__COREUTILS__ -is [System.Collections.Generic.HashSet[string]]) {
    foreach ($name in $global:__uni_term_names) { [void]$global:__COREUTILS__.Remove($name) }
    Remove-Variable name -ErrorAction Ignore
}

# 4. On Windows uni-term's commands replace a wildcard (*.txt) by the names it
# matches, as a Unix shell does before a command starts. In quotes ("*.txt")
# it is to stay as typed, but PowerShell takes the quotes off before a program
# sees them. So each typed line is read here first: a quoted argument of one
# of uni-term's commands gets a mark as its first character (U+E000), which
# the command takes off again. Your history keeps the line as you typed it.
function global:__uni_term_mark([string]$line) {
    if (-not $line -or -not $env:UNI_TERM_COMMANDS -or $line.IndexOfAny([char[]]"'`"") -lt 0) { return $line }
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($line, [ref]$null, [ref]$errors)
    if ($errors) { return $line }
    $at = foreach ($command in $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.CommandAst] }, $true)) {
        $name = $command.GetCommandName()
        if (-not $name -or -not $global:__uni_term_names.Contains(($name -replace '\.exe$', ''))) { continue }
        # Only when the name is uni-term's program now: not your function or alias of that name.
        $found = Get-Command -Name $name -ErrorAction Ignore | Select-Object -First 1
        if (-not $found -or $found.CommandType -ne 'Application' -or -not $found.Source.StartsWith($env:UNI_TERM_COMMANDS, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
        foreach ($word in $command.CommandElements | Select-Object -Skip 1) {
            $string = $word -is [System.Management.Automation.Language.StringConstantExpressionAst] -or $word -is [System.Management.Automation.Language.ExpandableStringExpressionAst]
            if ($string -and $word.StringConstantType -in 'SingleQuoted', 'DoubleQuoted') { $word.Extent.StartOffset + 1 }
        }
    }
    foreach ($offset in $at | Sort-Object -Descending -Unique) { $line = $line.Insert($offset, [string][char]0xE000) }
    $line
}
# The reader of typed lines that is there now (PSReadLine's, or one a profile put in its place) is kept and asked first.
$global:__uni_term_read = (Get-Command -Name PSConsoleHostReadLine -ErrorAction Ignore).ScriptBlock
if ($global:__uni_term_read) {
    function global:PSConsoleHostReadLine { __uni_term_mark (& $global:__uni_term_read) }
}
