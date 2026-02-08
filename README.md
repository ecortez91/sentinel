# Sentinel

A beautiful, feature-rich terminal system monitor with AI-powered analysis. Built in Rust with `ratatui`.

Sentinel gives you real-time visibility into CPU, RAM, swap, disk I/O, network, GPU, Docker containers, and running processes -- all from your terminal. An integrated AI assistant (Claude) can analyze your system live, explain processes, flag anomalies, and answer questions about what's happening on your machine.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

### Dashboard
- **System gauges** -- CPU, RAM, swap, load averages with color-coded bars
- **Per-core CPU chart** -- mini bar chart for each logical core
- **Sparkline history** -- rolling CPU and RAM graphs with zoomable time windows (1m / 5m / 15m / 1h)
- **GPU monitoring** -- NVIDIA GPU utilization, VRAM, temperature, power draw, fan speed (via NVML)
- **Network I/O** -- per-interface RX/TX rates and totals
- **Disk usage** -- filesystem gauge bars with read/write throughput
- **Docker containers** -- live container list with CPU%, memory, PIDs, state
- **Battery status** -- charge level and charging state (laptops)
- **AI insight card** -- auto-generated system health summary, refreshed periodically
- **Widget focus mode** -- press `f` to expand any dashboard widget fullscreen

### Process Management
- **Sortable process table** -- by PID, name, CPU%, memory, disk I/O, status
- **Process tree view** -- parent-child hierarchy with tree connectors
- **Process detail popup** -- open file descriptors, environment variables, full command line
- **Process filtering** -- type `/` to search by name
- **Kill processes** -- `k` for SIGTERM, `K` for SIGKILL
- **Signal picker** -- `x` to choose from 12 common Unix signals
- **Renice dialog** -- `n` to adjust process priority with a visual slider
- **Ask AI about a process** -- `a` sends the selected process to Claude for analysis

### Alert System
- **Automatic detection** -- high CPU, high memory, zombies, suspicious processes, memory leaks, security threats
- **Severity levels** -- Info, Warning, Critical, Danger with color coding
- **Deduplication** -- 60-second cooldown per (PID, category) to avoid noise
- **Configurable thresholds** -- via config file or defaults

### AI Integration
- **Live system context** -- Claude sees your real-time process data, CPU, RAM, alerts, and more
- **Streaming responses** -- answers stream in token-by-token
- **Auto-analysis** -- periodic health check on the dashboard (configurable interval)
- **Contextual queries** -- ask about specific processes directly from the process table
- **Multi-turn conversation** -- full chat history in the Ask AI tab
- **Auto-discovers credentials** -- checks `ANTHROPIC_API_KEY`, OpenCode OAuth, and Claude Code credentials

### Theming
- **6 built-in themes** -- Default dark, Gruvbox, Nord, Catppuccin, Dracula, Solarized
- **Custom themes** -- drop a TOML file in `~/.config/sentinel/themes/`
- **Runtime cycling** -- press `T` to switch themes instantly

### Internationalization (i18n)
- **5 languages** -- English, Japanese, Spanish, German, Simplified Chinese
- **Runtime switching** -- press `L` to cycle languages
- **CLI flag** -- `--lang ja` to start in Japanese
- **Config file** -- `lang = "zh"` in config.toml
- **Extensible** -- add new languages by dropping a TOML file in `locales/`

### Prometheus Metrics
- **Optional HTTP endpoint** -- `--prometheus 0.0.0.0:9100`
- **42+ metrics** -- CPU, RAM, swap, load, uptime, network, disk, GPU, battery, alerts, Docker
- **Standard format** -- Prometheus text exposition, compatible with Grafana

### Infrastructure
- **Config file** -- `~/.config/sentinel/config.toml` for thresholds, refresh rate, theme, language
- **CLI flags** -- `--no-ai`, `--theme`, `--refresh-rate`, `--no-auto-analysis`, `--prometheus`, `--lang`
- **Mouse support** -- scroll wheel, click tabs/rows, right-click for detail popup

## Installation

### From source

```bash
git clone https://github.com/ecortez91/sentinel.git
cd sentinel
cargo build --release
```

The binary is at `target/release/sentinel` (~4.5 MiB with LTO+strip).

### Requirements

- **Rust 1.70+** (2021 edition)
- **Linux** (reads `/proc`, `/sys`; WSL2 supported)
- **NVIDIA GPU monitoring** requires `libnvidia-ml.so` (comes with the NVIDIA driver)
- **Docker monitoring** requires the Docker daemon running with a Unix socket

## Usage

```bash
# Basic
sentinel

# With options
sentinel --theme gruvbox --lang ja --refresh-rate 500

# Disable AI features
sentinel --no-ai

# Enable Prometheus metrics
sentinel --prometheus 0.0.0.0:9100

# See all options
sentinel --help
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs |
| `1` `2` `3` `4` | Jump to tab (4 = Ask AI) |
| `j` / `k` / `Up` / `Down` | Scroll |
| `s` | Cycle sort column |
| `r` | Reverse sort direction |
| `/` | Filter processes |
| `Enter` | Process detail popup |
| `t` | Toggle tree view |
| `k` | SIGTERM selected process |
| `K` | SIGKILL selected process |
| `x` | Signal picker |
| `n` | Renice dialog |
| `a` | Ask AI about selected process |
| `T` | Cycle color theme |
| `L` | Cycle UI language |
| `f` | Focus/expand dashboard widget |
| `+` / `-` | Zoom history charts |
| `e` | Expand/collapse AI insight |
| `?` | Help overlay |
| `q` | Quit |

## Configuration

Create `~/.config/sentinel/config.toml`:

```toml
# Refresh interval in milliseconds
refresh_interval_ms = 1000

# Alert thresholds
cpu_warning_threshold = 50.0
cpu_critical_threshold = 90.0
mem_warning_threshold_mib = 1024
mem_critical_threshold_mib = 2048
sys_mem_warning_percent = 75.0
sys_mem_critical_percent = 90.0

# Auto-analysis interval (seconds, 0 = disabled)
auto_analysis_interval_secs = 300

# Theme: default, gruvbox, nord, catppuccin, dracula, solarized
theme = "catppuccin"

# Language: en, ja, es, de, zh
lang = "en"
```

### Custom Themes

Drop a TOML file in `~/.config/sentinel/themes/`:

```toml
# ~/.config/sentinel/themes/my-theme.toml
accent = "#FF8800"
bg_dark = "#1a1a2e"
text_primary = "#e0e0e0"
success = "#00ff88"
warning = "#ffaa00"
danger = "#ff4444"
```

Then set `theme = "my-theme"` in your config.

## AI Setup

Sentinel auto-discovers API credentials in this order:

1. `ANTHROPIC_API_KEY` environment variable
2. OpenCode OAuth token (`~/.local/share/opencode/auth.json`)
3. Claude Code credentials (`~/.claude/.credentials.json`)

No configuration needed if you already use OpenCode or Claude Code.

## Architecture

```
src/
  main.rs          -- Event loop, CLI args, keybinds, channel orchestration
  ai/
    client.rs      -- Claude API client with OAuth, streaming, token refresh
    context.rs     -- Builds system context string for AI prompts
    mod.rs
  alerts/
    mod.rs         -- Alert detection engine with deduplication
  config/
    mod.rs         -- Config file loading and validation
  metrics/
    mod.rs         -- Prometheus metrics HTTP server
  models/
    system.rs      -- SystemSnapshot, GpuInfo, NetworkInfo, DiskInfo, etc.
    process.rs     -- ProcessInfo, format_bytes
    alert.rs       -- Alert, AlertSeverity, AlertCategory
    mod.rs
  monitor/
    collector.rs   -- System data collection (sysinfo, NVML, /proc, /sys)
    docker.rs      -- Docker container monitoring (bollard)
    mod.rs
  ui/
    renderer.rs    -- All TUI rendering (~2700 lines)
    state.rs       -- AppState, Tab, SortColumn, popups, history buffers
    theme.rs       -- Theme system with 6 built-ins + custom TOML themes
    widgets.rs     -- GradientGauge, CpuMiniChart custom widgets
    mod.rs
locales/
  en.toml          -- English (base)
  ja.toml          -- Japanese
  es.toml          -- Spanish
  de.toml          -- German
  zh.toml          -- Simplified Chinese
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend |
| `sysinfo` | System/process information |
| `tokio` | Async runtime |
| `reqwest` | HTTP client for Claude API |
| `nvml-wrapper` | NVIDIA GPU monitoring |
| `bollard` | Docker API client |
| `clap` | CLI argument parsing |
| `rust-i18n` | Internationalization |
| `tiny_http` | Prometheus metrics server |
| `serde` / `toml` | Config file parsing |
| `libc` | POSIX signals and process control |

## License

MIT
