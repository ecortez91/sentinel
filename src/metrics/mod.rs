//! Prometheus metrics exporter for Sentinel.
//!
//! When enabled via `--prometheus <addr>`, runs a tiny HTTP server that exposes
//! system metrics in Prometheus text exposition format at `/metrics`.
//!
//! The main loop updates a shared `MetricsSnapshot` on each tick, and the HTTP
//! server reads it on each scrape request.

use std::sync::{Arc, Mutex};

use crate::models::{Alert, AlertSeverity, SystemSnapshot};
use crate::monitor::ContainerInfo;

/// Shared state that the main loop writes and the HTTP server reads.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub system: Option<SystemSnapshot>,
    pub process_count: usize,
    pub alerts: Vec<Alert>,
    pub containers: Vec<ContainerInfo>,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            system: None,
            process_count: 0,
            alerts: Vec::new(),
            containers: Vec::new(),
        }
    }
}

/// Thread-safe handle to the metrics state.
pub type SharedMetrics = Arc<Mutex<MetricsSnapshot>>;

/// Start the Prometheus metrics HTTP server on a background thread.
///
/// Returns the `SharedMetrics` handle that the caller must update each tick.
/// The server responds to `GET /metrics` with Prometheus text format.
/// All other paths return 404.
pub fn start_server(addr: &str) -> Result<SharedMetrics, String> {
    let server = tiny_http::Server::http(addr)
        .map_err(|e| format!("Failed to bind Prometheus on {}: {}", addr, e))?;

    let metrics: SharedMetrics = Arc::new(Mutex::new(MetricsSnapshot::default()));
    let metrics_clone = Arc::clone(&metrics);

    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let response_text = if request.url() == "/metrics" {
                let snap = metrics_clone.lock().unwrap().clone();
                render_metrics(&snap)
            } else {
                "404 Not Found\n".to_string()
            };

            let status_code = if request.url() == "/metrics" {
                200
            } else {
                404
            };
            let content_type = if request.url() == "/metrics" {
                "text/plain; version=0.0.4; charset=utf-8"
            } else {
                "text/plain"
            };

            let response = tiny_http::Response::from_string(&response_text)
                .with_status_code(status_code)
                .with_header(tiny_http::Header::from_bytes("Content-Type", content_type).unwrap());
            let _ = request.respond(response);
        }
    });

    Ok(metrics)
}

/// Render all metrics in Prometheus text exposition format.
fn render_metrics(snap: &MetricsSnapshot) -> String {
    let mut out = String::with_capacity(4096);

    // Header
    out.push_str("# Sentinel System Monitor - Prometheus Metrics\n\n");

    if let Some(ref sys) = snap.system {
        // ── CPU ────────────────────────────────────────────
        out.push_str("# HELP sentinel_cpu_usage_percent Global CPU usage percentage.\n");
        out.push_str("# TYPE sentinel_cpu_usage_percent gauge\n");
        push_metric(
            &mut out,
            "sentinel_cpu_usage_percent",
            &[],
            sys.global_cpu_usage as f64,
        );

        out.push_str("# HELP sentinel_cpu_core_usage_percent Per-core CPU usage percentage.\n");
        out.push_str("# TYPE sentinel_cpu_core_usage_percent gauge\n");
        for (i, &usage) in sys.cpu_usages.iter().enumerate() {
            push_metric(
                &mut out,
                "sentinel_cpu_core_usage_percent",
                &[("core", &i.to_string())],
                usage as f64,
            );
        }

        out.push_str("# HELP sentinel_cpu_count Number of logical CPU cores.\n");
        out.push_str("# TYPE sentinel_cpu_count gauge\n");
        push_metric(&mut out, "sentinel_cpu_count", &[], sys.cpu_count as f64);

        // ── Memory ─────────────────────────────────────────
        out.push_str("# HELP sentinel_memory_total_bytes Total system memory in bytes.\n");
        out.push_str("# TYPE sentinel_memory_total_bytes gauge\n");
        push_metric(
            &mut out,
            "sentinel_memory_total_bytes",
            &[],
            sys.total_memory as f64,
        );

        out.push_str("# HELP sentinel_memory_used_bytes Used system memory in bytes.\n");
        out.push_str("# TYPE sentinel_memory_used_bytes gauge\n");
        push_metric(
            &mut out,
            "sentinel_memory_used_bytes",
            &[],
            sys.used_memory as f64,
        );

        out.push_str("# HELP sentinel_memory_usage_percent Memory usage percentage.\n");
        out.push_str("# TYPE sentinel_memory_usage_percent gauge\n");
        push_metric(
            &mut out,
            "sentinel_memory_usage_percent",
            &[],
            sys.memory_percent() as f64,
        );

        // ── Swap ───────────────────────────────────────────
        out.push_str("# HELP sentinel_swap_total_bytes Total swap space in bytes.\n");
        out.push_str("# TYPE sentinel_swap_total_bytes gauge\n");
        push_metric(
            &mut out,
            "sentinel_swap_total_bytes",
            &[],
            sys.total_swap as f64,
        );

        out.push_str("# HELP sentinel_swap_used_bytes Used swap space in bytes.\n");
        out.push_str("# TYPE sentinel_swap_used_bytes gauge\n");
        push_metric(
            &mut out,
            "sentinel_swap_used_bytes",
            &[],
            sys.used_swap as f64,
        );

        // ── Load Averages ──────────────────────────────────
        out.push_str("# HELP sentinel_load_average System load averages.\n");
        out.push_str("# TYPE sentinel_load_average gauge\n");
        push_metric(
            &mut out,
            "sentinel_load_average",
            &[("period", "1m")],
            sys.load_avg_1,
        );
        push_metric(
            &mut out,
            "sentinel_load_average",
            &[("period", "5m")],
            sys.load_avg_5,
        );
        push_metric(
            &mut out,
            "sentinel_load_average",
            &[("period", "15m")],
            sys.load_avg_15,
        );

        // ── Uptime ─────────────────────────────────────────
        out.push_str("# HELP sentinel_uptime_seconds System uptime in seconds.\n");
        out.push_str("# TYPE sentinel_uptime_seconds gauge\n");
        push_metric(&mut out, "sentinel_uptime_seconds", &[], sys.uptime as f64);

        // ── Network I/O ────────────────────────────────────
        out.push_str(
            "# HELP sentinel_network_rx_bytes_total Total received bytes per interface.\n",
        );
        out.push_str("# TYPE sentinel_network_rx_bytes_total counter\n");
        for net in &sys.networks {
            push_metric(
                &mut out,
                "sentinel_network_rx_bytes_total",
                &[("interface", &net.name)],
                net.total_rx as f64,
            );
        }
        out.push_str(
            "# HELP sentinel_network_tx_bytes_total Total transmitted bytes per interface.\n",
        );
        out.push_str("# TYPE sentinel_network_tx_bytes_total counter\n");
        for net in &sys.networks {
            push_metric(
                &mut out,
                "sentinel_network_tx_bytes_total",
                &[("interface", &net.name)],
                net.total_tx as f64,
            );
        }

        // ── Disk Usage ─────────────────────────────────────
        out.push_str("# HELP sentinel_disk_total_bytes Total disk space in bytes.\n");
        out.push_str("# TYPE sentinel_disk_total_bytes gauge\n");
        out.push_str("# HELP sentinel_disk_available_bytes Available disk space in bytes.\n");
        out.push_str("# TYPE sentinel_disk_available_bytes gauge\n");
        out.push_str("# HELP sentinel_disk_read_bytes_per_sec Disk read throughput.\n");
        out.push_str("# TYPE sentinel_disk_read_bytes_per_sec gauge\n");
        out.push_str("# HELP sentinel_disk_write_bytes_per_sec Disk write throughput.\n");
        out.push_str("# TYPE sentinel_disk_write_bytes_per_sec gauge\n");
        for disk in &sys.disks {
            let labels = [("mount", &*disk.mount_point), ("fstype", &*disk.fs_type)];
            push_metric(
                &mut out,
                "sentinel_disk_total_bytes",
                &labels,
                disk.total_space as f64,
            );
            push_metric(
                &mut out,
                "sentinel_disk_available_bytes",
                &labels,
                disk.available_space as f64,
            );
            push_metric(
                &mut out,
                "sentinel_disk_read_bytes_per_sec",
                &labels,
                disk.read_bytes_per_sec as f64,
            );
            push_metric(
                &mut out,
                "sentinel_disk_write_bytes_per_sec",
                &labels,
                disk.write_bytes_per_sec as f64,
            );
        }

        // ── CPU Temperature ────────────────────────────────
        if let Some(ref temp) = sys.cpu_temp {
            if let Some(pkg) = temp.package_temp {
                out.push_str("# HELP sentinel_cpu_temp_celsius CPU package temperature.\n");
                out.push_str("# TYPE sentinel_cpu_temp_celsius gauge\n");
                push_metric(
                    &mut out,
                    "sentinel_cpu_temp_celsius",
                    &[("sensor", "package")],
                    pkg as f64,
                );
            }
            for (i, &core_t) in temp.core_temps.iter().enumerate() {
                push_metric(
                    &mut out,
                    "sentinel_cpu_temp_celsius",
                    &[("sensor", &format!("core{}", i))],
                    core_t as f64,
                );
            }
        }

        // ── GPU ────────────────────────────────────────────
        if let Some(ref gpu) = sys.gpu {
            out.push_str("# HELP sentinel_gpu_utilization_percent GPU utilization.\n");
            out.push_str("# TYPE sentinel_gpu_utilization_percent gauge\n");
            push_metric(
                &mut out,
                "sentinel_gpu_utilization_percent",
                &[("gpu", &gpu.name)],
                gpu.utilization as f64,
            );

            out.push_str("# HELP sentinel_gpu_memory_used_bytes GPU memory used.\n");
            out.push_str("# TYPE sentinel_gpu_memory_used_bytes gauge\n");
            push_metric(
                &mut out,
                "sentinel_gpu_memory_used_bytes",
                &[("gpu", &gpu.name)],
                gpu.memory_used as f64,
            );

            out.push_str("# HELP sentinel_gpu_memory_total_bytes GPU memory total.\n");
            out.push_str("# TYPE sentinel_gpu_memory_total_bytes gauge\n");
            push_metric(
                &mut out,
                "sentinel_gpu_memory_total_bytes",
                &[("gpu", &gpu.name)],
                gpu.memory_total as f64,
            );

            out.push_str("# HELP sentinel_gpu_temp_celsius GPU temperature.\n");
            out.push_str("# TYPE sentinel_gpu_temp_celsius gauge\n");
            push_metric(
                &mut out,
                "sentinel_gpu_temp_celsius",
                &[("gpu", &gpu.name)],
                gpu.temperature as f64,
            );

            out.push_str("# HELP sentinel_gpu_power_watts GPU power draw.\n");
            out.push_str("# TYPE sentinel_gpu_power_watts gauge\n");
            push_metric(
                &mut out,
                "sentinel_gpu_power_watts",
                &[("gpu", &gpu.name)],
                gpu.power_draw as f64,
            );

            if let Some(fan) = gpu.fan_speed {
                out.push_str("# HELP sentinel_gpu_fan_speed_percent GPU fan speed.\n");
                out.push_str("# TYPE sentinel_gpu_fan_speed_percent gauge\n");
                push_metric(
                    &mut out,
                    "sentinel_gpu_fan_speed_percent",
                    &[("gpu", &gpu.name)],
                    fan as f64,
                );
            }
        }

        // ── Battery ────────────────────────────────────────
        if let Some(ref bat) = sys.battery {
            out.push_str("# HELP sentinel_battery_percent Battery charge percentage.\n");
            out.push_str("# TYPE sentinel_battery_percent gauge\n");
            push_metric(
                &mut out,
                "sentinel_battery_percent",
                &[],
                bat.percent as f64,
            );
        }
    }

    // ── Process Count ──────────────────────────────────
    out.push_str("# HELP sentinel_process_count Total number of tracked processes.\n");
    out.push_str("# TYPE sentinel_process_count gauge\n");
    push_metric(
        &mut out,
        "sentinel_process_count",
        &[],
        snap.process_count as f64,
    );

    // ── Alerts ─────────────────────────────────────────
    out.push_str("# HELP sentinel_alert_count Number of active alerts by severity.\n");
    out.push_str("# TYPE sentinel_alert_count gauge\n");
    let mut info = 0u64;
    let mut warn = 0u64;
    let mut crit = 0u64;
    let mut danger = 0u64;
    for alert in &snap.alerts {
        match alert.severity {
            AlertSeverity::Info => info += 1,
            AlertSeverity::Warning => warn += 1,
            AlertSeverity::Critical => crit += 1,
            AlertSeverity::Danger => danger += 1,
        }
    }
    push_metric(
        &mut out,
        "sentinel_alert_count",
        &[("severity", "info")],
        info as f64,
    );
    push_metric(
        &mut out,
        "sentinel_alert_count",
        &[("severity", "warning")],
        warn as f64,
    );
    push_metric(
        &mut out,
        "sentinel_alert_count",
        &[("severity", "critical")],
        crit as f64,
    );
    push_metric(
        &mut out,
        "sentinel_alert_count",
        &[("severity", "danger")],
        danger as f64,
    );

    out.push_str("# HELP sentinel_alert_total Total number of active alerts.\n");
    out.push_str("# TYPE sentinel_alert_total gauge\n");
    push_metric(
        &mut out,
        "sentinel_alert_total",
        &[],
        snap.alerts.len() as f64,
    );

    // ── Docker Containers ──────────────────────────────
    if !snap.containers.is_empty() {
        let running = snap
            .containers
            .iter()
            .filter(|c| c.state == "running")
            .count();
        let total = snap.containers.len();

        out.push_str("# HELP sentinel_docker_containers_total Total Docker containers.\n");
        out.push_str("# TYPE sentinel_docker_containers_total gauge\n");
        push_metric(
            &mut out,
            "sentinel_docker_containers_total",
            &[],
            total as f64,
        );

        out.push_str("# HELP sentinel_docker_containers_running Running Docker containers.\n");
        out.push_str("# TYPE sentinel_docker_containers_running gauge\n");
        push_metric(
            &mut out,
            "sentinel_docker_containers_running",
            &[],
            running as f64,
        );

        out.push_str("# HELP sentinel_docker_container_cpu_percent Per-container CPU usage.\n");
        out.push_str("# TYPE sentinel_docker_container_cpu_percent gauge\n");
        out.push_str("# HELP sentinel_docker_container_memory_bytes Per-container memory usage.\n");
        out.push_str("# TYPE sentinel_docker_container_memory_bytes gauge\n");
        for c in &snap.containers {
            if c.state == "running" {
                push_metric(
                    &mut out,
                    "sentinel_docker_container_cpu_percent",
                    &[("name", &c.name), ("image", &c.image)],
                    c.cpu_percent,
                );
                push_metric(
                    &mut out,
                    "sentinel_docker_container_memory_bytes",
                    &[("name", &c.name), ("image", &c.image)],
                    c.memory_usage as f64,
                );
            }
        }
    }

    out
}

/// Write a single Prometheus metric line with optional labels.
fn push_metric(out: &mut String, name: &str, labels: &[(&str, &str)], value: f64) {
    out.push_str(name);
    if !labels.is_empty() {
        out.push('{');
        for (i, (k, v)) in labels.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(k);
            out.push_str("=\"");
            // Escape label values per Prometheus spec
            for ch in v.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        out.push('}');
    }
    out.push(' ');
    // Format: use integer when possible, otherwise 6 decimal places
    if value.fract() == 0.0 && value.abs() < 1e15 {
        out.push_str(&(value as i64).to_string());
    } else {
        out.push_str(&format!("{:.6}", value));
    }
    out.push('\n');
}
