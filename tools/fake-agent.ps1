# A stand-in for an agent's command line: reads the question on its standard
# input, writes it to a log when -Log names one, and prints a fixed answer.
#
#   pwsh tools/fake-agent.ps1 [-Log FILE]
param([string]$Log = '')
$question = [Console]::In.ReadToEnd()
if ($Log) { Add-Content -Path $Log -Value ($question -replace "`r?`n", ' ') -Encoding utf8 }
'{"command": "echo asked-by-agent", "note": "the fake agent always says this"}'
