//! Data models for Windows host monitoring (#1).
//!
//! Shared between the Sentinel TUI plugin and the sentinel-agent binary.
//! All structs derive `Serialize + Deserialize` for JSON transport.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_usage_pct_normal() {
        let snap = WindowsHostSnapshot {
            hostname: "DESKTOP".into(),
            os_version: "Windows 11".into(),
            uptime_secs: 3600,
            cpu_usage_pct: 25.0,
            cpu_cores: 8,
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            used_memory_bytes: 8 * 1024 * 1024 * 1024,
            top_processes: vec![],
            disks: vec![],
            gpu: None,
        };
        assert!((snap.memory_usage_pct() - 50.0).abs() < 0.1);
    }

    #[test]
    fn memory_usage_pct_zero_total() {
        let snap = WindowsHostSnapshot {
            hostname: String::new(),
            os_version: String::new(),
            uptime_secs: 0,
            cpu_usage_pct: 0.0,
            cpu_cores: 0,
            total_memory_bytes: 0,
            used_memory_bytes: 0,
            top_processes: vec![],
            disks: vec![],
            gpu: None,
        };
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
    fn snapshot_serialization_roundtrip() {
        let snap = WindowsHostSnapshot {
            hostname: "DESKTOP-TEST".into(),
            os_version: "Windows 11 23H2".into(),
            uptime_secs: 86400,
            cpu_usage_pct: 33.5,
            cpu_cores: 12,
            total_memory_bytes: 32 * 1024 * 1024 * 1024,
            used_memory_bytes: 16 * 1024 * 1024 * 1024,
            top_processes: vec![WindowsProcessInfo {
                pid: 1234,
                name: "chrome.exe".into(),
                cpu_pct: 5.2,
                memory_bytes: 512 * 1024 * 1024,
                status: "Running".into(),
            }],
            disks: vec![WindowsDiskInfo {
                mount: "C:".into(),
                total_bytes: 1_000_000_000_000,
                used_bytes: 400_000_000_000,
                fs_type: "NTFS".into(),
            }],
            gpu: Some(WindowsGpuInfo {
                name: "RTX 4090".into(),
                usage_pct: 45.0,
                temp_celsius: 65.0,
                vram_total_bytes: 24 * 1024 * 1024 * 1024,
                vram_used_bytes: 8 * 1024 * 1024 * 1024,
            }),
        };

        let json = serde_json::to_string(&snap).unwrap();
        let parsed: WindowsHostSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hostname, "DESKTOP-TEST");
        assert_eq!(parsed.top_processes.len(), 1);
        assert_eq!(parsed.disks.len(), 1);
        assert!(parsed.gpu.is_some());
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
}
