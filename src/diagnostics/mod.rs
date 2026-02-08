//! Diagnostic engine — the intelligence layer.
//!
//! Analyzes live system state + historical data from the event store to produce
//! structured findings. These findings are:
//! 1. Displayed directly in the command palette / diagnostic popup
//! 2. Fed into AI context for richer natural-language responses
//! 3. Used to generate actionable suggestions
//!
//! Each diagnostic function returns a `DiagnosticReport` with findings and
//! optional suggested actions.

use crate::models::{Alert, ProcessInfo, SystemSnapshot};
use crate::store::EventStore;

// ── Finding types ─────────────────────────────────────────────────

/// Severity of a diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

/// A single diagnostic finding.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: FindingSeverity,
    pub title: String,
    pub detail: String,
    /// Optional suggested action the user can take.
    pub action: Option<SuggestedAction>,
}

/// An action the user can confirm and execute.
#[derive(Debug, Clone)]
pub enum SuggestedAction {
    /// Kill a process: (pid, signal_name)
    KillProcess {
        pid: u32,
        name: String,
        signal: &'static str,
    },
    /// Renice a process
    ReniceProcess { pid: u32, name: String, nice: i32 },
    /// Free a port by killing the owning process
    FreePort { port: u16, pid: u32, name: String },
    /// Clean up a directory
    CleanDirectory { path: String, size_bytes: u64 },
    /// Informational — no executable action
    Info(String),
}

/// Complete diagnostic report from an analysis.
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub title: String,
    pub findings: Vec<Finding>,
}

impl DiagnosticReport {
    fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            findings: Vec::new(),
        }
    }

    fn push(&mut self, severity: FindingSeverity, title: &str, detail: &str) {
        self.findings.push(Finding {
            severity,
            title: title.to_string(),
            detail: detail.to_string(),
            action: None,
        });
    }

    fn push_with_action(
        &mut self,
        severity: FindingSeverity,
        title: &str,
        detail: &str,
        action: SuggestedAction,
    ) {
        self.findings.push(Finding {
            severity,
            title: title.to_string(),
            detail: detail.to_string(),
            action: Some(action),
        });
    }

    /// Render the report as a human-readable string (for display or AI context).
    pub fn to_text(&self) -> String {
        if self.findings.is_empty() {
            return format!("# {}\nNo issues found.", self.title);
        }
        let mut lines = vec![format!("# {}", self.title)];
        for f in &self.findings {
            let icon = match f.severity {
                FindingSeverity::Info => "ℹ",
                FindingSeverity::Warning => "⚠",
                FindingSeverity::Critical => "✖",
            };
            lines.push(format!("{} {}", icon, f.title));
            if !f.detail.is_empty() {
                lines.push(format!("  {}", f.detail));
            }
            if let Some(ref action) = f.action {
                let action_str = match action {
                    SuggestedAction::KillProcess { pid, name, signal } => {
                        format!("  → Kill PID {} ({}) with {}", pid, name, signal)
                    }
                    SuggestedAction::ReniceProcess { pid, name, nice } => {
                        format!("  → Set nice {} for PID {} ({})", nice, pid, name)
                    }
                    SuggestedAction::FreePort { port, pid, name } => {
                        format!("  → Kill PID {} ({}) to free port {}", pid, name, port)
                    }
                    SuggestedAction::CleanDirectory { path, size_bytes } => {
                        format!(
                            "  → Clean {} ({:.1} GB)",
                            path,
                            *size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                        )
                    }
                    SuggestedAction::Info(msg) => format!("  → {}", msg),
                };
                lines.push(action_str);
            }
        }
        lines.join("\n")
    }

    /// Return the highest severity in the report.
    pub fn max_severity(&self) -> Option<FindingSeverity> {
        self.findings.iter().map(|f| f.severity).max()
    }
}

// ── Diagnostic engine ─────────────────────────────────────────────

/// Runs diagnostics against live state and historical data.
pub struct DiagnosticEngine;

impl DiagnosticEngine {
    // ── Resource contention ───────────────────────────────────────

    /// "Why is my system slow?" — Analyze what's competing for resources.
    pub fn resource_contention(
        system: &SystemSnapshot,
        processes: &[ProcessInfo],
    ) -> DiagnosticReport {
        let mut report = DiagnosticReport::new("Resource Contention Analysis");

        // Overall system pressure
        let cpu = system.global_cpu_usage;
        let mem_pct = system.memory_percent();

        if cpu >= 90.0 {
            report.push(
                FindingSeverity::Critical,
                &format!("CPU saturated at {:.0}%", cpu),
                &format!(
                    "Load average: {:.1}/{:.1}/{:.1}",
                    system.load_avg_1, system.load_avg_5, system.load_avg_15
                ),
            );
        } else if cpu >= 70.0 {
            report.push(
                FindingSeverity::Warning,
                &format!("CPU under heavy load: {:.0}%", cpu),
                &format!(
                    "Load average: {:.1}/{:.1}/{:.1}",
                    system.load_avg_1, system.load_avg_5, system.load_avg_15
                ),
            );
        }

        if mem_pct >= 90.0 {
            let swap_pct = system.swap_percent();
            let swap_detail = if swap_pct > 0.0 {
                format!(" Swap usage: {:.0}%", swap_pct)
            } else {
                String::new()
            };
            report.push(
                FindingSeverity::Critical,
                &format!("Memory pressure: {:.0}% used", mem_pct),
                &format!(
                    "{} of {} used.{}",
                    format_bytes(system.used_memory),
                    format_bytes(system.total_memory),
                    swap_detail,
                ),
            );
        } else if mem_pct >= 75.0 {
            report.push(
                FindingSeverity::Warning,
                &format!("Memory usage elevated: {:.0}%", mem_pct),
                &format!(
                    "{} of {} used",
                    format_bytes(system.used_memory),
                    format_bytes(system.total_memory),
                ),
            );
        }

        // Top CPU consumers
        let mut by_cpu: Vec<&ProcessInfo> = processes.iter().collect();
        by_cpu.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top_cpu: Vec<&ProcessInfo> = by_cpu.iter().take(5).copied().collect();
        let cpu_hogs: Vec<&ProcessInfo> = top_cpu
            .iter()
            .filter(|p| p.cpu_usage > 50.0)
            .copied()
            .collect();

        if !cpu_hogs.is_empty() {
            for p in &cpu_hogs {
                let detail = format!(
                    "PID {} | CPU: {:.1}% | MEM: {} | User: {} | Cmd: {}",
                    p.pid,
                    p.cpu_usage,
                    format_bytes(p.memory_bytes),
                    p.user,
                    truncate_cmd(&p.cmd, 80),
                );
                report.push_with_action(
                    if p.cpu_usage > 90.0 {
                        FindingSeverity::Critical
                    } else {
                        FindingSeverity::Warning
                    },
                    &format!("{} consuming {:.0}% CPU", p.name, p.cpu_usage),
                    &detail,
                    SuggestedAction::KillProcess {
                        pid: p.pid,
                        name: p.name.clone(),
                        signal: "SIGTERM",
                    },
                );
            }
        } else if cpu >= 50.0 {
            // No single hog, but system is busy — show top 3
            let summary: Vec<String> = top_cpu
                .iter()
                .take(3)
                .map(|p| format!("{} ({:.0}%)", p.name, p.cpu_usage))
                .collect();
            report.push(
                FindingSeverity::Info,
                "CPU load spread across multiple processes",
                &format!("Top: {}", summary.join(", ")),
            );
        }

        // Top memory consumers
        let mut by_mem: Vec<&ProcessInfo> = processes.iter().collect();
        by_mem.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));

        let mem_hogs: Vec<&ProcessInfo> = by_mem
            .iter()
            .take(5)
            .filter(|p| p.memory_bytes > 1024 * 1024 * 1024) // > 1 GiB
            .copied()
            .collect();

        for p in &mem_hogs {
            let detail = format!(
                "PID {} | MEM: {} ({:.1}%) | CPU: {:.1}% | User: {}",
                p.pid,
                format_bytes(p.memory_bytes),
                p.memory_percent,
                p.cpu_usage,
                p.user,
            );
            report.push_with_action(
                if p.memory_bytes > 4 * 1024 * 1024 * 1024 {
                    FindingSeverity::Warning
                } else {
                    FindingSeverity::Info
                },
                &format!("{} using {}", p.name, format_bytes(p.memory_bytes)),
                &detail,
                SuggestedAction::KillProcess {
                    pid: p.pid,
                    name: p.name.clone(),
                    signal: "SIGTERM",
                },
            );
        }

        // Zombie processes
        let zombies: Vec<&ProcessInfo> = processes
            .iter()
            .filter(|p| p.status == crate::models::ProcessStatus::Zombie)
            .collect();
        if !zombies.is_empty() {
            let names: Vec<String> = zombies
                .iter()
                .take(5)
                .map(|z| format!("{} (PID {})", z.name, z.pid))
                .collect();
            report.push(
                FindingSeverity::Warning,
                &format!("{} zombie process(es)", zombies.len()),
                &names.join(", "),
            );
        }

        if report.findings.is_empty() {
            report.push(
                FindingSeverity::Info,
                "System is healthy",
                &format!(
                    "CPU: {:.0}% | Memory: {:.0}% | No resource contention detected",
                    cpu, mem_pct
                ),
            );
        }

        report
    }

    // ── Timeline / absence report ─────────────────────────────────

    /// "What happened while I was away?" — Summarize recent history.
    pub fn timeline_report(store: &EventStore, minutes: u64) -> DiagnosticReport {
        let mut report = DiagnosticReport::new(&format!("Timeline: Last {} minutes", minutes));

        // Event counts
        let counts = match store.event_counts(minutes) {
            Ok(c) => c,
            Err(_) => {
                report.push(FindingSeverity::Warning, "Could not read event store", "");
                return report;
            }
        };

        let starts = counts.get("process_start").copied().unwrap_or(0);
        let exits = counts.get("process_exit").copied().unwrap_or(0);
        let alerts = counts.get("alert").copied().unwrap_or(0);
        let port_binds = counts.get("port_bind").copied().unwrap_or(0);
        let port_releases = counts.get("port_release").copied().unwrap_or(0);

        // Process churn
        if starts > 0 || exits > 0 {
            let churn_severity = if starts + exits > 100 {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Info
            };
            report.push(
                churn_severity,
                &format!("Process activity: {} started, {} exited", starts, exits),
                &format!("Net change: {:+}", starts as i64 - exits as i64),
            );
        }

        // Alerts
        if alerts > 0 {
            let alert_severity = if alerts > 10 {
                FindingSeverity::Critical
            } else if alerts > 3 {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Info
            };
            report.push(
                alert_severity,
                &format!("{} alert(s) triggered", alerts),
                "Check the Alerts tab for details",
            );
        }

        // Port changes
        if port_binds > 0 || port_releases > 0 {
            report.push(
                FindingSeverity::Info,
                &format!(
                    "Network: {} port bind(s), {} release(s)",
                    port_binds, port_releases
                ),
                "",
            );
        }

        // System resource trend from snapshots
        let since_ms = crate::store::now_epoch_ms_pub() - (minutes as i64 * 60 * 1000);
        if let Ok(snapshots) = store.query_system_history(since_ms) {
            if snapshots.len() >= 2 {
                let first = &snapshots[0];
                let last = &snapshots[snapshots.len() - 1];

                // CPU trend
                let cpu_diff = last.cpu_global - first.cpu_global;
                if cpu_diff.abs() > 20.0 {
                    let direction = if cpu_diff > 0.0 {
                        "increased"
                    } else {
                        "decreased"
                    };
                    report.push(
                        if cpu_diff > 30.0 {
                            FindingSeverity::Warning
                        } else {
                            FindingSeverity::Info
                        },
                        &format!(
                            "CPU {} by {:.0} percentage points",
                            direction,
                            cpu_diff.abs()
                        ),
                        &format!("{:.0}% → {:.0}%", first.cpu_global, last.cpu_global),
                    );
                }

                // Memory trend
                if first.mem_total > 0 && last.mem_total > 0 {
                    let mem_pct_first = (first.mem_used as f64 / first.mem_total as f64) * 100.0;
                    let mem_pct_last = (last.mem_used as f64 / last.mem_total as f64) * 100.0;
                    let mem_diff = mem_pct_last - mem_pct_first;
                    if mem_diff.abs() > 10.0 {
                        let direction = if mem_diff > 0.0 {
                            "increased"
                        } else {
                            "decreased"
                        };
                        report.push(
                            if mem_diff > 20.0 {
                                FindingSeverity::Warning
                            } else {
                                FindingSeverity::Info
                            },
                            &format!(
                                "Memory {} by {:.0} percentage points",
                                direction,
                                mem_diff.abs()
                            ),
                            &format!("{:.0}% → {:.0}%", mem_pct_first, mem_pct_last),
                        );
                    }
                }

                // Peak detection
                let peak_cpu = snapshots
                    .iter()
                    .map(|s| s.cpu_global)
                    .fold(0.0f32, f32::max);
                if peak_cpu > 90.0 {
                    report.push(
                        FindingSeverity::Warning,
                        &format!("CPU peaked at {:.0}%", peak_cpu),
                        "A CPU spike was detected during this period",
                    );
                }
            }
        }

        if report.findings.is_empty() {
            report.push(
                FindingSeverity::Info,
                "Quiet period",
                "No significant activity detected",
            );
        }

        report
    }

    // ── Port diagnostics ──────────────────────────────────────────

    /// "What's using port X?" — Port investigation.
    pub fn port_diagnosis(store: &EventStore, port: u16) -> DiagnosticReport {
        let mut report = DiagnosticReport::new(&format!("Port {} Diagnosis", port));

        // Current listeners on this port
        if let Ok(listeners) = store.query_current_listeners() {
            let on_port: Vec<_> = listeners.iter().filter(|s| s.local_port == port).collect();
            if on_port.is_empty() {
                report.push(
                    FindingSeverity::Info,
                    &format!("Port {} is not currently in use", port),
                    "",
                );
            } else {
                for s in &on_port {
                    let pid_info = match (s.pid, &s.name) {
                        (Some(pid), Some(name)) => format!("PID {} ({})", pid, name),
                        (Some(pid), None) => format!("PID {}", pid),
                        _ => "unknown process".to_string(),
                    };
                    report.push_with_action(
                        FindingSeverity::Info,
                        &format!("Port {} bound by {}", port, pid_info),
                        &format!(
                            "Protocol: {} | Address: {} | State: {}",
                            s.protocol, s.local_addr, s.state
                        ),
                        SuggestedAction::FreePort {
                            port,
                            pid: s.pid.unwrap_or(0),
                            name: s.name.clone().unwrap_or_default(),
                        },
                    );
                }
            }
        }

        // Port bind/release history
        let since = crate::store::now_epoch_ms_pub() - (24 * 3600 * 1000); // last 24h
        if let Ok(history) = store.query_port_history(port, since) {
            let unique_pids: std::collections::HashSet<Option<u32>> =
                history.iter().map(|h| h.pid).collect();
            if unique_pids.len() > 1 {
                report.push(
                    FindingSeverity::Warning,
                    &format!(
                        "Port {} was used by {} different processes in 24h",
                        port,
                        unique_pids.len()
                    ),
                    "This may indicate port contention or service restarts",
                );
            }
            if !history.is_empty() {
                report.push(
                    FindingSeverity::Info,
                    &format!("{} socket record(s) in history", history.len()),
                    "",
                );
            }
        }

        report
    }

    // ── Process analysis ──────────────────────────────────────────

    /// "Tell me about PID X" — Deep process investigation.
    pub fn process_analysis(
        store: &EventStore,
        pid: u32,
        current: Option<&ProcessInfo>,
    ) -> DiagnosticReport {
        let name = current.map(|p| p.name.as_str()).unwrap_or("unknown");
        let mut report =
            DiagnosticReport::new(&format!("Process Analysis: {} (PID {})", name, pid));

        // Current state
        if let Some(p) = current {
            report.push(
                FindingSeverity::Info,
                "Current state",
                &format!(
                    "CPU: {:.1}% | Memory: {} ({:.1}%) | Status: {} | User: {} | Cmd: {}",
                    p.cpu_usage,
                    format_bytes(p.memory_bytes),
                    p.memory_percent,
                    p.status,
                    p.user,
                    truncate_cmd(&p.cmd, 100),
                ),
            );

            // Flag high resource usage
            if p.cpu_usage > 80.0 {
                report.push_with_action(
                    FindingSeverity::Warning,
                    &format!("High CPU usage: {:.1}%", p.cpu_usage),
                    "This process is consuming significant CPU time",
                    SuggestedAction::ReniceProcess {
                        pid,
                        name: p.name.clone(),
                        nice: 10,
                    },
                );
            }
            if p.memory_bytes > 2 * 1024 * 1024 * 1024 {
                report.push(
                    FindingSeverity::Warning,
                    &format!("High memory usage: {}", format_bytes(p.memory_bytes)),
                    "This process is using more than 2 GiB of RAM",
                );
            }
        } else {
            report.push(
                FindingSeverity::Warning,
                "Process not currently running",
                "",
            );
        }

        // Historical resource trend
        let since = crate::store::now_epoch_ms_pub() - (60 * 60 * 1000); // last 1h
        if let Ok(history) = store.query_process_history(pid, since) {
            if history.len() >= 2 {
                let first = &history[0];
                let last = &history[history.len() - 1];

                // Memory trend (potential leak)
                if first.mem_bytes > 0 && last.mem_bytes > 0 {
                    let growth = (last.mem_bytes as f64 - first.mem_bytes as f64)
                        / first.mem_bytes as f64
                        * 100.0;
                    if growth > 50.0 && last.mem_bytes > 100 * 1024 * 1024 {
                        report.push(
                            FindingSeverity::Critical,
                            &format!("Memory grew {:.0}% in the last hour", growth),
                            &format!(
                                "{} → {} — possible memory leak",
                                format_bytes(first.mem_bytes),
                                format_bytes(last.mem_bytes),
                            ),
                        );
                    } else if growth > 20.0 && last.mem_bytes > 100 * 1024 * 1024 {
                        report.push(
                            FindingSeverity::Warning,
                            &format!("Memory grew {:.0}% in the last hour", growth),
                            &format!(
                                "{} → {}",
                                format_bytes(first.mem_bytes),
                                format_bytes(last.mem_bytes),
                            ),
                        );
                    }
                }

                // CPU trend
                let avg_cpu: f32 =
                    history.iter().map(|h| h.cpu).sum::<f32>() / history.len() as f32;
                let peak_cpu = history.iter().map(|h| h.cpu).fold(0.0f32, f32::max);
                if peak_cpu > 0.0 {
                    report.push(
                        FindingSeverity::Info,
                        "Historical CPU usage",
                        &format!(
                            "Average: {:.1}% | Peak: {:.1}% | Samples: {}",
                            avg_cpu,
                            peak_cpu,
                            history.len()
                        ),
                    );
                }

                report.push(
                    FindingSeverity::Info,
                    "Tracking duration",
                    &format!(
                        "{} data points over {} minutes",
                        history.len(),
                        (last.ts - first.ts) / 60000,
                    ),
                );
            } else if history.is_empty() {
                report.push(
                    FindingSeverity::Info,
                    "No historical data",
                    "Process may be new or short-lived",
                );
            }
        }

        report
    }

    // ── Anomaly detection from history ────────────────────────────

    /// Detect anomalies in the recent system timeline.
    pub fn anomaly_scan(store: &EventStore, minutes: u64) -> DiagnosticReport {
        let mut report = DiagnosticReport::new(&format!("Anomaly Scan: Last {} minutes", minutes));
        let since_ms = crate::store::now_epoch_ms_pub() - (minutes as i64 * 60 * 1000);

        // CPU spike detection
        if let Ok(snapshots) = store.query_system_history(since_ms) {
            if snapshots.len() >= 5 {
                let avg_cpu: f32 =
                    snapshots.iter().map(|s| s.cpu_global).sum::<f32>() / snapshots.len() as f32;
                let std_dev = (snapshots
                    .iter()
                    .map(|s| (s.cpu_global - avg_cpu).powi(2))
                    .sum::<f32>()
                    / snapshots.len() as f32)
                    .sqrt();

                // Find spikes (> 2 standard deviations above mean)
                let spike_threshold = avg_cpu + 2.0 * std_dev;
                let spikes: Vec<_> = snapshots
                    .iter()
                    .filter(|s| s.cpu_global > spike_threshold && s.cpu_global > 70.0)
                    .collect();

                if !spikes.is_empty() {
                    let peak = spikes.iter().map(|s| s.cpu_global).fold(0.0f32, f32::max);
                    report.push(
                        FindingSeverity::Warning,
                        &format!(
                            "{} CPU spike(s) detected (peak: {:.0}%)",
                            spikes.len(),
                            peak
                        ),
                        &format!(
                            "Average: {:.0}% | Std dev: {:.1} | Spike threshold: {:.0}%",
                            avg_cpu, std_dev, spike_threshold
                        ),
                    );
                }

                // Memory trend — sustained increase
                if snapshots.len() >= 10 {
                    let first_10_avg: f64 = snapshots[..10]
                        .iter()
                        .filter(|s| s.mem_total > 0)
                        .map(|s| s.mem_used as f64 / s.mem_total as f64 * 100.0)
                        .sum::<f64>()
                        / 10.0;
                    let last_10_avg: f64 = snapshots[snapshots.len() - 10..]
                        .iter()
                        .filter(|s| s.mem_total > 0)
                        .map(|s| s.mem_used as f64 / s.mem_total as f64 * 100.0)
                        .sum::<f64>()
                        / 10.0;

                    let mem_drift = last_10_avg - first_10_avg;
                    if mem_drift > 15.0 {
                        report.push(
                            FindingSeverity::Warning,
                            &format!(
                                "Sustained memory increase: {:.0}% → {:.0}%",
                                first_10_avg, last_10_avg
                            ),
                            "Memory usage has been steadily rising — possible system-wide leak",
                        );
                    }
                }
            }
        }

        // Process churn anomaly (unusually high start/exit rate)
        if let Ok(counts) = store.event_counts(minutes) {
            let starts = counts.get("process_start").copied().unwrap_or(0);
            let exits = counts.get("process_exit").copied().unwrap_or(0);
            let total_churn = starts + exits;
            let churn_per_minute = if minutes > 0 {
                total_churn / minutes
            } else {
                total_churn
            };

            // More than ~5 process events per minute is noteworthy
            if churn_per_minute > 20 {
                report.push(
                    FindingSeverity::Warning,
                    &format!("High process churn: {} events/min", churn_per_minute),
                    &format!(
                        "{} starts + {} exits in {} min — possible crash loop or respawn storm",
                        starts, exits, minutes
                    ),
                );
            } else if churn_per_minute > 5 {
                report.push(
                    FindingSeverity::Info,
                    &format!("Moderate process churn: {} events/min", churn_per_minute),
                    &format!("{} starts, {} exits", starts, exits),
                );
            }
        }

        if report.findings.is_empty() {
            report.push(
                FindingSeverity::Info,
                "No anomalies detected",
                "System behavior appears normal",
            );
        }

        report
    }

    // ── Disk usage analysis ───────────────────────────────────────

    /// Analyze disk usage and find cleanup candidates.
    pub fn disk_analysis(system: &SystemSnapshot) -> DiagnosticReport {
        let mut report = DiagnosticReport::new("Disk Usage Analysis");

        for disk in &system.disks {
            let used = disk.total_space.saturating_sub(disk.available_space);
            let pct = if disk.total_space > 0 {
                (used as f64 / disk.total_space as f64) * 100.0
            } else {
                0.0
            };

            if pct >= 95.0 {
                report.push(
                    FindingSeverity::Critical,
                    &format!("{} is {:.0}% full", disk.mount_point, pct),
                    &format!(
                        "{} used of {} (only {} free)",
                        format_bytes(used),
                        format_bytes(disk.total_space),
                        format_bytes(disk.available_space),
                    ),
                );
            } else if pct >= 85.0 {
                report.push(
                    FindingSeverity::Warning,
                    &format!("{} is {:.0}% full", disk.mount_point, pct),
                    &format!(
                        "{} free of {}",
                        format_bytes(disk.available_space),
                        format_bytes(disk.total_space),
                    ),
                );
            } else {
                report.push(
                    FindingSeverity::Info,
                    &format!("{}: {:.0}% used", disk.mount_point, pct),
                    &format!("{} free", format_bytes(disk.available_space)),
                );
            }

            // High I/O
            if disk.read_bytes_per_sec > 100 * 1024 * 1024
                || disk.write_bytes_per_sec > 100 * 1024 * 1024
            {
                report.push(
                    FindingSeverity::Warning,
                    &format!("Heavy I/O on {}", disk.mount_point),
                    &format!(
                        "Read: {}/s | Write: {}/s",
                        format_bytes(disk.read_bytes_per_sec),
                        format_bytes(disk.write_bytes_per_sec),
                    ),
                );
            }
        }

        // Scan for large cleanup candidates
        let cleanup_dirs = scan_cleanup_candidates();
        for (path, size) in &cleanup_dirs {
            if *size > 1024 * 1024 * 1024 {
                report.push_with_action(
                    FindingSeverity::Info,
                    &format!("{}: {}", path, format_bytes(*size)),
                    "Potential cleanup candidate",
                    SuggestedAction::CleanDirectory {
                        path: path.clone(),
                        size_bytes: *size,
                    },
                );
            }
        }

        if report.findings.is_empty() {
            report.push(
                FindingSeverity::Info,
                "Disk usage is healthy",
                "No issues detected",
            );
        }

        report
    }

    // ── Combined report for AI context ────────────────────────────

    /// Build a comprehensive diagnostic summary for AI context enrichment.
    pub fn full_context_report(
        system: &SystemSnapshot,
        processes: &[ProcessInfo],
        _alerts: &[Alert],
        store: &EventStore,
    ) -> String {
        let mut sections = Vec::new();

        // Resource contention (always run)
        let contention = Self::resource_contention(system, processes);
        if contention.max_severity().unwrap_or(FindingSeverity::Info) >= FindingSeverity::Warning {
            sections.push(contention.to_text());
        }

        // Recent timeline (last 15 min)
        let timeline = Self::timeline_report(store, 15);
        if !timeline.findings.is_empty() {
            sections.push(timeline.to_text());
        }

        // Anomaly scan (last 30 min)
        let anomalies = Self::anomaly_scan(store, 30);
        if anomalies.max_severity().unwrap_or(FindingSeverity::Info) >= FindingSeverity::Warning {
            sections.push(anomalies.to_text());
        }

        // Port listeners summary
        if let Ok(listeners) = store.query_current_listeners() {
            if !listeners.is_empty() {
                let mut lines = vec!["# Active Listeners".to_string()];
                for s in listeners.iter().take(20) {
                    let who = match (&s.pid, &s.name) {
                        (Some(pid), Some(name)) => format!("{} (PID {})", name, pid),
                        (Some(pid), None) => format!("PID {}", pid),
                        _ => "unknown".to_string(),
                    };
                    lines.push(format!("  {}:{} ← {}", s.local_addr, s.local_port, who));
                }
                sections.push(lines.join("\n"));
            }
        }

        if sections.is_empty() {
            "No diagnostic findings.".to_string()
        } else {
            sections.join("\n\n")
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    crate::models::format_bytes(bytes)
}

fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        cmd.to_string()
    } else {
        format!("{}...", &cmd[..max.saturating_sub(3)])
    }
}

/// Scan common directories for cleanup candidates.
/// Returns Vec<(path, size_bytes)> sorted by size descending.
fn scan_cleanup_candidates() -> Vec<(String, u64)> {
    let home = crate::constants::home_dir();
    let candidates = [
        home.join(".cache"),
        home.join(".local/share/Trash"),
        home.join("Downloads"),
        home.join("tmp"),
        std::path::PathBuf::from("/var/log"),
        std::path::PathBuf::from("/tmp"),
        std::path::PathBuf::from("/var/cache"),
    ];

    let mut results: Vec<(String, u64)> = Vec::new();
    for path in &candidates {
        if path.is_dir() {
            if let Ok(size) = dir_size(path) {
                if size > 100 * 1024 * 1024 {
                    // Only report > 100 MB
                    results.push((path.to_string_lossy().to_string(), size));
                }
            }
        }
    }
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results
}

/// Calculate total size of a directory (non-recursive depth limit for safety).
fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    // Only scan top-level entries to keep it fast
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            // One level deep to get a rough size
            if let Ok(subdir) = std::fs::read_dir(entry.path()) {
                for sub_entry in subdir.flatten() {
                    if let Ok(sub_meta) = sub_entry.metadata() {
                        if sub_meta.is_file() {
                            total += sub_meta.len();
                        }
                    }
                }
            }
        }
    }
    Ok(total)
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn make_system(cpu: f32, mem_used: u64, mem_total: u64) -> SystemSnapshot {
        SystemSnapshot {
            total_memory: mem_total,
            used_memory: mem_used,
            total_swap: 0,
            used_swap: 0,
            cpu_count: 4,
            cpu_usages: vec![cpu; 4],
            global_cpu_usage: cpu,
            uptime: 3600,
            hostname: "test".to_string(),
            os_name: "Linux".to_string(),
            load_avg_1: cpu as f64 / 25.0,
            load_avg_5: cpu as f64 / 30.0,
            load_avg_15: cpu as f64 / 35.0,
            total_processes: 100,
            networks: vec![],
            disks: vec![],
            cpu_temp: None,
            gpu: None,
            battery: None,
        }
    }

    fn make_process(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: format!("/usr/bin/{}", name),
            cpu_usage: cpu,
            memory_bytes: mem,
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

    // ── DiagnosticReport ──────────────────────────────────────────

    #[test]
    fn report_to_text_empty() {
        let r = DiagnosticReport::new("Test");
        assert!(r.to_text().contains("No issues found"));
    }

    #[test]
    fn report_to_text_with_findings() {
        let mut r = DiagnosticReport::new("Test");
        r.push(FindingSeverity::Warning, "High CPU", "90% usage");
        let text = r.to_text();
        assert!(text.contains("High CPU"));
        assert!(text.contains("90% usage"));
    }

    #[test]
    fn report_max_severity() {
        let mut r = DiagnosticReport::new("Test");
        r.push(FindingSeverity::Info, "a", "");
        r.push(FindingSeverity::Critical, "b", "");
        r.push(FindingSeverity::Warning, "c", "");
        assert_eq!(r.max_severity(), Some(FindingSeverity::Critical));
    }

    #[test]
    fn report_max_severity_empty() {
        let r = DiagnosticReport::new("Test");
        assert_eq!(r.max_severity(), None);
    }

    // ── Resource contention ───────────────────────────────────────

    #[test]
    fn contention_healthy_system() {
        let sys = make_system(20.0, 4_000_000_000, 16_000_000_000);
        let procs = vec![make_process(1, "idle", 1.0, 1024)];
        let report = DiagnosticEngine::resource_contention(&sys, &procs);
        assert_eq!(report.max_severity(), Some(FindingSeverity::Info));
        assert!(report.to_text().contains("healthy"));
    }

    #[test]
    fn contention_high_cpu() {
        let sys = make_system(95.0, 4_000_000_000, 16_000_000_000);
        let procs = vec![
            make_process(1, "hog", 92.0, 1024),
            make_process(2, "idle", 1.0, 1024),
        ];
        let report = DiagnosticEngine::resource_contention(&sys, &procs);
        assert_eq!(report.max_severity(), Some(FindingSeverity::Critical));
    }

    #[test]
    fn contention_high_memory() {
        let total = 16_000_000_000u64;
        let used = 15_000_000_000u64; // ~94%
        let sys = make_system(20.0, used, total);
        let procs = vec![make_process(1, "big", 5.0, 12_000_000_000)];
        let report = DiagnosticEngine::resource_contention(&sys, &procs);
        assert!(report.max_severity().unwrap() >= FindingSeverity::Warning);
    }

    #[test]
    fn contention_detects_zombies() {
        let sys = make_system(20.0, 4_000_000_000, 16_000_000_000);
        let mut zombie = make_process(42, "defunct", 0.0, 0);
        zombie.status = ProcessStatus::Zombie;
        let procs = vec![zombie];
        let report = DiagnosticEngine::resource_contention(&sys, &procs);
        assert!(report.to_text().contains("zombie"));
    }

    // ── Timeline report ───────────────────────────────────────────

    #[test]
    fn timeline_empty_store() {
        let store = EventStore::open(None).unwrap();
        let report = DiagnosticEngine::timeline_report(&store, 60);
        assert!(!report.findings.is_empty()); // Should at least say "quiet period"
    }

    #[test]
    fn timeline_with_events() {
        let store = EventStore::open(None).unwrap();
        store
            .insert_event(
                crate::store::EventKind::ProcessStart,
                Some(1),
                Some("nginx"),
                None,
                None,
            )
            .unwrap();
        store
            .insert_event(
                crate::store::EventKind::Alert,
                Some(2),
                Some("test"),
                None,
                None,
            )
            .unwrap();
        let report = DiagnosticEngine::timeline_report(&store, 60);
        assert!(report.to_text().contains("started"));
        assert!(report.to_text().contains("alert"));
    }

    // ── Port diagnosis ────────────────────────────────────────────

    #[test]
    fn port_diagnosis_empty() {
        let store = EventStore::open(None).unwrap();
        let report = DiagnosticEngine::port_diagnosis(&store, 8080);
        assert!(report.to_text().contains("not currently in use"));
    }

    // ── Process analysis ──────────────────────────────────────────

    #[test]
    fn process_analysis_with_current() {
        let store = EventStore::open(None).unwrap();
        let p = make_process(42, "nginx", 25.0, 500_000_000);
        let report = DiagnosticEngine::process_analysis(&store, 42, Some(&p));
        assert!(report.to_text().contains("nginx"));
        assert!(report.to_text().contains("Current state"));
    }

    #[test]
    fn process_analysis_not_running() {
        let store = EventStore::open(None).unwrap();
        let report = DiagnosticEngine::process_analysis(&store, 99999, None);
        assert!(report.to_text().contains("not currently running"));
    }

    #[test]
    fn process_analysis_high_cpu_suggests_renice() {
        let store = EventStore::open(None).unwrap();
        let p = make_process(42, "compile", 95.0, 500_000_000);
        let report = DiagnosticEngine::process_analysis(&store, 42, Some(&p));
        let has_renice = report
            .findings
            .iter()
            .any(|f| matches!(f.action, Some(SuggestedAction::ReniceProcess { .. })));
        assert!(has_renice, "Should suggest renice for high-CPU process");
    }

    // ── Anomaly scan ──────────────────────────────────────────────

    #[test]
    fn anomaly_scan_empty() {
        let store = EventStore::open(None).unwrap();
        let report = DiagnosticEngine::anomaly_scan(&store, 30);
        assert!(report.to_text().contains("No anomalies"));
    }

    // ── Disk analysis ─────────────────────────────────────────────

    #[test]
    fn disk_analysis_with_disks() {
        let mut sys = make_system(20.0, 4_000_000_000, 16_000_000_000);
        sys.disks = vec![DiskInfo {
            mount_point: "/".to_string(),
            fs_type: "ext4".to_string(),
            total_space: 500_000_000_000,
            available_space: 100_000_000_000,
            disk_kind: "SSD".to_string(),
            read_bytes_per_sec: 0,
            write_bytes_per_sec: 0,
        }];
        let report = DiagnosticEngine::disk_analysis(&sys);
        assert!(report.to_text().contains("/"));
    }

    #[test]
    fn disk_analysis_nearly_full() {
        let mut sys = make_system(20.0, 4_000_000_000, 16_000_000_000);
        sys.disks = vec![DiskInfo {
            mount_point: "/".to_string(),
            fs_type: "ext4".to_string(),
            total_space: 500_000_000_000,
            available_space: 10_000_000_000, // 2% free = 98% full
            disk_kind: "SSD".to_string(),
            read_bytes_per_sec: 0,
            write_bytes_per_sec: 0,
        }];
        let report = DiagnosticEngine::disk_analysis(&sys);
        assert_eq!(report.max_severity(), Some(FindingSeverity::Critical));
    }

    // ── Helpers ───────────────────────────────────────────────────

    #[test]
    fn truncate_cmd_short() {
        assert_eq!(truncate_cmd("hello", 10), "hello");
    }

    #[test]
    fn truncate_cmd_long() {
        let long = "a".repeat(100);
        let result = truncate_cmd(&long, 20);
        assert!(result.len() <= 20);
        assert!(result.ends_with("..."));
    }
}
