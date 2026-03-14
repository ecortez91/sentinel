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

/// Maximum top processes to include in snapshot.
const MAX_TOP_PROCESSES: usize = 30;

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

    // Collect top processes by CPU usage
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .filter(|p| !p.name().to_string_lossy().is_empty())
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
}
