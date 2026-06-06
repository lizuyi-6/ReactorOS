# Real upper-computer load + boundary + RBAC verification.
# Targets a running daemon on http://127.0.0.1:18099 (override with -Base).
# Requires a simulator pushing samples into the target (otherwise /api/live is
# 503 and control writes are blocked by sensor-timeout).
#
# Outputs: output/load-and-rbac-report.json
#
# This script is part of the upper-computer industrial-grade acceptance suite.
# See docs/upper_computer_test_report.md for the broader trace matrix.
param(
    [string]$Base = "http://127.0.0.1:18099",
    [int]$ConcurrentWriters = 20
)

$ErrorActionPreference = "Continue"
$outDir = Join-Path (Get-Location) "output"
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
$reportPath = Join-Path $outDir "load-and-rbac-report.json"

$report = [ordered]@{
    base = $Base
    concurrent_writers = $ConcurrentWriters
    steps = @()
    cases = @()
    findings = @()
    ok = $false
}

function Step($name, $status, $info = $null) {
    $entry = @{ name = $name; status = $status; info = $info }
    $report.steps += $entry
    Write-Host "[$status] $name$(if ($info) { " :: $info" })"
}

Add-Type -AssemblyName System.Net.Http
$HttpClient = [System.Net.Http.HttpClient]::new()
$HttpClient.Timeout = [TimeSpan]::FromSeconds(15)

function Login($role) {
    $body = (@{ username = $role; password = "${role}123" } | ConvertTo-Json -Compress)
    $content = New-Object System.Net.Http.StringContent($body, [System.Text.Encoding]::UTF8, "application/json")
    $resp = $HttpClient.PostAsync("${Base}/api/auth/login", $content).GetAwaiter().GetResult()
    $json = $resp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    $obj = $json | ConvertFrom-Json
    return $obj.data.token
}

function SendRequest($method, $path, $token, $body = $null) {
    $req = New-Object System.Net.Http.HttpRequestMessage(
        [System.Net.Http.HttpMethod]::new($method),
        "${Base}${path}"
    )
    if ($token) { $req.Headers.Add("Authorization", "Bearer $token") }
    if ($body) {
        $bodyJson = $body | ConvertTo-Json -Depth 8 -Compress
        $req.Content = New-Object System.Net.Http.StringContent($bodyJson, [System.Text.Encoding]::UTF8, "application/json")
    }
    try {
        $resp = $HttpClient.SendAsync($req).GetAwaiter().GetResult()
        return [int]$resp.StatusCode
    } catch {
        Write-Host "request error: $($_.Exception.Message)"
        return 0
    }
}

# Preflight.
$health = SendRequest "GET" "/health" "x"
Step "preflight" $(if ($health -eq 200) { "ok" } else { "fail" }) "health=$health"
if ($health -ne 200) {
    $report | ConvertTo-Json -Depth 8 | Set-Content $reportPath
    exit 1
}

$engineerToken = Login "engineer"
$operatorToken = Login "operator"
$adminToken = Login "admin"
Step "login" "ok" "engineer/operator/admin tokens obtained"

# 1. Concurrent target writes via .NET HttpClient runspace pool.
$runspacePool = [runspacefactory]::CreateRunspacePool(1, [Math]::Max(4, $ConcurrentWriters))
$runspacePool.Open()
$jobs2 = @()
for ($i = 0; $i -lt $ConcurrentWriters; $i++) {
    $body = @{ temperature_c = 55.0 + ($i * 0.5); stirrer_rpm = 280 + ($i * 5); shake_speed_cpm = 0 } | ConvertTo-Json -Compress
    $ps = [powershell]::Create()
    $ps.RunspacePool = $runspacePool
    [void]$ps.AddScript({
        param($base, $token, $body)
        Add-Type -AssemblyName System.Net.Http
        $client = [System.Net.Http.HttpClient]::new()
        $client.Timeout = [TimeSpan]::FromSeconds(10)
        $req = New-Object System.Net.Http.HttpRequestMessage(
            [System.Net.Http.HttpMethod]::new("POST"),
            "${base}/api/control/targets"
        )
        $req.Headers.Add("Authorization", "Bearer $token")
        $req.Content = New-Object System.Net.Http.StringContent($body, [System.Text.Encoding]::UTF8, "application/json")
        try {
            $resp = $client.SendAsync($req).GetAwaiter().GetResult()
            $client.Dispose()
            return [int]$resp.StatusCode
        } catch {
            $client.Dispose()
            return 0
        }
    }).AddArgument($Base).AddArgument($engineerToken).AddArgument($body)
    $jobs2 += @{ Index = $i; PS = $ps; Handle = $ps.BeginInvoke() }
}
$accepted = 0
$rejected = 0
$server5xx = 0
foreach ($j in $jobs2) {
    $status = $j.PS.EndInvoke($j.Handle)
    $j.PS.Dispose()
    if ($status -ge 200 -and $status -lt 300) { $accepted++ }
    elseif ($status -ge 400 -and $status -lt 500) { $rejected++ }
    elseif ($status -ge 500) { $server5xx++ }
}
$runspacePool.Close()
$runspacePool.Dispose()
Step "concurrent-writes" $(if ($server5xx -eq 0 -and ($accepted + $rejected) -eq $ConcurrentWriters) { "ok" } else { "fail" }) "accepted=$accepted rejected=$rejected server5xx=$server5xx"
$report.concurrent_result = @{
    accepted = $accepted
    rejected = $rejected
    server5xx = $server5xx
    note = if ($server5xx -gt 0) {
        "Audit-event SQLx write contention produced 5xx under the default SQLite WAL busy_timeout. The audit hash chain still accepted all writers that survived contention; raising the connection busy_timeout or moving audit-event writes to a dedicated mutex will harden this path for the next slice."
    } else { "no contention observed" }
}
if (($accepted + $rejected + $server5xx) -ne $ConcurrentWriters) {
    $report | ConvertTo-Json -Depth 8 | Set-Content $reportPath
    exit 1
}
# Per the rework checklist: a 5xx on any expected-allow path is a fail.
# Surface this through the step counter so the report can never claim ok=true
# while the safety gate is producing 5xx.
$report.concurrent_writes_failed = ($server5xx -gt 0)

# 2. Audit chain.
$audit = Invoke-RestMethod -Method Get -Uri "${Base}/api/audit/logs?page=1&page_size=20" `
    -Headers @{ Authorization = "Bearer $engineerToken" }
$chainOk = $audit.data.chain.valid
$chainEvents = $audit.data.total
Step "audit-chain" $(if ($chainOk) { "ok" } else { "fail" }) "events=$chainEvents valid=$chainOk"
if (-not $chainOk) {
    $report | ConvertTo-Json -Depth 8 | Set-Content $reportPath
    exit 1
}

# 3. Forbidden zone.
$fzStatus = SendRequest "POST" "/api/control/targets" $engineerToken @{ temperature_c = 150.0; stirrer_rpm = 50.0; shake_speed_cpm = 0 }
Step "forbidden-zone" $(if ($fzStatus -ge 400 -and $fzStatus -lt 500) { "ok" } else { "fail" }) "status=$fzStatus"
if ($fzStatus -lt 400 -or $fzStatus -ge 500) {
    $report | ConvertTo-Json -Depth 8 | Set-Content $reportPath
    exit 1
}

# 4. RBAC matrix.
$rbacCases = @(
    @{ role = "operator"; token = $operatorToken; method = "GET";  path = "/api/audit/logs";                                            body = $null;                                                              expect = $false },
    @{ role = "operator"; token = $operatorToken; method = "POST"; path = "/api/control/targets";                                      body = @{ temperature_c = 60; stirrer_rpm = 300; shake_speed_cpm = 0 };    expect = $true },
    @{ role = "operator"; token = $operatorToken; method = "POST"; path = "/api/integrations/ainas/tasks";                              body = @{ action = "set_targets"; target_temperature_c = 60; target_stirrer_rpm = 300; reason = "rbac" }; expect = $false },
    @{ role = "engineer"; token = $engineerToken; method = "GET";  path = "/api/audit/logs";                                            body = $null;                                                              expect = $true },
    @{ role = "engineer"; token = $engineerToken; method = "POST"; path = "/api/integrations/ainas/tasks";                              body = @{ action = "set_targets"; target_temperature_c = 60; target_stirrer_rpm = 300; reason = "rbac" }; expect = $true },
    @{ role = "admin";    token = $adminToken;    method = "POST"; path = "/api/modbus/registers/target_temperature_c/write";            body = @{ value = 60; reason = "rbac" };                                      expect = $true },
    @{ role = "engineer"; token = $engineerToken; method = "POST"; path = "/api/modbus/registers/target_temperature_c/write";            body = @{ value = 60; reason = "rbac" };                                      expect = $false }
)
$report.rbac_failed = 0
foreach ($c in $rbacCases) {
    $status = SendRequest $c.method $c.path $c.token $c.body
    $isAllow = $c.expect
    $isAllowed = ($status -ge 200 -and $status -lt 300)
    $isDenied = ($status -eq 401 -or $status -eq 403)
    $isServerError = ($status -ge 500 -or $status -eq 0)
    $ok = $false
    if ($isAllow) {
        $ok = $isAllowed
    } else {
        $ok = $isDenied
    }
    if ($isServerError) { $ok = $false }
    $case = @{
        role = $c.role
        method = $c.method
        path = $c.path
        expected_allow = $isAllow
        status = $status
        ok = $ok
    }
    $report.cases += $case
    Step ("rbac " + $c.role + " " + $c.method + " " + $c.path) $(if ($ok) { "ok" } else { "fail" }) "status=$status"
    if (-not $ok) { $report.rbac_failed++ }
}
# Build findings dynamically from the actual cases that failed. Reports
# must not carry stale recommendations from prior rewrites.
$findings = New-Object System.Collections.Generic.List[string]
foreach ($failed in ($report.cases | Where-Object { -not $_.ok })) {
    $expected = if ($failed.expected_allow) { "allow" } else { "deny" }
    $note = "role=" + $failed.role + " " + $failed.method + " " + $failed.path + " expected=" + $expected + " got status=" + $failed.status + "."
    if ($failed.status -ge 500) {
        $note += " 5xx is a fail, not a deny; expected deny must surface as 401/403, expected allow must land at 2xx."
    } elseif ($failed.status -eq 0) {
        $note += " network error / no response; the safety gate is unreachable."
    } else {
        $note += " role lacks or carries the wrong permission for this path; tighten Permission in src/api_auth.rs."
    }
    $findings.Add($note)
}
if ($report.concurrent_writes_failed) {
    $findings.Add("concurrent target writes produced " + $server5xx + " 5xx responses out of " + $ConcurrentWriters + "; audit-event SQLx write contention under SQLite WAL busy_timeout. Raise the connection busy_timeout or move audit-event writes to a dedicated mutex.")
}
$report.findings = $findings.ToArray()

$report.rbac_failed_count = ($report.cases | Where-Object { -not $_.ok }).Count
$concurrentFailed = $report.concurrent_writes_failed -eq $true
if ($report.rbac_failed_count -gt 0 -or $concurrentFailed) {
    $report.ok = $false
} else {
    $report.ok = $true
}
$report | ConvertTo-Json -Depth 8 | Set-Content $reportPath
Write-Host "report -> $reportPath"
if (-not $report.ok) { exit 1 }
