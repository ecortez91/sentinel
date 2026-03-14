//! Data models for Windows host monitoring.
//!
//! Shared between the Sentinel TUI plugin and the sentinel-agent binary.
//! All structs derive `Serialize + Deserialize` for JSON transport.
//! New fields use `#[serde(default)]` for backward compatibility with
//! older agent versions that don't send them.

use serde::{Deserialize, Serialize};

/// Complete snapshot of Windows host system state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsHostSnapshot {
    /// Hostname of the Windows machine.
    pub hostname: String,
    /// OS version string (e.g., "Windows 11 23H2").
    pub os_version: String,
    /// System uptime in seconds.
    pub uptime_secs: u64,
    /// Overall CPU usage (0-100%).
    pub cpu_usage_pct: f32,
    /// Number of logical CPU cores.
    pub cpu_cores: u32,
    /// Total physical RAM in bytes.
    pub total_memory_bytes: u64,
    /// Used physical RAM in bytes.
    pub used_memory_bytes: u64,
    /// Top processes by CPU/memory.
    pub top_processes: Vec<WindowsProcessInfo>,
    /// Disk information.
    pub disks: Vec<WindowsDiskInfo>,
    /// GPU information (if available).
    pub gpu: Option<WindowsGpuInfo>,
    /// Network interfaces with traffic counters.
    #[serde(default)]
    pub networks: Vec<WindowsNetworkInfo>,
    /// Active TCP connections.
    #[serde(default)]
    pub tcp_connections: Vec<WindowsTcpConnection>,
    /// Listening ports.
    #[serde(default)]
    pub listening_ports: Vec<WindowsListeningPort>,
    /// Security status (firewall, defender, updates).
    #[serde(default)]
    pub security: Option<WindowsSecurityStatus>,
    /// Startup programs.
    #[serde(default)]
    pub startup_programs: Vec<WindowsStartupEntry>,
    /// Logged-in user sessions.
    #[serde(default)]
    pub logged_in_users: Vec<WindowsUserSession>,
}

impl WindowsHostSnapshot {
    /// Memory usage as a percentage (0-100).
    pub fn memory_usage_pct(&self) -> f32 {
        if self.total_memory_bytes == 0 {
            return 0.0;
        }
        (self.used_memory_bytes as f64 / self.total_memory_bytes as f64 * 100.0) as f32
    }
}

/// A Windows process snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsProcessInfo {
    /// Process ID.
    pub pid: u32,
    /// Process name.
    pub name: String,
    /// CPU usage percentage (0-100).
    pub cpu_pct: f32,
    /// Memory usage in bytes.
    pub memory_bytes: u64,
    /// Process status description.
    pub status: String,
}

/// Disk drive information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsDiskInfo {
    /// Drive letter or mount point (e.g., "C:").
    pub mount: String,
    /// Total capacity in bytes.
    pub total_bytes: u64,
    /// Used space in bytes.
    pub used_bytes: u64,
    /// Filesystem type (e.g., "NTFS").
    pub fs_type: String,
}

impl WindowsDiskInfo {
    /// Usage as a percentage (0-100).
    pub fn usage_pct(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.used_bytes as f64 / self.total_bytes as f64 * 100.0) as f32
    }
}

/// GPU information from the Windows host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsGpuInfo {
    /// GPU name (e.g., "NVIDIA RTX 4090").
    pub name: String,
    /// GPU usage percentage (0-100).
    pub usage_pct: f32,
    /// GPU temperature in Celsius.
    pub temp_celsius: f32,
    /// VRAM total in bytes.
    pub vram_total_bytes: u64,
    /// VRAM used in bytes.
    pub vram_used_bytes: u64,
}

// ── New data models for expanded agent ───────────────────────────

/// Network interface traffic counters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsNetworkInfo {
    /// Interface name.
    pub name: String,
    /// Total bytes received.
    pub rx_bytes: u64,
    /// Total bytes transmitted.
    pub tx_bytes: u64,
}

/// Active TCP connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsTcpConnection {
    /// Local IP address.
    pub local_addr: String,
    /// Local port.
    pub local_port: u16,
    /// Remote IP address.
    pub remote_addr: String,
    /// Remote port.
    pub remote_port: u16,
    /// Connection state (ESTABLISHED, TIME_WAIT, etc.).
    pub state: String,
    /// Process ID owning this connection.
    pub pid: u32,
    /// Process name owning this connection.
    pub process_name: String,
}

/// A port in LISTENING state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsListeningPort {
    /// Port number.
    pub port: u16,
    /// Process ID listening on this port.
    pub pid: u32,
    /// Process name listening on this port.
    pub process_name: String,
    /// Protocol (TCP or UDP).
    pub protocol: String,
}

/// Windows security status summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsSecurityStatus {
    /// Firewall status per profile (Domain, Private, Public).
    pub firewall_profiles: Vec<WindowsFirewallProfile>,
    /// Whether Windows Defender antimalware service is enabled.
    pub defender_enabled: Option<bool>,
    /// Whether Defender real-time protection is enabled.
    pub defender_realtime: Option<bool>,
    /// Days since last Windows Update was installed.
    pub last_update_days: Option<u64>,
}

/// Firewall status for a single network profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsFirewallProfile {
    /// Profile name (Domain, Private, Public).
    pub name: String,
    /// Whether the firewall is enabled for this profile.
    pub enabled: bool,
}

/// A program configured to run at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsStartupEntry {
    /// Program name.
    pub name: String,
    /// Command line.
    pub command: String,
    /// Registry location or source.
    pub location: String,
}

/// A logged-in user session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsUserSession {
    /// Username.
    pub username: String,
    /// Session type (Console, RDP, etc.).
    pub session_type: String,
    /// Session state (Active, Disconnected, etc.).
    pub state: String,
}

/// Agent health/status response.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    /// Agent version.
    pub version: String,
    /// Agent uptime in seconds.
    pub uptime_secs: u64,
    /// Whether the agent is collecting data.
    pub collecting: bool,
}

// ── Helper for creating test snapshots ───────────────────────────

/// Build a minimal snapshot for testing — all new fields default to empty.
#[cfg(test)]
pub fn make_test_snapshot() -> WindowsHostSnapshot {
    WindowsHostSnapshot {
        hostname: "DESKTOP-TEST".into(),
        os_version: "Windows 11".into(),
        uptime_secs: 3600,
        cpu_usage_pct: 25.0,
        cpu_cores: 8,
        total_memory_bytes: 16 * 1024 * 1024 * 1024,
        used_memory_bytes: 8 * 1024 * 1024 * 1024,
        top_processes: vec![
            WindowsProcessInfo {
                pid: 100,
                name: "chrome.exe".into(),
                cpu_pct: 12.5,
                memory_bytes: 500 * 1024 * 1024,
                status: "Running".into(),
            },
            WindowsProcessInfo {
                pid: 200,
                name: "explorer.exe".into(),
                cpu_pct: 2.0,
                memory_bytes: 100 * 1024 * 1024,
                status: "Running".into(),
            },
            WindowsProcessInfo {
                pid: 300,
                name: "code.exe".into(),
                cpu_pct: 8.0,
                memory_bytes: 800 * 1024 * 1024,
                status: "Running".into(),
            },
        ],
        disks: vec![WindowsDiskInfo {
            mount: "C:".into(),
            total_bytes: 500 * 1024 * 1024 * 1024,
            used_bytes: 250 * 1024 * 1024 * 1024,
            fs_type: "NTFS".into(),
        }],
        gpu: None,
        networks: Vec::new(),
        tcp_connections: Vec::new(),
        listening_ports: Vec::new(),
        security: None,
        startup_programs: Vec::new(),
        logged_in_users: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_usage_pct_normal() {
        let snap = make_test_snapshot();
        assert!((snap.memory_usage_pct() - 50.0).abs() < 0.1);
    }

    #[test]
    fn memory_usage_pct_zero_total() {
        let mut snap = make_test_snapshot();
        snap.total_memory_bytes = 0;
        snap.used_memory_bytes = 0;
        assert_eq!(snap.memory_usage_pct(), 0.0);
    }

    #[test]
    fn disk_usage_pct() {
        let disk = WindowsDiskInfo {
            mount: "C:".into(),
            total_bytes: 500 * 1024 * 1024 * 1024,
            used_bytes: 250 * 1024 * 1024 * 1024,
            fs_type: "NTFS".into(),
        };
        assert!((disk.usage_pct() - 50.0).abs() < 0.1);
    }

    #[test]
    fn disk_usage_pct_zero_total() {
        let disk = WindowsDiskInfo {
            mount: "D:".into(),
            total_bytes: 0,
            used_bytes: 0,
            fs_type: "".into(),
        };
        assert_eq!(disk.usage_pct(), 0.0);
    }

    #[test]
    fn snapshot_with_new_fields_roundtrip() {
        let mut snap = make_test_snapshot();
        snap.networks = vec![WindowsNetworkInfo {
            name: "Ethernet".into(),
            rx_bytes: 1_000_000,
            tx_bytes: 500_000,
        }];
        snap.tcp_connections = vec![WindowsTcpConnection {
            local_addr: "192.168.1.5".into(),
            local_port: 52301,
            remote_addr: "142.250.80.46".into(),
            remote_port: 443,
            state: "ESTABLISHED".into(),
            pid: 1234,
            process_name: "chrome.exe".into(),
        }];
        snap.listening_ports = vec![WindowsListeningPort {
            port: 8086,
            pid: 5678,
            process_name: "sentinel-agent.exe".into(),
            protocol: "TCP".into(),
        }];
        snap.security = Some(WindowsSecurityStatus {
            firewall_profiles: vec![
                WindowsFirewallProfile {
                    name: "Domain".into(),
                    enabled: true,
                },
                WindowsFirewallProfile {
                    name: "Private".into(),
                    enabled: true,
                },
                WindowsFirewallProfile {
                    name: "Public".into(),
                    enabled: false,
                },
            ],
            defender_enabled: Some(true),
            defender_realtime: Some(true),
            last_update_days: Some(5),
        });
        snap.startup_programs = vec![WindowsStartupEntry {
            name: "OneDrive".into(),
            command: "OneDrive.exe /background".into(),
            location: "HKCU\\...\\Run".into(),
        }];
        snap.logged_in_users = vec![WindowsUserSession {
            username: "admin".into(),
            session_type: "Console".into(),
            state: "Active".into(),
        }];

        let json = serde_json::to_string(&snap).unwrap();
        let parsed: WindowsHostSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hostname, "DESKTOP-TEST");
        assert_eq!(parsed.networks.len(), 1);
        assert_eq!(parsed.tcp_connections.len(), 1);
        assert_eq!(parsed.listening_ports.len(), 1);
        assert!(parsed.security.is_some());
        assert_eq!(parsed.startup_programs.len(), 1);
        assert_eq!(parsed.logged_in_users.len(), 1);
    }

    #[test]
    fn old_json_without_new_fields_parses() {
        // Simulate JSON from an old agent (no network/security fields)
        let old_json = r#"{
            "hostname": "OLD-AGENT",
            "os_version": "Windows 10",
            "uptime_secs": 100,
            "cpu_usage_pct": 10.0,
            "cpu_cores": 4,
            "total_memory_bytes": 8589934592,
            "used_memory_bytes": 4294967296,
            "top_processes": [],
            "disks": [],
            "gpu": null
        }"#;
        let parsed: WindowsHostSnapshot = serde_json::from_str(old_json).unwrap();
        assert_eq!(parsed.hostname, "OLD-AGENT");
        // All new fields should be empty/None due to #[serde(default)]
        assert!(parsed.networks.is_empty());
        assert!(parsed.tcp_connections.is_empty());
        assert!(parsed.listening_ports.is_empty());
        assert!(parsed.security.is_none());
        assert!(parsed.startup_programs.is_empty());
        assert!(parsed.logged_in_users.is_empty());
    }

    #[test]
    fn agent_status_serialization() {
        let status = AgentStatus {
            version: "0.1.0".into(),
            uptime_secs: 3600,
            collecting: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, "0.1.0");
        assert!(parsed.collecting);
    }

    #[test]
    fn network_info_roundtrip() {
        let net = WindowsNetworkInfo {
            name: "Wi-Fi".into(),
            rx_bytes: 123456789,
            tx_bytes: 987654321,
        };
        let json = serde_json::to_string(&net).unwrap();
        let parsed: WindowsNetworkInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Wi-Fi");
        assert_eq!(parsed.rx_bytes, 123456789);
    }

    #[test]
    fn security_status_roundtrip() {
        let sec = WindowsSecurityStatus {
            firewall_profiles: vec![WindowsFirewallProfile {
                name: "Domain".into(),
                enabled: true,
            }],
            defender_enabled: Some(true),
            defender_realtime: Some(false),
            last_update_days: Some(42),
        };
        let json = serde_json::to_string(&sec).unwrap();
        let parsed: WindowsSecurityStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.firewall_profiles.len(), 1);
        assert_eq!(parsed.defender_enabled, Some(true));
        assert_eq!(parsed.defender_realtime, Some(false));
        assert_eq!(parsed.last_update_days, Some(42));
    }
}
