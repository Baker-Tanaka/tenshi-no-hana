#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Routes external WiFi traffic from RP2040 to the micro-ROS Agent in WSL2/Docker.

.DESCRIPTION
    Two routing strategies are supported:

    Strategy A — Direct (preferred, simpler):
        RP2040 → Windows:<WiFiIP>:8888 → Docker Desktop port mapping → container
        Requires: Windows Firewall rule for TCP 8888 (added by this script).
        Works on Docker Desktop 4.x+ where WSL2 port forwarding is reliable.

    Strategy B — Portproxy (fallback):
        RP2040 → Windows:<WiFiIP>:9888 → portproxy → 127.0.0.1:8888 → container
        Use when Strategy A fails (e.g., Docker Desktop blocks external connections).
        NOTE: netsh portproxy (iphlpsvc) uses kernel sockets and may not reach
        Docker Desktop's user-space docker-proxy on 127.0.0.1:8888. If the
        post-setup diagnostic reports TcpTestSucceeded=False for 127.0.0.1:8888,
        Strategy B will also fail — use Strategy A or the socat relay instead.

    Routing chain (Strategy B):
        RP2040 (WiFi) → Windows:9888 (portproxy) → 127.0.0.1:8888
                       → Docker Desktop proxy → WSL2 docker-proxy → container

    After running this script, update wifi_config.json:
        Strategy A:  "agent_addr": "<Windows_WiFi_IP>:8888"
        Strategy B:  "agent_addr": "<Windows_WiFi_IP>:9888"

.NOTES
    Run as Administrator in PowerShell.
    Re-run after Docker Desktop restarts if the port mapping changes.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── 1. Verify Docker Desktop is running ──────────────────────────────────────

Write-Host "[1/6] Checking Docker Desktop proxy on 0.0.0.0:8888..." -ForegroundColor Cyan
$dockerProxy = netstat -an | Select-String "0\.0\.0\.0:8888\s+.*LISTEN"
if ($dockerProxy) {
    Write-Host "      Docker proxy listening on 0.0.0.0:8888 — OK" -ForegroundColor Green
} else {
    Write-Warning "      Nothing listening on 0.0.0.0:8888. Start Docker Desktop and 'docker compose up -d' first."
}

# ── 2. Remove stale portproxy entries ────────────────────────────────────────

Write-Host "[2/6] Removing stale portproxy entries..." -ForegroundColor Cyan

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

# ── 3. Diagnostic: can portproxy backend reach Docker? ───────────────────────

Write-Host "[3/6] Testing portproxy backend reachability (127.0.0.1:8888)..." -ForegroundColor Cyan
$tcpTest = Test-NetConnection -ComputerName 127.0.0.1 -Port 8888 -InformationLevel Quiet -WarningAction SilentlyContinue
if ($tcpTest) {
    Write-Host "      127.0.0.1:8888 reachable — portproxy Strategy B should work" -ForegroundColor Green
    $strategyBOk = $true
} else {
    Write-Warning "      127.0.0.1:8888 NOT reachable from PowerShell."
    Write-Warning "      netsh portproxy (kernel) also cannot reach this address."
    Write-Warning "      Strategy B (portproxy) will fail. Use Strategy A (direct) instead."
    $strategyBOk = $false
}

# ── 4. Add portproxy: Windows 9888 → localhost:8888 (Strategy B) ─────────────

Write-Host "[4/6] Adding portproxy 0.0.0.0:9888 -> 127.0.0.1:8888..." -ForegroundColor Cyan
netsh interface portproxy add v4tov4 `
    listenaddress=0.0.0.0 `
    listenport=9888 `
    connectaddress=127.0.0.1 `
    connectport=8888
Write-Host "      Portproxy added." -ForegroundColor Green

# ── 5. Firewall rules for both strategies ────────────────────────────────────

Write-Host "[5/6] Configuring firewall rules..." -ForegroundColor Cyan

foreach ($entry in @(
    @{ Port = 8888; Name = "micro-ROS Agent 8888 (direct)" },
    @{ Port = 9888; Name = "micro-ROS Agent 9888 (portproxy)" }
)) {
    $existing = Get-NetFirewallRule -DisplayName $entry.Name -ErrorAction SilentlyContinue
    if ($existing) {
        Remove-NetFirewallRule -DisplayName $entry.Name
        Write-Host "      Removed old rule: $($entry.Name)" -ForegroundColor Yellow
    }
    New-NetFirewallRule `
        -DisplayName $entry.Name `
        -Direction Inbound `
        -Protocol TCP `
        -LocalPort $entry.Port `
        -Action Allow `
        -Profile Any | Out-Null
    Write-Host "      Firewall rule created: TCP $($entry.Port)" -ForegroundColor Green
}

# ── 6. Verify ────────────────────────────────────────────────────────────────

Write-Host "[6/6] Verification" -ForegroundColor Cyan
Write-Host ""
Write-Host "Active portproxy table:" -ForegroundColor White
netsh interface portproxy show all

Write-Host ""
Write-Host "Firewall rules:" -ForegroundColor White
Get-NetFirewallRule -DisplayName "micro-ROS Agent*" |
    Format-Table DisplayName, Direction, Action, Enabled -AutoSize

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

if ($strategyBOk) {
    Write-Host " RECOMMENDED — Strategy B (portproxy, 127.0.0.1 reachable):" -ForegroundColor White
    Write-Host "   wifi_config.json:  `"agent_addr`": `"$winIp`:9888`"" -ForegroundColor Yellow
} else {
    Write-Host " RECOMMENDED — Strategy A (direct, portproxy backend unreachable):" -ForegroundColor White
    Write-Host "   wifi_config.json:  `"agent_addr`": `"$winIp`:8888`"" -ForegroundColor Yellow
}
Write-Host ""
Write-Host " ALSO TEST — Strategy A (direct, always worth trying first):" -ForegroundColor DarkGray
Write-Host "   wifi_config.json:  `"agent_addr`": `"$winIp`:8888`"" -ForegroundColor DarkGray
Write-Host ""
Write-Host " Steps:" -ForegroundColor White
Write-Host "   1. Update wifi_config.json with the agent_addr above" -ForegroundColor White
Write-Host "   2. Rebuild firmware:" -ForegroundColor White
Write-Host "        cargo build --no-default-features --features wifi --example microros_hello" -ForegroundColor DarkCyan
Write-Host "   3. Flash and watch defmt log for:" -ForegroundColor White
Write-Host "        [microros] XRCE-DDS session established" -ForegroundColor DarkCyan
Write-Host "   4. Check agent logs for incoming connection:" -ForegroundColor White
Write-Host "        docker logs -f micro_ros_agent" -ForegroundColor DarkCyan
Write-Host "   5. Verify topics:" -ForegroundColor White
Write-Host "        ros2 topic list" -ForegroundColor DarkCyan
Write-Host ""
Write-Host " NOTE: Re-run this script after Docker Desktop or WSL2 restarts." -ForegroundColor DarkYellow
