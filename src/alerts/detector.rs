use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::config::Config;
use crate::constants::*;
use crate::models::{
    Alert, AlertCategory, AlertSeverity, ProcessInfo, ProcessStatus, SystemSnapshot,
};

/// The alert detection engine. Analyzes process and system data
/// to generate warnings, threats, and anomalies.
///
/// Open/Closed Principle: new detection rules can be added as methods
/// without modifying existing ones.
pub struct AlertDetector {
    config: Config,
    /// Track memory over time per PID to detect leaks
    memory_history: HashMap<u32, VecDeque<u64>>,
    /// Max history entries per process
    max_history: usize,
    /// Cooldown tracking: (PID, category) -> last fire time
    alert_cooldowns: HashMap<(u32, AlertCategory), Instant>,
}

impl AlertDetector {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            memory_history: HashMap::new(),
            max_history: MAX_MEMORY_HISTORY, // ~30 seconds of history at 1s interval
            alert_cooldowns: HashMap::new(),
        }
    }

    /// Run all detection rules and return any triggered alerts.
    /// Applies a 60-second cooldown per (PID, category) to prevent flooding.
    pub fn analyze(&mut self, system: &SystemSnapshot, processes: &[ProcessInfo]) -> Vec<Alert> {
        let mut raw_alerts = Vec::new();

        // System-wide checks
        self.check_system_memory(&mut raw_alerts, system);
        self.check_system_cpu(&mut raw_alerts, system);

        // Per-process checks
        for proc in processes {
            self.check_cpu_usage(&mut raw_alerts, proc);
            self.check_memory_usage(&mut raw_alerts, proc);
            self.check_zombie(&mut raw_alerts, proc);
            self.check_suspicious(&mut raw_alerts, proc);
            self.check_security_threats(&mut raw_alerts, proc);
            self.check_memory_leak(&mut raw_alerts, proc);
            self.check_high_disk_io(&mut raw_alerts, proc);
        }

        // Apply cooldown deduplication
        let now = Instant::now();
        let cooldown = std::time::Duration::from_secs(ALERT_COOLDOWN_SECS);
        let alerts: Vec<Alert> = raw_alerts
            .into_iter()
            .filter(|alert| {
                let key = (alert.pid, alert.category);
                match self.alert_cooldowns.get(&key) {
                    Some(last_fired) if now.duration_since(*last_fired) < cooldown => false,
                    _ => {
                        self.alert_cooldowns.insert(key, now);
                        true
                    }
                }
            })
            .collect();

        // Clean up history and cooldowns for dead processes
        let active_pids: std::collections::HashSet<u32> = processes.iter().map(|p| p.pid).collect();
        self.memory_history
            .retain(|pid, _| active_pids.contains(pid));
        self.alert_cooldowns
            .retain(|(pid, _), _| active_pids.contains(pid) || *pid == 0);

        alerts
    }

    fn check_system_memory(&self, alerts: &mut Vec<Alert>, system: &SystemSnapshot) {
        let pct = system.memory_percent();
        if pct >= self.config.sys_mem_critical_percent {
            alerts.push(Alert::new(
                AlertSeverity::Danger,
                AlertCategory::SystemOverload,
                "SYSTEM",
                0,
                format!(
                    "System memory critically high: {:.1}% ({} / {})",
                    pct,
                    crate::models::format_bytes(system.used_memory),
                    crate::models::format_bytes(system.total_memory),
                ),
                pct as f64,
                self.config.sys_mem_critical_percent as f64,
            ));
        } else if pct >= self.config.sys_mem_warning_percent {
            alerts.push(Alert::new(
                AlertSeverity::Warning,
                AlertCategory::SystemOverload,
                "SYSTEM",
                0,
                format!(
                    "System memory high: {:.1}% ({} / {})",
                    pct,
                    crate::models::format_bytes(system.used_memory),
                    crate::models::format_bytes(system.total_memory),
                ),
                pct as f64,
                self.config.sys_mem_warning_percent as f64,
            ));
        }
    }

    fn check_system_cpu(&self, alerts: &mut Vec<Alert>, system: &SystemSnapshot) {
        if system.global_cpu_usage >= self.config.cpu_critical_threshold {
            alerts.push(Alert::new(
                AlertSeverity::Critical,
                AlertCategory::SystemOverload,
                "SYSTEM",
                0,
                format!(
                    "System CPU critically high: {:.1}%",
                    system.global_cpu_usage
                ),
                system.global_cpu_usage as f64,
                self.config.cpu_critical_threshold as f64,
            ));
        }
    }

    fn check_cpu_usage(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        if proc.cpu_usage >= self.config.cpu_critical_threshold {
            alerts.push(Alert::new(
                AlertSeverity::Critical,
                AlertCategory::HighCpu,
                &proc.name,
                proc.pid,
                format!("{} using {:.1}% CPU", proc.name, proc.cpu_usage),
                proc.cpu_usage as f64,
                self.config.cpu_critical_threshold as f64,
            ));
        } else if proc.cpu_usage >= self.config.cpu_warning_threshold {
            alerts.push(Alert::new(
                AlertSeverity::Warning,
                AlertCategory::HighCpu,
                &proc.name,
                proc.pid,
                format!("{} using {:.1}% CPU", proc.name, proc.cpu_usage),
                proc.cpu_usage as f64,
                self.config.cpu_warning_threshold as f64,
            ));
        }
    }

    fn check_memory_usage(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        if proc.memory_bytes >= self.config.mem_critical_threshold_bytes {
            alerts.push(Alert::new(
                AlertSeverity::Critical,
                AlertCategory::HighMemory,
                &proc.name,
                proc.pid,
                format!(
                    "{} using {} RAM ({:.1}%)",
                    proc.name,
                    proc.memory_display(),
                    proc.memory_percent
                ),
                proc.memory_bytes as f64,
                self.config.mem_critical_threshold_bytes as f64,
            ));
        } else if proc.memory_bytes >= self.config.mem_warning_threshold_bytes {
            alerts.push(Alert::new(
                AlertSeverity::Warning,
                AlertCategory::HighMemory,
                &proc.name,
                proc.pid,
                format!(
                    "{} using {} RAM ({:.1}%)",
                    proc.name,
                    proc.memory_display(),
                    proc.memory_percent
                ),
                proc.memory_bytes as f64,
                self.config.mem_warning_threshold_bytes as f64,
            ));
        }
    }

    fn check_zombie(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        if proc.status == ProcessStatus::Zombie {
            alerts.push(Alert::new(
                AlertSeverity::Warning,
                AlertCategory::Zombie,
                &proc.name,
                proc.pid,
                format!("Zombie process: {} (PID {})", proc.name, proc.pid),
                1.0,
                0.0,
            ));
        }
    }

    fn check_suspicious(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        self.check_patterns(
            alerts,
            proc,
            &self.config.suspicious_patterns,
            AlertSeverity::Warning,
            AlertCategory::Suspicious,
            "Suspicious process detected",
        );
    }

    fn check_security_threats(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        self.check_patterns(
            alerts,
            proc,
            &self.config.security_threat_patterns,
            AlertSeverity::Danger,
            AlertCategory::SecurityThreat,
            "SECURITY THREAT",
        );
    }

    fn check_patterns(
        &self,
        alerts: &mut Vec<Alert>,
        proc: &ProcessInfo,
        patterns: &[String],
        severity: AlertSeverity,
        category: AlertCategory,
        msg_prefix: &str,
    ) {
        let name_lower = proc.name.to_lowercase();
        let cmd_lower = proc.cmd.to_lowercase();

        for pattern in patterns {
            let pat = pattern.to_lowercase();
            if name_lower.contains(&pat) || cmd_lower.contains(&pat) {
                alerts.push(Alert::new(
                    severity,
                    category,
                    &proc.name,
                    proc.pid,
                    format!("{}: {} (matched '{}')", msg_prefix, proc.name, pattern),
                    0.0,
                    0.0,
                ));
                break;
            }
        }
    }

    fn check_memory_leak(&mut self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        let history = self.memory_history.entry(proc.pid).or_default();
        history.push_back(proc.memory_bytes);

        while history.len() > self.max_history {
            history.pop_front();
        }

        // Need at least LEAK_MIN_SAMPLES samples to detect a trend
        if history.len() >= LEAK_MIN_SAMPLES {
            let slice: Vec<u64> = history.iter().copied().collect();
            let first_half_avg: f64 =
                slice[..slice.len() / 2].iter().sum::<u64>() as f64 / (slice.len() / 2) as f64;
            let second_half_avg: f64 = slice[slice.len() / 2..].iter().sum::<u64>() as f64
                / (slice.len() - slice.len() / 2) as f64;

            if second_half_avg > first_half_avg * LEAK_GROWTH_FACTOR
                && proc.memory_bytes > LEAK_MIN_MEMORY_BYTES
            {
                let growth_pct = ((second_half_avg - first_half_avg) / first_half_avg) * 100.0;
                alerts.push(Alert::new(
                    AlertSeverity::Warning,
                    AlertCategory::MemoryLeak,
                    &proc.name,
                    proc.pid,
                    format!(
                        "Possible memory leak in {}: +{:.0}% growth trend",
                        proc.name, growth_pct
                    ),
                    growth_pct,
                    LEAK_ALERT_THRESHOLD_PCT,
                ));
            }
        }
    }

    fn check_high_disk_io(&self, alerts: &mut Vec<Alert>, proc: &ProcessInfo) {
        let total_io = proc.disk_read_bytes + proc.disk_write_bytes;
        if total_io > HIGH_DISK_IO_THRESHOLD {
            alerts.push(Alert::new(
                AlertSeverity::Info,
                AlertCategory::HighDiskIo,
                &proc.name,
                proc.pid,
                format!(
                    "High disk I/O: {} (R: {}, W: {})",
                    proc.name,
                    proc.disk_read_display(),
                    proc.disk_write_display(),
                ),
                total_io as f64,
                HIGH_DISK_IO_THRESHOLD as f64,
            ));
        }
    }
}
