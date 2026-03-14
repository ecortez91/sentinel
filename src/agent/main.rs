//! Sentinel Agent — cross-platform system monitor HTTP server (#3).
//!
//! Collects system data via `sysinfo` and serves JSON snapshots over HTTP.
//! Designed to run on Windows (or any OS) alongside a WSL2/Linux Sentinel TUI.
//!
//! Endpoints:
//!   GET /api/snapshot  — full system snapshot (WindowsHostSnapshot JSON)
//!   GET /api/status    — agent health check (AgentStatus JSON)
//!
//! Usage:
//!   sentinel-agent                    # listen on 0.0.0.0:8085
//!   sentinel-agent --port 9090        # custom port
//!   sentinel-agent --bind 127.0.0.1   # localhost only

use std::sync::{Arc, Mutex};
use std::time::Instant;

use sysinfo::System;
use tiny_http::{Header, Method, Response, Server};

/// Agent version (matches the sentinel crate version).
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default listen address.
const DEFAULT_BIND: &str = "0.0.0.0";

/// Default listen port.
const DEFAULT_PORT: u16 = 8086;

/// Maximum top processes to include in snapshot.
const MAX_TOP_PROCESSES: usize = 30;

/// JSON content type header value.
const CONTENT_TYPE_JSON: &str = "application/json";

/// CORS header value — allow any origin for local development.
const CORS_ALLOW_ORIGIN: &str = "*";

// ── Data models ──────────────────────────────────────────────────
// Re-use the same structs the TUI plugin expects. These are duplicated
// here (rather than shared via a library crate) to keep the agent as a
// single self-contained binary with zero workspace dependencies.

use serde::Serialize;

/// Complete snapshot of host system state.
#[derive(Serialize)]
struct Snapshot {
    hostname: String,
    os_version: String,
    uptime_secs: u64,
    cpu_usage_pct: f32,
    cpu_cores: u32,
    total_memory_bytes: u64,
    used_memory_bytes: u64,
    top_processes: Vec<ProcessInfo>,
    disks: Vec<DiskInfo>,
    gpu: Option<GpuInfo>,
}

#[derive(Serialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_pct: f32,
    memory_bytes: u64,
    status: String,
}

#[derive(Serialize)]
struct DiskInfo {
    mount: String,
    total_bytes: u64,
    used_bytes: u64,
    fs_type: String,
}

#[derive(Serialize)]
struct GpuInfo {
    name: String,
    usage_pct: f32,
    temp_celsius: f32,
    vram_total_bytes: u64,
    vram_used_bytes: u64,
}

/// Agent health response.
#[derive(Serialize)]
struct AgentStatus {
    version: String,
    uptime_secs: u64,
    collecting: bool,
}

// ── System collection ────────────────────────────────────────────

fn collect_snapshot(sys: &mut System) -> Snapshot {
    // Refresh CPU, memory, and process data
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let cpu_usage = sys.global_cpu_usage();
    let cpu_cores = sys.cpus().len() as u32;

    // Collect top processes by CPU usage
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .filter(|p| p.name().to_string_lossy().len() > 0)
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_pct: p.cpu_usage(),
            memory_bytes: p.memory(),
            status: format!("{:?}", p.status()),
        })
        .collect();

    // Sort by CPU usage descending, then truncate
    procs.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    procs.truncate(MAX_TOP_PROCESSES);

    // Collect disk info
    let disks_info = sysinfo::Disks::new_with_refreshed_list();
    let disks: Vec<DiskInfo> = disks_info
        .iter()
        .map(|d| DiskInfo {
            mount: d.mount_point().to_string_lossy().to_string(),
            total_bytes: d.total_space(),
            used_bytes: d.total_space().saturating_sub(d.available_space()),
            fs_type: d.file_system().to_string_lossy().to_string(),
        })
        .collect();

    // GPU detection (best-effort via NVML, returns None if unavailable)
    let gpu = try_collect_gpu();

    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
    let os_version = format!(
        "{} {}",
        System::name().unwrap_or_else(|| "Unknown OS".into()),
        System::os_version().unwrap_or_default(),
    );
    let uptime_secs = System::uptime();

    Snapshot {
        hostname,
        os_version,
        uptime_secs,
        cpu_usage_pct: cpu_usage,
        cpu_cores,
        total_memory_bytes: sys.total_memory(),
        used_memory_bytes: sys.used_memory(),
        top_processes: procs,
        disks,
        gpu,
    }
}

/// Try to read NVIDIA GPU data via NVML. Returns None on failure.
///
/// GPU collection is a future enhancement — for now we return None.
/// When implemented, this would use the nvml-wrapper crate.
fn try_collect_gpu() -> Option<GpuInfo> {
    None
}

// ── HTTP server ──────────────────────────────────────────────────

fn json_header() -> Header {
    Header::from_bytes("Content-Type", CONTENT_TYPE_JSON).unwrap()
}

fn cors_header() -> Header {
    Header::from_bytes("Access-Control-Allow-Origin", CORS_ALLOW_ORIGIN).unwrap()
}

fn main() {
    let bind = std::env::args()
        .position(|a| a == "--bind")
        .and_then(|i| std::env::args().nth(i + 1))
        .unwrap_or_else(|| DEFAULT_BIND.to_string());

    let port: u16 = std::env::args()
        .position(|a| a == "--port")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let addr = format!("{}:{}", bind, port);

    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to start HTTP server on {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    eprintln!("sentinel-agent v{} listening on {}", AGENT_VERSION, addr);
    eprintln!("Endpoints:");
    eprintln!("  GET /api/snapshot  — system snapshot");
    eprintln!("  GET /api/status    — agent health");

    let start_time = Instant::now();
    let sys = Arc::new(Mutex::new(System::new_all()));

    // Pre-refresh to get initial CPU readings (first read is always 0%)
    {
        let mut s = sys.lock().unwrap();
        s.refresh_cpu_all();
    }
    std::thread::sleep(std::time::Duration::from_millis(500));

    for request in server.incoming_requests() {
        let path = request.url().to_string();
        let method = request.method().clone();

        // Only handle GET requests
        if method != Method::Get {
            let resp = Response::from_string("Method Not Allowed")
                .with_status_code(405)
                .with_header(cors_header());
            let _ = request.respond(resp);
            continue;
        }

        match path.as_str() {
            "/api/snapshot" => {
                let mut s = sys.lock().unwrap();
                let snapshot = collect_snapshot(&mut s);
                drop(s);

                match serde_json::to_string(&snapshot) {
                    Ok(json) => {
                        let resp = Response::from_string(json)
                            .with_header(json_header())
                            .with_header(cors_header());
                        let _ = request.respond(resp);
                    }
                    Err(e) => {
                        let resp = Response::from_string(format!("{{\"error\":\"{}\"}}", e))
                            .with_status_code(500)
                            .with_header(json_header())
                            .with_header(cors_header());
                        let _ = request.respond(resp);
                    }
                }
            }
            "/api/status" => {
                let status = AgentStatus {
                    version: AGENT_VERSION.to_string(),
                    uptime_secs: start_time.elapsed().as_secs(),
                    collecting: true,
                };
                match serde_json::to_string(&status) {
                    Ok(json) => {
                        let resp = Response::from_string(json)
                            .with_header(json_header())
                            .with_header(cors_header());
                        let _ = request.respond(resp);
                    }
                    Err(e) => {
                        let resp = Response::from_string(format!("{{\"error\":\"{}\"}}", e))
                            .with_status_code(500)
                            .with_header(json_header())
                            .with_header(cors_header());
                        let _ = request.respond(resp);
                    }
                }
            }
            _ => {
                let resp = Response::from_string("Not Found")
                    .with_status_code(404)
                    .with_header(cors_header());
                let _ = request.respond(resp);
            }
        }
    }
}
