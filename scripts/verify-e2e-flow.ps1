# Verify E2E flow: start node_app, run API sequence, print results, stop node_app.
# Run from repo root. Requires: cargo build -p node_app, and ui/dist (npm run build in ui/).

$ErrorActionPreference = "Stop"
$script:RepoRoot = $PSScriptRoot + "\.."
# Resolve meshmind binary (GNU target on Windows, or default)
$script:NodeApp = $null
foreach ($p in @("target\x86_64-pc-windows-gnu\debug\meshmind.exe", "target\debug\meshmind.exe")) {
    $full = Join-Path $RepoRoot $p
    if (Test-Path $full) { $script:NodeApp = $full; break }
}
if (-not $script:NodeApp) { $script:NodeApp = Join-Path $RepoRoot "target\debug\meshmind.exe" }
$script:BaseUrl = "http://127.0.0.1:9900"
$script:Proc = $null

function Add-MinGwToPath {
    $mingwPaths = @(
        "C:\msys64\mingw64\bin",
        "C:\msys64\ucrt64\bin"
    )
    foreach ($p in $mingwPaths) {
        if (Test-Path $p) {
            $env:PATH = $p + ";" + $env:PATH
            Write-Host "Added MinGW to PATH: $p"
            return
        }
    }
    Write-Warning "MinGW not found in common locations; PATH unchanged."
}

function Start-NodeApp {
    if (-not (Test-Path $script:NodeApp)) {
        throw "meshmind not found. Run: cargo build -p node_app --target x86_64-pc-windows-gnu"
    }
    $script:Proc = Start-Process -FilePath $script:NodeApp -WorkingDirectory $script:RepoRoot -PassThru -NoNewWindow
    Write-Host "Started node_app (PID $($script:Proc.Id))"
}

function Stop-NodeApp {
    if ($script:Proc -and -not $script:Proc.HasExited) {
        Stop-Process -Id $script:Proc.Id -Force -ErrorAction SilentlyContinue
        Write-Host "Stopped node_app"
    }
}

function Wait-ForStatus {
    $url = "$script:BaseUrl/v1/status"
    $max = 30
    for ($i = 0; $i -lt $max; $i++) {
        try {
            $r = Invoke-RestMethod -Uri $url -Method Get -ErrorAction Stop
            if ($r) { return $r }
        } catch {
            Start-Sleep -Seconds 1
        }
    }
    throw "Timed out waiting for $url"
}

Add-MinGwToPath
Push-Location $script:RepoRoot
try {
    Start-NodeApp
    try {
        Write-Host "Waiting for server..."
        $status = Wait-ForStatus
        $adminToken = $status.admin_token
        if (-not $adminToken) { throw "admin_token not in status response" }
        Write-Host "Status: node_id=$($status.node_id) backend=$($status.backend)"

        $headers = @{
            "Authorization" = "Bearer $adminToken"
            "Content-Type"  = "application/json"
        }

        Write-Host "`nPOST /v1/admin/scan"
        $scan = Invoke-RestMethod -Uri "$script:BaseUrl/v1/admin/scan" -Method Post -Headers $headers
        Write-Host ($scan | ConvertTo-Json -Compress)

        Write-Host "`nPOST /v1/admin/sources/approve-all"
        $approve = Invoke-RestMethod -Uri "$script:BaseUrl/v1/admin/sources/approve-all" -Method Post -Headers $headers
        Write-Host ($approve | ConvertTo-Json -Compress)

        Write-Host "`nPOST /v1/admin/ingest-all"
        $ingest = Invoke-RestMethod -Uri "$script:BaseUrl/v1/admin/ingest-all" -Method Post -Headers $headers
        Write-Host ($ingest | ConvertTo-Json -Compress)

        Write-Host "`nPOST /v1/ask"
        $body = @{ question = "How many invoices do I have?" } | ConvertTo-Json
        $ask = Invoke-RestMethod -Uri "$script:BaseUrl/v1/ask" -Method Post -Headers $headers -Body $body
        Write-Host ($ask | ConvertTo-Json -Compress)
        Write-Host "`nAnswer: $($ask.answer)"
    } finally {
        Stop-NodeApp
    }
} finally {
    Pop-Location
}
