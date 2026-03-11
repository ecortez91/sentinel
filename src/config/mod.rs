use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::constants::*;

/// Application configuration with sensible defaults.
///
/// Can be overridden via ~/.config/sentinel/config.toml
#[derive(Debug, Clone)]
pub struct Config {
    /// Refresh interval in milliseconds
    pub refresh_interval_ms: u64,
    /// CPU usage threshold for warning (percent)
    pub cpu_warning_threshold: f32,
    /// CPU usage threshold for critical (percent)
    pub cpu_critical_threshold: f32,
    /// Memory usage per-process threshold for warning (bytes)
    pub mem_warning_threshold_bytes: u64,
    /// Memory usage per-process threshold for critical (bytes)
    pub mem_critical_threshold_bytes: u64,
    /// System memory usage threshold for warning (percent)
    pub sys_mem_warning_percent: f32,
    /// System memory usage threshold for critical (percent)
    pub sys_mem_critical_percent: f32,
    /// Max alerts to keep in history
    pub max_alerts: usize,
    /// Suspicious process name patterns
    pub suspicious_patterns: Vec<String>,
    /// Known crypto miners and malware patterns
    pub security_threat_patterns: Vec<String>,
    /// Parent process names whose zombie children are silently ignored
    pub ignored_zombie_parents: Vec<String>,
    /// Auto-analysis interval in seconds (0 = disabled)
    pub auto_analysis_interval_secs: u64,
    /// Theme name (built-in or custom)
    pub theme: String,
    /// UI language (en, ja, es, de, zh)
    pub lang: String,
    /// Unicode rendering mode: "auto", "unicode", or "ascii"
    pub unicode_mode: String,
    /// Thermal monitoring configuration
    pub thermal: ThermalConfig,
    /// Email notification configuration
    pub notifications: NotificationConfig,
    /// Market data plugin configuration
    pub market: MarketConfig,
    /// Security monitoring configuration (#16)
    pub security: SecurityConfig,
}

/// Thermal monitoring settings (LibreHardwareMonitor integration).
#[derive(Debug, Clone)]
pub struct ThermalConfig {
    /// LHM HTTP JSON endpoint URL.
    pub lhm_url: String,
    /// HTTP Basic Auth username (None = no auth from config).
    pub lhm_username: Option<String>,
    /// HTTP Basic Auth password (None = no auth from config).
    pub lhm_password: Option<String>,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Temperature warning threshold (Celsius).
    pub warning_threshold: f32,
    /// Temperature critical threshold (Celsius).
    pub critical_threshold: f32,
    /// Temperature emergency threshold (Celsius).
    pub emergency_threshold: f32,
    /// Sustained seconds at emergency before shutdown escalation.
    pub sustained_seconds: u64,
    /// Enable auto-shutdown state machine (OFF by default, also requires .env flag).
    pub auto_shutdown_enabled: bool,
    /// Schedule start hour (0-23, shutdown only active during window).
    pub shutdown_schedule_start: u8,
    /// Schedule end hour (0-23).
    pub shutdown_schedule_end: u8,
}

impl Default for ThermalConfig {
    fn default() -> Self {
        Self {
            lhm_url: DEFAULT_LHM_URL.to_string(),
            lhm_username: Some("TwisteD_Clawdbot".to_string()),
            lhm_password: Some("Test123!@".to_string()),
            poll_interval_secs: DEFAULT_THERMAL_POLL_SECS,
            warning_threshold: DEFAULT_THERMAL_WARNING_C,
            critical_threshold: DEFAULT_THERMAL_CRITICAL_C,
            emergency_threshold: DEFAULT_THERMAL_EMERGENCY_C,
            sustained_seconds: DEFAULT_THERMAL_SUSTAINED_SECS,
            auto_shutdown_enabled: false,
            shutdown_schedule_start: DEFAULT_SHUTDOWN_SCHEDULE_START,
            shutdown_schedule_end: DEFAULT_SHUTDOWN_SCHEDULE_END,
        }
    }
}

/// Market data plugin settings (Binance integration).
#[derive(Debug, Clone)]
pub struct MarketConfig {
    /// Whether the market data plugin is enabled.
    pub enabled: bool,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Watchlist of trading pairs (e.g., ["BTCUSDT", "ETHUSDT"]).
    pub tickers: Vec<String>,
    /// Default chart time range (1h, 4h, 1d, 7d, 30d).
    pub default_chart_range: String,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: DEFAULT_MARKET_POLL_SECS,
            tickers: vec![
                "BTCUSDT".to_string(),
                "ETHUSDT".to_string(),
                "SOLUSDT".to_string(),
                "BNBUSDT".to_string(),
                "XRPUSDT".to_string(),
            ],
            default_chart_range: "1d".to_string(),
        }
    }
}

/// Security monitoring settings (#16).
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Number of failed SSH login attempts from one IP to flag as brute-force.
    pub ssh_brute_force_threshold: usize,
    /// Security score threshold — alert when score drops below this.
    pub score_alert_threshold: u8,
    /// Maximum suspicious outbound connections to track.
    pub max_suspicious_outbound: usize,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            ssh_brute_force_threshold: SSH_BRUTE_FORCE_THRESHOLD,
            score_alert_threshold: SECURITY_SCORE_ALERT_THRESHOLD,
            max_suspicious_outbound: MAX_SUSPICIOUS_OUTBOUND,
        }
    }
}

/// Notification settings (email + Telegram).
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Whether email notifications are enabled (still requires .env SMTP credentials).
    pub email_enabled: bool,
    /// Whether Telegram notifications are enabled.
    pub telegram_enabled: bool,
    /// Telegram Bot API token (from @BotFather).
    pub telegram_bot_token: Option<String>,
    /// Telegram chat ID to send alerts to.
    pub telegram_chat_id: Option<String>,
    /// Minimum severity for Telegram alerts: "warning", "critical", or "danger".
    pub telegram_min_severity: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            email_enabled: true,
            telegram_enabled: false,
            telegram_bot_token: None,
            telegram_chat_id: None,
            telegram_min_severity: "warning".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_interval_ms: DEFAULT_REFRESH_MS,
            cpu_warning_threshold: DEFAULT_CPU_WARNING_PCT,
            cpu_critical_threshold: DEFAULT_CPU_CRITICAL_PCT,
            // 1 GiB warning, 2 GiB critical per process
            mem_warning_threshold_bytes: DEFAULT_MEM_WARNING_BYTES,
            mem_critical_threshold_bytes: DEFAULT_MEM_CRITICAL_BYTES,
            // System-wide memory
            sys_mem_warning_percent: DEFAULT_SYS_MEM_WARNING_PCT,
            sys_mem_critical_percent: DEFAULT_SYS_MEM_CRITICAL_PCT,
            max_alerts: DEFAULT_MAX_ALERTS,
            suspicious_patterns: vec![
                // Note: kworker is intentionally excluded — it's a legitimate
                // Linux kernel worker thread present on every system.
                "kdevtmpfsi".to_string(),
                "xmrig".to_string(),
                "minerd".to_string(),
                "cpuminer".to_string(),
                "cryptonight".to_string(),
                "stratum".to_string(),
            ],
            // Patterns unique to security threats; overlap with
            // suspicious_patterns (xmrig, minerd, cpuminer, kdevtmpfsi,
            // cryptonight) was removed to avoid duplication.
            security_threat_patterns: vec![
                "kinsing".to_string(),
                "bindshell".to_string(),
                "reverse_shell".to_string(),
                "nc -e".to_string(),
                "ncat -e".to_string(),
                "coinhive".to_string(),
            ],
            ignored_zombie_parents: DEFAULT_IGNORED_ZOMBIE_PARENTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            auto_analysis_interval_secs: DEFAULT_AUTO_ANALYSIS_SECS,
            theme: "dracula".to_string(),
            lang: "en".to_string(),
            unicode_mode: "auto".to_string(),
            thermal: ThermalConfig::default(),
            notifications: NotificationConfig::default(),
            market: MarketConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

/// TOML-deserializable config file format.
/// All fields are optional — missing fields use defaults.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct FileConfig {
    pub(crate) refresh_interval_ms: Option<u64>,
    pub(crate) cpu_warning_threshold: Option<f32>,
    pub(crate) cpu_critical_threshold: Option<f32>,
    /// Memory thresholds in MiB (more user-friendly than raw bytes)
    pub(crate) mem_warning_threshold_mib: Option<u64>,
    pub(crate) mem_critical_threshold_mib: Option<u64>,
    pub(crate) sys_mem_warning_percent: Option<f32>,
    pub(crate) sys_mem_critical_percent: Option<f32>,
    pub(crate) max_alerts: Option<usize>,
    pub(crate) suspicious_patterns: Option<Vec<String>>,
    pub(crate) security_threat_patterns: Option<Vec<String>>,
    pub(crate) ignored_zombie_parents: Option<Vec<String>>,
    pub(crate) auto_analysis_interval_secs: Option<u64>,
    pub(crate) theme: Option<String>,
    pub(crate) lang: Option<String>,
    pub(crate) unicode_mode: Option<String>,
    pub(crate) thermal: Option<FileThermalConfig>,
    pub(crate) notifications: Option<FileNotificationConfig>,
    pub(crate) market: Option<FileMarketConfig>,
    pub(crate) security: Option<FileSecurityConfig>,
}

/// TOML-deserializable thermal config section.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct FileThermalConfig {
    pub(crate) lhm_url: Option<String>,
    pub(crate) lhm_username: Option<String>,
    pub(crate) lhm_password: Option<String>,
    pub(crate) poll_interval_secs: Option<u64>,
    pub(crate) warning_threshold: Option<f32>,
    pub(crate) critical_threshold: Option<f32>,
    pub(crate) emergency_threshold: Option<f32>,
    pub(crate) sustained_seconds: Option<u64>,
    pub(crate) auto_shutdown_enabled: Option<bool>,
    pub(crate) shutdown_schedule_start: Option<u8>,
    pub(crate) shutdown_schedule_end: Option<u8>,
}

/// TOML-deserializable notification config section.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct FileNotificationConfig {
    pub(crate) email_enabled: Option<bool>,
    pub(crate) telegram_enabled: Option<bool>,
    pub(crate) telegram_bot_token: Option<String>,
    pub(crate) telegram_chat_id: Option<String>,
    pub(crate) telegram_min_severity: Option<String>,
}

/// TOML-deserializable security config section (#16).
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct FileSecurityConfig {
    pub(crate) ssh_brute_force_threshold: Option<usize>,
    pub(crate) score_alert_threshold: Option<u8>,
    pub(crate) max_suspicious_outbound: Option<usize>,
}

/// TOML-deserializable market config section.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct FileMarketConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) poll_interval_secs: Option<u64>,
    pub(crate) tickers: Option<Vec<String>>,
    pub(crate) default_chart_range: Option<String>,
}

impl Config {
    /// Load config from ~/.config/sentinel/config.toml, falling back to defaults
    /// for any missing fields. If the file doesn't exist, returns pure defaults.
    pub fn load() -> Self {
        let mut config = Config::default();

        let config_path = crate::constants::config_file_path();
        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => return config, // No config file — use defaults
        };

        let file_config: FileConfig = match toml::from_str(&content) {
            Ok(fc) => fc,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse {}: {}. Using defaults.",
                    config_path.display(),
                    e
                );
                return config;
            }
        };

        // Merge file values over defaults
        if let Some(v) = file_config.refresh_interval_ms {
            config.refresh_interval_ms = v.max(MIN_REFRESH_MS);
        }
        if let Some(v) = file_config.cpu_warning_threshold {
            config.cpu_warning_threshold = v.clamp(1.0, 100.0);
        }
        if let Some(v) = file_config.cpu_critical_threshold {
            config.cpu_critical_threshold = v.clamp(1.0, 100.0);
        }
        if let Some(v) = file_config.mem_warning_threshold_mib {
            config.mem_warning_threshold_bytes = v * 1024 * 1024;
        }
        if let Some(v) = file_config.mem_critical_threshold_mib {
            config.mem_critical_threshold_bytes = v * 1024 * 1024;
        }
        if let Some(v) = file_config.sys_mem_warning_percent {
            config.sys_mem_warning_percent = v.clamp(1.0, 100.0);
        }
        if let Some(v) = file_config.sys_mem_critical_percent {
            config.sys_mem_critical_percent = v.clamp(1.0, 100.0);
        }
        if let Some(v) = file_config.max_alerts {
            config.max_alerts = v.max(MIN_MAX_ALERTS);
        }
        if let Some(v) = file_config.suspicious_patterns {
            if !v.is_empty() {
                config.suspicious_patterns = v;
            }
        }
        if let Some(v) = file_config.security_threat_patterns {
            if !v.is_empty() {
                config.security_threat_patterns = v;
            }
        }
        if let Some(v) = file_config.ignored_zombie_parents {
            if !v.is_empty() {
                config.ignored_zombie_parents = v;
            }
        }
        if let Some(v) = file_config.auto_analysis_interval_secs {
            config.auto_analysis_interval_secs = v; // 0 = disabled
        }
        if let Some(v) = file_config.theme {
            if !v.is_empty() {
                config.theme = v;
            }
        }
        if let Some(v) = file_config.lang {
            if !v.is_empty() {
                config.lang = v;
            }
        }
        if let Some(v) = file_config.unicode_mode {
            if !v.is_empty() {
                config.unicode_mode = v;
            }
        }

        // Merge thermal config
        if let Some(t) = file_config.thermal {
            if let Some(v) = t.lhm_url {
                if !v.is_empty() {
                    config.thermal.lhm_url = v;
                }
            }
            if let Some(v) = t.lhm_username {
                config.thermal.lhm_username = if v.is_empty() { None } else { Some(v) };
            }
            if let Some(v) = t.lhm_password {
                config.thermal.lhm_password = if v.is_empty() { None } else { Some(v) };
            }
            if let Some(v) = t.poll_interval_secs {
                config.thermal.poll_interval_secs = v.max(1);
            }
            if let Some(v) = t.warning_threshold {
                config.thermal.warning_threshold = v.clamp(30.0, 150.0);
            }
            if let Some(v) = t.critical_threshold {
                config.thermal.critical_threshold = v.clamp(30.0, 150.0);
            }
            if let Some(v) = t.emergency_threshold {
                config.thermal.emergency_threshold = v.clamp(30.0, 150.0);
            }
            if let Some(v) = t.sustained_seconds {
                config.thermal.sustained_seconds = v.max(5);
            }
            if let Some(v) = t.auto_shutdown_enabled {
                config.thermal.auto_shutdown_enabled = v;
            }
            if let Some(v) = t.shutdown_schedule_start {
                config.thermal.shutdown_schedule_start = v.min(23);
            }
            if let Some(v) = t.shutdown_schedule_end {
                config.thermal.shutdown_schedule_end = v.min(24);
            }
        }

        // Merge notification config
        if let Some(n) = file_config.notifications {
            if let Some(v) = n.email_enabled {
                config.notifications.email_enabled = v;
            }
            if let Some(v) = n.telegram_enabled {
                config.notifications.telegram_enabled = v;
            }
            if let Some(v) = n.telegram_bot_token {
                config.notifications.telegram_bot_token = Some(v);
            }
            if let Some(v) = n.telegram_chat_id {
                config.notifications.telegram_chat_id = Some(v);
            }
            if let Some(v) = n.telegram_min_severity {
                config.notifications.telegram_min_severity = v;
            }
        }

        // Merge market config
        if let Some(m) = file_config.market {
            if let Some(v) = m.enabled {
                config.market.enabled = v;
            }
            if let Some(v) = m.poll_interval_secs {
                config.market.poll_interval_secs = v.max(10); // min 10s
            }
            if let Some(v) = m.tickers {
                if !v.is_empty() {
                    config.market.tickers = v;
                }
            }
            if let Some(v) = m.default_chart_range {
                if !v.is_empty() {
                    config.market.default_chart_range = v;
                }
            }
        }

        // Merge security config (#16)
        if let Some(s) = file_config.security {
            if let Some(v) = s.ssh_brute_force_threshold {
                config.security.ssh_brute_force_threshold = v.max(1);
            }
            if let Some(v) = s.score_alert_threshold {
                config.security.score_alert_threshold = v.min(100);
            }
            if let Some(v) = s.max_suspicious_outbound {
                config.security.max_suspicious_outbound = v.max(1);
            }
        }

        config
    }

    /// Persist the current configuration to `~/.config/sentinel/config.toml`.
    ///
    /// Uses atomic write (temp file + rename) to prevent corruption on crash.
    /// The generated file includes a header noting that advanced settings
    /// (arrays like `suspicious_patterns`) can be edited directly in the file.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&crate::constants::config_file_path())
    }

    /// Persist configuration to an arbitrary path (used by tests).
    ///
    /// Creates parent directories if they don't exist. Writes to a
    /// temporary file first, then atomically renames to avoid partial writes.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let write_config = WriteConfig::from(self);
        let toml_string = toml::to_string_pretty(&write_config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let content = format!(
            "# Sentinel Configuration\n\
             # This file is managed by Sentinel. Changes made in the TUI\n\
             # settings editor are saved here automatically.\n\
             #\n\
             # Advanced settings (suspicious_patterns, security_threat_patterns,\n\
             # thermal schedules, etc.) can be edited directly in this file.\n\
             # See https://github.com/sentinel for all options.\n\n{}",
            toml_string
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, path)?;

        Ok(())
    }
}

// ── Serializable config for writing to disk ────────────────────
//
// Separate from `FileConfig` (which uses `Option<T>` for partial reads)
// because the writer always emits complete values — different responsibility (SRP).

/// Top-level TOML-serializable config. All fields are concrete.
#[derive(Debug, Serialize)]
struct WriteConfig {
    refresh_interval_ms: u64,
    cpu_warning_threshold: f32,
    cpu_critical_threshold: f32,
    mem_warning_threshold_mib: u64,
    mem_critical_threshold_mib: u64,
    sys_mem_warning_percent: f32,
    sys_mem_critical_percent: f32,
    max_alerts: usize,
    suspicious_patterns: Vec<String>,
    security_threat_patterns: Vec<String>,
    ignored_zombie_parents: Vec<String>,
    auto_analysis_interval_secs: u64,
    theme: String,
    lang: String,
    unicode_mode: String,
    thermal: WriteThermalConfig,
    notifications: WriteNotificationConfig,
    market: WriteMarketConfig,
    security: WriteSecurityConfig,
}

#[derive(Debug, Serialize)]
struct WriteThermalConfig {
    lhm_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lhm_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lhm_password: Option<String>,
    poll_interval_secs: u64,
    warning_threshold: f32,
    critical_threshold: f32,
    emergency_threshold: f32,
    sustained_seconds: u64,
    auto_shutdown_enabled: bool,
    shutdown_schedule_start: u8,
    shutdown_schedule_end: u8,
}

#[derive(Debug, Serialize)]
struct WriteNotificationConfig {
    email_enabled: bool,
    telegram_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    telegram_bot_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    telegram_chat_id: Option<String>,
    telegram_min_severity: String,
}

#[derive(Debug, Serialize)]
struct WriteSecurityConfig {
    ssh_brute_force_threshold: usize,
    score_alert_threshold: u8,
    max_suspicious_outbound: usize,
}

#[derive(Debug, Serialize)]
struct WriteMarketConfig {
    enabled: bool,
    poll_interval_secs: u64,
    tickers: Vec<String>,
    default_chart_range: String,
}

impl From<&Config> for WriteConfig {
    fn from(c: &Config) -> Self {
        Self {
            refresh_interval_ms: c.refresh_interval_ms,
            cpu_warning_threshold: c.cpu_warning_threshold,
            cpu_critical_threshold: c.cpu_critical_threshold,
            mem_warning_threshold_mib: c.mem_warning_threshold_bytes / (1024 * 1024),
            mem_critical_threshold_mib: c.mem_critical_threshold_bytes / (1024 * 1024),
            sys_mem_warning_percent: c.sys_mem_warning_percent,
            sys_mem_critical_percent: c.sys_mem_critical_percent,
            max_alerts: c.max_alerts,
            suspicious_patterns: c.suspicious_patterns.clone(),
            security_threat_patterns: c.security_threat_patterns.clone(),
            ignored_zombie_parents: c.ignored_zombie_parents.clone(),
            auto_analysis_interval_secs: c.auto_analysis_interval_secs,
            theme: c.theme.clone(),
            lang: c.lang.clone(),
            unicode_mode: c.unicode_mode.clone(),
            thermal: WriteThermalConfig::from(&c.thermal),
            notifications: WriteNotificationConfig::from(&c.notifications),
            market: WriteMarketConfig::from(&c.market),
            security: WriteSecurityConfig::from(&c.security),
        }
    }
}

impl From<&ThermalConfig> for WriteThermalConfig {
    fn from(t: &ThermalConfig) -> Self {
        Self {
            lhm_url: t.lhm_url.clone(),
            lhm_username: t.lhm_username.clone(),
            lhm_password: t.lhm_password.clone(),
            poll_interval_secs: t.poll_interval_secs,
            warning_threshold: t.warning_threshold,
            critical_threshold: t.critical_threshold,
            emergency_threshold: t.emergency_threshold,
            sustained_seconds: t.sustained_seconds,
            auto_shutdown_enabled: t.auto_shutdown_enabled,
            shutdown_schedule_start: t.shutdown_schedule_start,
            shutdown_schedule_end: t.shutdown_schedule_end,
        }
    }
}

impl From<&NotificationConfig> for WriteNotificationConfig {
    fn from(n: &NotificationConfig) -> Self {
        Self {
            email_enabled: n.email_enabled,
            telegram_enabled: n.telegram_enabled,
            telegram_bot_token: n.telegram_bot_token.clone(),
            telegram_chat_id: n.telegram_chat_id.clone(),
            telegram_min_severity: n.telegram_min_severity.clone(),
        }
    }
}

impl From<&SecurityConfig> for WriteSecurityConfig {
    fn from(s: &SecurityConfig) -> Self {
        Self {
            ssh_brute_force_threshold: s.ssh_brute_force_threshold,
            score_alert_threshold: s.score_alert_threshold,
            max_suspicious_outbound: s.max_suspicious_outbound,
        }
    }
}

impl From<&MarketConfig> for WriteMarketConfig {
    fn from(m: &MarketConfig) -> Self {
        Self {
            enabled: m.enabled,
            poll_interval_secs: m.poll_interval_secs,
            tickers: m.tickers.clone(),
            default_chart_range: m.default_chart_range.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Config::save_to writes valid TOML that round-trips through deserialization.
    #[test]
    fn save_roundtrip_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = Config::default();
        config.cpu_warning_threshold = 42.0;
        config.theme = "nord".to_string();
        config.market.tickers = vec!["BTCUSDT".into(), "DOGEUSDT".into()];

        config.save_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: FileConfig = toml::from_str(&content).unwrap();

        assert_eq!(parsed.cpu_warning_threshold, Some(42.0));
        assert_eq!(parsed.theme, Some("nord".to_string()));
        assert_eq!(
            parsed.market.unwrap().tickers,
            Some(vec!["BTCUSDT".into(), "DOGEUSDT".into()])
        );
    }

    /// Config::save_to creates parent directories when missing.
    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("config.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();

        assert!(path.exists());
    }

    /// WriteConfig converts memory bytes to MiB correctly.
    #[test]
    fn write_config_bytes_to_mib_conversion() {
        let mut config = Config::default();
        config.mem_warning_threshold_bytes = 512 * 1024 * 1024; // 512 MiB
        config.mem_critical_threshold_bytes = 2048 * 1024 * 1024; // 2 GiB

        let wc = WriteConfig::from(&config);
        assert_eq!(wc.mem_warning_threshold_mib, 512);
        assert_eq!(wc.mem_critical_threshold_mib, 2048);
    }

    /// Saved file includes the documentation header.
    #[test]
    fn save_includes_documentation_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        Config::default().save_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# Sentinel Configuration"));
        assert!(content.contains("Advanced settings"));
    }

    /// Atomic write: no partial file left if the temp file exists.
    #[test]
    fn save_atomic_no_temp_file_remains() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        Config::default().save_to(&path).unwrap();

        let tmp_path = path.with_extension("toml.tmp");
        assert!(
            !tmp_path.exists(),
            "temp file should be cleaned up by rename"
        );
        assert!(path.exists());
    }

    /// Full save-then-load round-trip through Config::load-style parsing.
    #[test]
    fn save_load_full_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut original = Config::default();
        original.refresh_interval_ms = 2000;
        original.max_alerts = 42;
        original.market.poll_interval_secs = 60;
        original.thermal.warning_threshold = 55.0;
        original.notifications.email_enabled = false;
        original.save_to(&path).unwrap();

        // Parse back using the same FileConfig struct that Config::load uses
        let content = std::fs::read_to_string(&path).unwrap();
        let fc: FileConfig = toml::from_str(&content).unwrap();

        assert_eq!(fc.refresh_interval_ms, Some(2000));
        assert_eq!(fc.max_alerts, Some(42));
        assert_eq!(fc.market.unwrap().poll_interval_secs, Some(60));
        assert_eq!(fc.thermal.unwrap().warning_threshold, Some(55.0));
        assert_eq!(fc.notifications.unwrap().email_enabled, Some(false));
    }

    /// Thermal credentials survive a save-then-load round-trip.
    #[test]
    fn save_load_thermal_credentials_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = Config::default();
        config.thermal.lhm_username = Some("MyUser".into());
        config.thermal.lhm_password = Some("S3cret!@#".into());
        config.thermal.lhm_url = "http://10.0.0.5:8085/data.json".into();
        config.save_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let fc: FileConfig = toml::from_str(&content).unwrap();
        let thermal = fc.thermal.expect("thermal section should be present");

        assert_eq!(thermal.lhm_username, Some("MyUser".into()));
        assert_eq!(thermal.lhm_password, Some("S3cret!@#".into()));
        assert_eq!(
            thermal.lhm_url,
            Some("http://10.0.0.5:8085/data.json".into())
        );
    }

    /// When thermal credentials are None, they should be omitted from TOML
    /// (via skip_serializing_if) rather than written as empty strings.
    #[test]
    fn save_omits_none_thermal_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = Config::default();
        config.thermal.lhm_username = None;
        config.thermal.lhm_password = None;
        config.save_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("lhm_username"),
            "None credentials should be omitted from TOML, got:\n{}",
            content
        );
        assert!(
            !content.contains("lhm_password"),
            "None credentials should be omitted from TOML, got:\n{}",
            content
        );
    }
}
