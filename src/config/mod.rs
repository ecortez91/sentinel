use serde::Deserialize;
use std::path::PathBuf;

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_interval_ms: 1000,
            cpu_warning_threshold: 50.0,
            cpu_critical_threshold: 90.0,
            // 1 GiB warning, 2 GiB critical per process
            mem_warning_threshold_bytes: 1024 * 1024 * 1024,
            mem_critical_threshold_bytes: 2 * 1024 * 1024 * 1024,
            // System-wide memory
            sys_mem_warning_percent: 75.0,
            sys_mem_critical_percent: 90.0,
            max_alerts: 200,
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
            security_threat_patterns: vec![
                "xmrig".to_string(),
                "minerd".to_string(),
                "cpuminer".to_string(),
                "kdevtmpfsi".to_string(),
                "kinsing".to_string(),
                "bindshell".to_string(),
                "reverse_shell".to_string(),
                "nc -e".to_string(),
                "ncat -e".to_string(),
                "cryptonight".to_string(),
                "coinhive".to_string(),
            ],
            auto_analysis_interval_secs: 300, // 5 minutes
            theme: "default".to_string(),
            lang: "en".to_string(),
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
}

impl Config {
    /// Load config from ~/.config/sentinel/config.toml, falling back to defaults
    /// for any missing fields. If the file doesn't exist, returns pure defaults.
    pub fn load() -> Self {
        let mut config = Config::default();

        let config_path = config_file_path();
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
            config.refresh_interval_ms = v.max(100); // Floor at 100ms
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
            config.max_alerts = v.max(10); // At least 10
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

        config
    }
}

/// Returns ~/.config/sentinel/config.toml
fn config_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("sentinel")
        .join("config.toml")
}
