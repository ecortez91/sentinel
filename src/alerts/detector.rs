use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::config::Config;
use crate::constants::*;
use crate::models::{
    Alert, AlertCategory, AlertSeverity, ProcessInfo, ProcessStatus, SystemSnapshot,
};
use crate::thermal::ThermalSnapshot;

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

    /// Get the thermal warning threshold from config.
    pub fn config_thermal_warning(&self) -> f32 {
        self.config.thermal.warning_threshold
    }

    /// Get the thermal critical threshold from config.
    pub fn config_thermal_critical(&self) -> f32 {
        self.config.thermal.critical_threshold
    }

    /// Get the thermal emergency threshold from config.
    pub fn config_thermal_emergency(&self) -> f32 {
        self.config.thermal.emergency_threshold
    }

    /// Run all detection rules and return any triggered alerts.
    /// Applies a 60-second cooldown per (PID, category) to prevent flooding.
    pub fn analyze(&mut self, system: &SystemSnapshot, processes: &[ProcessInfo]) -> Vec<Alert> {
        let mut raw_alerts = Vec::new();

        // System-wide checks
        self.check_system_memory(&mut raw_alerts, system);
        self.check_system_cpu(&mut raw_alerts, system);

        // Build PID -> process name lookup for parent resolution (zombie filtering)
        let pid_names: std::collections::HashMap<u32, &str> =
            processes.iter().map(|p| (p.pid, p.name.as_str())).collect();

        // Per-process checks
        for proc in processes {
            self.check_cpu_usage(&mut raw_alerts, proc);
            self.check_memory_usage(&mut raw_alerts, proc);
            self.check_zombie(&mut raw_alerts, proc, &pid_names);
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

    fn check_zombie(
        &self,
        alerts: &mut Vec<Alert>,
        proc: &ProcessInfo,
        pid_names: &std::collections::HashMap<u32, &str>,
    ) {
        if proc.status != ProcessStatus::Zombie {
            return;
        }

        // Skip zombies whose parent is in the ignore list
        if let Some(ppid) = proc.parent_pid {
            if let Some(parent_name) = pid_names.get(&ppid) {
                let parent_lower = parent_name.to_lowercase();
                if self
                    .config
                    .ignored_zombie_parents
                    .iter()
                    .any(|pat| parent_lower.contains(&pat.to_lowercase()))
                {
                    return;
                }
            }
        }

        let parent_suffix = proc
            .parent_pid
            .and_then(|ppid| pid_names.get(&ppid))
            .map(|pn| format!(" [parent: {}]", pn))
            .unwrap_or_default();

        alerts.push(Alert::new(
            AlertSeverity::Warning,
            AlertCategory::Zombie,
            &proc.name,
            proc.pid,
            format!(
                "Zombie process: {} (PID {}){}",
                proc.name, proc.pid, parent_suffix
            ),
            1.0,
            0.0,
        ));
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

    /// Check thermal data for temperature-related alerts.
    /// Uses the same cooldown system as process alerts (PID 0 for system-level).
    pub fn check_thermal(&mut self, thermal: &ThermalSnapshot) -> Vec<Alert> {
        let warning = self.config.thermal.warning_threshold;
        let critical = self.config.thermal.critical_threshold;
        let emergency = self.config.thermal.emergency_threshold;

        let mut raw_alerts = Vec::new();

        // Check CPU package temperature
        if let Some(pkg) = thermal.cpu_package {
            self.emit_thermal_alert(
                &mut raw_alerts,
                "CPU Package",
                pkg,
                warning,
                critical,
                emergency,
            );
        }

        // Check individual CPU cores
        for core in &thermal.cpu_cores {
            self.emit_thermal_alert(
                &mut raw_alerts,
                &core.name,
                core.value,
                warning,
                critical,
                emergency,
            );
        }

        // Check GPU temperatures
        if let Some(gpu) = thermal.gpu_temp {
            self.emit_thermal_alert(
                &mut raw_alerts,
                "GPU Core",
                gpu,
                warning,
                critical,
                emergency,
            );
        }
        if let Some(hotspot) = thermal.gpu_hotspot {
            self.emit_thermal_alert(
                &mut raw_alerts,
                "GPU Hot Spot",
                hotspot,
                warning,
                critical,
                emergency,
            );
        }

        // Apply cooldown deduplication
        let now = Instant::now();
        let cooldown = std::time::Duration::from_secs(ALERT_COOLDOWN_SECS);
        raw_alerts
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
            .collect()
    }

    fn emit_thermal_alert(
        &self,
        alerts: &mut Vec<Alert>,
        sensor_name: &str,
        temp: f32,
        warning: f32,
        critical: f32,
        emergency: f32,
    ) {
        // Use a stable pseudo-PID derived from sensor name so different sensors
        // don't collide in the cooldown dedup map (which keys on (pid, category)).
        let pseudo_pid = sensor_name
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));

        if temp >= emergency {
            alerts.push(Alert::new(
                AlertSeverity::Danger,
                AlertCategory::ThermalEmergency,
                sensor_name,
                pseudo_pid,
                format!(
                    "EMERGENCY: {} at {:.1}°C (threshold: {:.0}°C)",
                    sensor_name, temp, emergency
                ),
                temp as f64,
                emergency as f64,
            ));
        } else if temp >= critical {
            alerts.push(Alert::new(
                AlertSeverity::Critical,
                AlertCategory::ThermalCritical,
                sensor_name,
                pseudo_pid,
                format!(
                    "CRITICAL: {} at {:.1}°C (threshold: {:.0}°C)",
                    sensor_name, temp, critical
                ),
                temp as f64,
                critical as f64,
            ));
        } else if temp >= warning {
            alerts.push(Alert::new(
                AlertSeverity::Warning,
                AlertCategory::ThermalWarning,
                sensor_name,
                pseudo_pid,
                format!(
                    "{} running hot: {:.1}°C (threshold: {:.0}°C)",
                    sensor_name, temp, warning
                ),
                temp as f64,
                warning as f64,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn make_detector() -> AlertDetector {
        let config = Config::default();
        AlertDetector::new(config)
    }

    fn make_thermal(cpu_pkg: Option<f32>, gpu: Option<f32>) -> ThermalSnapshot {
        ThermalSnapshot {
            timestamp: Instant::now(),
            cpu_package: cpu_pkg,
            cpu_cores: Vec::new(),
            gpu_temp: gpu,
            gpu_hotspot: None,
            ssd_temps: Vec::new(),
            fan_rpms: Vec::new(),
            motherboard_temps: Vec::new(),
            ram_temps: Vec::new(),
            max_temp: cpu_pkg.unwrap_or(0.0).max(gpu.unwrap_or(0.0)),
            max_cpu_temp: cpu_pkg.unwrap_or(0.0),
            max_gpu_temp: gpu.unwrap_or(0.0),
            max_ram_temp: 0.0,
        }
    }

    #[test]
    fn thermal_no_alert_below_warning() {
        let mut det = make_detector();
        let snap = make_thermal(Some(60.0), Some(55.0));
        let alerts = det.check_thermal(&snap);
        assert!(alerts.is_empty());
    }

    #[test]
    fn thermal_warning_at_threshold() {
        let mut det = make_detector();
        let snap = make_thermal(Some(85.0), None);
        let alerts = det.check_thermal(&snap);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::ThermalWarning);
        assert_eq!(alerts[0].severity, AlertSeverity::Warning);
    }

    #[test]
    fn thermal_critical_at_threshold() {
        let mut det = make_detector();
        let snap = make_thermal(Some(95.0), None);
        let alerts = det.check_thermal(&snap);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::ThermalCritical);
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
    }

    #[test]
    fn thermal_emergency_at_threshold() {
        let mut det = make_detector();
        let snap = make_thermal(Some(100.0), None);
        let alerts = det.check_thermal(&snap);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::ThermalEmergency);
        assert_eq!(alerts[0].severity, AlertSeverity::Danger);
    }

    #[test]
    fn thermal_multiple_sensors() {
        let mut det = make_detector();
        // Both CPU and GPU above warning
        let snap = make_thermal(Some(90.0), Some(88.0));
        let alerts = det.check_thermal(&snap);
        // CPU at 90 = critical (>= 85 warning but >= 95 critical? No, 90 < 95, so warning)
        // Actually: warning=85, critical=95, emergency=100
        // CPU 90 >= 85 = warning, GPU 88 >= 85 = warning
        assert_eq!(alerts.len(), 2);
        assert!(alerts
            .iter()
            .all(|a| a.category == AlertCategory::ThermalWarning));
    }

    #[test]
    fn thermal_cooldown_dedup() {
        let mut det = make_detector();
        let snap = make_thermal(Some(90.0), None);
        let alerts1 = det.check_thermal(&snap);
        assert_eq!(alerts1.len(), 1);
        // Second call within cooldown — should be filtered
        let alerts2 = det.check_thermal(&snap);
        assert!(alerts2.is_empty());
    }

    // ── Zombie filter tests ───────────────────────────────────────

    fn make_system_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            global_cpu_usage: 20.0,
            cpu_usages: vec![20.0],
            cpu_count: 1,
            total_memory: 16_000_000_000,
            used_memory: 4_000_000_000,
            total_swap: 1_000_000_000,
            used_swap: 0,
            load_avg_1: 0.5,
            load_avg_5: 0.3,
            load_avg_15: 0.2,
            uptime: 3600,
            hostname: "test".to_string(),
            os_name: "Linux".to_string(),
            total_processes: 1,
            networks: Vec::new(),
            disks: Vec::new(),
            cpu_temp: None,
            gpu: None,
            battery: None,
        }
    }

    fn make_running_process(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: name.to_string(),
            cpu_usage: 1.0,
            memory_bytes: 1024,
            memory_percent: 0.0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: ProcessStatus::Running,
            user: "test".to_string(),
            start_time: 0,
            parent_pid: None,
            thread_count: None,
        }
    }

    fn make_zombie_process(pid: u32, name: &str, parent_pid: Option<u32>) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: name.to_string(),
            cpu_usage: 0.0,
            memory_bytes: 0,
            memory_percent: 0.0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: ProcessStatus::Zombie,
            user: "test".to_string(),
            start_time: 0,
            parent_pid,
            thread_count: None,
        }
    }

    #[test]
    fn zombie_ignored_when_parent_in_ignore_list() {
        let mut det = make_detector();
        let parent = make_running_process(100, "opencode");
        let zombie = make_zombie_process(200, "sh", Some(100));
        let system = make_system_snapshot();
        let procs = vec![parent, zombie];
        let alerts = det.analyze(&system, &procs);
        assert!(
            alerts.iter().all(|a| a.category != AlertCategory::Zombie),
            "Zombie with ignored parent should not generate an alert"
        );
    }

    #[test]
    fn zombie_alert_fires_when_parent_not_ignored() {
        let mut det = make_detector();
        let parent = make_running_process(100, "bash");
        let zombie = make_zombie_process(200, "defunct", Some(100));
        let system = make_system_snapshot();
        let procs = vec![parent, zombie];
        let alerts = det.analyze(&system, &procs);
        assert!(
            alerts.iter().any(|a| a.category == AlertCategory::Zombie),
            "Zombie with non-ignored parent should generate an alert"
        );
    }

    #[test]
    fn zombie_alert_fires_when_no_parent() {
        let mut det = make_detector();
        let zombie = make_zombie_process(200, "orphan", None);
        let system = make_system_snapshot();
        let procs = vec![zombie];
        let alerts = det.analyze(&system, &procs);
        assert!(
            alerts.iter().any(|a| a.category == AlertCategory::Zombie),
            "Zombie without parent should still generate an alert"
        );
    }

    #[test]
    fn zombie_alert_message_includes_parent_name() {
        let mut det = make_detector();
        let parent = make_running_process(100, "bash");
        let zombie = make_zombie_process(200, "defunct", Some(100));
        let system = make_system_snapshot();
        let procs = vec![parent, zombie];
        let alerts = det.analyze(&system, &procs);
        let zombie_alert = alerts
            .iter()
            .find(|a| a.category == AlertCategory::Zombie)
            .expect("Expected a zombie alert");
        assert!(
            zombie_alert.message.contains("[parent: bash]"),
            "Alert message should include parent name, got: {}",
            zombie_alert.message
        );
    }

    #[test]
    fn zombie_parent_ignore_is_case_insensitive() {
        let mut det = make_detector();
        // Config has "opencode" lowercase; parent has "OpenCode" mixed case
        let parent = make_running_process(100, "OpenCode");
        let zombie = make_zombie_process(200, "sh", Some(100));
        let system = make_system_snapshot();
        let procs = vec![parent, zombie];
        let alerts = det.analyze(&system, &procs);
        assert!(
            alerts.iter().all(|a| a.category != AlertCategory::Zombie),
            "Parent ignore matching should be case-insensitive"
        );
    }
}
