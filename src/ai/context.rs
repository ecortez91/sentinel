use crate::models::{format_bytes, Alert, AlertSeverity, ProcessInfo, SystemSnapshot};

/// Builds a rich system context string from live data for the LLM.
///
/// Strategy Pattern: different context "profiles" could be swapped in,
/// but KISS says one good one is better than three mediocre ones.
pub struct ContextBuilder;

impl ContextBuilder {
    /// Serialize the current system state into a structured prompt context.
    /// This is what gives Claude "eyes" into your machine.
    pub fn build(
        system: Option<&SystemSnapshot>,
        processes: &[ProcessInfo],
        alerts: &[Alert],
    ) -> String {
        let mut ctx = String::with_capacity(8192);

        ctx.push_str("=== LIVE SYSTEM STATE ===\n\n");

        // ── System Overview ────────────────────────────────────
        if let Some(sys) = system {
            ctx.push_str("## System Overview\n");
            ctx.push_str(&format!("Hostname: {}\n", sys.hostname));
            ctx.push_str(&format!("OS: {}\n", sys.os_name));
            ctx.push_str(&format!("CPUs: {} cores\n", sys.cpu_count));
            ctx.push_str(&format!(
                "Uptime: {}h {}m\n",
                sys.uptime / 3600,
                (sys.uptime % 3600) / 60
            ));
            ctx.push_str(&format!(
                "CPU Usage: {:.1}% (global)\n",
                sys.global_cpu_usage
            ));

            // Per-core breakdown
            ctx.push_str("Per-core CPU: ");
            for (i, &usage) in sys.cpu_usages.iter().enumerate() {
                if i > 0 {
                    ctx.push_str(", ");
                }
                ctx.push_str(&format!("C{}: {:.0}%", i, usage));
            }
            ctx.push('\n');

            ctx.push_str(&format!(
                "Memory: {} / {} ({:.1}% used)\n",
                format_bytes(sys.used_memory),
                format_bytes(sys.total_memory),
                sys.memory_percent()
            ));
            ctx.push_str(&format!(
                "Swap: {} / {} ({:.1}% used)\n",
                format_bytes(sys.used_swap),
                format_bytes(sys.total_swap),
                sys.swap_percent()
            ));
            ctx.push_str(&format!(
                "Load Average: 1m={:.2} 5m={:.2} 15m={:.2}\n",
                sys.load_avg_1, sys.load_avg_5, sys.load_avg_15
            ));
            ctx.push_str(&format!("Total Processes: {}\n", sys.total_processes));

            // CPU Temperature
            if let Some(ref temp) = sys.cpu_temp {
                if let Some(pkg) = temp.package_temp {
                    ctx.push_str(&format!("CPU Temperature: {:.0}°C\n", pkg));
                }
                if !temp.core_temps.is_empty() {
                    ctx.push_str("Per-core Temps: ");
                    for (i, &t) in temp.core_temps.iter().enumerate() {
                        if i > 0 {
                            ctx.push_str(", ");
                        }
                        ctx.push_str(&format!("C{}: {:.0}°C", i, t));
                    }
                    ctx.push('\n');
                }
            }

            // GPU
            if let Some(ref gpu) = sys.gpu {
                ctx.push_str(&format!(
                    "GPU: {} | Util: {}% | VRAM: {}/{} | Temp: {}°C | Power: {:.0}W\n",
                    gpu.name,
                    gpu.utilization,
                    format_bytes(gpu.memory_used),
                    format_bytes(gpu.memory_total),
                    gpu.temperature,
                    gpu.power_draw,
                ));
            }

            // Battery
            if let Some(ref bat) = sys.battery {
                ctx.push_str(&format!("Battery: {:.0}% ({:?})", bat.percent, bat.status,));
                if let Some(ref t) = bat.time_remaining {
                    ctx.push_str(&format!(" - {}", t));
                }
                ctx.push('\n');
            }

            ctx.push('\n');
        }

        // ── Top Processes by CPU ───────────────────────────────
        ctx.push_str("## Top 25 Processes by CPU Usage\n");
        ctx.push_str(&format!(
            "{:<8} {:<25} {:>7} {:>12} {:>7} {:>10} {:>10} {:<10} {}\n",
            "PID", "NAME", "CPU%", "MEMORY", "MEM%", "DISK_R", "DISK_W", "STATUS", "COMMAND"
        ));
        ctx.push_str(&"-".repeat(110));
        ctx.push('\n');

        let mut by_cpu = processes.to_vec();
        by_cpu.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for p in by_cpu.iter().take(25) {
            ctx.push_str(&format!(
                "{:<8} {:<25} {:>6.1}% {:>12} {:>6.1}% {:>10} {:>10} {:<10} {}\n",
                p.pid,
                truncate(&p.name, 25),
                p.cpu_usage,
                p.memory_display(),
                p.memory_percent,
                p.disk_read_display(),
                p.disk_write_display(),
                p.status.to_string(),
                truncate(&p.cmd, 80),
            ));
        }
        ctx.push('\n');

        // ── Top Processes by Memory ────────────────────────────
        ctx.push_str("## Top 15 Processes by Memory Usage\n");
        ctx.push_str(&format!(
            "{:<8} {:<25} {:>12} {:>7} {:>7}\n",
            "PID", "NAME", "MEMORY", "MEM%", "CPU%"
        ));
        ctx.push_str(&"-".repeat(70));
        ctx.push('\n');

        let mut by_mem = processes.to_vec();
        by_mem.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));

        for p in by_mem.iter().take(15) {
            ctx.push_str(&format!(
                "{:<8} {:<25} {:>12} {:>6.1}% {:>6.1}%\n",
                p.pid,
                truncate(&p.name, 25),
                p.memory_display(),
                p.memory_percent,
                p.cpu_usage,
            ));
        }
        ctx.push('\n');

        // ── Process Groups (aggregate by name) ─────────────────
        ctx.push_str("## Process Groups (aggregated by name)\n");
        let groups = aggregate_by_name(processes);
        ctx.push_str(&format!(
            "{:<25} {:>6} {:>8} {:>12}\n",
            "NAME", "COUNT", "CPU%", "MEMORY"
        ));
        ctx.push_str(&"-".repeat(55));
        ctx.push('\n');
        for (name, count, cpu, mem) in groups.iter().take(20) {
            ctx.push_str(&format!(
                "{:<25} {:>6} {:>7.1}% {:>12}\n",
                truncate(name, 25),
                count,
                cpu,
                format_bytes(*mem),
            ));
        }
        ctx.push('\n');

        // ── Active Alerts ──────────────────────────────────────
        if !alerts.is_empty() {
            ctx.push_str("## Active Alerts (most recent first)\n");
            for (i, a) in alerts.iter().take(30).enumerate() {
                ctx.push_str(&format!(
                    "{}. [{}][{}] {} (PID:{}) at {}\n",
                    i + 1,
                    a.severity,
                    a.category,
                    a.message,
                    a.pid,
                    a.timestamp.format("%H:%M:%S"),
                ));
            }
            ctx.push('\n');

            // Alert summary
            let danger = alerts
                .iter()
                .filter(|a| a.severity == AlertSeverity::Danger)
                .count();
            let critical = alerts
                .iter()
                .filter(|a| a.severity == AlertSeverity::Critical)
                .count();
            let warning = alerts
                .iter()
                .filter(|a| a.severity == AlertSeverity::Warning)
                .count();
            let info = alerts
                .iter()
                .filter(|a| a.severity == AlertSeverity::Info)
                .count();
            ctx.push_str(&format!(
                "Alert Summary: {} danger, {} critical, {} warning, {} info\n\n",
                danger, critical, warning, info
            ));
        } else {
            ctx.push_str("## Alerts: None - system appears healthy\n\n");
        }

        // ── Network I/O ────────────────────────────────────────
        if let Some(sys) = system {
            if !sys.networks.is_empty() {
                ctx.push_str("## Network Interfaces\n");
                ctx.push_str(&format!(
                    "{:<16} {:>12} {:>12} {:>14} {:>14}\n",
                    "INTERFACE", "RX/s", "TX/s", "TOTAL_RX", "TOTAL_TX"
                ));
                ctx.push_str(&"-".repeat(72));
                ctx.push('\n');
                let mut nets: Vec<_> = sys
                    .networks
                    .iter()
                    .filter(|n| n.total_rx + n.total_tx > 0)
                    .collect();
                nets.sort_by(|a, b| (b.total_rx + b.total_tx).cmp(&(a.total_rx + a.total_tx)));
                for n in nets.iter().take(10) {
                    ctx.push_str(&format!(
                        "{:<16} {:>12} {:>12} {:>14} {:>14}\n",
                        truncate(&n.name, 16),
                        format_bytes(n.rx_bytes),
                        format_bytes(n.tx_bytes),
                        format_bytes(n.total_rx),
                        format_bytes(n.total_tx),
                    ));
                }
                ctx.push('\n');
            }

            if !sys.disks.is_empty() {
                ctx.push_str("## Filesystems\n");
                ctx.push_str(&format!(
                    "{:<20} {:>12} {:>12} {:>7} {:<10}\n",
                    "MOUNT", "USED", "TOTAL", "USE%", "FS_TYPE"
                ));
                ctx.push_str(&"-".repeat(65));
                ctx.push('\n');
                for d in &sys.disks {
                    let used = d.total_space - d.available_space;
                    let pct = if d.total_space > 0 {
                        (used as f64 / d.total_space as f64) * 100.0
                    } else {
                        0.0
                    };
                    ctx.push_str(&format!(
                        "{:<20} {:>12} {:>12} {:>6.1}% {:<10}\n",
                        truncate(&d.mount_point, 20),
                        format_bytes(used),
                        format_bytes(d.total_space),
                        pct,
                        d.fs_type,
                    ));
                }
                ctx.push('\n');
            }
        }

        // ── Special Interest: tokio, node, claude patterns ─────
        let interesting: Vec<&ProcessInfo> = processes
            .iter()
            .filter(|p| {
                let n = p.name.to_lowercase();
                let c = p.cmd.to_lowercase();
                n.contains("tokio")
                    || n.contains("node")
                    || n.contains("claude")
                    || n.contains("cargo")
                    || n.contains("rustc")
                    || n.contains("python")
                    || n.contains("docker")
                    || n.contains("code")
                    || n.contains("worker")
                    || c.contains("tokio")
                    || c.contains("claude")
                    || c.contains("node")
                    || c.contains("worker")
            })
            .collect();

        if !interesting.is_empty() {
            ctx.push_str("## Developer-Relevant Processes (tokio, node, cargo, claude, docker, vscode, workers)\n");
            for p in &interesting {
                ctx.push_str(&format!(
                    "  PID:{} name={} cpu={:.1}% mem={} cmd={}\n",
                    p.pid,
                    p.name,
                    p.cpu_usage,
                    p.memory_display(),
                    truncate(&p.cmd, 120),
                ));
            }
            ctx.push('\n');
        }

        ctx
    }
}

/// Group processes by name: (name, count, total_cpu, total_memory)
fn aggregate_by_name(processes: &[ProcessInfo]) -> Vec<(String, usize, f32, u64)> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, (usize, f32, u64)> = HashMap::new();
    for p in processes {
        let entry = groups.entry(p.name.clone()).or_insert((0, 0.0, 0));
        entry.0 += 1;
        entry.1 += p.cpu_usage;
        entry.2 += p.memory_bytes;
    }
    let mut sorted: Vec<_> = groups
        .into_iter()
        .map(|(name, (count, cpu, mem))| (name, count, cpu, mem))
        .collect();
    sorted.sort_by(|a, b| b.3.cmp(&a.3)); // sort by total memory desc
    sorted
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
