# SENTINEL — Terminal System Monitor

## Overview
SENTINEL is a Rust-based TUI (Terminal User Interface) system monitor with AI-powered analysis, Telegram notifications, Docker monitoring, and cross-platform support (Windows + Linux/WSL).

## Architecture
- **`src/main.rs`** — TUI entry point (ratatui + crossterm)
- **`src/agent/main.rs`** — HTTP agent binary (`sentinel-agent`) that collects system snapshots via `sysinfo` and serves JSON over HTTP
- **`src/app.rs`** — Core application state and event loop
- **`src/ui/`** — UI widgets, theme, renderers (dashboard, processes, alerts, etc.)
- **`src/plugins/`** — Plugin system (market, windows, settings)
- **`src/ai/`** — Claude API integration for AI analysis
- **`src/monitor/`** — System data collection (processes, Docker)
- **`src/notifications/`** — Telegram alert delivery
- **`src/alerts/`** — Alert detection and grouping logic
- **`src/models/`** — Shared data models (ProcessInfo, SystemSnapshot, etc.)

## Build & Run
```bash
cargo build                    # Build both binaries
cargo run --bin sentinel       # Run TUI
cargo run --bin sentinel-agent # Run agent (HTTP server on :8086)
cargo check                    # Type-check without building
cargo test                     # Run tests
```

## Coding Conventions
- Use `#[cfg(windows)]` / `#[cfg(not(windows))]` / `#[cfg(target_os = "linux")]` for platform-specific code
- Windows-specific FFI lives in inline `mod` blocks with `#[cfg(windows)]`
- Process memory: Windows uses Private Working Set (Win32 API), Linux uses sysinfo RSS
- Keep agent binary self-contained — minimal dependencies beyond `sysinfo` + `tiny_http`
- Sort processes by memory descending by default
- Filter processes below `MIN_PROCESS_MEMORY` (1 MB) threshold

## Key Dependencies
- `sysinfo 0.32` — Cross-platform system info
- `ratatui 0.29` — TUI framework
- `tiny_http` — Lightweight HTTP server (agent)
- `reqwest` — HTTP client (AI, notifications)
- `bollard` — Docker API client
