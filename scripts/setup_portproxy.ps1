#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Routes external WiFi traffic from RP2040 to the micro-ROS Agent in WSL2/Docker.

.DESCRIPTION
    Docker Desktop on Windows accepts TCP connections from external WiFi clients
    on port 8888 but does NOT forward them to the WSL2 container (known limitation).

    Fix: create a netsh portproxy on port 9888 that routes to 127.0.0.1:8888.
    Connections arriving at localhost:8888 are treated as local by Docker Desktop
    and ARE forwarded to the container — unlike connections from external NICs.

    Routing chain:
        RP2040 (WiFi) → Windows:9888 (portproxy) → 127.0.0.1:8888
                       → Docker Desktop proxy → WSL2 docker-proxy → container

    After running this script, update wifi_config.json:
        "agent_addr": "<Windows_WiFi_IP>:9888"

.NOTES
    Run as Administrator in PowerShell.
    This script does NOT need to be re-run after WSL2 restarts (uses localhost).
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── 1. Verify Docker Desktop is running ──────────────────────────────────────

Write-Host "[1/5] Checking Docker Desktop proxy on localhost:8888..." -ForegroundColor Cyan
$dockerProxy = netstat -an | Select-String "0\.0\.0\.0:8888\s+.*LISTEN"
if ($dockerProxy) {
    Write-Host "      Docker proxy listening on 0.0.0.0:8888 — OK" -ForegroundColor Green
} else {
    Write-Warning "      Nothing listening on 0.0.0.0:8888. Start Docker Desktop and run 'docker compose up -d' first."
}

# ── 2. Remove stale portproxy entries ────────────────────────────────────────

Write-Host "[2/5] Removing stale portproxy entries..." -ForegroundColor Cyan

foreach ($port in @(8888, 9888)) {
    $existing = netsh interface portproxy show v4tov4 |
        Select-String "0\.0\.0\.0\s+$port\b"
    if ($existing) {
        netsh interface portproxy delete v4tov4 listenaddress=0.0.0.0 listenport=$port | Out-Null
        Write-Host "      Removed portproxy on 0.0.0.0:$port" -ForegroundColor Yellow
    } else {
        Write-Host "      No portproxy on 0.0.0.0:$port (skip)" -ForegroundColor DarkGray
    }
}

# ── 3. Add portproxy: Windows 9888 → localhost:8888 ──────────────────────────

Write-Host "[3/5] Adding portproxy 0.0.0.0:9888 -> 127.0.0.1:8888..." -ForegroundColor Cyan
netsh interface portproxy add v4tov4 `
    listenaddress=0.0.0.0 `
    listenport=9888 `
    connectaddress=127.0.0.1 `
    connectport=8888
Write-Host "      Portproxy added." -ForegroundColor Green

# ── 4. Firewall rule ─────────────────────────────────────────────────────────

Write-Host "[4/5] Configuring firewall rule for TCP 9888..." -ForegroundColor Cyan
$ruleName = "micro-ROS Agent 9888"
$existing = Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
if ($existing) {
    Remove-NetFirewallRule -DisplayName $ruleName
    Write-Host "      Removed old rule." -ForegroundColor Yellow
}
New-NetFirewallRule `
    -DisplayName $ruleName `
    -Direction Inbound `
    -Protocol TCP `
    -LocalPort 9888 `
    -Action Allow `
    -Profile Any | Out-Null
Write-Host "      Firewall rule created." -ForegroundColor Green

# ── 5. Verify ────────────────────────────────────────────────────────────────

Write-Host "[5/5] Verification" -ForegroundColor Cyan
Write-Host ""
Write-Host "Active portproxy table:" -ForegroundColor White
netsh interface portproxy show all

Write-Host ""
Write-Host "Port 9888 firewall:" -ForegroundColor White
Get-NetFirewallRule -DisplayName $ruleName | Format-Table DisplayName, Direction, Action, Enabled -AutoSize

# ── Summary ──────────────────────────────────────────────────────────────────

$winIp = (Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object { $_.InterfaceAlias -notmatch 'Loopback|vEthernet' } |
    Sort-Object -Property PrefixLength |
    Select-Object -First 1).IPAddress

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host " Setup complete!" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host ""
Write-Host " Next steps:" -ForegroundColor White
Write-Host "   1. Update wifi_config.json in the project root:" -ForegroundColor White
Write-Host "        `"agent_addr`": `"$winIp`:9888`"" -ForegroundColor Yellow
Write-Host "   2. Rebuild firmware:" -ForegroundColor White
Write-Host "        cargo build --no-default-features --features wifi,sensor \" -ForegroundColor DarkCyan
Write-Host "              --example wifi_microros_sensors --release" -ForegroundColor DarkCyan
Write-Host "   3. Flash and monitor defmt log for:" -ForegroundColor White
Write-Host "        [microros] XRCE-DDS session established" -ForegroundColor DarkCyan
Write-Host "   4. Verify topics:" -ForegroundColor White
Write-Host "        ros2 topic list" -ForegroundColor DarkCyan
Write-Host ""
Write-Host " NOTE: No re-run needed after WSL2 restarts (uses localhost)." -ForegroundColor DarkYellow
Write-Host "       Ensure Docker Desktop is up and container is running first." -ForegroundColor DarkYellow
