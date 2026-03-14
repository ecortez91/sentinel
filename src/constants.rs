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
/// Disabled by default to save tokens — users opt in via config.
pub const DEFAULT_AUTO_ANALYSIS_SECS: u64 = 0;
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
pub const MAX_CONVERSATION_HISTORY: usize = 20;
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
/// Default LHM port (used when auto-detecting WSL host IP).
pub const DEFAULT_LHM_PORT: u16 = 8085;
/// Environment variable: LHM basic auth username.
pub const ENV_LHM_USER: &str = "SENTINEL_LHM_USER";
/// Environment variable: LHM basic auth password.
pub const ENV_LHM_PASSWORD: &str = "SENTINEL_LHM_PASSWORD";
/// Environment variable: override LHM URL entirely.
pub const ENV_LHM_URL: &str = "SENTINEL_LHM_URL";
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

// ── Market data (Binance) ─────────────────────────────────────────
/// Default market data polling interval (seconds).
pub const DEFAULT_MARKET_POLL_SECS: u64 = 30;
/// Maximum news items to retain per coin.
pub const MAX_NEWS_ITEMS: usize = 20;
/// Maximum news headline length for display.
pub const NEWS_HEADLINE_MAX_LEN: usize = 80;
/// Candlestick chart: minimum candle body height (characters).
pub const CANDLE_MIN_BODY_HEIGHT: u16 = 1;
/// CryptoCompare news API base URL (free tier, no key required).
pub const CRYPTOCOMPARE_NEWS_URL: &str = "https://min-api.cryptocompare.com/data/v2/news/";
/// Optional env var for CryptoCompare API key (higher rate limits).
pub const ENV_CRYPTOCOMPARE_API_KEY: &str = "SENTINEL_CRYPTOCOMPARE_API_KEY";
/// News feed polling interval (seconds).
pub const NEWS_POLL_INTERVAL_SECS: u64 = 300;
/// Candlestick chart: wick character (thin vertical line).
pub const CANDLE_WICK_CHAR: char = '│';
/// Candlestick chart: body character (full block).
pub const CANDLE_BODY_CHAR: char = '█';
/// Candlestick chart: half body top (upper half block).
pub const CANDLE_HALF_TOP: char = '▀';
/// Candlestick chart: half body bottom (lower half block).
pub const CANDLE_HALF_BOTTOM: char = '▄';
/// Candlestick chart: minimum rows required for rendering.
pub const CANDLE_MIN_CHART_ROWS: u16 = 4;
/// Candlestick chart: column width per candle (char cells including gap).
pub const CANDLE_COL_WIDTH: u16 = 2;
/// Candlestick chart: price label width on the right axis.
pub const CANDLE_PRICE_LABEL_WIDTH: u16 = 12;

// ── Windows Host Agent ────────────────────────────────────────────
/// Default sentinel-agent HTTP snapshot endpoint.
pub const DEFAULT_AGENT_URL: &str = "http://localhost:8085/api/snapshot";
/// Default sentinel-agent HTTP port.
pub const DEFAULT_AGENT_PORT: u16 = 8085;
/// Default polling interval for the Windows host agent (seconds).
pub const DEFAULT_AGENT_POLL_SECS: u64 = 5;
/// Environment variable: override agent snapshot URL entirely.
pub const ENV_AGENT_URL: &str = "SENTINEL_AGENT_URL";
/// Maximum top processes returned by the agent snapshot.
pub const AGENT_MAX_TOP_PROCESSES: usize = 30;

// ── Email Notifications ───────────────────────────────────────────
/// Minimum interval between emails of the same event type (seconds).
pub const EMAIL_RATE_LIMIT_SECS: u64 = 300;
/// Default SMTP port for Gmail STARTTLS.
pub const DEFAULT_SMTP_PORT: u16 = 587;
/// Default SMTP server.
pub const DEFAULT_SMTP_SERVER: &str = "smtp.gmail.com";

// ── Telegram Notifications ────────────────────────────────────────
/// Telegram Bot API base URL.
pub const TELEGRAM_API_BASE: &str = "https://api.telegram.org";
/// Minimum interval between Telegram messages per (category, PID) pair (seconds).
pub const TELEGRAM_RATE_LIMIT_SECS: u64 = 300;
/// Environment variable: Telegram bot token (from @BotFather).
pub const ENV_TELEGRAM_BOT_TOKEN: &str = "SENTINEL_TELEGRAM_BOT_TOKEN";
/// Environment variable: Telegram chat ID.
pub const ENV_TELEGRAM_CHAT_ID: &str = "SENTINEL_TELEGRAM_CHAT_ID";
/// Default digest interval (seconds). 0 = disabled (individual messages).
pub const DEFAULT_TELEGRAM_DIGEST_SECS: u64 = 0;
/// Maximum alerts batched per digest message.
pub const TELEGRAM_DIGEST_MAX_ALERTS: usize = 20;

// ── Security Dashboard ────────────────────────────────────────────
/// Security data refresh interval (ticks). At 1s/tick this is ~5s.
pub const SECURITY_REFRESH_TICKS: u64 = 5;
/// Slow operations (dpkg --verify) refresh interval in security refresh cycles.
/// At 5s per cycle, 12 cycles = ~60s.
pub const SECURITY_SLOW_REFRESH_CYCLES: u64 = 12;
/// Maximum security events retained in the timeline.
pub const MAX_SECURITY_EVENTS: usize = 200;
/// Maximum auth events to include in the security timeline per refresh.
pub const MAX_AUTH_EVENTS: usize = 20;
/// Maximum connections shown.
pub const MAX_SECURITY_CONNECTIONS: usize = 100;
/// Security score threshold for "FAIR" — alert when dropping below this.
pub const SECURITY_SCORE_ALERT_THRESHOLD: u8 = 60;
/// Score penalty weights.
pub const SCORE_PENALTY_THREAT: u8 = 20;
pub const SCORE_PENALTY_SUSPICIOUS: u8 = 10;
pub const SCORE_PENALTY_UNOWNED_LISTENER: u8 = 5;
pub const SCORE_PENALTY_RISKY_PORT: u8 = 5;
pub const SCORE_PENALTY_NO_AUTH_LOG: u8 = 3;
pub const SCORE_PENALTY_MODIFIED_PKG: u8 = 2;
/// Penalty when an active SSH brute-force attempt is detected.
pub const SCORE_PENALTY_SSH_BRUTE_FORCE: u8 = 15;
/// Number of failed SSH attempts from a single IP to flag as brute-force.
pub const SSH_BRUTE_FORCE_THRESHOLD: usize = 5;
/// Penalty per suspicious outbound connection (capped).
pub const SCORE_PENALTY_SUSPICIOUS_OUTBOUND: u8 = 5;
/// Maximum total deduction from suspicious outbound connections.
pub const SCORE_SUSPICIOUS_OUTBOUND_CAP: u8 = 20;
/// Maximum cron entries to display.
pub const MAX_CRON_ENTRIES: usize = 50;
/// Maximum systemd timers to display.
pub const MAX_SYSTEMD_TIMERS: usize = 50;
/// Maximum suspicious outbound connections to track.
pub const MAX_SUSPICIOUS_OUTBOUND: usize = 50;

/// Standard outbound destination ports that are considered normal.
/// Connections to remote ports NOT on this list are flagged as suspicious.
pub const STANDARD_OUTBOUND_PORTS: &[u16] = &[
    22, 53, 80, 443, 465, 587, 853, 993, 995, 3306, 5432, 6379, 8080, 8443, 9090, 27017,
];

/// Known standard ports and their expected services.
/// Format: (port, expected_process_substring).
pub const KNOWN_PORTS: &[(u16, &str)] = &[
    (22, "ssh"),
    (53, "systemd-resolve"),
    (80, "nginx"),
    (443, "nginx"),
    (3000, "node"),
    (3306, "mysql"),
    (3307, "mysql"),
    (4000, "node"),
    (5432, "postgres"),
    (5050, ""), // generic — accept any known process
    (6379, "redis"),
    (8080, ""), // generic HTTP — accept any
    (8443, ""), // generic HTTPS — accept any
    (9090, ""), // generic monitoring
    (27017, "mongod"),
];

// ── Zombie Alert Filtering ─────────────────────────────────────────
/// Default parent process names whose zombie children are silently ignored.
/// Coding tools spawn transient subprocesses (sh, git, etc.) that briefly
/// become zombies before being reaped — these are noise, not real issues.
pub const DEFAULT_IGNORED_ZOMBIE_PARENTS: &[&str] = &["opencode", "claude", "codex", "node"];

// ── Auto-Shutdown ─────────────────────────────────────────────────
/// Default schedule start hour (24h format) — shutdown only active during this window.
pub const DEFAULT_SHUTDOWN_SCHEDULE_START: u8 = 0;
/// Default schedule end hour (24h format).
pub const DEFAULT_SHUTDOWN_SCHEDULE_END: u8 = 24;
/// Grace period before actual shutdown (seconds).
pub const SHUTDOWN_GRACE_PERIOD_SECS: u64 = 30;

// ── AI / Claude API ───────────────────────────────────────────────
/// Premium model — used only for interactive chat and process questions.
pub const CLAUDE_MODEL_PREMIUM: &str = "claude-opus-4-6";
/// Middle-tier model — balanced quality/cost for general-purpose tasks.
pub const CLAUDE_MODEL_MIDDLE: &str = "claude-sonnet-4-6";
/// Cheap model — used for auto-analysis, command palette, and plugin AI.
pub const CLAUDE_MODEL_CHEAP: &str = "claude-haiku-4-5";
/// Legacy alias (kept for ClaudeClient default construction — uses middle tier).
pub const CLAUDE_MODEL: &str = CLAUDE_MODEL_MIDDLE;

/// Max tokens: interactive chat (premium).
pub const CHAT_MAX_TOKENS: u32 = 4096;
/// Max tokens: auto-analysis dashboard insight (cheap).
pub const AUTO_ANALYSIS_MAX_TOKENS: u32 = 1024;
/// Max tokens: command palette AI fallback (cheap).
pub const COMMAND_AI_MAX_TOKENS: u32 = 1024;
/// Max tokens: plugin-initiated AI analysis (cheap).
pub const PLUGIN_AI_MAX_TOKENS: u32 = 512;
/// Legacy alias (kept for fallback).
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

// Full context limits (interactive chat)
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

// Light context limits (auto-analysis, command palette — saves ~50-60% input tokens)
/// Top processes by CPU in light context.
pub const CONTEXT_LIGHT_TOP_CPU: usize = 10;
/// Top processes by memory in light context.
pub const CONTEXT_LIGHT_TOP_MEM: usize = 5;
/// Maximum process groups in light context.
pub const CONTEXT_LIGHT_MAX_GROUPS: usize = 5;
/// Maximum alerts in light context.
pub const CONTEXT_LIGHT_MAX_ALERTS: usize = 10;

// Command palette AI guard
/// Minimum input length before routing to AI (prevents typo triggers).
pub const COMMAND_AI_MIN_INPUT_LEN: usize = 5;
/// CPU delta threshold for idle detection (percentage points).
pub const AUTO_ANALYSIS_IDLE_CPU_DELTA: f32 = 5.0;

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Model tier identity tests ────────────────────────────────

    #[test]
    fn model_premium_is_opus() {
        assert_eq!(CLAUDE_MODEL_PREMIUM, "claude-opus-4-6");
    }

    #[test]
    fn model_middle_is_sonnet() {
        assert_eq!(CLAUDE_MODEL_MIDDLE, "claude-sonnet-4-6");
    }

    #[test]
    fn model_cheap_is_haiku() {
        assert_eq!(CLAUDE_MODEL_CHEAP, "claude-haiku-4-5");
    }

    #[test]
    fn model_legacy_alias_equals_middle() {
        assert_eq!(
            CLAUDE_MODEL, CLAUDE_MODEL_MIDDLE,
            "Legacy CLAUDE_MODEL should alias to middle tier, not premium"
        );
    }

    // ── Model string format validation ───────────────────────────

    #[test]
    fn all_models_start_with_claude_prefix() {
        for (label, model) in [
            ("PREMIUM", CLAUDE_MODEL_PREMIUM),
            ("MIDDLE", CLAUDE_MODEL_MIDDLE),
            ("CHEAP", CLAUDE_MODEL_CHEAP),
        ] {
            assert!(
                model.starts_with("claude-"),
                "{} model '{}' should start with 'claude-'",
                label,
                model
            );
        }
    }

    #[test]
    fn all_model_tiers_are_distinct() {
        assert_ne!(CLAUDE_MODEL_PREMIUM, CLAUDE_MODEL_MIDDLE);
        assert_ne!(CLAUDE_MODEL_MIDDLE, CLAUDE_MODEL_CHEAP);
        assert_ne!(CLAUDE_MODEL_PREMIUM, CLAUDE_MODEL_CHEAP);
    }

    #[test]
    fn premium_is_not_used_as_default() {
        assert_ne!(
            CLAUDE_MODEL, CLAUDE_MODEL_PREMIUM,
            "Default model must not be the premium tier to avoid unnecessary cost"
        );
    }
}
