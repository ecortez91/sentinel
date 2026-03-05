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
use crate::constants::{TELEGRAM_API_BASE, TELEGRAM_RATE_LIMIT_SECS};
use crate::models::{Alert, AlertCategory, AlertSeverity};

/// Telegram notification manager with rate limiting and severity filtering.
pub struct TelegramNotifier {
    bot_token: String,
    chat_id: String,
    min_severity: AlertSeverity,
    client: reqwest::Client,
    /// Last send time per (category, PID) for rate limiting.
    last_sent: HashMap<(AlertCategory, u32), Instant>,
    rate_limit: Duration,
}

impl TelegramNotifier {
    /// Create a notifier from notification config.
    ///
    /// Returns `None` if Telegram is not configured (missing token or chat ID).
    pub fn from_config(config: &NotificationConfig) -> Option<Self> {
        let token = config.telegram_bot_token.as_deref().unwrap_or("");
        let chat_id = config.telegram_chat_id.as_deref().unwrap_or("");

        if token.is_empty() || chat_id.is_empty() {
            return None;
        }

        Some(Self {
            bot_token: token.to_string(),
            chat_id: chat_id.to_string(),
            min_severity: parse_min_severity(&config.telegram_min_severity),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            last_sent: HashMap::new(),
            rate_limit: Duration::from_secs(TELEGRAM_RATE_LIMIT_SECS),
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

    /// Send an alert via Telegram if it passes filters.
    ///
    /// The actual HTTP request is spawned as a background task so it never
    /// blocks the main event loop.
    pub fn send_alert(&mut self, alert: &Alert, hostname: &str) {
        if !self.should_send(alert) {
            return;
        }

        // Mark sent before spawning (prevents duplicates during async flight)
        self.last_sent
            .insert((alert.category, alert.pid), Instant::now());

        let text = format_alert(alert, hostname);
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
}
