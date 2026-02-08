use std::fmt;

/// Represents a single process snapshot with all relevant metrics.
/// This is our core domain entity - immutable snapshot of process state.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub memory_percent: f32,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub status: ProcessStatus,
    pub user: String,
    pub start_time: u64,
    pub parent_pid: Option<u32>,
    pub thread_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Dead,
    Unknown,
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessStatus::Running => write!(f, "Running"),
            ProcessStatus::Sleeping => write!(f, "Sleeping"),
            ProcessStatus::Stopped => write!(f, "Stopped"),
            ProcessStatus::Zombie => write!(f, "Zombie"),
            ProcessStatus::Dead => write!(f, "Dead"),
            ProcessStatus::Unknown => write!(f, "Unknown"),
        }
    }
}

impl ProcessInfo {
    /// Format memory in human-readable form
    pub fn memory_display(&self) -> String {
        format_bytes(self.memory_bytes)
    }

    /// Format disk read in human-readable form
    pub fn disk_read_display(&self) -> String {
        format_bytes(self.disk_read_bytes)
    }

    /// Format disk write in human-readable form
    pub fn disk_write_display(&self) -> String {
        format_bytes(self.disk_write_bytes)
    }
}

/// Formats bytes into human-readable string (KiB, MiB, GiB)
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}
