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

## Step 2: Run the Agent on Windows

```powershell
# Default: listen on all interfaces, port 8086
sentinel-agent.exe

# Custom port
sentinel-agent.exe --port 9090

# Localhost only (no external access)
sentinel-agent.exe --bind 127.0.0.1
```

On startup, the agent prints:

```
sentinel-agent v0.3.0 listening on 0.0.0.0:8086
Endpoints:
  GET /api/snapshot  - system snapshot
  GET /api/status    - agent health
```

### Verify it works

From PowerShell:

```powershell
Invoke-RestMethod http://localhost:8086/api/status
```

From WSL2:

```bash
curl http://$(grep nameserver /etc/resolv.conf | awk '{print $2}'):8086/api/status
```

You should see JSON like:

```json
{"version":"0.3.0","uptime_secs":42,"collecting":true}
```

### Run as a background service (optional)

To keep the agent running after logout, you can use Task Scheduler or NSSM:

```powershell
# Using NSSM (Non-Sucking Service Manager)
nssm install SentinelAgent "C:\path\to\sentinel-agent.exe"
nssm start SentinelAgent
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

- CPU usage and core count
- Memory usage (used / total)
- Top processes (by CPU)
- Disk usage per volume
- OS version and uptime

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

## Security Notes

- The agent serves **read-only** system metrics. It cannot execute commands or
  modify the system.
- By default it binds to `0.0.0.0` (all interfaces). Use `--bind 127.0.0.1`
  to restrict to localhost if the machine is on a shared network.
- No authentication is required. If you need auth, run behind a reverse proxy.
- CORS headers are permissive (`*`) for local development.
