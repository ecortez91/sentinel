//! Application-wide constants.
//!
//! Centralizes all magic numbers, thresholds, and configuration defaults
//! that were previously scattered across the codebase.

use std::path::PathBuf;

// ── Timing ────────────────────────────────────────────────────────
/// Minimum allowed refresh rate (ms) to prevent excessive CPU usage.
pub const MIN_REFRESH_MS: u64 = 100;
/// Default refresh interval (ms).
pub const DEFAULT_REFRESH_MS: u64 = 1000;
/// Event poll timeout (ms) -- how often the UI checks for input.
pub const EVENT_POLL_MS: u64 = 50;
/// Ticks between system data refreshes (at EVENT_POLL_MS intervals).
pub const REFRESH_THROTTLE_TICKS: u64 = 20;
/// Ticks to wait after startup before first auto-analysis.
pub const STARTUP_SETTLE_TICKS: u64 = 5;
/// Default auto-analysis interval (seconds, 0 = disabled).
pub const DEFAULT_AUTO_ANALYSIS_SECS: u64 = 300;
/// Docker container polling interval (seconds).
pub const DOCKER_POLL_SECS: u64 = 5;
/// Status message display duration (seconds).
pub const STATUS_MESSAGE_TIMEOUT_SECS: u64 = 5;
/// Alert deduplication cooldown (seconds).
pub const ALERT_COOLDOWN_SECS: u64 = 60;
/// Initial system data settling delay (ms).
pub const INITIAL_SETTLE_MS: u64 = 250;

// ── Capacities ────────────────────────────────────────────────────
/// History buffer capacity (1 hour at 1 sample/sec).
pub const HISTORY_CAPACITY: usize = 3600;
/// Maximum conversation messages to retain.
pub const MAX_CONVERSATION_HISTORY: usize = 50;
/// Maximum alerts to keep in history.
pub const DEFAULT_MAX_ALERTS: usize = 200;
/// Minimum max_alerts floor.
pub const MIN_MAX_ALERTS: usize = 10;
/// Maximum memory history entries per process for leak detection.
pub const MAX_MEMORY_HISTORY: usize = 30;

// ── UI Layout ─────────────────────────────────────────────────────
/// Tab bar x-offset for click detection (after logo area).
pub const TAB_BAR_X_OFFSET: u16 = 22;
/// Process table content rows start at this terminal row.
pub const PROCESS_TABLE_ROW_START: u16 = 7;
/// Page up/down step size.
pub const PAGE_SIZE: usize = 20;
/// Scroll step for PageUp/PageDown in detail popup.
pub const DETAIL_PAGE_STEP: usize = 10;

// ── Process Management ────────────────────────────────────────────
/// Minimum nice value (highest priority).
pub const NICE_MIN: i32 = -20;
/// Maximum nice value (lowest priority).
pub const NICE_MAX: i32 = 19;
/// Nice value adjustment step for Up/Down arrows.
pub const NICE_STEP: i32 = 5;
/// Default signal picker selection index (SIGTERM).
pub const DEFAULT_SIGNAL_INDEX: usize = 6;
/// Maximum file descriptors to sample in process detail.
pub const MAX_FD_SAMPLE: usize = 20;
/// Maximum environment variables to show in process detail.
pub const MAX_ENV_VARS: usize = 50;
/// Maximum tree depth guard to prevent infinite recursion.
pub const MAX_TREE_DEPTH: usize = 20;

// ── Memory Thresholds ─────────────────────────────────────────────
/// 1 GiB in bytes.
pub const ONE_GIB: u64 = 1024 * 1024 * 1024;
/// Per-process memory warning threshold (bytes).
pub const DEFAULT_MEM_WARNING_BYTES: u64 = ONE_GIB;
/// Per-process memory critical threshold (bytes).
pub const DEFAULT_MEM_CRITICAL_BYTES: u64 = 2 * ONE_GIB;

// ── Alert Detection ───────────────────────────────────────────────
/// Default CPU warning threshold (percent).
pub const DEFAULT_CPU_WARNING_PCT: f32 = 50.0;
/// Default CPU critical threshold (percent).
pub const DEFAULT_CPU_CRITICAL_PCT: f32 = 90.0;
/// Default system memory warning (percent).
pub const DEFAULT_SYS_MEM_WARNING_PCT: f32 = 75.0;
/// Default system memory critical (percent).
pub const DEFAULT_SYS_MEM_CRITICAL_PCT: f32 = 90.0;
/// Minimum samples before memory leak detection triggers.
pub const LEAK_MIN_SAMPLES: usize = 10;
/// Growth factor to consider a memory leak (1.2 = 20% growth).
pub const LEAK_GROWTH_FACTOR: f64 = 1.2;
/// Minimum memory (bytes) for leak detection to apply.
pub const LEAK_MIN_MEMORY_BYTES: u64 = 100 * 1024 * 1024;
/// Memory growth percentage threshold for leak alert.
pub const LEAK_ALERT_THRESHOLD_PCT: f64 = 20.0;
/// Disk I/O threshold for high disk I/O alert (bytes).
pub const HIGH_DISK_IO_THRESHOLD: u64 = 500 * 1024 * 1024;

// ── Usage Color Thresholds ────────────────────────────────────────
/// Usage percentage above which color is "critical".
pub const USAGE_CRITICAL_PCT: f32 = 90.0;
/// Usage percentage above which color is "high".
pub const USAGE_HIGH_PCT: f32 = 70.0;
/// Usage percentage above which color is "mid".
pub const USAGE_MID_PCT: f32 = 40.0;
/// Temperature above which color is "critical" (Celsius).
pub const TEMP_CRITICAL_C: f32 = 90.0;
/// Temperature above which color is "high" (Celsius).
pub const TEMP_HIGH_C: f32 = 75.0;
/// Temperature above which color is "mid" (Celsius).
pub const TEMP_MID_C: f32 = 60.0;

// ── Thermal Monitoring ─────────────────────────────────────────────
/// Default LibreHardwareMonitor HTTP JSON URL.
pub const DEFAULT_LHM_URL: &str = "http://localhost:8085/data.json";
/// Default thermal polling interval (seconds).
pub const DEFAULT_THERMAL_POLL_SECS: u64 = 5;
/// Default thermal warning threshold (Celsius).
pub const DEFAULT_THERMAL_WARNING_C: f32 = 85.0;
/// Default thermal critical threshold (Celsius).
pub const DEFAULT_THERMAL_CRITICAL_C: f32 = 95.0;
/// Default thermal emergency threshold (Celsius).
pub const DEFAULT_THERMAL_EMERGENCY_C: f32 = 100.0;
/// Default sustained seconds before shutdown escalation.
pub const DEFAULT_THERMAL_SUSTAINED_SECS: u64 = 30;
/// Thermal history ring buffer capacity (for sparklines).
pub const THERMAL_HISTORY_CAPACITY: usize = 120;

// ── Email Notifications ───────────────────────────────────────────
/// Minimum interval between emails of the same event type (seconds).
pub const EMAIL_RATE_LIMIT_SECS: u64 = 300;
/// Default SMTP port for Gmail STARTTLS.
pub const DEFAULT_SMTP_PORT: u16 = 587;
/// Default SMTP server.
pub const DEFAULT_SMTP_SERVER: &str = "smtp.gmail.com";

// ── Auto-Shutdown ─────────────────────────────────────────────────
/// Default schedule start hour (24h format) — shutdown only active during this window.
pub const DEFAULT_SHUTDOWN_SCHEDULE_START: u8 = 0;
/// Default schedule end hour (24h format).
pub const DEFAULT_SHUTDOWN_SCHEDULE_END: u8 = 24;
/// Grace period before actual shutdown (seconds).
pub const SHUTDOWN_GRACE_PERIOD_SECS: u64 = 30;

// ── AI / Claude API ───────────────────────────────────────────────
/// Claude model identifier.
pub const CLAUDE_MODEL: &str = "claude-opus-4-6";
/// Maximum tokens for Claude responses.
pub const CLAUDE_MAX_TOKENS: u32 = 4096;
/// Claude API version string.
pub const CLAUDE_API_VERSION: &str = "2023-06-01";
/// Claude beta feature flags for OAuth.
pub const CLAUDE_BETA_FLAGS: &str = "claude-code-20250219,oauth-2025-04-20";
/// OAuth client ID for token refresh.
pub const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Token expiry buffer (ms) -- refresh before actual expiry.
pub const TOKEN_EXPIRY_BUFFER_MS: i64 = 5 * 60 * 1000;
/// Context builder initial string capacity.
pub const CONTEXT_INITIAL_CAPACITY: usize = 8192;
/// Top processes by CPU to include in AI context.
pub const CONTEXT_TOP_CPU_COUNT: usize = 25;
/// Top processes by memory to include in AI context.
pub const CONTEXT_TOP_MEM_COUNT: usize = 15;
/// Maximum process groups in AI context.
pub const CONTEXT_MAX_GROUPS: usize = 20;
/// Maximum alerts to include in AI context.
pub const CONTEXT_MAX_ALERTS: usize = 30;
/// Maximum network interfaces in AI context.
pub const CONTEXT_MAX_NET_INTERFACES: usize = 10;
/// Maximum command line length in AI context.
pub const CONTEXT_MAX_CMD_LEN: usize = 120;

// ── Prometheus Metrics ────────────────────────────────────────────
/// Prometheus metrics output buffer initial capacity.
pub const PROM_BUFFER_CAPACITY: usize = 4096;

// ── Monitor / Collector ───────────────────────────────────────────
/// Minimum disk size to include in monitoring (bytes).
pub const MIN_DISK_SIZE_BYTES: u64 = 1_000_000_000;
/// Maximum hwmon temperature sensor index to probe.
pub const MAX_HWMON_SENSORS: u32 = 32;
/// Maximum thermal zone index to probe.
pub const MAX_THERMAL_ZONES: u32 = 10;
/// Disk sector size (bytes) for I/O calculation.
pub const SECTOR_SIZE_BYTES: u64 = 512;
/// Minimum fields expected in a /proc/diskstats line.
pub const MIN_DISKSTATS_FIELDS: usize = 14;
/// Docker container ID short display length.
pub const DOCKER_SHORT_ID_LEN: usize = 12;

// ── Popup Dimensions ──────────────────────────────────────────────
/// Process detail popup max width.
pub const DETAIL_POPUP_WIDTH: u16 = 80;
/// Process detail popup max height.
pub const DETAIL_POPUP_HEIGHT: u16 = 40;
/// Help overlay width.
pub const HELP_POPUP_WIDTH: u16 = 55;
/// Help overlay height.
pub const HELP_POPUP_HEIGHT: u16 = 40;

// ── Spinner Animation ─────────────────────────────────────────────
/// Spinner character sequence for loading indicators.
pub const SPINNER_CHARS: &[&str] = &["◐", "◓", "◑", "◒"];

// ── Supported Languages ───────────────────────────────────────────
/// Available UI languages for cycling.
pub const LANGUAGES: &[&str] = &["en", "ja", "es", "de", "zh"];

// ── System Prompt Template ────────────────────────────────────────
/// Separator between system prompt and live data context.
pub const AI_CONTEXT_SEPARATOR: &str = "\n\n--- LIVE SYSTEM DATA (captured at this moment) ---\n\n";
/// Shortened separator for auto-analysis.
pub const AI_CONTEXT_SEPARATOR_SHORT: &str = "\n\n--- LIVE SYSTEM DATA ---\n\n";

// ── Paths ─────────────────────────────────────────────────────────

/// Returns the user's home directory, falling back to /tmp.
pub fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
}

/// Returns `~/.config/sentinel/`.
pub fn config_dir() -> PathBuf {
    home_dir().join(".config").join("sentinel")
}

/// Returns `~/.config/sentinel/config.toml`.
pub fn config_file_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Returns `~/.config/sentinel/themes/`.
pub fn custom_theme_dir() -> PathBuf {
    config_dir().join("themes")
}

/// Returns `~/.config/sentinel/themes/<name>.toml`.
pub fn custom_theme_path(name: &str) -> PathBuf {
    custom_theme_dir().join(format!("{}.toml", name))
}

/// Returns `~/.config/sentinel/.env` (SMTP credentials, never committed).
pub fn env_file_path() -> PathBuf {
    config_dir().join(".env")
}

/// Returns `~/.local/share/sentinel/`.
pub fn data_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("sentinel")
}
