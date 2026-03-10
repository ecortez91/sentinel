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
    DEFAULT_TELEGRAM_DIGEST_SECS, TELEGRAM_API_BASE, TELEGRAM_DIGEST_MAX_ALERTS,
    TELEGRAM_RATE_LIMIT_SECS,
};
use crate::models::{Alert, AlertCategory, AlertSeverity};

/// Telegram notification manager with rate limiting, severity filtering,
/// and optional digest mode (#8).
pub struct TelegramNotifier {
    bot_token: String,
    chat_id: String,
    min_severity: AlertSeverity,
    client: reqwest::Client,
    /// Last send time per (category, PID) for rate limiting.
    last_sent: HashMap<(AlertCategory, u32), Instant>,
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
        // Severity gate
        if alert.severity < self.min_severity {
            return false;
        }
        // Rate limit per (category, PID)
        let key = (alert.category, alert.pid);
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
            .insert((alert.category, alert.pid), Instant::now());

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

/// Format an alert as an HTML Telegram message.
///
/// Uses severity emoji, bold title, and `<code>` blocks for values.
pub fn format_alert(alert: &Alert, hostname: &str) -> String {
    let emoji = match alert.severity {
        AlertSeverity::Danger => "\u{1F6A8}",  // 🚨
        AlertSeverity::Critical => "\u{2757}",  // ❗
        AlertSeverity::Warning => "\u{26A0}\u{FE0F}",  // ⚠️
        AlertSeverity::Info => "\u{2139}\u{FE0F}",     // ℹ️
    };

    let severity_label = match alert.severity {
        AlertSeverity::Danger => "DANGER",
        AlertSeverity::Critical => "CRITICAL",
        AlertSeverity::Warning => "WARNING",
        AlertSeverity::Info => "INFO",
    };

    let timestamp = alert.timestamp.format("%Y-%m-%d %H:%M:%S");

    format!(
        "{} <b>{}: {}</b>\n\n\
         {}\n\n\
         Process: <code>{}</code> (PID {})\n\
         Host: <code>{}</code>\n\
         Time: <code>{}</code>",
        emoji,
        severity_label,
        alert.category,
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

    text
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
        let emoji = match alert.severity {
            AlertSeverity::Danger => "\u{1F6A8}",
            AlertSeverity::Critical => "\u{2757}",
            AlertSeverity::Warning => "\u{26A0}\u{FE0F}",
            AlertSeverity::Info => "\u{2139}\u{FE0F}",
        };
        text.push_str(&format!(
            "\n{} <code>{}</code>: {}",
            emoji, alert.category, alert.message
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

        // Simulate a send
        notifier
            .last_sent
            .insert((alert.category, alert.pid), Instant::now());

        // Second should be rate-limited
        assert!(!notifier.should_send(&alert));

        // Different PID should still pass
        let mut diff_pid = alert.clone();
        diff_pid.pid = 5678;
        assert!(notifier.should_send(&diff_pid));

        // Different category should still pass
        let diff_cat = make_alert(AlertSeverity::Warning, AlertCategory::HighMemory);
        assert!(notifier.should_send(&diff_cat));
    }

    #[test]
    fn format_alert_contains_key_fields() {
        let alert = make_alert(AlertSeverity::Critical, AlertCategory::HighCpu);
        let text = format_alert(&alert, "my-machine");

        assert!(text.contains("CRITICAL"), "Should contain severity");
        assert!(text.contains("CPU"), "Should contain category");
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
        for i in 0..30 {
            let mut alert = make_alert(AlertSeverity::Warning, AlertCategory::HighCpu);
            alert.pid = i; // Unique PIDs to bypass rate limit
            notifier.send_alert(&alert, "host");
        }

        assert!(
            notifier.digest_buffer.len() <= TELEGRAM_DIGEST_MAX_ALERTS,
            "Buffer should be capped at {}, got {}",
            TELEGRAM_DIGEST_MAX_ALERTS,
            notifier.digest_buffer.len(),
        );
    }
}
