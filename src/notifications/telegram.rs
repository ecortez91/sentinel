//! Telegram Bot API notification system for Sentinel alerts.
//!
//! Sends alert messages via the Telegram Bot API using a simple HTTP POST.
//! No extra crate needed — `reqwest` (already a dependency) handles the request.
//!
//! Rate-limited: max 1 message per (category, PID) pair per 5 minutes.
//! Severity-filtered: only sends alerts at or above the configured minimum severity.
//!
//! ## Setup
//!
//! 1. Create a bot via @BotFather on Telegram to get a bot token.
//! 2. Start a chat with the bot (or add it to a group) to get a chat ID.
//! 3. Configure in Settings TUI > Notifications, or in config.toml.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::NotificationConfig;
use crate::constants::{
    DEFAULT_TELEGRAM_DIGEST_SECS, MAX_PARENT_WALK_DEPTH, MAX_WORKER_DISPLAY, TELEGRAM_API_BASE,
    TELEGRAM_DIGEST_MAX_ALERTS, TELEGRAM_RATE_LIMIT_SECS, WORKER_EXTREME_MULTIPLIER,
};
use crate::models::{Alert, AlertCategory, AlertSeverity, ProcessInfo};

/// Telegram notification manager with rate limiting, severity filtering,
/// and optional digest mode (#8).
pub struct TelegramNotifier {
    bot_token: String,
    chat_id: String,
    min_severity: AlertSeverity,
    client: reqwest::Client,
    /// Last send time per (category, app_name) for app-based rate limiting.
    last_sent: HashMap<(AlertCategory, String), Instant>,
    rate_limit: Duration,
    /// Digest mode: if > 0, batch alerts and send a summary every N seconds.
    digest_interval: Duration,
    /// Buffered alerts waiting for the next digest flush.
    digest_buffer: Vec<(Alert, String)>,
    /// Last time a digest was flushed.
    #[allow(dead_code)] // read by tick_digest
    last_digest: Option<Instant>,
}

impl TelegramNotifier {
    /// Create a notifier from notification config.
    ///
    /// Priority: config.toml credentials > .env credentials (#10).
    /// Returns `None` if Telegram is not configured (missing token or chat ID).
    pub fn from_config(config: &NotificationConfig) -> Option<Self> {
        // Config takes priority, then fall back to env vars (#10)
        let token = config
            .telegram_bot_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                std::env::var(crate::constants::ENV_TELEGRAM_BOT_TOKEN)
                    .ok()
                    .filter(|s| !s.is_empty())
            })?;

        let chat_id = config
            .telegram_chat_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                std::env::var(crate::constants::ENV_TELEGRAM_CHAT_ID)
                    .ok()
                    .filter(|s| !s.is_empty())
            })?;

        Some(Self {
            bot_token: token,
            chat_id,
            min_severity: parse_min_severity(&config.telegram_min_severity),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            last_sent: HashMap::new(),
            rate_limit: Duration::from_secs(TELEGRAM_RATE_LIMIT_SECS),
            digest_interval: Duration::from_secs(DEFAULT_TELEGRAM_DIGEST_SECS),
            digest_buffer: Vec::new(),
            last_digest: None,
        })
    }

    /// Get the bot token (for creating temporary notifiers in async tasks).
    #[allow(dead_code)]
    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    /// Get the chat ID (for creating temporary notifiers in async tasks).
    #[allow(dead_code)]
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// Check whether an alert passes the severity filter and rate limit.
    fn should_send(&self, alert: &Alert) -> bool {
        self.should_send_for_app(alert, &alert.process_name)
    }

    /// Check severity filter and rate limit using an app name key.
    fn should_send_for_app(&self, alert: &Alert, app_name: &str) -> bool {
        // Severity gate
        if alert.severity < self.min_severity {
            return false;
        }
        // Rate limit per (category, app_name)
        let key = (alert.category, app_name.to_string());
        match self.last_sent.get(&key) {
            Some(last) if last.elapsed() < self.rate_limit => false,
            _ => true,
        }
    }

    /// Set the digest interval. If > 0, alerts are batched and flushed
    /// periodically as a single summary message (#8).
    #[allow(dead_code)]
    pub fn set_digest_interval(&mut self, secs: u64) {
        self.digest_interval = Duration::from_secs(secs);
    }

    /// Send an alert via Telegram if it passes filters.
    ///
    /// In digest mode (#8): buffers the alert for periodic flushing.
    /// In immediate mode: spawns a background task so it never blocks the main loop.
    pub fn send_alert(&mut self, alert: &Alert, hostname: &str) {
        self.send_alert_with_context(alert, hostname, None);
    }

    /// Send an alert with optional system context (#9).
    ///
    /// `context` provides a snapshot of system state (CPU, memory, thermal, security)
    /// appended to the message for richer alerting.
    pub fn send_alert_with_context(
        &mut self,
        alert: &Alert,
        hostname: &str,
        context: Option<&AlertContext>,
    ) {
        if !self.should_send(alert) {
            return;
        }

        // Mark sent before spawning (prevents duplicates during async flight)
        self.last_sent
            .insert((alert.category, alert.process_name.clone()), Instant::now());

        let text = match context {
            Some(ctx) => format_alert_with_context(alert, hostname, ctx),
            None => format_alert(alert, hostname),
        };

        if self.digest_interval.as_secs() > 0 {
            // Digest mode: buffer the alert (#8)
            self.digest_buffer.push((alert.clone(), text));
            if self.digest_buffer.len() > TELEGRAM_DIGEST_MAX_ALERTS {
                self.digest_buffer
                    .drain(..self.digest_buffer.len() - TELEGRAM_DIGEST_MAX_ALERTS);
            }
        } else {
            // Immediate mode: send now
            self.spawn_send(text);
        }
    }

    /// Send alerts grouped by parent application.
    ///
    /// Groups worker-thread alerts (e.g. 8 V8Worker threads → 1 "node" notification)
    /// while leaving system-level alerts (thermal, security) ungrouped.
    /// Applies severity filtering and rate limiting per (category, app_name).
    pub fn send_grouped_alerts(
        &mut self,
        alerts: &[Alert],
        processes: &[ProcessInfo],
        hostname: &str,
        context: Option<&AlertContext>,
    ) {
        if alerts.is_empty() {
            return;
        }

        let groups = group_alerts_by_app(alerts, processes);

        for group in &groups {
            // Use the first alert as a representative for severity check
            let representative = Alert {
                severity: group.severity,
                category: group.category,
                process_name: group.app_name.clone(),
                pid: *group.pids.first().unwrap_or(&0),
                message: group.representative_message.clone(),
                timestamp: chrono::Local::now(),
                value: group.total_value,
                threshold: group.threshold,
            };

            if !self.should_send_for_app(&representative, &group.app_name) {
                continue;
            }

            // Mark sent for this (category, app_name) pair
            self.last_sent.insert(
                (group.category, group.app_name.clone()),
                Instant::now(),
            );

            let text = if group.worker_count > 1 {
                let mut msg = format_grouped_alert(group, hostname);
                if let Some(ctx) = context {
                    append_inline_context(&mut msg, ctx);
                }
                msg
            } else {
                match context {
                    Some(ctx) => format_alert_with_context(&representative, hostname, ctx),
                    None => format_alert(&representative, hostname),
                }
            };

            if self.digest_interval.as_secs() > 0 {
                self.digest_buffer.push((representative, text));
                if self.digest_buffer.len() > TELEGRAM_DIGEST_MAX_ALERTS {
                    self.digest_buffer
                        .drain(..self.digest_buffer.len() - TELEGRAM_DIGEST_MAX_ALERTS);
                }
            } else {
                self.spawn_send(text);
            }
        }
    }

    /// Tick the digest timer. Call this periodically from the event loop.
    /// If the digest interval has elapsed and there are buffered alerts,
    /// sends a summary message (#8).
    #[allow(dead_code)] // called from app.rs when digest mode is enabled
    pub fn tick_digest(&mut self, hostname: &str) {
        if self.digest_interval.as_secs() == 0 || self.digest_buffer.is_empty() {
            return;
        }

        let should_flush = match self.last_digest {
            Some(last) => last.elapsed() >= self.digest_interval,
            None => true, // First flush
        };

        if !should_flush {
            return;
        }

        self.last_digest = Some(Instant::now());
        let alerts: Vec<(Alert, String)> = self.digest_buffer.drain(..).collect();
        let text = format_digest(&alerts, hostname);
        self.spawn_send(text);
    }

    /// Spawn a background task to send a message (non-blocking).
    fn spawn_send(&self, text: String) {
        let url = format!(
            "{}/bot{}/sendMessage",
            TELEGRAM_API_BASE, self.bot_token
        );
        let chat_id = self.chat_id.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            let _ = client
                .post(&url)
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "text": text,
                    "parse_mode": "HTML",
                }))
                .send()
                .await;
        });
    }

    /// Send a test message (bypasses rate limiting and severity filter).
    pub async fn send_test(&self) -> Result<(), String> {
        let hostname = std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "sentinel-host".to_string());
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

        let text = format!(
            "\u{2705} <b>Sentinel Test Message</b>\n\n\
             Telegram notifications are working.\n\n\
             Host: <code>{}</code>\n\
             Time: <code>{}</code>",
            hostname, timestamp,
        );

        self.send_message(&text).await
    }

    /// Low-level POST to Telegram's sendMessage endpoint.
    async fn send_message(&self, text: &str) -> Result<(), String> {
        let url = format!(
            "{}/bot{}/sendMessage",
            TELEGRAM_API_BASE, self.bot_token
        );

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "text": text,
                "parse_mode": "HTML",
            }))
            .send()
            .await
            .map_err(|e| format!("Telegram request failed: {}", e))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Telegram API error {}: {}", status, body))
        }
    }
}

/// Map a severity string from config to an [`AlertSeverity`] value.
///
/// Defaults to `Warning` for unrecognized values.
pub fn parse_min_severity(s: &str) -> AlertSeverity {
    match s.to_lowercase().as_str() {
        "danger" => AlertSeverity::Danger,
        "critical" => AlertSeverity::Critical,
        "warning" | "warn" => AlertSeverity::Warning,
        "info" => AlertSeverity::Info,
        _ => AlertSeverity::Warning,
    }
}

/// System context snapshot for rich alert messages (#9).
#[derive(Debug, Clone, Default)]
pub struct AlertContext {
    /// Overall CPU usage percentage.
    pub cpu_pct: Option<f32>,
    /// Memory usage percentage.
    pub mem_pct: Option<f32>,
    /// Max thermal temperature (if LHM connected).
    pub max_temp: Option<f32>,
    /// Security score (0-100).
    pub security_score: Option<u8>,
    /// Number of active processes.
    pub process_count: Option<usize>,
    /// System uptime string (e.g., "3d 12h").
    pub uptime: Option<String>,
}

// ── Alert Grouping ──────────────────────────────────────────────

/// Categories that should NOT be grouped (system-level, not per-process worker alerts).
fn is_groupable_category(cat: AlertCategory) -> bool {
    !matches!(
        cat,
        AlertCategory::ThermalWarning
            | AlertCategory::ThermalCritical
            | AlertCategory::ThermalEmergency
            | AlertCategory::SecurityThreat
            | AlertCategory::SecurityScore
            | AlertCategory::SystemOverload
            | AlertCategory::WindowsFirewall
            | AlertCategory::WindowsDefender
            | AlertCategory::WindowsUpdates
    )
}

/// Per-worker detail for rich grouped alert messages.
///
/// Captures the individual metrics of each worker process so the grouped
/// notification can show a per-PID breakdown rather than just aggregates.
#[derive(Debug, Clone)]
pub struct WorkerDetail {
    /// Process ID.
    pub pid: u32,
    /// Worker thread/process name (e.g. "V8Worker", "tokio-rt-worker").
    pub name: String,
    /// The alert-relevant value (bytes for memory, percentage for CPU).
    pub value: f64,
    /// Usage as a percentage (memory_percent or cpu_usage).
    pub percent: f32,
}

/// A group of related alerts collapsed into a single notification.
///
/// Instead of 8 messages for "node-V8Worker (PID 100)", "node-V8Worker (PID 101)", etc.,
/// we send ONE message: "Node.js — 8 workers — Total: 3.9 GiB".
#[derive(Debug, Clone)]
pub struct GroupedAlert {
    /// The resolved parent application name (e.g. "node", "firefox").
    pub app_name: String,
    /// Alert category (all alerts in the group share this).
    pub category: AlertCategory,
    /// Highest severity across all alerts in the group.
    pub severity: AlertSeverity,
    /// Number of worker processes (alerts) in the group.
    pub worker_count: usize,
    /// Sum of `alert.value` across all grouped alerts.
    pub total_value: f64,
    /// Average percentage across all grouped alerts (for display).
    pub avg_percent: f64,
    /// Threshold from the first alert (all share the same threshold).
    pub threshold: f64,
    /// PIDs of all grouped processes.
    pub pids: Vec<u32>,
    /// Per-worker details for rich per-PID breakdown display.
    pub worker_details: Vec<WorkerDetail>,
    /// The representative message from the highest-severity alert.
    pub representative_message: String,
}

/// Walk up the parent_pid chain to find the real application name.
///
/// Worker threads like `tokio-rt-worker`, `node-V8Worker`, or `ThreadPoolForeg`
/// are children of the actual application. This walks up to [`MAX_PARENT_WALK_DEPTH`]
/// hops to find a "real" parent name.
///
/// Stops at PID <= 1 (init/systemd) or when the chain is exhausted.
pub fn resolve_parent_app_name(
    pid: u32,
    process_name: &str,
    pid_map: &HashMap<u32, &ProcessInfo>,
) -> String {
    // If it doesn't look like a worker thread, return as-is
    if !is_likely_worker_name(process_name) {
        return process_name.to_string();
    }

    let mut current_pid = pid;
    for _ in 0..MAX_PARENT_WALK_DEPTH {
        let proc = match pid_map.get(&current_pid) {
            Some(p) => p,
            None => break,
        };
        let parent = match proc.parent_pid {
            Some(ppid) if ppid > 1 => ppid,
            _ => break,
        };
        if let Some(parent_proc) = pid_map.get(&parent) {
            if !is_likely_worker_name(&parent_proc.name) {
                return parent_proc.name.clone();
            }
            current_pid = parent;
        } else {
            break;
        }
    }

    // Couldn't resolve — return original name
    process_name.to_string()
}

/// Heuristic: names that look like worker/pool threads rather than real apps.
fn is_likely_worker_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("worker")
        || lower.contains("threadpool")
        || lower.contains("pool-")
        || lower.starts_with("tokio-")
        || lower.starts_with("actix-")
        || lower.contains("rt-worker")
        || lower.ends_with("-thread")
}

/// Group alerts by (resolved app name, category).
///
/// For each group: sums values, takes max severity, collects PIDs.
/// Non-groupable categories (thermal, security, system) are returned as
/// single-element groups so they format normally.
pub fn group_alerts_by_app(
    alerts: &[Alert],
    processes: &[ProcessInfo],
) -> Vec<GroupedAlert> {
    // Build a PID → ProcessInfo lookup
    let pid_map: HashMap<u32, &ProcessInfo> = processes.iter().map(|p| (p.pid, p)).collect();

    // Accumulator: (app_name, category) → partial GroupedAlert
    let mut groups: HashMap<(String, AlertCategory), GroupedAlert> = HashMap::new();

    for alert in alerts {
        let app_name = if is_groupable_category(alert.category) {
            resolve_parent_app_name(alert.pid, &alert.process_name, &pid_map)
        } else {
            alert.process_name.clone()
        };

        let key = (app_name.clone(), alert.category);

        let entry = groups.entry(key).or_insert_with(|| GroupedAlert {
            app_name: app_name.clone(),
            category: alert.category,
            severity: alert.severity,
            worker_count: 0,
            total_value: 0.0,
            avg_percent: 0.0,
            threshold: alert.threshold,
            pids: Vec::new(),
            worker_details: Vec::new(),
            representative_message: alert.message.clone(),
        });

        entry.worker_count += 1;
        entry.total_value += alert.value;
        if alert.severity > entry.severity {
            entry.severity = alert.severity;
            entry.representative_message = alert.message.clone();
        }
        entry.pids.push(alert.pid);

        // Populate per-worker details from ProcessInfo if available
        let (value, percent) = if let Some(proc) = pid_map.get(&alert.pid) {
            match alert.category {
                AlertCategory::HighMemory | AlertCategory::MemoryLeak => {
                    (proc.memory_bytes as f64, proc.memory_percent)
                }
                AlertCategory::HighCpu => {
                    (proc.cpu_usage as f64, proc.cpu_usage)
                }
                _ => (alert.value, 0.0),
            }
        } else {
            (alert.value, 0.0)
        };

        entry.worker_details.push(WorkerDetail {
            pid: alert.pid,
            name: alert.process_name.clone(),
            value,
            percent,
        });
    }

    // Compute averages and collect
    let mut result: Vec<GroupedAlert> = groups
        .into_values()
        .map(|mut g| {
            if g.worker_count > 0 {
                g.avg_percent = g.total_value / g.worker_count as f64;
            }
            g
        })
        .collect();

    // Sort: highest severity first, then by worker count descending
    result.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(b.worker_count.cmp(&a.worker_count))
    });

    result
}

/// Human-readable display name for an [`AlertCategory`].
///
/// Returns full descriptive names (e.g. "High Memory Usage") instead of
/// terse abbreviations ("MEM"), suitable for notification titles.
pub fn category_display_name(cat: AlertCategory) -> &'static str {
    match cat {
        AlertCategory::HighCpu => "High CPU Usage",
        AlertCategory::HighMemory => "High Memory Usage",
        AlertCategory::HighDiskIo => "High Disk I/O",
        AlertCategory::Zombie => "Zombie Processes",
        AlertCategory::Suspicious => "Suspicious Activity",
        AlertCategory::SystemOverload => "System Overload",
        AlertCategory::MemoryLeak => "Memory Leak Detected",
        AlertCategory::SecurityThreat => "Security Threat",
        AlertCategory::SecurityScore => "Security Score Drop",
        AlertCategory::ThermalWarning => "Thermal Warning",
        AlertCategory::ThermalCritical => "Thermal Critical",
        AlertCategory::ThermalEmergency => "Thermal Emergency",
        AlertCategory::WindowsFirewall => "Windows Firewall Issue",
        AlertCategory::WindowsDefender => "Windows Defender Issue",
        AlertCategory::WindowsUpdates => "Windows Updates Issue",
    }
}

/// Generate a contextual "What's happening" explanation for a grouped alert.
///
/// The explanation varies based on category, worker count, and how the
/// total value compares to the threshold.
pub fn explain_group(group: &GroupedAlert) -> String {
    let multiplier = if group.threshold > 0.0 {
        group.total_value / group.threshold
    } else {
        1.0
    };

    let intensity = if multiplier >= WORKER_EXTREME_MULTIPLIER {
        "significant"
    } else {
        "elevated"
    };

    match group.category {
        AlertCategory::HighMemory => {
            let total_display = format_bytes_f64(group.total_value);
            format!(
                "{app} spawned {n} worker threads, each consuming {intensity} memory. \
                 Combined usage is {total} ({mult:.0}x threshold).",
                app = group.app_name,
                n = group.worker_count,
                intensity = intensity,
                total = total_display,
                mult = multiplier,
            )
        }
        AlertCategory::HighCpu => {
            format!(
                "{app} spawned {n} worker threads, each consuming {intensity} CPU. \
                 Combined usage is {total:.1}% ({mult:.0}x threshold).",
                app = group.app_name,
                n = group.worker_count,
                intensity = intensity,
                total = group.total_value,
                mult = multiplier,
            )
        }
        AlertCategory::HighDiskIo => {
            let total_display = format_bytes_f64(group.total_value);
            format!(
                "{app} has {n} workers generating heavy disk I/O. \
                 Combined throughput is {total} ({mult:.0}x threshold).",
                app = group.app_name,
                n = group.worker_count,
                total = total_display,
                mult = multiplier,
            )
        }
        AlertCategory::MemoryLeak => {
            let total_display = format_bytes_f64(group.total_value);
            format!(
                "{app} has {n} workers with suspected memory leaks. \
                 Combined memory growth is {total}, suggesting systematic allocation issues.",
                app = group.app_name,
                n = group.worker_count,
                total = total_display,
            )
        }
        AlertCategory::Zombie => {
            format!(
                "{app} has {n} zombie child processes that haven't been reaped. \
                 This may indicate a process management issue in the parent.",
                app = group.app_name,
                n = group.worker_count,
            )
        }
        _ => {
            format!(
                "{app} has {n} workers triggering {cat} alerts.",
                app = group.app_name,
                n = group.worker_count,
                cat = category_display_name(group.category),
            )
        }
    }
}

/// Generate actionable suggestions for a grouped alert based on category and severity.
///
/// Combines the alert category (what kind of resource issue) with severity
/// (how urgent) to produce specific, actionable advice.
pub fn suggest_action(group: &GroupedAlert) -> String {
    match (group.category, group.severity) {
        // Memory
        (AlertCategory::HighMemory, AlertSeverity::Danger | AlertSeverity::Critical) => {
            "Immediate action needed: consider restarting the application or killing runaway workers. \
             Investigate with `pmap` or heap profiling tools.".to_string()
        }
        (AlertCategory::HighMemory, _) => {
            "Check for memory leaks or increase memory limits if this workload is expected.".to_string()
        }
        // Memory leak
        (AlertCategory::MemoryLeak, AlertSeverity::Danger | AlertSeverity::Critical) => {
            "Memory leak confirmed: restart the application to reclaim memory, then profile with \
             valgrind or a language-specific heap profiler.".to_string()
        }
        (AlertCategory::MemoryLeak, _) => {
            "Monitor memory growth over the next few minutes. If it continues, restart and profile.".to_string()
        }
        // CPU
        (AlertCategory::HighCpu, AlertSeverity::Danger | AlertSeverity::Critical) => {
            "CPU saturation: check for infinite loops or runaway computations. Consider `nice`/`cpulimit` \
             to throttle, or scale horizontally.".to_string()
        }
        (AlertCategory::HighCpu, _) => {
            "Review worker thread concurrency settings. Consider reducing parallelism or optimizing hot paths.".to_string()
        }
        // Disk I/O
        (AlertCategory::HighDiskIo, AlertSeverity::Danger | AlertSeverity::Critical) => {
            "Disk I/O saturation: check for excessive logging, large file operations, or swap thrashing. \
             Use `iotop` or `strace` to identify the source.".to_string()
        }
        (AlertCategory::HighDiskIo, _) => {
            "Review disk I/O patterns. Consider buffering writes or moving to faster storage.".to_string()
        }
        // Zombie
        (AlertCategory::Zombie, _) => {
            "Zombie processes indicate the parent isn't calling wait(). \
             Check the parent process for signal handling issues.".to_string()
        }
        // Fallback
        _ => {
            format!(
                "Investigate {} in {}. Check application logs and system metrics.",
                category_display_name(group.category),
                group.app_name,
            )
        }
    }
}

/// Format a grouped alert as an HTML Telegram message.
///
/// For groups with >1 worker, uses a detailed per-worker breakdown:
/// ```text
/// ⚠️ WARNING: High Memory Usage
///
/// 📦 node (4 workers)
///
/// ┌─ Per-Worker Breakdown:
/// │ PID 101 — V8Worker — 1.2 GiB (75.0%)
/// │ PID 102 — V8Worker — 1.1 GiB (68.8%)
/// └─ Total: 2.3 GiB | Avg: 1.15 GiB | Threshold: 1.0 GiB
///
/// 🔍 What's happening:
/// node spawned 2 worker threads, each consuming significant memory.
///
/// 💡 Action: ...
///
/// 🖥️ CPU 45% | RAM 78% | 245 procs
/// 📍 my-server | 🕐 2026-03-16 12:00:00
/// ```
///
/// For single-alert groups, falls back to a standard layout with human-readable
/// category names.
pub fn format_grouped_alert(group: &GroupedAlert, hostname: &str) -> String {
    let cat_name = category_display_name(group.category);

    if group.worker_count <= 1 {
        // Fall back to standard single-alert format with human-readable category
        let emoji = severity_emoji(group.severity);
        let sev = severity_label(group.severity);
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let pid_str = group
            .pids
            .first()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".to_string());

        return format!(
            "{} <b>{}: {}</b>\n\n\
             {}\n\n\
             Process: <code>{}</code> (PID {})\n\
             Host: <code>{}</code>\n\
             Time: <code>{}</code>",
            emoji, sev, cat_name,
            group.representative_message,
            group.app_name, pid_str,
            hostname, timestamp,
        );
    }

    let emoji = severity_emoji(group.severity);
    let sev = severity_label(group.severity);
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    // Header
    let mut msg = format!(
        "{} <b>{}: {}</b>\n\n\
         \u{1F4E6} <b>{}</b> ({} workers)\n",
        emoji, sev, cat_name,
        group.app_name, group.worker_count,
    );

    // Per-worker breakdown
    msg.push_str("\n\u{250C}\u{2500} Per-Worker Breakdown:");

    // Sort worker details by value descending for display
    let mut sorted_details = group.worker_details.clone();
    sorted_details.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let display_count = sorted_details.len().min(MAX_WORKER_DISPLAY);
    let remaining = sorted_details.len().saturating_sub(MAX_WORKER_DISPLAY);

    for (i, detail) in sorted_details.iter().take(display_count).enumerate() {
        let value_str = format_worker_value(group.category, detail.value);
        let is_last = i == display_count - 1 && remaining == 0;
        let prefix = if is_last { "\u{2514}\u{2500}" } else { "\u{2502} " };

        // Extract short worker name (strip app prefix for readability)
        let short_name = detail
            .name
            .strip_prefix(&format!("{}-", group.app_name))
            .unwrap_or(&detail.name);

        msg.push_str(&format!(
            "\n{} PID {} \u{2014} {} \u{2014} {} ({:.1}%)",
            prefix, detail.pid, short_name, value_str, detail.percent,
        ));
    }

    if remaining > 0 {
        msg.push_str(&format!(
            "\n\u{2502}  ... +{} more workers",
            remaining,
        ));
    }

    // Summary line
    let (total_display, _) = format_value_for_category(group.category, group.total_value, group.avg_percent);
    let avg_display = format_avg_value(group.category, group.avg_percent);
    let threshold_display = format_threshold_for_category(group.category, group.threshold);

    if remaining > 0 {
        msg.push_str(&format!(
            "\n\u{2514}\u{2500} Total: {} | Avg: {} | Threshold: {}",
            total_display, avg_display, threshold_display,
        ));
    } else if sorted_details.len() > 1 {
        // Summary was not added yet (no remaining, but we need it after the last detail)
        msg.push_str(&format!(
            "\nTotal: {} | Avg: {} | Threshold: {}",
            total_display, avg_display, threshold_display,
        ));
    }

    // What's happening
    let explanation = explain_group(group);
    msg.push_str(&format!(
        "\n\n\u{1F50D} <b>What's happening:</b>\n{}",
        explanation,
    ));

    // Suggested action
    let action = suggest_action(group);
    msg.push_str(&format!(
        "\n\n\u{1F4A1} <b>Action:</b> {}",
        action,
    ));

    // Footer
    msg.push_str(&format!(
        "\n\n\u{1F4CD} {} | \u{1F550} {}",
        hostname, timestamp,
    ));

    msg
}

/// Emoji for a severity level.
fn severity_emoji(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Danger => "\u{1F6A8}",          // 🚨
        AlertSeverity::Critical => "\u{2757}",          // ❗
        AlertSeverity::Warning => "\u{26A0}\u{FE0F}",   // ⚠️
        AlertSeverity::Info => "\u{2139}\u{FE0F}",      // ℹ️
    }
}

/// Label for a severity level.
fn severity_label(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Danger => "DANGER",
        AlertSeverity::Critical => "CRITICAL",
        AlertSeverity::Warning => "WARNING",
        AlertSeverity::Info => "INFO",
    }
}

/// Format total/avg values for display based on alert category.
fn format_value_for_category(
    category: AlertCategory,
    total: f64,
    avg: f64,
) -> (String, String) {
    match category {
        AlertCategory::HighMemory | AlertCategory::MemoryLeak => {
            // Values are in bytes
            let total_str = format_bytes_f64(total);
            let avg_str = format_bytes_f64(avg);
            (
                format!("{} RAM", total_str),
                format!("{}/worker", avg_str),
            )
        }
        AlertCategory::HighCpu => (
            format!("{:.1}% CPU", total),
            format!("{:.1}%/worker", avg),
        ),
        AlertCategory::HighDiskIo => {
            let total_str = format_bytes_f64(total);
            let avg_str = format_bytes_f64(avg);
            (
                format!("{} I/O", total_str),
                format!("{}/worker", avg_str),
            )
        }
        _ => (
            format!("{:.1}", total),
            format!("{:.1}/worker", avg),
        ),
    }
}

/// Format a threshold value for display.
fn format_threshold_for_category(category: AlertCategory, threshold: f64) -> String {
    match category {
        AlertCategory::HighMemory | AlertCategory::MemoryLeak => {
            format_bytes_f64(threshold)
        }
        AlertCategory::HighCpu => format!("{:.1}%", threshold),
        AlertCategory::HighDiskIo => format_bytes_f64(threshold),
        _ => format!("{:.1}", threshold),
    }
}

/// Format the average value for display in grouped alert summaries.
fn format_avg_value(category: AlertCategory, avg: f64) -> String {
    match category {
        AlertCategory::HighMemory | AlertCategory::MemoryLeak => format_bytes_f64(avg),
        AlertCategory::HighCpu => format!("{:.1}%", avg),
        AlertCategory::HighDiskIo => format_bytes_f64(avg),
        _ => format!("{:.1}", avg),
    }
}

/// Format a single worker's value for per-PID breakdown display.
fn format_worker_value(category: AlertCategory, value: f64) -> String {
    match category {
        AlertCategory::HighMemory | AlertCategory::MemoryLeak => format_bytes_f64(value),
        AlertCategory::HighCpu => format!("{:.1}%", value),
        AlertCategory::HighDiskIo => format_bytes_f64(value),
        _ => format!("{:.1}", value),
    }
}

/// Format bytes (as f64) into human-readable string.
fn format_bytes_f64(bytes: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{:.0} B", bytes)
    }
}

/// Format an alert as an HTML Telegram message.
///
/// Uses severity emoji, bold title with human-readable category name,
/// and `<code>` blocks for values.
pub fn format_alert(alert: &Alert, hostname: &str) -> String {
    let emoji = severity_emoji(alert.severity);
    let sev_label = severity_label(alert.severity);
    let cat_name = category_display_name(alert.category);
    let timestamp = alert.timestamp.format("%Y-%m-%d %H:%M:%S");

    format!(
        "{} <b>{}: {}</b>\n\n\
         {}\n\n\
         Process: <code>{}</code> (PID {})\n\
         Host: <code>{}</code>\n\
         Time: <code>{}</code>",
        emoji,
        sev_label,
        cat_name,
        alert.message,
        alert.process_name,
        alert.pid,
        hostname,
        timestamp,
    )
}

/// Format an alert with system context (#9).
///
/// Appends a "System Status" section with CPU, memory, thermal, and security info.
pub fn format_alert_with_context(alert: &Alert, hostname: &str, ctx: &AlertContext) -> String {
    let mut text = format_alert(alert, hostname);
    text.push_str("\n\n<b>System Status:</b>");
    append_context(&mut text, ctx);
    text
}

/// Append compact inline system context to a grouped alert message.
///
/// Uses the format: `🖥️ CPU 45% | RAM 78% | 245 procs`
/// This is inserted before the footer line (📍 hostname | 🕐 time).
fn append_inline_context(text: &mut String, ctx: &AlertContext) {
    let mut parts = Vec::new();
    if let Some(cpu) = ctx.cpu_pct {
        parts.push(format!("CPU {:.0}%", cpu));
    }
    if let Some(mem) = ctx.mem_pct {
        parts.push(format!("RAM {:.0}%", mem));
    }
    if let Some(procs) = ctx.process_count {
        parts.push(format!("{} procs", procs));
    }
    if let Some(temp) = ctx.max_temp {
        parts.push(format!("{:.0}°C", temp));
    }
    if !parts.is_empty() {
        // Insert before the last line (📍 footer)
        // Find the last 📍 line and insert before it
        if let Some(pos) = text.rfind("\n\n\u{1F4CD}") {
            let context_line = format!("\n\u{1F5A5}\u{FE0F} {}", parts.join(" | "));
            text.insert_str(pos, &context_line);
        } else {
            text.push_str(&format!("\n\u{1F5A5}\u{FE0F} {}", parts.join(" | ")));
        }
    }
}

/// Append system context lines to an existing message.
fn append_context(text: &mut String, ctx: &AlertContext) {
    if let Some(cpu) = ctx.cpu_pct {
        text.push_str(&format!("\nCPU: <code>{:.1}%</code>", cpu));
    }
    if let Some(mem) = ctx.mem_pct {
        text.push_str(&format!("\nMemory: <code>{:.1}%</code>", mem));
    }
    if let Some(temp) = ctx.max_temp {
        text.push_str(&format!("\nMax Temp: <code>{:.1}°C</code>", temp));
    }
    if let Some(score) = ctx.security_score {
        text.push_str(&format!("\nSecurity: <code>{}/100</code>", score));
    }
    if let Some(procs) = ctx.process_count {
        text.push_str(&format!("\nProcesses: <code>{}</code>", procs));
    }
    if let Some(ref uptime) = ctx.uptime {
        text.push_str(&format!("\nUptime: <code>{}</code>", uptime));
    }
}

/// Format a digest summary of multiple alerts (#8).
///
/// Groups alerts by severity and produces a single summary message.
#[allow(dead_code)] // called by tick_digest
pub fn format_digest(alerts: &[(Alert, String)], hostname: &str) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    let danger_count = alerts
        .iter()
        .filter(|(a, _)| a.severity == AlertSeverity::Danger)
        .count();
    let critical_count = alerts
        .iter()
        .filter(|(a, _)| a.severity == AlertSeverity::Critical)
        .count();
    let warning_count = alerts
        .iter()
        .filter(|(a, _)| a.severity == AlertSeverity::Warning)
        .count();
    let info_count = alerts
        .iter()
        .filter(|(a, _)| a.severity == AlertSeverity::Info)
        .count();

    let mut text = format!(
        "\u{1F4CB} <b>Sentinel Digest — {} alerts</b>\n\
         Host: <code>{}</code>\n\
         Time: <code>{}</code>\n",
        alerts.len(),
        hostname,
        timestamp,
    );

    // Severity breakdown
    if danger_count > 0 {
        text.push_str(&format!("\n\u{1F6A8} Danger: {}", danger_count));
    }
    if critical_count > 0 {
        text.push_str(&format!("\n\u{2757} Critical: {}", critical_count));
    }
    if warning_count > 0 {
        text.push_str(&format!("\n\u{26A0}\u{FE0F} Warning: {}", warning_count));
    }
    if info_count > 0 {
        text.push_str(&format!("\n\u{2139}\u{FE0F} Info: {}", info_count));
    }

    // List individual alerts (truncated)
    text.push_str("\n\n<b>Details:</b>");
    for (alert, _) in alerts.iter().take(10) {
        text.push_str(&format!(
            "\n{} <code>{}</code>: {}",
            severity_emoji(alert.severity), alert.category, alert.message
        ));
    }

    if alerts.len() > 10 {
        text.push_str(&format!("\n... and {} more", alerts.len() - 10));
    }

    text
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn make_alert(severity: AlertSeverity, category: AlertCategory) -> Alert {
        Alert {
            severity,
            category,
            process_name: "test_proc".into(),
            pid: 1234,
            message: "Test alert message".into(),
            timestamp: Local::now(),
            value: 92.3,
            threshold: 50.0,
        }
    }

    #[test]
    fn from_config_returns_none_when_token_missing() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let _guard = EnvGuard::new(&[
            crate::constants::ENV_TELEGRAM_BOT_TOKEN,
            crate::constants::ENV_TELEGRAM_CHAT_ID,
        ]);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_BOT_TOKEN);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_CHAT_ID);

        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: None,
            telegram_chat_id: Some("12345".into()),
            ..NotificationConfig::default()
        };
        assert!(TelegramNotifier::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_none_when_chat_id_missing() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let _guard = EnvGuard::new(&[
            crate::constants::ENV_TELEGRAM_BOT_TOKEN,
            crate::constants::ENV_TELEGRAM_CHAT_ID,
        ]);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_BOT_TOKEN);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_CHAT_ID);

        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("token123".into()),
            telegram_chat_id: None,
            ..NotificationConfig::default()
        };
        assert!(TelegramNotifier::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_none_when_token_empty() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let _guard = EnvGuard::new(&[
            crate::constants::ENV_TELEGRAM_BOT_TOKEN,
            crate::constants::ENV_TELEGRAM_CHAT_ID,
        ]);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_BOT_TOKEN);
        std::env::remove_var(crate::constants::ENV_TELEGRAM_CHAT_ID);

        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some(String::new()),
            telegram_chat_id: Some("12345".into()),
            ..NotificationConfig::default()
        };
        assert!(TelegramNotifier::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_some_when_configured() {
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("bot123:ABC".into()),
            telegram_chat_id: Some("12345".into()),
            telegram_min_severity: "critical".into(),
            ..NotificationConfig::default()
        };
        let notifier = TelegramNotifier::from_config(&config).expect("Should create notifier");
        assert_eq!(notifier.bot_token, "bot123:ABC");
        assert_eq!(notifier.chat_id, "12345");
        assert_eq!(notifier.min_severity, AlertSeverity::Critical);
    }

    #[test]
    fn parse_min_severity_variants() {
        assert_eq!(parse_min_severity("warning"), AlertSeverity::Warning);
        assert_eq!(parse_min_severity("warn"), AlertSeverity::Warning);
        assert_eq!(parse_min_severity("critical"), AlertSeverity::Critical);
        assert_eq!(parse_min_severity("danger"), AlertSeverity::Danger);
        assert_eq!(parse_min_severity("info"), AlertSeverity::Info);
        // Unknown defaults to Warning
        assert_eq!(parse_min_severity("unknown"), AlertSeverity::Warning);
        assert_eq!(parse_min_severity(""), AlertSeverity::Warning);
    }

    #[test]
    fn severity_filter_blocks_low_severity() {
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("token".into()),
            telegram_chat_id: Some("chat".into()),
            telegram_min_severity: "critical".into(),
            ..NotificationConfig::default()
        };
        let notifier = TelegramNotifier::from_config(&config).unwrap();

        // Warning is below Critical — should be blocked
        let warning = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);
        assert!(!notifier.should_send(&warning));

        // Critical should pass
        let critical = make_alert(AlertSeverity::Critical, AlertCategory::HighCpu);
        assert!(notifier.should_send(&critical));

        // Danger is above Critical — should pass
        let danger = make_alert(AlertSeverity::Danger, AlertCategory::ThermalEmergency);
        assert!(notifier.should_send(&danger));
    }

    #[test]
    fn rate_limiting_blocks_duplicate() {
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("token".into()),
            telegram_chat_id: Some("chat".into()),
            telegram_min_severity: "warning".into(),
            ..NotificationConfig::default()
        };
        let mut notifier = TelegramNotifier::from_config(&config).unwrap();

        let alert = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);

        // First should pass
        assert!(notifier.should_send(&alert));

        // Simulate a send (key is now (category, app_name))
        notifier
            .last_sent
            .insert((alert.category, alert.process_name.clone()), Instant::now());

        // Second should be rate-limited (same process_name)
        assert!(!notifier.should_send(&alert));

        // Different process name should still pass
        let mut diff_name = alert.clone();
        diff_name.process_name = "other_proc".into();
        assert!(notifier.should_send(&diff_name));

        // Different category should still pass
        let diff_cat = make_alert(AlertSeverity::Warning, AlertCategory::HighMemory);
        assert!(notifier.should_send(&diff_cat));
    }

    #[test]
    fn format_alert_contains_key_fields() {
        let alert = make_alert(AlertSeverity::Critical, AlertCategory::HighCpu);
        let text = format_alert(&alert, "my-machine");

        assert!(text.contains("CRITICAL"), "Should contain severity");
        assert!(text.contains("High CPU Usage"), "Should contain human-readable category");
        assert!(text.contains("test_proc"), "Should contain process name");
        assert!(text.contains("1234"), "Should contain PID");
        assert!(text.contains("my-machine"), "Should contain hostname");
        assert!(text.contains("Test alert message"), "Should contain message");
    }

    #[test]
    fn format_alert_severity_emojis() {
        let warning = format_alert(
            &make_alert(AlertSeverity::Warning, AlertCategory::HighCpu),
            "host",
        );
        assert!(warning.contains("\u{26A0}"), "Warning should have ⚠️");

        let danger = format_alert(
            &make_alert(AlertSeverity::Danger, AlertCategory::ThermalEmergency),
            "host",
        );
        assert!(danger.contains("\u{1F6A8}"), "Danger should have 🚨");
    }

    // ── Env var fallback tests (#10) ─────────────────────────────

    // Mutex for env var tests (prevent parallel interference)
    use std::sync::Mutex;
    static TG_ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&str]) -> Self {
            let saved = keys
                .iter()
                .map(|k| (k.to_string(), std::env::var(k).ok()))
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, val) in &self.saved {
                match val {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn from_config_falls_back_to_env_vars() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let token_key = crate::constants::ENV_TELEGRAM_BOT_TOKEN;
        let chat_key = crate::constants::ENV_TELEGRAM_CHAT_ID;
        let _guard = EnvGuard::new(&[token_key, chat_key]);

        std::env::set_var(token_key, "env_bot_token:ABC");
        std::env::set_var(chat_key, "env_chat_12345");

        // Config has no credentials — should fall back to env
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: None,
            telegram_chat_id: None,
            ..NotificationConfig::default()
        };
        let notifier = TelegramNotifier::from_config(&config)
            .expect("Should create notifier from env vars");
        assert_eq!(notifier.bot_token, "env_bot_token:ABC");
        assert_eq!(notifier.chat_id, "env_chat_12345");
    }

    #[test]
    fn from_config_prefers_config_over_env() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let token_key = crate::constants::ENV_TELEGRAM_BOT_TOKEN;
        let chat_key = crate::constants::ENV_TELEGRAM_CHAT_ID;
        let _guard = EnvGuard::new(&[token_key, chat_key]);

        std::env::set_var(token_key, "env_token");
        std::env::set_var(chat_key, "env_chat");

        // Config has credentials — should take priority
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("config_token".into()),
            telegram_chat_id: Some("config_chat".into()),
            ..NotificationConfig::default()
        };
        let notifier = TelegramNotifier::from_config(&config)
            .expect("Should create notifier from config");
        assert_eq!(notifier.bot_token, "config_token");
        assert_eq!(notifier.chat_id, "config_chat");
    }

    #[test]
    fn from_config_returns_none_when_both_missing() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let token_key = crate::constants::ENV_TELEGRAM_BOT_TOKEN;
        let chat_key = crate::constants::ENV_TELEGRAM_CHAT_ID;
        let _guard = EnvGuard::new(&[token_key, chat_key]);

        std::env::remove_var(token_key);
        std::env::remove_var(chat_key);

        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: None,
            telegram_chat_id: None,
            ..NotificationConfig::default()
        };
        assert!(TelegramNotifier::from_config(&config).is_none());
    }

    // ── Rich context tests (#9) ──────────────────────────────────

    #[test]
    fn format_alert_with_context_includes_system_info() {
        let alert = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);
        let ctx = AlertContext {
            cpu_pct: Some(92.5),
            mem_pct: Some(78.3),
            max_temp: Some(85.0),
            security_score: Some(72),
            process_count: Some(245),
            uptime: Some("3d 12h".into()),
        };
        let text = format_alert_with_context(&alert, "my-host", &ctx);

        assert!(text.contains("System Status"), "Should have system status section");
        assert!(text.contains("92.5%"), "Should contain CPU %");
        assert!(text.contains("78.3%"), "Should contain memory %");
        assert!(text.contains("85.0°C"), "Should contain temp");
        assert!(text.contains("72/100"), "Should contain security score");
        assert!(text.contains("245"), "Should contain process count");
        assert!(text.contains("3d 12h"), "Should contain uptime");
    }

    #[test]
    fn format_alert_with_context_handles_partial_info() {
        let alert = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);
        let ctx = AlertContext {
            cpu_pct: Some(50.0),
            ..AlertContext::default()
        };
        let text = format_alert_with_context(&alert, "host", &ctx);

        assert!(text.contains("50.0%"), "Should contain CPU");
        assert!(!text.contains("Memory:"), "Should not contain memory when None");
        assert!(!text.contains("Max Temp:"), "Should not contain temp when None");
    }

    // ── Digest tests (#8) ────────────────────────────────────────

    #[test]
    fn format_digest_summarizes_alerts() {
        let alerts = vec![
            (make_alert(AlertSeverity::Danger, AlertCategory::SecurityThreat), String::new()),
            (make_alert(AlertSeverity::Critical, AlertCategory::HighCpu), String::new()),
            (make_alert(AlertSeverity::Warning, AlertCategory::HighMemory), String::new()),
            (make_alert(AlertSeverity::Warning, AlertCategory::HighDiskIo), String::new()),
        ];
        let text = format_digest(&alerts, "my-server");

        assert!(text.contains("4 alerts"), "Should show total count");
        assert!(text.contains("my-server"), "Should show hostname");
        assert!(text.contains("Danger: 1"), "Should count danger alerts");
        assert!(text.contains("Critical: 1"), "Should count critical alerts");
        assert!(text.contains("Warning: 2"), "Should count warning alerts");
        assert!(text.contains("Details:"), "Should have details section");
    }

    #[test]
    fn format_digest_truncates_long_list() {
        let alerts: Vec<(Alert, String)> = (0..15)
            .map(|_| (make_alert(AlertSeverity::Warning, AlertCategory::HighCpu), String::new()))
            .collect();
        let text = format_digest(&alerts, "host");

        assert!(text.contains("... and 5 more"), "Should truncate after 10");
    }

    #[test]
    fn digest_buffer_caps_at_max() {
        let _lock = TG_ENV_MUTEX.lock().unwrap();
        let token_key = crate::constants::ENV_TELEGRAM_BOT_TOKEN;
        let chat_key = crate::constants::ENV_TELEGRAM_CHAT_ID;
        let _guard = EnvGuard::new(&[token_key, chat_key]);

        std::env::set_var(token_key, "test_token");
        std::env::set_var(chat_key, "test_chat");

        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("test_token".into()),
            telegram_chat_id: Some("test_chat".into()),
            telegram_min_severity: "info".into(),
            ..NotificationConfig::default()
        };
        let mut notifier = TelegramNotifier::from_config(&config).unwrap();
        notifier.digest_interval = Duration::from_secs(60); // Enable digest

        // Buffer 30 alerts (cap is 20)
        for i in 0..30u32 {
            let mut alert = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);
            alert.pid = i;
            alert.process_name = format!("proc_{}", i); // Unique names to bypass rate limit
            notifier.send_alert(&alert, "host");
        }

        assert!(
            notifier.digest_buffer.len() <= TELEGRAM_DIGEST_MAX_ALERTS,
            "Buffer should be capped at {}, got {}",
            TELEGRAM_DIGEST_MAX_ALERTS,
            notifier.digest_buffer.len(),
        );
    }

    // ── Alert grouping tests ─────────────────────────────────────

    use crate::models::{ProcessInfo, ProcessStatus};

    fn make_process(pid: u32, name: &str, parent_pid: Option<u32>) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: name.to_string(),
            cpu_usage: 0.0,
            memory_bytes: 0,
            memory_percent: 0.0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: ProcessStatus::Running,
            user: "test".to_string(),
            start_time: 0,
            parent_pid,
            thread_count: None,
        }
    }

    fn make_alert_with_name(
        severity: AlertSeverity,
        category: AlertCategory,
        name: &str,
        pid: u32,
        value: f64,
        threshold: f64,
    ) -> Alert {
        Alert {
            severity,
            category,
            process_name: name.into(),
            pid,
            message: format!("{} using {:.1}", name, value),
            timestamp: Local::now(),
            value,
            threshold,
        }
    }

    // ── is_likely_worker_name ─────────────────────────────────────

    #[test]
    fn worker_name_detection() {
        assert!(is_likely_worker_name("tokio-rt-worker"));
        assert!(is_likely_worker_name("node-V8Worker"));
        assert!(is_likely_worker_name("ThreadPoolForeg"));
        assert!(is_likely_worker_name("actix-rt-worker"));
        assert!(is_likely_worker_name("pool-2-thread"));
        assert!(!is_likely_worker_name("node"));
        assert!(!is_likely_worker_name("firefox"));
        assert!(!is_likely_worker_name("postgres"));
        assert!(!is_likely_worker_name("nginx"));
    }

    // ── resolve_parent_app_name ──────────────────────────────────

    #[test]
    fn resolve_parent_direct_parent() {
        let procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
        ];
        let pid_map: HashMap<u32, &ProcessInfo> = procs.iter().map(|p| (p.pid, p)).collect();

        let name = resolve_parent_app_name(101, "node-V8Worker", &pid_map);
        assert_eq!(name, "node");
    }

    #[test]
    fn resolve_parent_two_hops() {
        // worker → intermediate-worker → real-app
        let procs = vec![
            make_process(100, "myapp", None),
            make_process(101, "pool-worker", Some(100)),
            make_process(102, "pool-worker-thread", Some(101)),
        ];
        let pid_map: HashMap<u32, &ProcessInfo> = procs.iter().map(|p| (p.pid, p)).collect();

        let name = resolve_parent_app_name(102, "pool-worker-thread", &pid_map);
        assert_eq!(name, "myapp");
    }

    #[test]
    fn resolve_parent_non_worker_returns_self() {
        let procs = vec![make_process(100, "firefox", None)];
        let pid_map: HashMap<u32, &ProcessInfo> = procs.iter().map(|p| (p.pid, p)).collect();

        let name = resolve_parent_app_name(100, "firefox", &pid_map);
        assert_eq!(name, "firefox");
    }

    #[test]
    fn resolve_parent_stops_at_init() {
        // Worker whose parent is PID 1 (init) — can't resolve further
        let procs = vec![
            make_process(1, "systemd", None),
            make_process(50, "tokio-rt-worker", Some(1)),
        ];
        let pid_map: HashMap<u32, &ProcessInfo> = procs.iter().map(|p| (p.pid, p)).collect();

        let name = resolve_parent_app_name(50, "tokio-rt-worker", &pid_map);
        assert_eq!(name, "tokio-rt-worker", "Should return self when parent is init");
    }

    #[test]
    fn resolve_parent_missing_parent_returns_self() {
        let procs = vec![make_process(200, "actix-rt-worker", Some(999))];
        let pid_map: HashMap<u32, &ProcessInfo> = procs.iter().map(|p| (p.pid, p)).collect();

        let name = resolve_parent_app_name(200, "actix-rt-worker", &pid_map);
        assert_eq!(name, "actix-rt-worker", "Should return self when parent PID not in map");
    }

    // ── group_alerts_by_app ──────────────────────────────────────

    #[test]
    fn group_alerts_workers_same_parent() {
        let procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
            make_process(102, "node-V8Worker", Some(100)),
            make_process(103, "node-V8Worker", Some(100)),
            make_process(104, "node-V8Worker", Some(100)),
        ];
        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighMemory, "node-V8Worker", 101, 1e9, 1e9),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighMemory, "node-V8Worker", 102, 1e9, 1e9),
            make_alert_with_name(AlertSeverity::Critical, AlertCategory::HighMemory, "node-V8Worker", 103, 1.5e9, 1e9),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighMemory, "node-V8Worker", 104, 0.5e9, 1e9),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);

        assert_eq!(groups.len(), 1, "Should produce 1 group");
        let g = &groups[0];
        assert_eq!(g.app_name, "node");
        assert_eq!(g.category, AlertCategory::HighMemory);
        assert_eq!(g.severity, AlertSeverity::Critical, "Should take max severity");
        assert_eq!(g.worker_count, 4);
        assert!((g.total_value - 4e9).abs() < 1.0, "Should sum values");
        assert_eq!(g.pids.len(), 4);
    }

    #[test]
    fn group_alerts_non_groupable_stays_separate() {
        let procs = vec![make_process(0, "thermal", None)];
        let alerts = vec![
            make_alert_with_name(AlertSeverity::Critical, AlertCategory::ThermalCritical, "thermal", 0, 95.0, 85.0),
            make_alert_with_name(AlertSeverity::Danger, AlertCategory::ThermalEmergency, "thermal", 0, 100.0, 95.0),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);

        assert_eq!(groups.len(), 2, "Thermal alerts should not be grouped together (different categories)");
    }

    #[test]
    fn group_alerts_mixed_apps() {
        let procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
            make_process(102, "node-V8Worker", Some(100)),
            make_process(200, "firefox", None),
        ];
        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 101, 60.0, 50.0),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 102, 70.0, 50.0),
            make_alert_with_name(AlertSeverity::Critical, AlertCategory::HighCpu, "firefox", 200, 95.0, 50.0),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);

        assert_eq!(groups.len(), 2, "Should produce 2 groups (node + firefox)");
        // Sorted by severity desc, so firefox (Critical) should be first
        assert_eq!(groups[0].app_name, "firefox");
        assert_eq!(groups[0].worker_count, 1);
        assert_eq!(groups[1].app_name, "node");
        assert_eq!(groups[1].worker_count, 2);
    }

    #[test]
    fn group_alerts_empty() {
        let groups = group_alerts_by_app(&[], &[]);
        assert!(groups.is_empty());
    }

    // ── format_grouped_alert ─────────────────────────────────────

    #[test]
    fn format_grouped_single_falls_back() {
        let group = GroupedAlert {
            app_name: "firefox".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Critical,
            worker_count: 1,
            total_value: 95.0,
            avg_percent: 95.0,
            threshold: 50.0,
            pids: vec![200],
            worker_details: vec![WorkerDetail {
                pid: 200,
                name: "firefox".into(),
                value: 95.0,
                percent: 95.0,
            }],
            representative_message: "High CPU usage".into(),
        };

        let text = format_grouped_alert(&group, "my-server");
        assert!(text.contains("CRITICAL"));
        assert!(text.contains("High CPU Usage"), "Should use human-readable category name");
        assert!(text.contains("firefox"));
        assert!(text.contains("200"));
        assert!(text.contains("my-server"));
        assert!(!text.contains("workers"), "Single alert should not show 'workers'");
    }

    #[test]
    fn format_grouped_multi_shows_detailed_breakdown() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Warning,
            worker_count: 4,
            total_value: 4.0 * gib,
            avg_percent: gib,
            threshold: gib,
            pids: vec![101, 102, 103, 104],
            worker_details: vec![
                WorkerDetail { pid: 101, name: "node-V8Worker".into(), value: 1.2 * gib, percent: 75.0 },
                WorkerDetail { pid: 102, name: "node-V8Worker".into(), value: 1.1 * gib, percent: 68.8 },
                WorkerDetail { pid: 103, name: "node-V8Worker".into(), value: 1.1 * gib, percent: 68.8 },
                WorkerDetail { pid: 104, name: "node-V8Worker".into(), value: 0.6 * gib, percent: 31.3 },
            ],
            representative_message: "High memory usage".into(),
        };

        let text = format_grouped_alert(&group, "my-server");
        assert!(text.contains("WARNING"), "Should contain severity label");
        assert!(text.contains("High Memory Usage"), "Should use human-readable category name");
        assert!(text.contains("node"), "Should contain app name");
        assert!(text.contains("4 workers"), "Should show worker count");
        assert!(text.contains("GiB"), "Should format bytes for memory");
        assert!(text.contains("Per-Worker Breakdown"), "Should have per-worker section");
        assert!(text.contains("PID 101"), "Should list individual PIDs");
        assert!(text.contains("PID 104"), "Should list all PIDs");
        assert!(text.contains("V8Worker"), "Should show worker name");
        assert!(text.contains("75.0%"), "Should show percent for first worker");
        assert!(text.contains("What's happening"), "Should have explanation section");
        assert!(text.contains("Action"), "Should have action section");
        assert!(text.contains("my-server"), "Should contain hostname");
        assert!(text.contains("\u{2514}"), "Should contain tree end char └");
    }

    #[test]
    fn format_grouped_truncates_many_workers() {
        let worker_details: Vec<WorkerDetail> = (100..112u32)
            .map(|pid| WorkerDetail {
                pid,
                name: "node-V8Worker".into(),
                value: 60.0,
                percent: 60.0,
            })
            .collect();
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Warning,
            worker_count: 12,
            total_value: 720.0,
            avg_percent: 60.0,
            threshold: 50.0,
            pids: (100..112).collect(),
            worker_details,
            representative_message: "High CPU".into(),
        };

        let text = format_grouped_alert(&group, "host");
        assert!(text.contains("+4 more"), "Should truncate worker list after MAX_WORKER_DISPLAY (8), showing +4 more");
    }

    // ── format_bytes_f64 ─────────────────────────────────────────

    #[test]
    fn format_bytes_f64_ranges() {
        assert_eq!(format_bytes_f64(512.0), "512 B");
        assert_eq!(format_bytes_f64(1024.0), "1.0 KiB");
        assert_eq!(format_bytes_f64(1024.0 * 1024.0), "1.0 MiB");
        assert_eq!(format_bytes_f64(1024.0 * 1024.0 * 1024.0), "1.0 GiB");
        assert_eq!(format_bytes_f64(1.5 * 1024.0 * 1024.0 * 1024.0), "1.5 GiB");
    }

    // ── is_groupable_category ────────────────────────────────────

    #[test]
    fn groupable_categories() {
        assert!(is_groupable_category(AlertCategory::HighCpu));
        assert!(is_groupable_category(AlertCategory::HighMemory));
        assert!(is_groupable_category(AlertCategory::HighDiskIo));
        assert!(is_groupable_category(AlertCategory::Zombie));
        assert!(is_groupable_category(AlertCategory::MemoryLeak));

        // Non-groupable
        assert!(!is_groupable_category(AlertCategory::ThermalWarning));
        assert!(!is_groupable_category(AlertCategory::ThermalCritical));
        assert!(!is_groupable_category(AlertCategory::ThermalEmergency));
        assert!(!is_groupable_category(AlertCategory::SecurityThreat));
        assert!(!is_groupable_category(AlertCategory::SecurityScore));
        assert!(!is_groupable_category(AlertCategory::SystemOverload));
    }

    // ── send_grouped_alerts rate limiting ─────────────────────────

    #[test]
    fn send_grouped_alerts_rate_limits_by_app_name() {
        let config = NotificationConfig {
            telegram_enabled: true,
            telegram_bot_token: Some("token".into()),
            telegram_chat_id: Some("chat".into()),
            telegram_min_severity: "warning".into(),
            ..NotificationConfig::default()
        };
        let mut notifier = TelegramNotifier::from_config(&config).unwrap();
        notifier.digest_interval = Duration::from_secs(60); // buffer mode so we can inspect

        let procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
            make_process(102, "node-V8Worker", Some(100)),
        ];
        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 101, 60.0, 50.0),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 102, 70.0, 50.0),
        ];

        // First call should produce a grouped message
        notifier.send_grouped_alerts(&alerts, &procs, "host", None);
        assert_eq!(notifier.digest_buffer.len(), 1, "Should send 1 grouped message");

        // Second call should be rate-limited
        notifier.send_grouped_alerts(&alerts, &procs, "host", None);
        assert_eq!(notifier.digest_buffer.len(), 1, "Should still be 1 (rate-limited)");
    }

    // ── avg_percent calculation ───────────────────────────────────

    #[test]
    fn group_computes_correct_average() {
        let procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
            make_process(102, "node-V8Worker", Some(100)),
        ];
        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 101, 60.0, 50.0),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 102, 80.0, 50.0),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert!((g.total_value - 140.0).abs() < f64::EPSILON);
        assert!((g.avg_percent - 70.0).abs() < f64::EPSILON);
    }

    // ── category_display_name ─────────────────────────────────────

    #[test]
    fn category_display_name_returns_human_readable() {
        assert_eq!(category_display_name(AlertCategory::HighCpu), "High CPU Usage");
        assert_eq!(category_display_name(AlertCategory::HighMemory), "High Memory Usage");
        assert_eq!(category_display_name(AlertCategory::HighDiskIo), "High Disk I/O");
        assert_eq!(category_display_name(AlertCategory::Zombie), "Zombie Processes");
        assert_eq!(category_display_name(AlertCategory::Suspicious), "Suspicious Activity");
        assert_eq!(category_display_name(AlertCategory::SystemOverload), "System Overload");
        assert_eq!(category_display_name(AlertCategory::MemoryLeak), "Memory Leak Detected");
        assert_eq!(category_display_name(AlertCategory::SecurityThreat), "Security Threat");
        assert_eq!(category_display_name(AlertCategory::SecurityScore), "Security Score Drop");
        assert_eq!(category_display_name(AlertCategory::ThermalWarning), "Thermal Warning");
        assert_eq!(category_display_name(AlertCategory::ThermalCritical), "Thermal Critical");
        assert_eq!(category_display_name(AlertCategory::ThermalEmergency), "Thermal Emergency");
        assert_eq!(category_display_name(AlertCategory::WindowsFirewall), "Windows Firewall Issue");
        assert_eq!(category_display_name(AlertCategory::WindowsDefender), "Windows Defender Issue");
        assert_eq!(category_display_name(AlertCategory::WindowsUpdates), "Windows Updates Issue");
    }

    #[test]
    fn category_display_name_not_abbreviated() {
        // Ensure no category returns the old terse abbreviations
        let all_categories = [
            AlertCategory::HighCpu, AlertCategory::HighMemory,
            AlertCategory::HighDiskIo, AlertCategory::Zombie,
            AlertCategory::Suspicious, AlertCategory::SystemOverload,
            AlertCategory::MemoryLeak, AlertCategory::SecurityThreat,
            AlertCategory::SecurityScore, AlertCategory::ThermalWarning,
            AlertCategory::ThermalCritical, AlertCategory::ThermalEmergency,
            AlertCategory::WindowsFirewall, AlertCategory::WindowsDefender,
            AlertCategory::WindowsUpdates,
        ];
        for cat in &all_categories {
            let name = category_display_name(*cat);
            assert!(name.len() > 5, "Display name for {:?} should be descriptive, got '{}'", cat, name);
            assert!(name.contains(' '), "Display name for {:?} should have spaces, got '{}'", cat, name);
        }
    }

    // ── WorkerDetail ──────────────────────────────────────────────

    #[test]
    fn worker_detail_fields() {
        let detail = WorkerDetail {
            pid: 101,
            name: "node-V8Worker".into(),
            value: 1.2 * 1024.0 * 1024.0 * 1024.0,
            percent: 75.0,
        };
        assert_eq!(detail.pid, 101);
        assert_eq!(detail.name, "node-V8Worker");
        assert!((detail.percent - 75.0).abs() < f32::EPSILON);
    }

    #[test]
    fn worker_details_populated_from_process_info() {
        let mut procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
            make_process(102, "node-V8Worker", Some(100)),
        ];
        // Set realistic memory values on the process info
        procs[1].memory_bytes = 1_073_741_824; // 1 GiB
        procs[1].memory_percent = 25.0;
        procs[2].memory_bytes = 536_870_912; // 512 MiB
        procs[2].memory_percent = 12.5;

        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighMemory, "node-V8Worker", 101, 1e9, 1e9),
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighMemory, "node-V8Worker", 102, 5e8, 1e9),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.worker_details.len(), 2, "Should have 2 worker details");

        // Check that worker_details were populated from ProcessInfo (memory_bytes)
        let detail_101 = g.worker_details.iter().find(|d| d.pid == 101).unwrap();
        assert_eq!(detail_101.value as u64, 1_073_741_824, "Should use memory_bytes from ProcessInfo");
        assert!((detail_101.percent - 25.0).abs() < f32::EPSILON, "Should use memory_percent from ProcessInfo");

        let detail_102 = g.worker_details.iter().find(|d| d.pid == 102).unwrap();
        assert_eq!(detail_102.value as u64, 536_870_912);
        assert!((detail_102.percent - 12.5).abs() < f32::EPSILON);
    }

    #[test]
    fn worker_details_populated_for_cpu_category() {
        let mut procs = vec![
            make_process(100, "node", None),
            make_process(101, "node-V8Worker", Some(100)),
        ];
        procs[1].cpu_usage = 85.5;

        let alerts = vec![
            make_alert_with_name(AlertSeverity::Warning, AlertCategory::HighCpu, "node-V8Worker", 101, 85.5, 50.0),
        ];

        let groups = group_alerts_by_app(&alerts, &procs);
        assert_eq!(groups.len(), 1);
        let detail = &groups[0].worker_details[0];
        assert!((detail.value - 85.5).abs() < 0.1, "CPU value should come from cpu_usage");
        assert!((detail.percent - 85.5).abs() < 0.1, "CPU percent should equal cpu_usage");
    }

    // ── explain_group ─────────────────────────────────────────────

    #[test]
    fn explain_group_memory_high() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Warning,
            worker_count: 4,
            total_value: 4.0 * gib,
            avg_percent: gib,
            threshold: gib,
            pids: vec![101, 102, 103, 104],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = explain_group(&group);
        assert!(text.contains("node"), "Should mention app name");
        assert!(text.contains("4"), "Should mention worker count");
        assert!(text.contains("memory"), "Should mention memory for HighMemory");
        assert!(text.contains("4x threshold"), "Should show multiplier");
    }

    #[test]
    fn explain_group_cpu_high() {
        let group = GroupedAlert {
            app_name: "myapp".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Critical,
            worker_count: 2,
            total_value: 180.0,
            avg_percent: 90.0,
            threshold: 50.0,
            pids: vec![201, 202],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = explain_group(&group);
        assert!(text.contains("myapp"), "Should mention app name");
        assert!(text.contains("CPU"), "Should mention CPU for HighCpu");
        assert!(text.contains("180.0%"), "Should show total value");
    }

    #[test]
    fn explain_group_memory_leak() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let group = GroupedAlert {
            app_name: "java".into(),
            category: AlertCategory::MemoryLeak,
            severity: AlertSeverity::Warning,
            worker_count: 3,
            total_value: 2.0 * gib,
            avg_percent: 0.67 * gib,
            threshold: gib,
            pids: vec![301, 302, 303],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = explain_group(&group);
        assert!(text.contains("memory leak"), "Should mention memory leak");
        assert!(text.contains("java"), "Should mention app name");
    }

    #[test]
    fn explain_group_zombie() {
        let group = GroupedAlert {
            app_name: "nginx".into(),
            category: AlertCategory::Zombie,
            severity: AlertSeverity::Warning,
            worker_count: 5,
            total_value: 5.0,
            avg_percent: 1.0,
            threshold: 1.0,
            pids: vec![401, 402, 403, 404, 405],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = explain_group(&group);
        assert!(text.contains("zombie"), "Should mention zombie");
        assert!(text.contains("nginx"), "Should mention app name");
        assert!(text.contains("5"), "Should mention count");
    }

    #[test]
    fn explain_group_extreme_multiplier() {
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Danger,
            worker_count: 8,
            total_value: 400.0,
            avg_percent: 50.0,
            threshold: 50.0,
            pids: (1..9).collect(),
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = explain_group(&group);
        // 400 / 50 = 8x > WORKER_EXTREME_MULTIPLIER (3.0)
        assert!(text.contains("significant"), "Should use 'significant' for extreme multiplier");
    }

    // ── suggest_action ────────────────────────────────────────────

    #[test]
    fn suggest_action_memory_warning() {
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Warning,
            worker_count: 2,
            total_value: 2e9,
            avg_percent: 1e9,
            threshold: 1e9,
            pids: vec![101, 102],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("memory"), "Should mention memory");
        assert!(!text.contains("Immediate"), "Warning should not say 'Immediate action'");
    }

    #[test]
    fn suggest_action_memory_critical() {
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Critical,
            worker_count: 2,
            total_value: 4e9,
            avg_percent: 2e9,
            threshold: 1e9,
            pids: vec![101, 102],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("Immediate"), "Critical should say 'Immediate action'");
    }

    #[test]
    fn suggest_action_cpu_warning() {
        let group = GroupedAlert {
            app_name: "python".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Warning,
            worker_count: 3,
            total_value: 180.0,
            avg_percent: 60.0,
            threshold: 50.0,
            pids: vec![201, 202, 203],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("concurrency") || text.contains("parallelism"),
            "CPU warning should mention concurrency or parallelism");
    }

    #[test]
    fn suggest_action_cpu_danger() {
        let group = GroupedAlert {
            app_name: "python".into(),
            category: AlertCategory::HighCpu,
            severity: AlertSeverity::Danger,
            worker_count: 3,
            total_value: 280.0,
            avg_percent: 93.3,
            threshold: 50.0,
            pids: vec![201, 202, 203],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("saturation") || text.contains("infinite loop"),
            "CPU danger should mention saturation or infinite loops");
    }

    #[test]
    fn suggest_action_disk_io() {
        let group = GroupedAlert {
            app_name: "rsync".into(),
            category: AlertCategory::HighDiskIo,
            severity: AlertSeverity::Warning,
            worker_count: 2,
            total_value: 1e9,
            avg_percent: 5e8,
            threshold: 5e8,
            pids: vec![301, 302],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.to_lowercase().contains("disk") || text.to_lowercase().contains("i/o"),
            "Disk I/O action should mention disk");
    }

    #[test]
    fn suggest_action_zombie() {
        let group = GroupedAlert {
            app_name: "bash".into(),
            category: AlertCategory::Zombie,
            severity: AlertSeverity::Warning,
            worker_count: 3,
            total_value: 3.0,
            avg_percent: 1.0,
            threshold: 1.0,
            pids: vec![401, 402, 403],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("wait()") || text.contains("parent"),
            "Zombie action should mention wait() or parent");
    }

    #[test]
    fn suggest_action_fallback_category() {
        let group = GroupedAlert {
            app_name: "scanner".into(),
            category: AlertCategory::Suspicious,
            severity: AlertSeverity::Warning,
            worker_count: 2,
            total_value: 2.0,
            avg_percent: 1.0,
            threshold: 1.0,
            pids: vec![501, 502],
            worker_details: Vec::new(),
            representative_message: String::new(),
        };

        let text = suggest_action(&group);
        assert!(text.contains("scanner"), "Fallback should mention app name");
        assert!(text.contains("Suspicious Activity"), "Fallback should use category_display_name");
    }

    // ── Worker details sorting in format ──────────────────────────

    #[test]
    fn format_grouped_sorts_workers_by_value_descending() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Warning,
            worker_count: 3,
            total_value: 3.0 * gib,
            avg_percent: gib,
            threshold: gib,
            pids: vec![101, 102, 103],
            worker_details: vec![
                WorkerDetail { pid: 101, name: "node-V8Worker".into(), value: 0.5 * gib, percent: 10.0 },
                WorkerDetail { pid: 102, name: "node-V8Worker".into(), value: 1.5 * gib, percent: 30.0 },
                WorkerDetail { pid: 103, name: "node-V8Worker".into(), value: 1.0 * gib, percent: 20.0 },
            ],
            representative_message: String::new(),
        };

        let text = format_grouped_alert(&group, "host");
        // PID 102 (1.5 GiB) should appear before PID 103 (1.0 GiB) and PID 101 (0.5 GiB)
        let pos_102 = text.find("PID 102").expect("Should contain PID 102");
        let pos_103 = text.find("PID 103").expect("Should contain PID 103");
        let pos_101 = text.find("PID 101").expect("Should contain PID 101");
        assert!(pos_102 < pos_103, "PID 102 (1.5 GiB) should appear before PID 103 (1.0 GiB)");
        assert!(pos_103 < pos_101, "PID 103 (1.0 GiB) should appear before PID 101 (0.5 GiB)");
    }

    // ── Inline context in grouped alerts ──────────────────────────

    #[test]
    fn format_grouped_with_context_shows_inline() {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let group = GroupedAlert {
            app_name: "node".into(),
            category: AlertCategory::HighMemory,
            severity: AlertSeverity::Warning,
            worker_count: 2,
            total_value: 2.0 * gib,
            avg_percent: gib,
            threshold: gib,
            pids: vec![101, 102],
            worker_details: vec![
                WorkerDetail { pid: 101, name: "node-V8Worker".into(), value: gib, percent: 50.0 },
                WorkerDetail { pid: 102, name: "node-V8Worker".into(), value: gib, percent: 50.0 },
            ],
            representative_message: String::new(),
        };

        let mut text = format_grouped_alert(&group, "my-server");
        let ctx = AlertContext {
            cpu_pct: Some(45.0),
            mem_pct: Some(78.0),
            process_count: Some(245),
            ..AlertContext::default()
        };
        append_inline_context(&mut text, &ctx);

        assert!(text.contains("CPU 45%"), "Should show CPU inline");
        assert!(text.contains("RAM 78%"), "Should show RAM inline");
        assert!(text.contains("245 procs"), "Should show process count inline");
    }
}
