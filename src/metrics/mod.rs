//! Prometheus metrics exporter for Sentinel.
//!
//! When enabled via `--prometheus <addr>`, runs a tiny HTTP server that exposes
//! system metrics in Prometheus text exposition format at `/metrics`.
//!
//! The main loop updates a shared `MetricsSnapshot` on each tick, and the HTTP
//! server reads it on each scrape request.

use std::sync::{Arc, Mutex};

use crate::constants::PROM_BUFFER_CAPACITY;
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
            let is_metrics = request.url() == "/metrics";

            let response_text = if is_metrics {
                match metrics_clone.lock() {
                    Ok(snap) => render_metrics(&snap),
                    Err(_) => "# error: metrics lock poisoned\n".to_string(),
                }
            } else {
                "404 Not Found\n".to_string()
            };

            let (status_code, content_type) = if is_metrics {
                (200, "text/plain; version=0.0.4; charset=utf-8")
            } else {
                (404, "text/plain")
            };

            let response = tiny_http::Response::from_string(&response_text)
                .with_status_code(status_code)
                .with_header(tiny_http::Header::from_bytes("Content-Type", content_type).unwrap());
            let _ = request.respond(response);
        }
    });

    Ok(metrics)
}

// ── Metric definition helpers ────────────────────────────────

/// A metric definition: name, help text, and type.
struct MetricDef {
    name: &'static str,
    help: &'static str,
    mtype: &'static str,
}

impl MetricDef {
    const fn gauge(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            mtype: "gauge",
        }
    }

    const fn counter(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            mtype: "counter",
        }
    }

    /// Write the HELP and TYPE lines for this metric.
    fn write_header(&self, out: &mut String) {
        out.push_str("# HELP ");
        out.push_str(self.name);
        out.push(' ');
        out.push_str(self.help);
        out.push('\n');
        out.push_str("# TYPE ");
        out.push_str(self.name);
        out.push(' ');
        out.push_str(self.mtype);
        out.push('\n');
    }

    /// Write header + a single unlabeled value.
    fn emit(&self, out: &mut String, value: f64) {
        self.write_header(out);
        push_metric(out, self.name, &[], value);
    }

    /// Write header + a single labeled value.
    fn emit_labeled(&self, out: &mut String, labels: &[(&str, &str)], value: f64) {
        self.write_header(out);
        push_metric(out, self.name, labels, value);
    }
}

// ── Static metric definitions ────────────────────────────────

const M_CPU_USAGE: MetricDef =
    MetricDef::gauge("sentinel_cpu_usage_percent", "Global CPU usage percentage.");
const M_CPU_CORE: MetricDef = MetricDef::gauge(
    "sentinel_cpu_core_usage_percent",
    "Per-core CPU usage percentage.",
);
const M_CPU_COUNT: MetricDef =
    MetricDef::gauge("sentinel_cpu_count", "Number of logical CPU cores.");
const M_MEM_TOTAL: MetricDef = MetricDef::gauge(
    "sentinel_memory_total_bytes",
    "Total system memory in bytes.",
);
const M_MEM_USED: MetricDef =
    MetricDef::gauge("sentinel_memory_used_bytes", "Used system memory in bytes.");
const M_MEM_PCT: MetricDef =
    MetricDef::gauge("sentinel_memory_usage_percent", "Memory usage percentage.");
const M_SWAP_TOTAL: MetricDef =
    MetricDef::gauge("sentinel_swap_total_bytes", "Total swap space in bytes.");
const M_SWAP_USED: MetricDef =
    MetricDef::gauge("sentinel_swap_used_bytes", "Used swap space in bytes.");
const M_LOAD: MetricDef = MetricDef::gauge("sentinel_load_average", "System load averages.");
const M_UPTIME: MetricDef =
    MetricDef::gauge("sentinel_uptime_seconds", "System uptime in seconds.");
const M_NET_RX: MetricDef = MetricDef::counter(
    "sentinel_network_rx_bytes_total",
    "Total received bytes per interface.",
);
const M_NET_TX: MetricDef = MetricDef::counter(
    "sentinel_network_tx_bytes_total",
    "Total transmitted bytes per interface.",
);
const M_DISK_TOTAL: MetricDef =
    MetricDef::gauge("sentinel_disk_total_bytes", "Total disk space in bytes.");
const M_DISK_AVAIL: MetricDef = MetricDef::gauge(
    "sentinel_disk_available_bytes",
    "Available disk space in bytes.",
);
const M_DISK_READ: MetricDef =
    MetricDef::gauge("sentinel_disk_read_bytes_per_sec", "Disk read throughput.");
const M_DISK_WRITE: MetricDef = MetricDef::gauge(
    "sentinel_disk_write_bytes_per_sec",
    "Disk write throughput.",
);
const M_CPU_TEMP: MetricDef = MetricDef::gauge("sentinel_cpu_temp_celsius", "CPU temperature.");
const M_GPU_UTIL: MetricDef =
    MetricDef::gauge("sentinel_gpu_utilization_percent", "GPU utilization.");
const M_GPU_MEM_USED: MetricDef =
    MetricDef::gauge("sentinel_gpu_memory_used_bytes", "GPU memory used.");
const M_GPU_MEM_TOTAL: MetricDef =
    MetricDef::gauge("sentinel_gpu_memory_total_bytes", "GPU memory total.");
const M_GPU_TEMP: MetricDef = MetricDef::gauge("sentinel_gpu_temp_celsius", "GPU temperature.");
const M_GPU_POWER: MetricDef = MetricDef::gauge("sentinel_gpu_power_watts", "GPU power draw.");
const M_GPU_FAN: MetricDef = MetricDef::gauge("sentinel_gpu_fan_speed_percent", "GPU fan speed.");
const M_BATTERY: MetricDef =
    MetricDef::gauge("sentinel_battery_percent", "Battery charge percentage.");
const M_PROC_COUNT: MetricDef = MetricDef::gauge(
    "sentinel_process_count",
    "Total number of tracked processes.",
);
const M_ALERT_COUNT: MetricDef = MetricDef::gauge(
    "sentinel_alert_count",
    "Number of active alerts by severity.",
);
const M_ALERT_TOTAL: MetricDef =
    MetricDef::gauge("sentinel_alert_total", "Total number of active alerts.");
const M_DOCKER_TOTAL: MetricDef = MetricDef::gauge(
    "sentinel_docker_containers_total",
    "Total Docker containers.",
);
const M_DOCKER_RUNNING: MetricDef = MetricDef::gauge(
    "sentinel_docker_containers_running",
    "Running Docker containers.",
);
const M_DOCKER_CPU: MetricDef = MetricDef::gauge(
    "sentinel_docker_container_cpu_percent",
    "Per-container CPU usage.",
);
const M_DOCKER_MEM: MetricDef = MetricDef::gauge(
    "sentinel_docker_container_memory_bytes",
    "Per-container memory usage.",
);

// ── Rendering ────────────────────────────────────────────────

/// Render all metrics in Prometheus text exposition format.
fn render_metrics(snap: &MetricsSnapshot) -> String {
    let mut out = String::with_capacity(PROM_BUFFER_CAPACITY);

    out.push_str("# Sentinel System Monitor - Prometheus Metrics\n\n");

    if let Some(ref sys) = snap.system {
        render_system_metrics(&mut out, sys);
    }

    render_process_and_alert_metrics(&mut out, snap);
    render_docker_metrics(&mut out, snap);

    out
}

fn render_system_metrics(out: &mut String, sys: &SystemSnapshot) {
    // CPU
    M_CPU_USAGE.emit(out, sys.global_cpu_usage as f64);

    M_CPU_CORE.write_header(out);
    for (i, &usage) in sys.cpu_usages.iter().enumerate() {
        push_metric(
            out,
            M_CPU_CORE.name,
            &[("core", &i.to_string())],
            usage as f64,
        );
    }

    M_CPU_COUNT.emit(out, sys.cpu_count as f64);

    // Memory
    M_MEM_TOTAL.emit(out, sys.total_memory as f64);
    M_MEM_USED.emit(out, sys.used_memory as f64);
    M_MEM_PCT.emit(out, sys.memory_percent() as f64);

    // Swap
    M_SWAP_TOTAL.emit(out, sys.total_swap as f64);
    M_SWAP_USED.emit(out, sys.used_swap as f64);

    // Load averages
    M_LOAD.write_header(out);
    push_metric(out, M_LOAD.name, &[("period", "1m")], sys.load_avg_1);
    push_metric(out, M_LOAD.name, &[("period", "5m")], sys.load_avg_5);
    push_metric(out, M_LOAD.name, &[("period", "15m")], sys.load_avg_15);

    // Uptime
    M_UPTIME.emit(out, sys.uptime as f64);

    // Network I/O
    M_NET_RX.write_header(out);
    for net in &sys.networks {
        push_metric(
            out,
            M_NET_RX.name,
            &[("interface", &net.name)],
            net.total_rx as f64,
        );
    }
    M_NET_TX.write_header(out);
    for net in &sys.networks {
        push_metric(
            out,
            M_NET_TX.name,
            &[("interface", &net.name)],
            net.total_tx as f64,
        );
    }

    // Disk usage
    M_DISK_TOTAL.write_header(out);
    M_DISK_AVAIL.write_header(out);
    M_DISK_READ.write_header(out);
    M_DISK_WRITE.write_header(out);
    for disk in &sys.disks {
        let labels = [("mount", &*disk.mount_point), ("fstype", &*disk.fs_type)];
        push_metric(out, M_DISK_TOTAL.name, &labels, disk.total_space as f64);
        push_metric(out, M_DISK_AVAIL.name, &labels, disk.available_space as f64);
        push_metric(
            out,
            M_DISK_READ.name,
            &labels,
            disk.read_bytes_per_sec as f64,
        );
        push_metric(
            out,
            M_DISK_WRITE.name,
            &labels,
            disk.write_bytes_per_sec as f64,
        );
    }

    // CPU temperature
    if let Some(ref temp) = sys.cpu_temp {
        M_CPU_TEMP.write_header(out);
        if let Some(pkg) = temp.package_temp {
            push_metric(out, M_CPU_TEMP.name, &[("sensor", "package")], pkg as f64);
        }
        for (i, &core_t) in temp.core_temps.iter().enumerate() {
            push_metric(
                out,
                M_CPU_TEMP.name,
                &[("sensor", &format!("core{}", i))],
                core_t as f64,
            );
        }
    }

    // GPU
    if let Some(ref gpu) = sys.gpu {
        let gpu_label = [("gpu", &*gpu.name)];
        M_GPU_UTIL.emit_labeled(out, &gpu_label, gpu.utilization as f64);
        M_GPU_MEM_USED.emit_labeled(out, &gpu_label, gpu.memory_used as f64);
        M_GPU_MEM_TOTAL.emit_labeled(out, &gpu_label, gpu.memory_total as f64);
        M_GPU_TEMP.emit_labeled(out, &gpu_label, gpu.temperature as f64);
        M_GPU_POWER.emit_labeled(out, &gpu_label, gpu.power_draw as f64);
        if let Some(fan) = gpu.fan_speed {
            M_GPU_FAN.emit_labeled(out, &gpu_label, fan as f64);
        }
    }

    // Battery
    if let Some(ref bat) = sys.battery {
        M_BATTERY.emit(out, bat.percent as f64);
    }
}

fn render_process_and_alert_metrics(out: &mut String, snap: &MetricsSnapshot) {
    M_PROC_COUNT.emit(out, snap.process_count as f64);

    // Alerts by severity
    M_ALERT_COUNT.write_header(out);
    let mut counts = [0u64; 4]; // info, warn, crit, danger
    for alert in &snap.alerts {
        match alert.severity {
            AlertSeverity::Info => counts[0] += 1,
            AlertSeverity::Warning => counts[1] += 1,
            AlertSeverity::Critical => counts[2] += 1,
            AlertSeverity::Danger => counts[3] += 1,
        }
    }
    for (label, count) in [
        ("info", counts[0]),
        ("warning", counts[1]),
        ("critical", counts[2]),
        ("danger", counts[3]),
    ] {
        push_metric(
            out,
            M_ALERT_COUNT.name,
            &[("severity", label)],
            count as f64,
        );
    }

    M_ALERT_TOTAL.emit(out, snap.alerts.len() as f64);
}

fn render_docker_metrics(out: &mut String, snap: &MetricsSnapshot) {
    if snap.containers.is_empty() {
        return;
    }

    let running = snap
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    M_DOCKER_TOTAL.emit(out, snap.containers.len() as f64);
    M_DOCKER_RUNNING.emit(out, running as f64);

    M_DOCKER_CPU.write_header(out);
    M_DOCKER_MEM.write_header(out);
    for c in &snap.containers {
        if c.state == "running" {
            let labels = [("name", &*c.name), ("image", &*c.image)];
            push_metric(out, M_DOCKER_CPU.name, &labels, c.cpu_percent);
            push_metric(out, M_DOCKER_MEM.name, &labels, c.memory_usage as f64);
        }
    }
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
    if value.fract() == 0.0 && value.abs() < 1e15 {
        out.push_str(&(value as i64).to_string());
    } else {
        out.push_str(&format!("{:.6}", value));
    }
    out.push('\n');
}
