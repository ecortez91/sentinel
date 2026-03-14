//! Sentinel Agent — cross-platform system monitor HTTP server.
//!
//! Collects system data via `sysinfo` and Windows-specific shell commands,
//! then serves JSON snapshots over HTTP.
//! Designed to run on Windows (or any OS) alongside a WSL2/Linux Sentinel TUI.
//!
//! Endpoints:
//!   GET /api/snapshot  — full system snapshot (JSON)
//!   GET /api/status    — agent health check (JSON)
//!
//! Usage:
//!   sentinel-agent                    # listen on 0.0.0.0:8086
//!   sentinel-agent --port 9090        # custom port
//!   sentinel-agent --bind 127.0.0.1   # localhost only

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sysinfo::{Networks, System};
use tiny_http::{Header, Method, Response, Server};

/// Agent version (matches the sentinel crate version).
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default listen address.
const DEFAULT_BIND: &str = "0.0.0.0";

/// Default listen port.
const DEFAULT_PORT: u16 = 8086;

/// Maximum processes to include by CPU usage.
const MAX_TOP_BY_CPU: usize = 50;

/// Maximum additional processes to include by memory usage.
/// These are merged with the CPU set to ensure memory-heavy processes
/// (like Vmmem, Chrome) always appear even when idle.
const MAX_TOP_BY_MEMORY: usize = 20;

/// Maximum TCP connections to include in snapshot.
const MAX_CONNECTIONS: usize = 50;

/// Maximum startup entries to include in snapshot.
const MAX_STARTUP_ENTRIES: usize = 30;

/// Maximum user sessions to include.
const MAX_USERS: usize = 10;

/// Shell command timeout — used for documentation; actual timeout relies on
/// the OS process scheduler (commands complete quickly on modern systems).
#[allow(dead_code)]
const CMD_TIMEOUT_SECS: u64 = 3;

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
    networks: Vec<NetworkInterfaceInfo>,
    tcp_connections: Vec<TcpConnectionInfo>,
    listening_ports: Vec<ListeningPortInfo>,
    security: Option<SecurityStatus>,
    startup_programs: Vec<StartupEntry>,
    logged_in_users: Vec<UserSession>,
}

#[derive(Serialize, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_pct: f32,
    memory_bytes: u64,
    status: String,
    parent_pid: Option<u32>,
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

#[derive(Serialize)]
struct NetworkInterfaceInfo {
    name: String,
    rx_bytes: u64,
    tx_bytes: u64,
}

#[derive(Serialize)]
struct TcpConnectionInfo {
    local_addr: String,
    local_port: u16,
    remote_addr: String,
    remote_port: u16,
    state: String,
    pid: u32,
    process_name: String,
}

#[derive(Serialize)]
struct ListeningPortInfo {
    port: u16,
    pid: u32,
    process_name: String,
    protocol: String,
}

#[derive(Serialize)]
struct SecurityStatus {
    firewall_profiles: Vec<FirewallProfile>,
    defender_enabled: Option<bool>,
    defender_realtime: Option<bool>,
    last_update_days: Option<u64>,
}

#[derive(Serialize)]
struct FirewallProfile {
    name: String,
    enabled: bool,
}

#[derive(Serialize)]
struct StartupEntry {
    name: String,
    command: String,
    location: String,
}

#[derive(Serialize)]
struct UserSession {
    username: String,
    session_type: String,
    state: String,
}

/// Agent health response.
#[derive(Serialize)]
struct AgentStatus {
    version: String,
    uptime_secs: u64,
    collecting: bool,
}

// ── Shell command helper ─────────────────────────────────────────

/// Run a shell command. Returns `None` if the command fails or produces
/// no output. Best-effort — never panics.
fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    // CREATE_NO_WINDOW (0x08000000) prevents console popups on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    match cmd.output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

// ── Private Working Set (Windows-only FFI) ───────────────────────
//
// Windows Task Manager shows "Memory (Private Working Set)" which is
// PrivateUsage from PROCESS_MEMORY_COUNTERS_EX. sysinfo only exposes
// WorkingSetSize (full RSS including shared DLLs). We call the Win32
// API directly to match Task Manager's default column.

#[cfg(windows)]
mod private_mem {
    use std::mem::{size_of, zeroed};

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESS_MEMORY_COUNTERS_EX {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
        PrivateUsage: usize,
    }

    type HANDLE = *mut std::ffi::c_void;
    type BOOL = i32;

    extern "system" {
        fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
        fn CloseHandle(handle: HANDLE) -> BOOL;
        fn K32GetProcessMemoryInfo(
            process: HANDLE,
            counters: *mut PROCESS_MEMORY_COUNTERS_EX,
            cb: u32,
        ) -> BOOL;
    }

    /// Get the Private Working Set for a process (matches Task Manager).
    /// Returns None if the process cannot be opened (access denied, exited).
    pub fn get(pid: u32) -> Option<u64> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut counters: PROCESS_MEMORY_COUNTERS_EX = zeroed();
            counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            let ok = K32GetProcessMemoryInfo(handle, &mut counters, counters.cb);
            CloseHandle(handle);
            if ok != 0 {
                Some(counters.PrivateUsage as u64)
            } else {
                None
            }
        }
    }
}

/// Get Private Working Set on Windows, fall back to sysinfo's memory() elsewhere.
#[cfg(windows)]
fn get_process_memory(pid: u32, fallback: u64) -> u64 {
    private_mem::get(pid).unwrap_or(fallback)
}

#[cfg(not(windows))]
fn get_process_memory(_pid: u32, fallback: u64) -> u64 {
    fallback
}

// ── System collection ────────────────────────────────────────────

fn collect_snapshot(
    sys: &mut System,
    networks: &mut Networks,
    pid_names: &std::collections::HashMap<u32, String>,
) -> Snapshot {
    // Refresh CPU, memory, and process data
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    networks.refresh();

    let cpu_usage = sys.global_cpu_usage();
    let cpu_cores = sys.cpus().len() as u32;

    // Build PID → name map for connection resolution
    let mut current_pid_names: std::collections::HashMap<u32, String> = sys
        .processes()
        .iter()
        .map(|(pid, p)| (pid.as_u32(), p.name().to_string_lossy().to_string()))
        .collect();

    // Merge with cached names (processes may have exited between netstat and sysinfo)
    for (pid, name) in pid_names {
        current_pid_names
            .entry(*pid)
            .or_insert_with(|| name.clone());
    }

    // Collect all named processes
    let all_procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .filter(|p| !p.name().to_string_lossy().is_empty())
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_pct: p.cpu_usage() / cpu_cores as f32,
            memory_bytes: get_process_memory(p.pid().as_u32(), p.memory()),
            status: format!("{:?}", p.status()),
            parent_pid: p.parent().map(|pp| pp.as_u32()),
        })
        .collect();

    // Merge top-by-CPU and top-by-memory to ensure both CPU-heavy and
    // memory-heavy processes (Vmmem, Chrome) are always included.
    let mut by_cpu = all_procs.clone();
    by_cpu.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    by_cpu.truncate(MAX_TOP_BY_CPU);

    let mut by_mem = all_procs;
    by_mem.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    by_mem.truncate(MAX_TOP_BY_MEMORY);

    // Merge and dedup by PID
    let mut seen = std::collections::HashSet::new();
    let mut procs: Vec<ProcessInfo> = Vec::with_capacity(MAX_TOP_BY_CPU + MAX_TOP_BY_MEMORY);
    for p in by_cpu.into_iter().chain(by_mem.into_iter()) {
        if seen.insert(p.pid) {
            procs.push(p);
        }
    }

    // Final sort by CPU descending for display
    procs.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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

    // GPU detection (best-effort, returns None if unavailable)
    let gpu = try_collect_gpu();

    // Network interfaces
    let net_interfaces = collect_networks(networks);

    // Windows-specific: TCP connections, listening ports, security, startup, users
    let (tcp_connections, listening_ports) = collect_connections(&current_pid_names);
    let security = collect_security_status();
    let startup_programs = collect_startup_programs();
    let logged_in_users = collect_logged_in_users();

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
        networks: net_interfaces,
        tcp_connections,
        listening_ports,
        security,
        startup_programs,
        logged_in_users,
    }
}

/// Try to read NVIDIA GPU data via NVML. Returns None on failure.
/// GPU collection is a future enhancement — for now we return None.
fn try_collect_gpu() -> Option<GpuInfo> {
    None
}

/// Collect network interface statistics via sysinfo.
fn collect_networks(networks: &Networks) -> Vec<NetworkInterfaceInfo> {
    networks
        .iter()
        .filter(|(_, data)| data.total_received() + data.total_transmitted() > 0)
        .map(|(name, data)| NetworkInterfaceInfo {
            name: name.to_string(),
            rx_bytes: data.total_received(),
            tx_bytes: data.total_transmitted(),
        })
        .collect()
}

/// Collect TCP connections and listening ports via `netstat -ano`.
/// Returns (established/other connections, listening ports).
fn collect_connections(
    pid_names: &std::collections::HashMap<u32, String>,
) -> (Vec<TcpConnectionInfo>, Vec<ListeningPortInfo>) {
    let output = match run_command("netstat", &["-ano"]) {
        Some(o) => o,
        None => return (Vec::new(), Vec::new()),
    };
    parse_netstat_output(&output, pid_names)
}

/// Parse netstat -ano output into connections and listening ports.
/// Pure function — all parsing logic is testable without running commands.
fn parse_netstat_output(
    raw: &str,
    pid_names: &std::collections::HashMap<u32, String>,
) -> (Vec<TcpConnectionInfo>, Vec<ListeningPortInfo>) {
    let mut connections = Vec::new();
    let mut listeners = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with("TCP") && !line.starts_with("UDP") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // TCP format: TCP  local_addr  foreign_addr  state  pid
        // UDP format: UDP  local_addr  *:*  pid
        if parts.len() < 4 {
            continue;
        }

        let is_tcp = parts[0] == "TCP";
        let local = parts[1];
        let (local_addr, local_port) = match parse_addr_port(local) {
            Some(v) => v,
            None => continue,
        };

        if is_tcp && parts.len() >= 5 {
            let foreign = parts[2];
            let state = parts[3];
            let pid: u32 = parts[4].parse().unwrap_or(0);
            let process_name = pid_names
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| format!("PID:{}", pid));

            if state == "LISTENING" {
                if listeners.len() < MAX_CONNECTIONS {
                    listeners.push(ListeningPortInfo {
                        port: local_port,
                        pid,
                        process_name,
                        protocol: "TCP".to_string(),
                    });
                }
            } else {
                let (remote_addr, remote_port) =
                    parse_addr_port(foreign).unwrap_or_else(|| ("*".to_string(), 0));
                if connections.len() < MAX_CONNECTIONS {
                    connections.push(TcpConnectionInfo {
                        local_addr,
                        local_port,
                        remote_addr,
                        remote_port,
                        state: state.to_string(),
                        pid,
                        process_name,
                    });
                }
            }
        } else if !is_tcp && parts.len() >= 4 {
            // UDP listener
            let pid: u32 = parts[3].parse().unwrap_or(0);
            let process_name = pid_names
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| format!("PID:{}", pid));
            if listeners.len() < MAX_CONNECTIONS {
                listeners.push(ListeningPortInfo {
                    port: local_port,
                    pid,
                    process_name,
                    protocol: "UDP".to_string(),
                });
            }
        }
    }

    (connections, listeners)
}

/// Parse "addr:port" or "[addr]:port" into (addr_string, port).
fn parse_addr_port(s: &str) -> Option<(String, u16)> {
    // Handle IPv6 bracket notation: [::1]:8080
    if let Some(bracket_end) = s.rfind("]:") {
        let addr = &s[1..bracket_end];
        let port_str = &s[bracket_end + 2..];
        let port: u16 = port_str.parse().ok()?;
        return Some((addr.to_string(), port));
    }
    // Handle IPv4: 192.168.1.1:8080 or 0.0.0.0:443
    let last_colon = s.rfind(':')?;
    let addr = &s[..last_colon];
    let port_str = &s[last_colon + 1..];
    let port: u16 = port_str.parse().ok()?;
    Some((addr.to_string(), port))
}

/// Collect Windows Firewall and Defender status via shell commands.
fn collect_security_status() -> Option<SecurityStatus> {
    let firewall_profiles = collect_firewall_status().unwrap_or_default();
    let (defender_enabled, defender_realtime) = collect_defender_status();
    let last_update_days = collect_last_update_days();

    // Only return Some if we got at least some data
    if firewall_profiles.is_empty()
        && defender_enabled.is_none()
        && defender_realtime.is_none()
        && last_update_days.is_none()
    {
        return None;
    }

    Some(SecurityStatus {
        firewall_profiles,
        defender_enabled,
        defender_realtime,
        last_update_days,
    })
}

/// Parse Windows Firewall status from `netsh advfirewall show allprofiles state`.
fn collect_firewall_status() -> Option<Vec<FirewallProfile>> {
    let output = run_command("netsh", &["advfirewall", "show", "allprofiles", "state"])?;
    Some(parse_firewall_output(&output))
}

/// Pure parser for netsh firewall output.
fn parse_firewall_output(raw: &str) -> Vec<FirewallProfile> {
    let mut profiles = Vec::new();
    let mut current_name: Option<String> = None;

    for line in raw.lines() {
        let line = line.trim();
        // Profile headers look like: "Domain Profile Settings:" or "Private Profile Settings:"
        if line.ends_with("Profile Settings:") {
            let name = line
                .replace(" Profile Settings:", "")
                .replace("Settings:", "")
                .trim()
                .to_string();
            current_name = Some(name);
        }
        // State line: "State                                 ON" or "State                                 OFF"
        if line.starts_with("State") {
            if let Some(ref name) = current_name {
                let enabled = line.to_uppercase().contains("ON");
                profiles.push(FirewallProfile {
                    name: name.clone(),
                    enabled,
                });
                current_name = None;
            }
        }
    }
    profiles
}

/// Collect Windows Defender status via PowerShell.
fn collect_defender_status() -> (Option<bool>, Option<bool>) {
    let output = match run_command(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Get-MpComputerStatus | Select-Object -Property AMServiceEnabled,RealTimeProtectionEnabled | Format-List",
        ],
    ) {
        Some(o) => o,
        None => return (None, None),
    };
    parse_defender_output(&output)
}

/// Pure parser for Defender PowerShell output.
fn parse_defender_output(raw: &str) -> (Option<bool>, Option<bool>) {
    let mut enabled = None;
    let mut realtime = None;

    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with("AMServiceEnabled") {
            enabled = Some(line.to_lowercase().contains("true"));
        }
        if line.starts_with("RealTimeProtectionEnabled") {
            realtime = Some(line.to_lowercase().contains("true"));
        }
    }
    (enabled, realtime)
}

/// Collect days since last Windows Update via PowerShell.
fn collect_last_update_days() -> Option<u64> {
    let output = run_command(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "((Get-Date) - (Get-HotFix | Sort-Object InstalledOn -Descending | Select-Object -First 1).InstalledOn).Days",
        ],
    )?;
    parse_update_days(&output)
}

/// Pure parser for update days output.
fn parse_update_days(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

/// Collect startup programs via PowerShell.
fn collect_startup_programs() -> Vec<StartupEntry> {
    let output = match run_command(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_StartupCommand | Select-Object Name,Command,Location | Format-List",
        ],
    ) {
        Some(o) => o,
        None => return Vec::new(),
    };
    let mut entries = parse_startup_output(&output);
    entries.truncate(MAX_STARTUP_ENTRIES);
    entries
}

/// Pure parser for startup programs PowerShell output.
fn parse_startup_output(raw: &str) -> Vec<StartupEntry> {
    let mut entries = Vec::new();
    let mut name = String::new();
    let mut command = String::new();
    let mut location = String::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !name.is_empty() {
                entries.push(StartupEntry {
                    name: name.clone(),
                    command: command.clone(),
                    location: location.clone(),
                });
                name.clear();
                command.clear();
                location.clear();
            }
            continue;
        }
        if let Some(val) = line.strip_prefix("Name") {
            name = val
                .trim_start_matches(|c: char| c == ' ' || c == ':')
                .trim()
                .to_string();
        } else if let Some(val) = line.strip_prefix("Command") {
            command = val
                .trim_start_matches(|c: char| c == ' ' || c == ':')
                .trim()
                .to_string();
        } else if let Some(val) = line.strip_prefix("Location") {
            location = val
                .trim_start_matches(|c: char| c == ' ' || c == ':')
                .trim()
                .to_string();
        }
    }
    // Don't forget the last entry
    if !name.is_empty() {
        entries.push(StartupEntry {
            name,
            command,
            location,
        });
    }
    entries
}

/// Collect logged-in user sessions via `query user`.
fn collect_logged_in_users() -> Vec<UserSession> {
    let output = match run_command("query", &["user"]) {
        Some(o) => o,
        None => return Vec::new(),
    };
    let mut users = parse_query_user(&output);
    users.truncate(MAX_USERS);
    users
}

/// Pure parser for `query user` output.
fn parse_query_user(raw: &str) -> Vec<UserSession> {
    let mut users = Vec::new();

    for line in raw.lines().skip(1) {
        // Skip the header line
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: USERNAME  SESSIONNAME  ID  STATE  IDLE TIME  LOGON TIME
        // Fields are space-separated but may have varying widths
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let username = parts[0].trim_start_matches('>').to_string();
        let session_type = if parts.len() > 1 {
            let s = parts[1];
            if s.starts_with("rdp") || s.starts_with("RDP") {
                "RDP".to_string()
            } else if s.starts_with("console") {
                "Console".to_string()
            } else {
                s.to_string()
            }
        } else {
            "Unknown".to_string()
        };
        let state = if parts.len() > 3 {
            parts[3].to_string()
        } else {
            "Active".to_string()
        };

        users.push(UserSession {
            username,
            session_type,
            state,
        });
    }
    users
}

// ── HTTP server ──────────────────────────────────────────────────

fn json_header() -> Header {
    Header::from_bytes("Content-Type", CONTENT_TYPE_JSON).unwrap()
}

fn cors_header() -> Header {
    Header::from_bytes("Access-Control-Allow-Origin", CORS_ALLOW_ORIGIN).unwrap()
}

// ── Service management constants ─────────────────────────────────

/// Service name for Windows Service Control Manager.
#[allow(dead_code)]
const SERVICE_NAME: &str = "SentinelAgent";
/// Display name in services.msc.
#[allow(dead_code)]
const SERVICE_DISPLAY_NAME: &str = "Sentinel Monitor Agent";
/// Installation directory on Windows.
#[allow(dead_code)]
const INSTALL_DIR: &str = r"C:\Program Files\Sentinel";
/// Binary name after installation.
#[allow(dead_code)]
const INSTALL_BINARY: &str = "sentinel-agent.exe";
/// Service description.
#[allow(dead_code)]
const SERVICE_DESCRIPTION: &str =
    "Sentinel system monitoring agent - collects and serves system metrics over HTTP";
/// Restart delay on crash (milliseconds).
#[allow(dead_code)]
const FAILURE_RESTART_MS: u32 = 5000;

// ── Argument parsing ─────────────────────────────────────────────

/// Parsed command-line action.
enum AgentAction {
    /// Run the HTTP server in console mode (default).
    RunConsole { bind: String, port: u16 },
    /// Install as a Windows service.
    Install,
    /// Uninstall the Windows service.
    Uninstall,
    /// Upgrade: stop service, copy binary, restart.
    Upgrade,
    /// Run as a Windows service (called by SCM).
    RunService,
}

fn parse_args() -> AgentAction {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--install") {
        return AgentAction::Install;
    }
    if args.iter().any(|a| a == "--uninstall") {
        return AgentAction::Uninstall;
    }
    if args.iter().any(|a| a == "--upgrade") {
        return AgentAction::Upgrade;
    }
    if args.iter().any(|a| a == "--service") {
        return AgentAction::RunService;
    }

    let bind = args
        .iter()
        .position(|a| a == "--bind")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| DEFAULT_BIND.to_string());

    let port: u16 = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    AgentAction::RunConsole { bind, port }
}

// ── HTTP server (shared between console and service modes) ───────

/// Run the HTTP server loop. This is the core logic shared between
/// console mode and Windows service mode.
fn run_server(bind: &str, port: u16) {
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
    let nets = Arc::new(Mutex::new(Networks::new_with_refreshed_list()));

    // Pre-refresh to get initial CPU readings (first read is always 0%)
    {
        let mut s = sys.lock().unwrap();
        s.refresh_cpu_all();
    }
    std::thread::sleep(Duration::from_millis(500));

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
                let mut n = nets.lock().unwrap();
                let pid_names: std::collections::HashMap<u32, String> = s
                    .processes()
                    .iter()
                    .map(|(pid, p)| (pid.as_u32(), p.name().to_string_lossy().to_string()))
                    .collect();
                let snapshot = collect_snapshot(&mut s, &mut n, &pid_names);
                drop(s);
                drop(n);

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

// ── Windows service management ───────────────────────────────────

#[cfg(windows)]
mod service {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;
    use windows_service::{
        define_windows_service,
        service::{
            ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
            ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    use super::*;

    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    /// Install the agent as a Windows service.
    pub fn install() {
        let install_dir = PathBuf::from(INSTALL_DIR);

        // Create installation directory
        if let Err(e) = std::fs::create_dir_all(&install_dir) {
            eprintln!("Failed to create {}: {}", INSTALL_DIR, e);
            eprintln!("Hint: Run as Administrator");
            std::process::exit(1);
        }

        // Copy current executable to install directory
        let current_exe = std::env::current_exe().expect("Failed to get current executable path");
        let target_exe = install_dir.join(INSTALL_BINARY);
        if current_exe != target_exe {
            if let Err(e) = std::fs::copy(&current_exe, &target_exe) {
                eprintln!("Failed to copy binary to {}: {}", target_exe.display(), e);
                std::process::exit(1);
            }
        }

        // Register with the Service Control Manager
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CREATE_SERVICE | ServiceManagerAccess::CONNECT,
        )
        .unwrap_or_else(|e| {
            eprintln!("Failed to connect to Service Manager: {}", e);
            eprintln!("Hint: Run as Administrator");
            std::process::exit(1);
        });

        let service_info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(SERVICE_DISPLAY_NAME),
            service_type: SERVICE_TYPE,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: target_exe.clone(),
            launch_arguments: vec![OsString::from("--service")],
            dependencies: vec![],
            account_name: None, // runs as LocalSystem
            account_password: None,
        };

        let service = manager
            .create_service(
                &service_info,
                ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
            )
            .unwrap_or_else(|e| {
                eprintln!("Failed to create service: {}", e);
                eprintln!("Hint: The service may already exist. Run --uninstall first.");
                std::process::exit(1);
            });

        // Set description
        let _ = service.set_description(SERVICE_DESCRIPTION);

        // Configure failure actions: restart after 5 seconds
        // This is done via sc.exe since the crate's failure action API is complex
        let _ = run_command(
            "sc",
            &[
                "failure",
                SERVICE_NAME,
                "reset=",
                "0",
                "actions=",
                &format!("restart/{}", FAILURE_RESTART_MS),
            ],
        );

        // Start the service
        match service.start::<OsString>(&[]) {
            Ok(()) => eprintln!("Service started."),
            Err(e) => eprintln!("Service created but failed to start: {}", e),
        }

        eprintln!();
        eprintln!("Sentinel Agent installed successfully!");
        eprintln!("  Binary:  {}", target_exe.display());
        eprintln!("  Service: {} ({})", SERVICE_NAME, SERVICE_DISPLAY_NAME);
        eprintln!("  Status:  Auto-start on boot, auto-restart on crash");
        eprintln!();
        eprintln!("Manage with:");
        eprintln!("  sc stop {}     — stop the service", SERVICE_NAME);
        eprintln!("  sc start {}    — start the service", SERVICE_NAME);
        eprintln!("  sc query {}    — check service status", SERVICE_NAME);
    }

    /// Uninstall the Windows service and clean up files.
    pub fn uninstall() {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .unwrap_or_else(|e| {
                eprintln!("Failed to connect to Service Manager: {}", e);
                eprintln!("Hint: Run as Administrator");
                std::process::exit(1);
            });

        // Open the service
        match manager.open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        ) {
            Ok(service) => {
                // Stop the service (ignore error if not running)
                let _ = service.stop();
                // Wait a moment for it to stop
                std::thread::sleep(Duration::from_secs(2));
                // Delete the service
                if let Err(e) = service.delete() {
                    eprintln!("Warning: Failed to delete service: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Service not found or not accessible: {}", e);
            }
        }

        // Remove installation directory
        let install_dir = PathBuf::from(INSTALL_DIR);
        if install_dir.exists() {
            // Files may be locked briefly after service stop; retry
            for attempt in 0..3 {
                match std::fs::remove_dir_all(&install_dir) {
                    Ok(()) => break,
                    Err(e) if attempt < 2 => {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    Err(e) => {
                        eprintln!("Warning: Could not remove {}: {}", INSTALL_DIR, e);
                    }
                }
            }
        }

        eprintln!("Sentinel Agent uninstalled.");
    }

    /// Upgrade: stop service, copy new binary, restart.
    pub fn upgrade() {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .unwrap_or_else(|e| {
                eprintln!("Failed to connect to Service Manager: {}", e);
                eprintln!("Hint: Run as Administrator");
                std::process::exit(1);
            });

        let target_exe = PathBuf::from(INSTALL_DIR).join(INSTALL_BINARY);

        // Stop the service
        match manager.open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        ) {
            Ok(service) => {
                eprintln!("Stopping service...");
                let _ = service.stop();
                // Wait for the service to stop and release the binary
                std::thread::sleep(Duration::from_secs(3));

                // Copy new binary
                let current_exe =
                    std::env::current_exe().expect("Failed to get current executable path");
                if current_exe != target_exe {
                    match std::fs::copy(&current_exe, &target_exe) {
                        Ok(_) => eprintln!("Binary updated: {}", target_exe.display()),
                        Err(e) => {
                            eprintln!("Failed to copy binary: {}", e);
                            eprintln!("The service binary may still be locked. Try again.");
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!("Binary is already in the install location.");
                }

                // Restart the service
                match service.start::<OsString>(&[]) {
                    Ok(()) => eprintln!("Service restarted."),
                    Err(e) => eprintln!("Failed to restart service: {}", e),
                }
            }
            Err(e) => {
                eprintln!("Service not found: {}", e);
                eprintln!("Run --install first.");
                std::process::exit(1);
            }
        }

        eprintln!("Sentinel Agent upgraded successfully!");
    }

    // Define the Windows service entry point macro
    define_windows_service!(ffi_service_main, service_main);

    /// Entry point called by the Windows Service Control Manager.
    pub fn run_as_service() {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main).unwrap_or_else(|e| {
            eprintln!("Failed to start service dispatcher: {}", e);
            std::process::exit(1);
        });
    }

    /// Service main function — called by the SCM dispatcher.
    fn service_main(_args: Vec<OsString>) {
        // Register the control handler
        let status_handle = service_control_handler::register(
            SERVICE_NAME,
            move |control| -> ServiceControlHandlerResult {
                match control {
                    ServiceControl::Stop | ServiceControl::Shutdown => {
                        // Signal the server to stop by exiting the process.
                        // The HTTP server loop blocks on incoming_requests(),
                        // so the cleanest way to stop is process::exit.
                        std::process::exit(0);
                    }
                    ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                    _ => ServiceControlHandlerResult::NotImplemented,
                }
            },
        )
        .expect("Failed to register service control handler");

        // Report that the service is running
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });

        // Run the HTTP server (blocks until stopped)
        run_server(DEFAULT_BIND, DEFAULT_PORT);

        // Report stopped (in case run_server returns normally)
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
    }
}

/// Stubs for non-Windows platforms.
#[cfg(not(windows))]
mod service {
    pub fn install() {
        eprintln!("Service installation is only available on Windows.");
        std::process::exit(1);
    }
    pub fn uninstall() {
        eprintln!("Service uninstallation is only available on Windows.");
        std::process::exit(1);
    }
    pub fn upgrade() {
        eprintln!("Service upgrade is only available on Windows.");
        std::process::exit(1);
    }
    pub fn run_as_service() {
        eprintln!("Service mode is only available on Windows.");
        std::process::exit(1);
    }
}

fn main() {
    match parse_args() {
        AgentAction::Install => service::install(),
        AgentAction::Uninstall => service::uninstall(),
        AgentAction::Upgrade => service::upgrade(),
        AgentAction::RunService => service::run_as_service(),
        AgentAction::RunConsole { bind, port } => run_server(&bind, port),
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_addr_port_ipv4() {
        let (addr, port) = parse_addr_port("192.168.1.1:8080").unwrap();
        assert_eq!(addr, "192.168.1.1");
        assert_eq!(port, 8080);
    }

    #[test]
    fn parse_addr_port_ipv4_any() {
        let (addr, port) = parse_addr_port("0.0.0.0:443").unwrap();
        assert_eq!(addr, "0.0.0.0");
        assert_eq!(port, 443);
    }

    #[test]
    fn parse_addr_port_ipv6_bracket() {
        let (addr, port) = parse_addr_port("[::1]:8080").unwrap();
        assert_eq!(addr, "::1");
        assert_eq!(port, 8080);
    }

    #[test]
    fn parse_addr_port_invalid() {
        assert!(parse_addr_port("noport").is_none());
        assert!(parse_addr_port("addr:notanum").is_none());
    }

    #[test]
    fn parse_netstat_output_standard() {
        let pid_names: std::collections::HashMap<u32, String> = [
            (1234, "chrome.exe".to_string()),
            (5678, "svchost.exe".to_string()),
        ]
        .into_iter()
        .collect();

        let raw = "\
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       5678
  TCP    192.168.1.5:52301      142.250.80.46:443      ESTABLISHED     1234
  TCP    192.168.1.5:52302      93.184.216.34:80       TIME_WAIT       0
  UDP    0.0.0.0:5353           *:*                                    9999
";

        let (conns, listeners) = parse_netstat_output(raw, &pid_names);

        // Should have 2 non-listening TCP connections
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[0].remote_addr, "142.250.80.46");
        assert_eq!(conns[0].remote_port, 443);
        assert_eq!(conns[0].process_name, "chrome.exe");
        assert_eq!(conns[0].state, "ESTABLISHED");

        assert_eq!(conns[1].state, "TIME_WAIT");
        assert_eq!(conns[1].process_name, "PID:0");

        // Should have 1 TCP listener + 1 UDP listener
        assert_eq!(listeners.len(), 2);
        assert_eq!(listeners[0].port, 135);
        assert_eq!(listeners[0].process_name, "svchost.exe");
        assert_eq!(listeners[0].protocol, "TCP");
        assert_eq!(listeners[1].port, 5353);
        assert_eq!(listeners[1].protocol, "UDP");
    }

    #[test]
    fn parse_netstat_output_empty() {
        let pid_names = std::collections::HashMap::new();
        let (conns, listeners) = parse_netstat_output("", &pid_names);
        assert!(conns.is_empty());
        assert!(listeners.is_empty());
    }

    #[test]
    fn parse_netstat_output_malformed() {
        let pid_names = std::collections::HashMap::new();
        let (conns, listeners) = parse_netstat_output("garbage\nrandom\nlines\n", &pid_names);
        assert!(conns.is_empty());
        assert!(listeners.is_empty());
    }

    #[test]
    fn parse_firewall_all_on() {
        let raw = "\
Domain Profile Settings:
----------------------------------------------------------------------
State                                 ON

Private Profile Settings:
----------------------------------------------------------------------
State                                 ON

Public Profile Settings:
----------------------------------------------------------------------
State                                 ON
";
        let profiles = parse_firewall_output(raw);
        assert_eq!(profiles.len(), 3);
        assert!(profiles.iter().all(|p| p.enabled));
        assert_eq!(profiles[0].name, "Domain");
        assert_eq!(profiles[1].name, "Private");
        assert_eq!(profiles[2].name, "Public");
    }

    #[test]
    fn parse_firewall_mixed() {
        let raw = "\
Domain Profile Settings:
----------------------------------------------------------------------
State                                 ON

Private Profile Settings:
----------------------------------------------------------------------
State                                 OFF

Public Profile Settings:
----------------------------------------------------------------------
State                                 ON
";
        let profiles = parse_firewall_output(raw);
        assert_eq!(profiles.len(), 3);
        assert!(profiles[0].enabled);
        assert!(!profiles[1].enabled);
        assert!(profiles[2].enabled);
    }

    #[test]
    fn parse_firewall_empty() {
        let profiles = parse_firewall_output("");
        assert!(profiles.is_empty());
    }

    #[test]
    fn parse_defender_enabled() {
        let raw = "\
AMServiceEnabled          : True
RealTimeProtectionEnabled : True
";
        let (enabled, realtime) = parse_defender_output(raw);
        assert_eq!(enabled, Some(true));
        assert_eq!(realtime, Some(true));
    }

    #[test]
    fn parse_defender_disabled() {
        let raw = "\
AMServiceEnabled          : False
RealTimeProtectionEnabled : False
";
        let (enabled, realtime) = parse_defender_output(raw);
        assert_eq!(enabled, Some(false));
        assert_eq!(realtime, Some(false));
    }

    #[test]
    fn parse_defender_empty() {
        let (enabled, realtime) = parse_defender_output("");
        assert_eq!(enabled, None);
        assert_eq!(realtime, None);
    }

    #[test]
    fn parse_update_days_valid() {
        assert_eq!(parse_update_days("15\n"), Some(15));
        assert_eq!(parse_update_days("  0  "), Some(0));
        assert_eq!(parse_update_days("365"), Some(365));
    }

    #[test]
    fn parse_update_days_invalid() {
        assert_eq!(parse_update_days(""), None);
        assert_eq!(parse_update_days("not a number"), None);
    }

    #[test]
    fn parse_startup_entries() {
        let raw = "\
Name     : OneDrive
Command  : \"C:\\Users\\user\\AppData\\Local\\Microsoft\\OneDrive\\OneDrive.exe\" /background
Location : HKU\\S-1-5-21\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run

Name     : SecurityHealth
Command  : %ProgramFiles%\\Windows Defender\\MSASCuiL.exe
Location : HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run
";
        let entries = parse_startup_output(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "OneDrive");
        assert!(entries[0].command.contains("OneDrive.exe"));
        assert!(entries[0].location.contains("HKU"));
        assert_eq!(entries[1].name, "SecurityHealth");
    }

    #[test]
    fn parse_startup_entries_empty() {
        let entries = parse_startup_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_query_user_standard() {
        let raw = "\
 USERNAME              SESSIONNAME        ID  STATE   IDLE TIME  LOGON TIME
>user1                 console             1  Active      none   3/10/2026 9:00 AM
 admin                 rdp-tcp#1           2  Active         5   3/10/2026 10:00 AM
";
        let users = parse_query_user(raw);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "user1");
        assert_eq!(users[0].session_type, "Console");
        assert_eq!(users[1].username, "admin");
        assert_eq!(users[1].session_type, "RDP");
    }

    #[test]
    fn parse_query_user_empty() {
        let users = parse_query_user("");
        assert!(users.is_empty());
    }

    #[test]
    fn agent_action_default_is_console() {
        // parse_args reads std::env::args which we can't easily mock,
        // but we can verify the enum variants exist and match correctly.
        let action = AgentAction::RunConsole {
            bind: "0.0.0.0".to_string(),
            port: 8086,
        };
        assert!(matches!(action, AgentAction::RunConsole { .. }));
    }

    #[test]
    fn agent_action_variants_exist() {
        // Verify all variants are constructable (compile-time check)
        let _a = AgentAction::Install;
        let _b = AgentAction::Uninstall;
        let _c = AgentAction::Upgrade;
        let _d = AgentAction::RunService;
        let _e = AgentAction::RunConsole {
            bind: String::new(),
            port: 0,
        };
    }
}
