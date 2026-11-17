# A stand-in for a language model's service, on this machine only: it
# answers uni-term's questions in the Messages API's shape with fixed
# answers, and writes every request's body to a log, one JSON a line.
#
#   pwsh tools/fake-model.ps1 -Port 8765 -Log requests.jsonl [-DelaySeconds 0]
#
# What it answers, by the shape the question asks for:
#   completion       what was typed, then " --from-model"
#   command, reason  "echo fixed-by-model"
#   parts            the command cut at its spaces, each part "part N"
# The key `bad-key` gets 401, as a wrong key would.
param(
    [Parameter(Mandatory)][int]$Port,
    [Parameter(Mandatory)][string]$Log,
    [int]$DelaySeconds = 0
)
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
    $user = [string]$request.messages[0].content
    $schema = $request.output_config.format.schema.properties
    $answer = if ($schema.completion) {
        $typed = [regex]::Match($user, '(?m)^Typed so far: (.*)$').Groups[1].Value
        @{ completion = "$typed --from-model" }
    } elseif ($schema.parts) {
        $command = [regex]::Match($user, '(?m)^Command: (.*)$').Groups[1].Value
        $n = 0
        @{ parts = @($command -split ' ' | Where-Object { $_ } | ForEach-Object { $n++; @{ text = $_; meaning = "part $n" } }) }
    } else {
        @{ command = 'echo fixed-by-model'; reason = 'the fake model always says this' }
    }
    if ($DelaySeconds -gt 0) { Start-Sleep -Seconds $DelaySeconds }
    $text = $answer | ConvertTo-Json -Depth 5 -Compress
    $reply = @{ content = @(@{ type = 'text'; text = $text }); stop_reason = 'end_turn' } | ConvertTo-Json -Depth 5 -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($reply)
    $context.Response.ContentType = 'application/json'
    $context.Response.ContentLength64 = $bytes.Length
    $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
    $context.Response.Close()
}
