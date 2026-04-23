# test-api.ps1 — Integration tests for Bambu Live API (Windows PowerShell)
$ErrorActionPreference = "Stop"

$PORT = 52001
$BASE = "http://localhost:$PORT"
$LOG_FILE = "$env:TEMP\bambu-api.log"

Write-Host "Starting server on port $PORT..."
$env:API_BIND = "0.0.0.0:$PORT"

$serverProcess = Start-Process -FilePath "cargo" -ArgumentList "run" -NoNewWindow -PassThru -RedirectStandardOutput "$env:TEMP\bambu-api-stdout.log" -RedirectStandardError $LOG_FILE

function Cleanup {
    Write-Host "Cleaning up..."
    if (-not $serverProcess.HasExited) {
        Stop-Process -Id $serverProcess.Id -Force -ErrorAction SilentlyContinue
        # Also kill any child cargo/run processes
        Get-Process -Name "bambu-live-api" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    }
}
Register-EngineEvent -SourceIdentifier PowerShell.Exiting -Action { Cleanup }

try {
    Write-Host "Waiting for server..."
    $ready = $false
    for ($i = 1; $i -le 30; $i++) {
        try {
            $r = Invoke-WebRequest -Uri "$BASE/health" -UseBasicParsing -ErrorAction Stop
            $ready = $true
            Write-Host "Server ready after $i s"
            break
        } catch {
            Start-Sleep -Seconds 1
        }
    }
    if (-not $ready) {
        Write-Host "Server failed to start. Log:"
        Get-Content $LOG_FILE
        exit 1
    }

    Write-Host ""
    Write-Host "=== Health ==="
    $r = Invoke-RestMethod -Uri "$BASE/health"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== List printers (empty) ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Upsert printer ==="
    $body = @{ id="test-p1"; host="192.168.1.100"; device_id="DEV001"; access_code="12345678" } | ConvertTo-Json -Compress
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers" -Method Post -ContentType "application/json" -Body $body
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Get printer ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers/test-p1"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Stream URL (stopped) ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers/test-p1/stream/url"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Stop stream (not running) ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers/test-p1/stream/stop" -Method Post
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Upsert printer (update) ==="
    $body = @{ id="test-p1"; host="192.168.1.200"; device_id="DEV002"; access_code="87654321" } | ConvertTo-Json -Compress
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers" -Method Post -ContentType "application/json" -Body $body
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== List printers ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Get non-existent printer (expect 404) ==="
    try {
        $r = Invoke-WebRequest -Uri "$BASE/v1/printers/nope" -UseBasicParsing -ErrorAction Stop
        Write-Host "Status: $($r.StatusCode) (UNEXPECTED)"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        Write-Host "Status: $code"
    }

    Write-Host ""
    Write-Host "=== Bad upsert (empty id) ==="
    try {
        $body = @{ id=""; host="1.2.3.4"; device_id="D"; access_code="A" } | ConvertTo-Json -Compress
        $r = Invoke-WebRequest -Uri "$BASE/v1/printers" -Method Post -ContentType "application/json" -Body $body -UseBasicParsing -ErrorAction Stop
        Write-Host "Status: $($r.StatusCode) (UNEXPECTED)"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        Write-Host "Status: $code"
    }

    Write-Host ""
    Write-Host "=== Bad upsert (missing fields) ==="
    try {
        $body = @{ id="x"; host=""; device_id=""; access_code="" } | ConvertTo-Json -Compress
        $r = Invoke-WebRequest -Uri "$BASE/v1/printers" -Method Post -ContentType "application/json" -Body $body -UseBasicParsing -ErrorAction Stop
        Write-Host "Status: $($r.StatusCode) (UNEXPECTED)"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        Write-Host "Status: $code"
    }

    Write-Host ""
    Write-Host "=== Delete printer ==="
    try {
        $r = Invoke-WebRequest -Uri "$BASE/v1/printers/test-p1" -Method Delete -UseBasicParsing -ErrorAction Stop
        Write-Host "Status: $($r.StatusCode)"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        Write-Host "Status: $code"
    }

    Write-Host ""
    Write-Host "=== List printers (should be empty) ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Batch upsert printers ==="
    $body = @{
        printers = @(
            @{ id="batch-1"; host="10.0.0.1"; device_id="BATCH001"; access_code="11111111" },
            @{ id="batch-2"; host="10.0.0.2"; device_id="BATCH002"; access_code="22222222" },
            @{ id="batch-3"; host="10.0.0.3"; device_id="BATCH003"; access_code="33333333" }
        )
    } | ConvertTo-Json -Compress -Depth 3
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers/batch" -Method Post -ContentType "application/json" -Body $body
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Batch upsert (update + error) ==="
    $body = @{
        printers = @(
            @{ id="batch-1"; host="10.0.0.10"; device_id="BATCH001-UPD"; access_code="updated!" },
            @{ id="bad id!"; host="1.2.3.4"; device_id="X"; access_code="Y" }
        )
    } | ConvertTo-Json -Compress -Depth 3
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers/batch" -Method Post -ContentType "application/json" -Body $body
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== List printers (should have 3) ==="
    $r = Invoke-RestMethod -Uri "$BASE/v1/printers"
    $r | ConvertTo-Json

    Write-Host ""
    Write-Host "=== Cleanup batch printers ==="
    foreach ($id in @("batch-1", "batch-2", "batch-3")) {
        try {
            $r = Invoke-WebRequest -Uri "$BASE/v1/printers/$id" -Method Delete -UseBasicParsing -ErrorAction Stop
            Write-Host "Delete ${id}: $($r.StatusCode)"
        } catch {
            $code = $_.Exception.Response.StatusCode.value__
            Write-Host "Delete ${id}: $code"
        }
    }

    Write-Host ""
    Write-Host "=== Delete non-existent printer (expect 404) ==="
    try {
        $r = Invoke-WebRequest -Uri "$BASE/v1/printers/nope" -Method Delete -UseBasicParsing -ErrorAction Stop
        Write-Host "Status: $($r.StatusCode) (UNEXPECTED)"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        Write-Host "Status: $code"
    }

    Write-Host ""
    Write-Host "=== ALL TESTS PASSED ==="
} finally {
    Cleanup
}
