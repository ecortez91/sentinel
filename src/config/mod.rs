use serde::Deserialize;

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
    /// Auto-analysis interval in seconds (0 = disabled)
    pub auto_analysis_interval_secs: u64,
    /// Theme name (built-in or custom)
    pub theme: String,
    /// UI language (en, ja, es, de, zh)
    pub lang: String,
    /// Thermal monitoring configuration
    pub thermal: ThermalConfig,
    /// Email notification configuration
    pub notifications: NotificationConfig,
}

/// Thermal monitoring settings (LibreHardwareMonitor integration).
#[derive(Debug, Clone)]
pub struct ThermalConfig {
    /// LHM HTTP JSON endpoint URL.
    pub lhm_url: String,
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

/// Email notification settings.
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Whether email notifications are enabled (still requires .env SMTP credentials).
    pub email_enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            email_enabled: true,
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
            auto_analysis_interval_secs: DEFAULT_AUTO_ANALYSIS_SECS,
            theme: "default".to_string(),
            lang: "en".to_string(),
            thermal: ThermalConfig::default(),
            notifications: NotificationConfig::default(),
        }
    }
}

/// TOML-deserializable config file format.
/// All fields are optional — missing fields use defaults.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    refresh_interval_ms: Option<u64>,
    cpu_warning_threshold: Option<f32>,
    cpu_critical_threshold: Option<f32>,
    /// Memory thresholds in MiB (more user-friendly than raw bytes)
    mem_warning_threshold_mib: Option<u64>,
    mem_critical_threshold_mib: Option<u64>,
    sys_mem_warning_percent: Option<f32>,
    sys_mem_critical_percent: Option<f32>,
    max_alerts: Option<usize>,
    suspicious_patterns: Option<Vec<String>>,
    security_threat_patterns: Option<Vec<String>>,
    auto_analysis_interval_secs: Option<u64>,
    theme: Option<String>,
    lang: Option<String>,
    thermal: Option<FileThermalConfig>,
    notifications: Option<FileNotificationConfig>,
}

/// TOML-deserializable thermal config section.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FileThermalConfig {
    lhm_url: Option<String>,
    poll_interval_secs: Option<u64>,
    warning_threshold: Option<f32>,
    critical_threshold: Option<f32>,
    emergency_threshold: Option<f32>,
    sustained_seconds: Option<u64>,
    auto_shutdown_enabled: Option<bool>,
    shutdown_schedule_start: Option<u8>,
    shutdown_schedule_end: Option<u8>,
}

/// TOML-deserializable notification config section.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FileNotificationConfig {
    email_enabled: Option<bool>,
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

        // Merge thermal config
        if let Some(t) = file_config.thermal {
            if let Some(v) = t.lhm_url {
                if !v.is_empty() {
                    config.thermal.lhm_url = v;
                }
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
        }

        config
    }
}
