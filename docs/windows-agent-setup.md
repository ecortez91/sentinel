# Windows Agent Setup Guide

Sentinel can monitor a Windows host from WSL2/Linux via the `sentinel-agent`
binary. The agent runs on Windows as a lightweight HTTP server, exposing system
metrics that the Sentinel TUI polls and displays in a dedicated **Windows Host**
tab.

## Architecture

```
 ┌─────────────────────────┐         HTTP/JSON          ┌──────────────────────┐
 │  WSL2 / Linux           │  ──────────────────────►   │  Windows Host        │
 │  Sentinel TUI           │  GET /api/snapshot         │  sentinel-agent.exe  │
 │  (Windows Host plugin)  │  ◄──────────────────────   │  port 8086           │
 └─────────────────────────┘      System snapshot       └──────────────────────┘
```

## Prerequisites

- **Rust toolchain** on the Windows machine (or cross-compile from WSL2)
- **Windows 10/11** (the agent uses `sysinfo` which supports Windows natively)
- **Network access** from WSL2 to the Windows host (automatic on WSL2 — the
  host IP is the default gateway)

## Step 1: Build the Agent

### Option A: Build on Windows directly

Open PowerShell or CMD in the project directory:

```powershell
cargo build --release --bin sentinel-agent
```

The binary will be at `target\release\sentinel-agent.exe`.

### Option B: Cross-compile from WSL2

Install the Windows target and cross-linker:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64
cargo build --release --bin sentinel-agent --target x86_64-pc-windows-gnu
```

The binary will be at `target/x86_64-pc-windows-gnu/release/sentinel-agent.exe`.
Copy it to a location on the Windows host.

## Step 2: Install the Agent as a Windows Service

Open PowerShell **as Administrator** (right-click → "Run as Administrator"):

```powershell
.\sentinel-agent.exe --install
```

This automatically:
- Creates `C:\Program Files\Sentinel\` and copies the binary there
- Registers `SentinelAgent` as a Windows Service
- Configures **auto-start on boot** and **auto-restart on crash** (5s delay)
- Starts the service immediately

Windows Firewall will prompt you on first run — click **Allow**.

### Verify it works

```powershell
# Check service status
sc query SentinelAgent

# Check the HTTP endpoint
Invoke-RestMethod http://localhost:8086/api/status
```

From WSL2:

```bash
curl http://$(grep nameserver /etc/resolv.conf | awk '{print $2}'):8086/api/status
```

### Updating the Agent

When you rebuild the agent with new features, update the installed service:

```powershell
# Run as Administrator
.\sentinel-agent.exe --upgrade
```

This stops the service, copies the new binary over the old one, and restarts.

### Uninstalling

```powershell
# Run as Administrator
.\sentinel-agent.exe --uninstall
```

This stops the service, removes it from Windows, and deletes the install directory.

### Manual Service Control

```powershell
sc stop SentinelAgent     # Stop the service
sc start SentinelAgent    # Start the service
sc query SentinelAgent    # Check status
services.msc              # Open Services GUI (find "Sentinel Monitor Agent")
```

### Console Mode (for debugging)

To run the agent in the foreground (not as a service):

```powershell
sentinel-agent.exe                      # listen on 0.0.0.0:8086
sentinel-agent.exe --port 9090          # custom port
sentinel-agent.exe --bind 127.0.0.1     # localhost only
```

## Step 3: Configure Sentinel TUI

Edit `~/.config/sentinel/config.toml` on the WSL2/Linux side:

```toml
[windows]
enabled = true
agent_url = "http://localhost:8086/api/snapshot"
poll_interval_secs = 5
```

### URL resolution

Sentinel automatically resolves `localhost` to the Windows host IP on WSL2 by
reading `/etc/resolv.conf`. You do **not** need to hard-code the IP.

If auto-detection fails, override via environment variable:

```bash
export SENTINEL_AGENT_URL="http://172.28.160.1:8086/api/snapshot"
```

Or set it in `~/.config/sentinel/.env`:

```
SENTINEL_AGENT_URL=http://172.28.160.1:8086/api/snapshot
```

## Step 4: Launch Sentinel

```bash
cargo run --release --bin sentinel
```

If the agent is reachable, a **Windows Host** tab will appear showing:

- **Security status** — Firewall per profile (ON/OFF), Defender status, last update age
- CPU usage and core count
- Memory usage (used / total)
- Top processes (sortable by CPU/RAM/PID/Name — press `s`)
- Disk usage per volume
- Network interfaces with RX/TX counters
- Active TCP connections (suspicious connections highlighted)
- Listening ports with process mapping
- Startup programs (registry autorun entries)
- Logged-in users (with RDP session detection)
- OS version and uptime

### Security Alerts

When Windows security issues are detected, they appear in the main **Alerts**
tab (and Telegram notifications if configured):

- Firewall profile OFF (Warning)
- Windows Defender disabled (Danger)
- Defender real-time protection OFF (Warning)
- Windows updates stale >30 days (Warning)

### AI Security Analysis

Press `a` on the Windows Host tab to trigger an AI security assessment. The AI
analyzes the full snapshot (system, connections, firewall, defender, startup
programs, users) and provides actionable recommendations. This uses Haiku and
is **manual only** — never automatic.

## API Reference

### GET /api/snapshot

Returns a full system snapshot:

```json
{
  "hostname": "DESKTOP-ABC123",
  "os_version": "Windows 11 22631",
  "uptime_secs": 86400,
  "cpu_usage_pct": 12.5,
  "cpu_cores": 8,
  "total_memory_bytes": 34359738368,
  "used_memory_bytes": 17179869184,
  "top_processes": [
    {
      "pid": 1234,
      "name": "firefox.exe",
      "cpu_pct": 8.2,
      "memory_bytes": 524288000,
      "status": "Run"
    }
  ],
  "disks": [
    {
      "mount": "C:\\",
      "total_bytes": 512110190592,
      "used_bytes": 256055095296,
      "fs_type": "NTFS"
    }
  ],
  "gpu": null
}
```

### GET /api/status

Returns agent health:

```json
{
  "version": "0.3.0",
  "uptime_secs": 3600,
  "collecting": true
}
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Tab not showing | Check `[windows] enabled = true` in config.toml |
| Connection refused | Verify the agent is running: `curl http://<host-ip>:8086/api/status` |
| Wrong host IP on WSL2 | Set `SENTINEL_AGENT_URL` env var explicitly |
| Firewall blocking | Allow port 8086 inbound in Windows Firewall |
| Agent crashes on start | Check if port 8086 is already in use: `netstat -an | findstr 8086` |
| LHM port conflict | LHM defaults to 8085, agent to 8086 — no conflict by default |

## Keyboard Shortcuts (Windows Host Tab)

| Key | Action |
|-----|--------|
| `j`/`k` or `Up`/`Down` | Navigate process list |
| `s` | Cycle sort field (CPU → RAM → PID → Name) |
| `S` | Toggle sort direction (asc/desc) |
| `f` | Focus/expand current panel |
| `F` | Cycle between panels in focus mode |
| `a` | AI security analysis (Haiku) |
| `PgUp`/`PgDn` | Page through process list |
| `Home`/`End` | Jump to first/last process |

## Security Notes

- The agent serves **read-only** system metrics. It cannot execute commands or
  modify the system.
- The agent runs shell commands (`netstat`, `netsh`, `powershell Get-*`,
  `query user`) to collect security data — all are read-only queries.
- By default it binds to `0.0.0.0` (all interfaces). Use `--bind 127.0.0.1`
  to restrict to localhost if the machine is on a shared network.
- No authentication is required. If you need auth, run behind a reverse proxy.
- CORS headers are permissive (`*`) for local development.
