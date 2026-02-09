//! Email notification system for Sentinel alerts.
//!
//! Uses lettre with Gmail SMTP (STARTTLS on port 587, rustls).
//! Credentials are loaded from `~/.config/sentinel/.env` via dotenvy.
//! Rate-limited: max 1 email per event type per 5 minutes.
//!
//! IMPORTANT: Credentials (SMTP user, password, recipient) are ONLY
//! stored in the .env file and NEVER committed to source control.

use std::collections::HashMap;
use std::time::Instant;

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::constants::{DEFAULT_SMTP_PORT, DEFAULT_SMTP_SERVER, EMAIL_RATE_LIMIT_SECS};

/// Event types for rate limiting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotifyEvent {
    /// Temperature crossed critical threshold.
    ThermalCritical,
    /// Sustained emergency temperature.
    ThermalEmergency,
    /// Shutdown imminent (grace period started).
    ShutdownImminent,
    /// System recovered from thermal emergency.
    Recovered,
    /// Test email.
    Test,
}

impl NotifyEvent {
    /// Subject line for each event type.
    fn subject(&self) -> &str {
        match self {
            NotifyEvent::ThermalCritical => "[Sentinel] CRITICAL: Temperature threshold exceeded",
            NotifyEvent::ThermalEmergency => "[Sentinel] EMERGENCY: Sustained high temperature",
            NotifyEvent::ShutdownImminent => "[Sentinel] SHUTDOWN IMMINENT: Auto-shutdown triggered",
            NotifyEvent::Recovered => "[Sentinel] RECOVERED: Temperature returned to normal",
            NotifyEvent::Test => "[Sentinel] Test email - notifications working",
        }
    }
}

/// SMTP configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub server: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub recipient: String,
}

impl SmtpConfig {
    /// Load SMTP config from environment variables.
    /// Returns None if required vars are missing.
    pub fn from_env() -> Option<Self> {
        let username = std::env::var("SENTINEL_SMTP_USER").ok()?;
        let password = std::env::var("SENTINEL_SMTP_PASSWORD").ok()?;
        let recipient = std::env::var("SENTINEL_SMTP_RECIPIENT").ok()?;

        if username.is_empty() || password.is_empty() || recipient.is_empty() {
            return None;
        }

        let server = std::env::var("SENTINEL_SMTP_SERVER")
            .unwrap_or_else(|_| DEFAULT_SMTP_SERVER.to_string());
        let port = std::env::var("SENTINEL_SMTP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SMTP_PORT);

        Some(Self {
            server,
            port,
            username,
            password,
            recipient,
        })
    }
}

/// Email notification manager with rate limiting.
pub struct EmailNotifier {
    config: SmtpConfig,
    /// Last send time per event type for rate limiting.
    last_sent: HashMap<NotifyEvent, Instant>,
    /// Rate limit interval.
    rate_limit: std::time::Duration,
}

impl EmailNotifier {
    /// Create a new notifier from SMTP config.
    pub fn new(config: SmtpConfig) -> Self {
        Self {
            config,
            last_sent: HashMap::new(),
            rate_limit: std::time::Duration::from_secs(EMAIL_RATE_LIMIT_SECS),
        }
    }

    /// Try to create a notifier from environment variables.
    /// Returns None if SMTP credentials are not configured.
    pub fn from_env() -> Option<Self> {
        SmtpConfig::from_env().map(Self::new)
    }

    /// Whether the notifier has valid config (always true if constructed).
    #[allow(dead_code)]
    pub fn is_configured(&self) -> bool {
        !self.config.username.is_empty() && !self.config.recipient.is_empty()
    }

    /// Check if we can send for this event type (rate limiting).
    fn can_send(&self, event: &NotifyEvent) -> bool {
        match self.last_sent.get(event) {
            Some(last) => last.elapsed() >= self.rate_limit,
            None => true,
        }
    }

    /// Public rate limit check (for use before spawning async tasks).
    pub fn can_send_check(&self, event: &NotifyEvent) -> bool {
        self.can_send(event)
    }

    /// Mark an event as sent (for pre-spawn rate limit tracking).
    pub fn mark_sent(&mut self, event: &NotifyEvent) {
        self.last_sent.insert(event.clone(), Instant::now());
    }

    /// Get a reference to the SMTP config (for creating temporary notifiers).
    pub fn config(&self) -> &SmtpConfig {
        &self.config
    }

    /// Send a notification email. Returns Ok(()) on success.
    /// Rate-limited: returns Ok(()) silently if within cooldown.
    pub async fn notify(&mut self, event: NotifyEvent, body: &str) -> Result<(), String> {
        if !self.can_send(&event) {
            return Ok(()); // Rate limited, silently skip
        }

        let result = self.send_email(event.subject(), body).await;

        if result.is_ok() {
            self.last_sent.insert(event, Instant::now());
        }

        result
    }

    /// Send a test email (bypasses rate limiting).
    pub async fn send_test(&mut self) -> Result<(), String> {
        let body = format!(
            "Sentinel email notifications are working.\n\n\
             SMTP Server: {}:{}\n\
             From: {}\n\
             To: {}\n\n\
             This is a test email sent by the :email-test command.",
            self.config.server, self.config.port,
            self.config.username, self.config.recipient,
        );

        let result = self.send_email(NotifyEvent::Test.subject(), &body).await;

        if result.is_ok() {
            self.last_sent.insert(NotifyEvent::Test, Instant::now());
        }

        result
    }

    /// Low-level email send via lettre.
    async fn send_email(&self, subject: &str, body: &str) -> Result<(), String> {
        let email = Message::builder()
            .from(
                self.config
                    .username
                    .parse()
                    .map_err(|e| format!("Invalid from address: {}", e))?,
            )
            .to(self.config
                .recipient
                .parse()
                .map_err(|e| format!("Invalid recipient address: {}", e))?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| format!("Failed to build email: {}", e))?;

        let creds = Credentials::new(
            self.config.username.clone(),
            self.config.password.clone(),
        );

        let mailer: AsyncSmtpTransport<Tokio1Executor> =
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.config.server)
                .map_err(|e| format!("SMTP relay error: {}", e))?
                .port(self.config.port)
                .credentials(creds)
                .build();

        mailer
            .send(email)
            .await
            .map_err(|e| format!("SMTP send error: {}", e))?;

        Ok(())
    }
}

/// Build a thermal alert email body.
pub fn thermal_alert_body(
    event: &NotifyEvent,
    temp: f32,
    sensor: &str,
    hostname: &str,
) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    match event {
        NotifyEvent::ThermalCritical => format!(
            "Sentinel Thermal Alert\n\
             ======================\n\n\
             Severity: CRITICAL\n\
             Sensor: {}\n\
             Temperature: {:.1}°C\n\
             Host: {}\n\
             Time: {}\n\n\
             The temperature has exceeded the critical threshold.\n\
             If this persists, auto-shutdown may be triggered.",
            sensor, temp, hostname, timestamp,
        ),
        NotifyEvent::ThermalEmergency => format!(
            "Sentinel Thermal EMERGENCY\n\
             ==========================\n\n\
             Severity: EMERGENCY\n\
             Sensor: {}\n\
             Temperature: {:.1}°C\n\
             Host: {}\n\
             Time: {}\n\n\
             Temperature has been at emergency levels for a sustained period.\n\
             Auto-shutdown may be initiated if enabled.",
            sensor, temp, hostname, timestamp,
        ),
        NotifyEvent::ShutdownImminent => format!(
            "Sentinel AUTO-SHUTDOWN IMMINENT\n\
             ===============================\n\n\
             Severity: EMERGENCY\n\
             Sensor: {}\n\
             Temperature: {:.1}°C\n\
             Host: {}\n\
             Time: {}\n\n\
             The system will shut down in 30 seconds unless:\n\
             - Temperature drops below critical threshold\n\
             - User aborts via Ctrl+X in Sentinel\n\n\
             This shutdown is to protect hardware from thermal damage.",
            sensor, temp, hostname, timestamp,
        ),
        NotifyEvent::Recovered => format!(
            "Sentinel Recovery Notice\n\
             ========================\n\n\
             Status: RECOVERED\n\
             Sensor: {}\n\
             Current Temperature: {:.1}°C\n\
             Host: {}\n\
             Time: {}\n\n\
             Temperature has returned to safe levels. The system is operating normally.",
            sensor, temp, hostname, timestamp,
        ),
        NotifyEvent::Test => format!(
            "Sentinel Test Email\n\
             Host: {}\n\
             Time: {}",
            hostname, timestamp,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_config_from_env_missing_returns_none() {
        // Ensure env vars are not set (they shouldn't be in tests)
        std::env::remove_var("SENTINEL_SMTP_USER");
        std::env::remove_var("SENTINEL_SMTP_PASSWORD");
        std::env::remove_var("SENTINEL_SMTP_RECIPIENT");
        assert!(SmtpConfig::from_env().is_none());
    }

    #[test]
    fn notify_event_subjects() {
        assert!(NotifyEvent::ThermalCritical.subject().contains("CRITICAL"));
        assert!(NotifyEvent::ThermalEmergency.subject().contains("EMERGENCY"));
        assert!(NotifyEvent::ShutdownImminent.subject().contains("SHUTDOWN"));
        assert!(NotifyEvent::Recovered.subject().contains("RECOVERED"));
        assert!(NotifyEvent::Test.subject().contains("Test"));
    }

    #[test]
    fn rate_limiting_works() {
        let config = SmtpConfig {
            server: "localhost".to_string(),
            port: 587,
            username: "test@test.com".to_string(),
            password: "pass".to_string(),
            recipient: "dest@test.com".to_string(),
        };
        let mut notifier = EmailNotifier::new(config);

        // First check should pass
        assert!(notifier.can_send(&NotifyEvent::ThermalCritical));

        // Simulate a send
        notifier.last_sent.insert(NotifyEvent::ThermalCritical, Instant::now());

        // Second check should fail (within rate limit)
        assert!(!notifier.can_send(&NotifyEvent::ThermalCritical));

        // Different event type should still pass
        assert!(notifier.can_send(&NotifyEvent::Recovered));
    }

    #[test]
    fn thermal_alert_body_formatting() {
        let body = thermal_alert_body(
            &NotifyEvent::ThermalCritical,
            98.5,
            "CPU Package",
            "my-pc",
        );
        assert!(body.contains("CRITICAL"));
        assert!(body.contains("CPU Package"));
        assert!(body.contains("98.5°C"));
        assert!(body.contains("my-pc"));
    }

    #[test]
    fn is_configured_checks_fields() {
        let config = SmtpConfig {
            server: "smtp.gmail.com".to_string(),
            port: 587,
            username: "user@gmail.com".to_string(),
            password: "pass".to_string(),
            recipient: "dest@gmail.com".to_string(),
        };
        let notifier = EmailNotifier::new(config);
        assert!(notifier.is_configured());
    }
}
