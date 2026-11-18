# Every script in the repository parses: the .ps1 files, the .sh files, and the
# scripts written inside the workflows' `run:` steps. A workflow's script is
# checked in each shell it will run in: pwsh on a Windows runner, bash on the
# others, or the step's own `shell:`.
#
#   pwsh tools/scripts-check.ps1     prints one line per kind; exit 1 on a failure
#
# Needs python with PyYAML (to read the workflows) and bash (through WSL on Windows).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$failed = 0
function Report([string]$what, [bool]$ok, [string]$detail) {
    if (-not $ok) { $script:failed++ }
    "{0}  {1,-34} {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $what, $detail
}
function Test-PowerShell([string]$text) {
    $errors = $null
    $null = [System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$null, [ref]$errors)
    @($errors | ForEach-Object { "line $($_.Extent.StartLineNumber): $($_.Message)" })
}
function Test-Bash([string]$path) {
    if ($IsWindows) {
        $full = (Resolve-Path $path).Path
        $linux = '/mnt/' + $full.Substring(0, 1).ToLower() + ($full.Substring(2) -replace '\\', '/')
        $out = wsl -- bash -n $linux 2>&1
    } else {
        $out = bash -n $path 2>&1
    }
    if ($LASTEXITCODE -ne 0) { @("$out") } else { @() }
}
$tracked = @(git -C $root ls-files --cached --others --exclude-standard)

# 1. The .ps1 files.
$problems = @()
$ps = @($tracked | Where-Object { $_ -like '*.ps1' })
foreach ($file in $ps) {
    # A template's @NAME@ is filled in by the program before PowerShell reads it: nothing stands for it here.
    $text = [regex]::Replace([IO.File]::ReadAllText((Join-Path $root $file)), '@[A-Z_]+@', '')
    foreach ($e in Test-PowerShell $text) { $problems += "${file}: $e" }
}
Report 'PowerShell files parse' ($problems.Count -eq 0) ("{0} files{1}" -f $ps.Count, $(if ($problems) { "; $($problems -join ' | ')" }))

# 2. The .sh files.
$problems = @()
$sh = @($tracked | Where-Object { $_ -like '*.sh' })
foreach ($file in $sh) {
    foreach ($e in Test-Bash (Join-Path $root $file)) { $problems += "${file}: $e" }
}
Report 'shell files parse' ($problems.Count -eq 0) ("{0} files{1}" -f $sh.Count, $(if ($problems) { "; $($problems -join ' | ')" }))

# 3. The scripts inside the workflows.
$reader = @'
import json, re, sys, yaml

def shells(job, step, top):
    own = step.get("shell") or (job.get("defaults") or {}).get("run", {}).get("shell") or (top.get("defaults") or {}).get("run", {}).get("shell")
    if own:
        return [own]
    # A step for one system only runs in that system's shell.
    only = re.search(r"runner\.os\s*==\s*'(\w+)'", str(step.get("if", "")))
    if only:
        return ["pwsh" if only.group(1) == "Windows" else "bash"]
    runs_on = str(job.get("runs-on", ""))
    systems = [runs_on]
    asked = re.match(r"\$\{\{\s*matrix\.(\w+)\s*\}\}", runs_on)
    if asked:
        matrix = (job.get("strategy") or {}).get("matrix") or {}
        systems = [str(v) for v in matrix.get(asked.group(1), [])]
        systems += [str(entry[asked.group(1)]) for entry in matrix.get("include", []) if asked.group(1) in entry]
    return sorted({"pwsh" if "windows" in system else "bash" for system in systems})

found = []
for path in sys.argv[1:]:
    with open(path, encoding="utf-8") as handle:
        top = yaml.safe_load(handle)
    for job_name, job in (top.get("jobs") or {}).items():
        for index, step in enumerate(job.get("steps") or []):
            if "run" in step:
                found.append({"file": path, "job": job_name, "step": step.get("name") or f"step {index + 1}", "shells": shells(job, step, top), "run": step["run"]})
print(json.dumps(found))
'@
$workflows = @($tracked | Where-Object { $_ -match '(^|/)workflows/[^/]+\.ya?ml$' })
$temp = Join-Path ([IO.Path]::GetTempPath()) "unifyterm-scripts-check-$PID"
New-Item -ItemType Directory -Force $temp | Out-Null
try {
    [IO.File]::WriteAllText((Join-Path $temp 'read.py'), $reader)
    $json = python (Join-Path $temp 'read.py') @($workflows | ForEach-Object { Join-Path $root $_ })
    if ($LASTEXITCODE -ne 0) { throw 'python with PyYAML could not read the workflows' }
    $steps = @($json | ConvertFrom-Json)
    $problems = @()
    $checked = 0
    foreach ($step in $steps) {
        # An expression is filled in before the shell sees the script: a plain word stands for it here.
        $text = [regex]::Replace($step.run, '\$\{\{.*?\}\}', 'filled')
        $where = "$([IO.Path]::GetFileName($step.file)) / $($step.job) / $($step.step)"
        foreach ($shell in $step.shells) {
            $checked++
            if ($shell -in 'pwsh', 'powershell') {
                foreach ($e in Test-PowerShell $text) { $problems += "$where (pwsh): $e" }
            } elseif ($shell -in 'bash', 'sh') {
                $file = Join-Path $temp 'step.sh'
                [IO.File]::WriteAllText($file, $text.Replace("`r", ''))
                foreach ($e in Test-Bash $file) { $problems += "$where (bash): $e" }
            } else {
                $problems += "${where}: a shell this check does not know: $shell"
            }
        }
    }
    Report 'scripts in the workflows parse' ($problems.Count -eq 0) ("{0} workflows, {1} steps, {2} checks{3}" -f $workflows.Count, $steps.Count, $checked, $(if ($problems) { "; $($problems -join ' | ')" }))
} finally {
    Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
}

"$failed failed"
exit $(if ($failed) { 1 } else { 0 })
