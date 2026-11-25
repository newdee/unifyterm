# A stand-in for a language model's service, on this machine only: it
# answers unifyterm's questions with fixed answers, and writes every
# request's body to a log, one JSON a line. It speaks the Messages API's
# shape, and OpenAI's chat-completions shape for a path that ends in
# /chat/completions.
#
#   pwsh tools/fake-model.ps1 -Port 8765 -Log requests.jsonl [-DelaySeconds 0] [-Meanings FILE] [-Takes schema|object|none]
#
# What it answers, by what the question's instructions ask for:
#   completion       what was typed, then " --from-model"
#   command, reason  "echo fixed-by-model"
#   parts            the command cut at its spaces, each part "part N" (or, with -Meanings,
#                    the meaning that JSON file gives the part, as {"tar": "..."})
#   command, note    for a sentence: "echo asked-by-model"; a sentence with "delete" in it gets
#                    "rm -rf zz-not-there", with "two lines" a command of two lines, with
#                    "nothing" an empty command, with "short form" "git status --short", with
#                    "frob" "git status --frobnicate" (an option git does not have); with -Asked,
#                    the command and note that JSON file gives the sentence, as
#                    {"the five largest files here": {"command": "...", "note": "..."}}
# The key `bad-key` gets 401, as a wrong key would.
# -Takes: which `response_format` the chat-completions shape accepts. `object` refuses
# json_schema with 400, as a service that knows only JSON mode does; `none` refuses both.
param(
    [Parameter(Mandatory)][int]$Port,
    [Parameter(Mandatory)][string]$Log,
    [int]$DelaySeconds = 0,
    [string]$Meanings = '',
    [string]$Asked = '',
    [ValidateSet('schema', 'object', 'none')][string]$Takes = 'schema'
)
$known = if ($Meanings) { Get-Content -Raw $Meanings | ConvertFrom-Json -AsHashtable } else { @{} }
$answers = if ($Asked) { Get-Content -Raw $Asked | ConvertFrom-Json -AsHashtable } else { @{} }
$listener = [System.Net.HttpListener]::new()
$listener.Prefixes.Add("http://localhost:$Port/")
$listener.Start()
while ($listener.IsListening) {
    $context = $listener.GetContext()
    $reader = [System.IO.StreamReader]::new($context.Request.InputStream, [System.Text.Encoding]::UTF8)
    $body = $reader.ReadToEnd()
    Add-Content -Path $Log -Value ($body -replace "`r?`n", ' ') -Encoding utf8
    if ($context.Request.Headers['x-api-key'] -eq 'bad-key') {
        $context.Response.StatusCode = 401
        $bytes = [System.Text.Encoding]::UTF8.GetBytes('{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}')
        $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
        $context.Response.Close()
        continue
    }
    $request = $body | ConvertFrom-Json
    $chat = $context.Request.Url.AbsolutePath.EndsWith('/chat/completions')
    if ($chat) {
        $format = [string]$request.response_format.type
        $refused = ($Takes -eq 'object' -and $format -eq 'json_schema') -or ($Takes -eq 'none' -and $format)
        if ($refused) {
            $context.Response.StatusCode = 400
            $bytes = [System.Text.Encoding]::UTF8.GetBytes("{`"error`":{`"message`":`"response_format.type $format is not supported`"}}")
            $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
            $context.Response.Close()
            continue
        }
    }
    # The instructions and the question: where each shape has them.
    $system = if ($chat) { [string]$request.messages[0].content } else { [string]$request.system }
    $user = if ($chat) { [string]$request.messages[1].content } else { [string]$request.messages[0].content }
    $answer = if ($system.Contains('"completion"')) {
        $typed = [regex]::Match($user, '(?m)^Typed so far: (.*)$').Groups[1].Value
        @{ completion = "$typed --from-model" }
    } elseif ($system.Contains('"parts"')) {
        $command = [regex]::Match($user, '(?m)^Command: (.*)$').Groups[1].Value
        $n = 0
        @{ parts = @($command -split ' ' | Where-Object { $_ } | ForEach-Object { $n++; @{ text = $_; meaning = $(if ($known.ContainsKey($_)) { $known[$_] } else { "part $n" }) } }) }
    } elseif ($system.Contains('"note"')) {
        $words = [regex]::Match($user, '(?m)^Request: (.*)$').Groups[1].Value
        if ($answers.ContainsKey($words)) { $answers[$words] } else {
        $command =if ($words -match 'delete') { 'rm -rf zz-not-there' } elseif ($words -match 'two lines') { "echo one`necho two" } elseif ($words -match 'nothing') { '' } elseif ($words -match 'frob') { 'git status --frobnicate' } elseif ($words -match 'short form') { 'git status --short' } else { 'echo asked-by-model' }
        @{ command = $command; note = 'the fake model always says this' }
        }
    } else {
        @{ command = 'echo fixed-by-model'; reason = 'the fake model always says this' }
    }
    if ($DelaySeconds -gt 0) { Start-Sleep -Seconds $DelaySeconds }
    $text = $answer | ConvertTo-Json -Depth 5 -Compress
    $reply = if ($chat) {
        @{ choices = @(@{ message = @{ role = 'assistant'; content = $text } }) } | ConvertTo-Json -Depth 6 -Compress
    } else {
        @{ content = @(@{ type = 'text'; text = $text }); stop_reason = 'end_turn' } | ConvertTo-Json -Depth 5 -Compress
    }
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($reply)
    $context.Response.ContentType = 'application/json'
    $context.Response.ContentLength64 = $bytes.Length
    $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
    $context.Response.Close()
}
