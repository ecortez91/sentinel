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

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_bytes ──────────────────────────────────────────────

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_exact_kib() {
        assert_eq!(format_bytes(1024), "1.0 KiB");
    }

    #[test]
    fn format_bytes_kib_range() {
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(10 * 1024), "10.0 KiB");
    }

    #[test]
    fn format_bytes_exact_mib() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn format_bytes_mib_range() {
        assert_eq!(format_bytes(256 * 1024 * 1024), "256.0 MiB");
        // 1.5 MiB
        assert_eq!(format_bytes(1024 * 1024 + 512 * 1024), "1.5 MiB");
    }

    #[test]
    fn format_bytes_exact_gib() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GiB");
    }

    #[test]
    fn format_bytes_large_gib() {
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GiB");
    }

    // ── ProcessStatus Display ─────────────────────────────────────

    #[test]
    fn process_status_display() {
        assert_eq!(ProcessStatus::Running.to_string(), "Running");
        assert_eq!(ProcessStatus::Sleeping.to_string(), "Sleeping");
        assert_eq!(ProcessStatus::Stopped.to_string(), "Stopped");
        assert_eq!(ProcessStatus::Zombie.to_string(), "Zombie");
        assert_eq!(ProcessStatus::Dead.to_string(), "Dead");
        assert_eq!(ProcessStatus::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn process_status_equality() {
        assert_eq!(ProcessStatus::Running, ProcessStatus::Running);
        assert_ne!(ProcessStatus::Running, ProcessStatus::Sleeping);
    }

    // ── ProcessInfo display helpers ───────────────────────────────

    fn make_process(pid: u32, name: &str, cpu: f32, mem_bytes: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: name.to_string(),
            cpu_usage: cpu,
            memory_bytes: mem_bytes,
            memory_percent: 0.0,
            disk_read_bytes: 1024 * 1024, // 1 MiB
            disk_write_bytes: 2048,       // 2 KiB
            status: ProcessStatus::Running,
            user: "test".to_string(),
            start_time: 0,
            parent_pid: None,
            thread_count: None,
        }
    }

    #[test]
    fn process_info_memory_display() {
        let p = make_process(1, "test", 0.0, 256 * 1024 * 1024);
        assert_eq!(p.memory_display(), "256.0 MiB");
    }

    #[test]
    fn process_info_disk_displays() {
        let p = make_process(1, "test", 0.0, 0);
        assert_eq!(p.disk_read_display(), "1.0 MiB");
        assert_eq!(p.disk_write_display(), "2.0 KiB");
    }
}
