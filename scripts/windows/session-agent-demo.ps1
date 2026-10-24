<#
.SYNOPSIS
    Demonstrate the Session Agent path end to end with the real Core.

.DESCRIPTION
    Starts a throwaway Core (its own database, workspace and RPC pipe in a
    temporary directory) and walks one synthetic operation through it:

      1. A client submits "healthcheck demo". The worker proposes the typed
         call session_agent.healthcheck, which is class B.
      2. The Core asks for approval; the request is shown as a person sees it.
      3. The approval is granted for that exact fingerprint. The Core consumes
         it, issues a single-use grant, starts jarvis-session-agent and has it
         perform the health check once.
      4. The task, the job and the audit log show what happened. The agent's
         report stays with the Core; the worker gets a receipt.
      5. The same approval is offered again and refused: it was consumed and
         its grant used. A second identical request needs its own approval,
         which is declined here, and nothing more is performed.

    The health check touches no desktop, clipboard or input. Run as your
    normal user from the repository root. Nothing outside the temporary
    directory changes, apart from the worker's AppContainer profile, which
    the Core creates for every run.

.PARAMETER Python
    Absolute path of a python.exe (3.11+) for the worker.

.EXAMPLE
    scripts\windows\session-agent-demo.ps1 -Python C:\Python314\python.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Python
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Step([string]$text) { Write-Host "`n== $text" -ForegroundColor Cyan }

Step 'Build the Core and the Session Agent'
& cargo build --quiet --locked -p jarvis-core --bins
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
$core = Join-Path $repo 'target\debug\jarvis-core.exe'

$work = Join-Path ([IO.Path]::GetTempPath()) ("jarvis-agent-demo-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null
$config = Join-Path $work 'jarvis.toml'
$pipe = 'jarvis-demo-' + (Split-Path $work -Leaf)
@"
database = 'jarvis.db'

[worker]
program = '$Python'
args = ["-m", "jarvis_worker"]
cwd = '$repo\services\intelligence-python\src'

[fixtures]
root = '$repo\tests\fixtures\files'

[workspace]
root = 'workspace'

[rpc]
pipe = '$pipe'
allow_shutdown = true

[policy]
"system.info" = "allow"
"session_agent.healthcheck" = "require_confirmation"
"@ | Set-Content -Encoding utf8 -Path $config
New-Item -ItemType Directory -Path (Join-Path $work 'workspace') | Out-Null

function Core([string[]]$arguments) {
    # A refusal is an expected answer here, not a script error.
    $ErrorActionPreference = 'Continue'
    $output = & $core @arguments --config $config 2>&1 | ForEach-Object { "$_" }
    $script:status = $LASTEXITCODE
    $output
}

Step 'Start the Core'
$out = Join-Path $work 'core.out'
$err = Join-Path $work 'core.err'
$serve = Start-Process -FilePath $core -ArgumentList @('serve', '--config', "`"$config`"") `
    -RedirectStandardOutput $out -RedirectStandardError $err -PassThru -NoNewWindow
try {
    $deadline = (Get-Date).AddSeconds(60)
    while (-not ((Test-Path $out) -and (Select-String -Path $out -Pattern '^ready ' -Quiet))) {
        if ($serve.HasExited -or (Get-Date) -gt $deadline) {
            Get-Content $err | Select-Object -Last 20
            throw 'the Core did not become ready'
        }
        Start-Sleep -Milliseconds 200
    }
    Get-Content $out

    Step '1. A client asks for a synthetic action'
    $job = (Core @('submit', 'healthcheck demo', '--json')) | ConvertFrom-Json
    "job $($job.job_id): $($job.goal)"

    Step '2. The Core asks a person, in words'
    $pending = (Core @('approvals', 'list', '--wait', '30s', '--json')) | ConvertFrom-Json
    $approval = @($pending)[0]
    Core @('approvals', 'show', $approval.approval_id)

    Step '3. Approve exactly what was shown'
    Core @('approvals', 'approve', $approval.approval_id, '--fingerprint', $approval.fingerprint, '--yes')
    Core @('job', $job.job_id, '--wait', '--timeout', '60s')

    Step '4. What was recorded'
    Core @('tasks', '--limit', '5')
    Core @('audit', '--limit', '40') | Select-String -Pattern 'approval_|execution_|session_agent_'

    Step '5. The same approval cannot be used again'
    Core @('approvals', 'approve', $approval.approval_id, '--fingerprint', $approval.fingerprint, '--yes')
    if ($status -eq 0) { throw 'a consumed approval was accepted again' }
    "refused, exit code $status"

    Step '   ...and the same request again needs a new approval'
    $again = (Core @('submit', 'healthcheck demo', '--json')) | ConvertFrom-Json
    $second = @((Core @('approvals', 'list', '--wait', '30s', '--json')) | ConvertFrom-Json)[0]
    if ($second.approval_id -eq $approval.approval_id) { throw 'the old approval was reused' }
    "new approval $($second.approval_id) for job $($again.job_id); declining it"
    Core @('approvals', 'deny', $second.approval_id, '--reason', 'demo: not again')
    Core @('job', $again.job_id, '--wait', '--timeout', '60s')
    $performed = (Core @('audit', '--limit', '200') | Select-String -Pattern 'session_agent_completed').Count
    "Session Agent operations performed: $performed"
    if ($performed -ne 1) { throw "expected exactly one performed operation, got $performed" }
}
finally {
    Step 'Stop the Core'
    Core @('shutdown') | Out-Null
    if (-not $serve.WaitForExit(15000)) { Stop-Process -Id $serve.Id -Force }
    "work directory: $work"
}
