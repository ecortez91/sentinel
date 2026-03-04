//! Settings plugin: in-app configuration editor.
//!
//! Provides a tab for viewing and editing common settings directly in the TUI.
//! Advanced settings (arrays like `suspicious_patterns`, credential paths, etc.)
//! can be edited in `~/.config/sentinel/config.toml`.
//!
//! ## Design
//!
//! Each setting carries a [`SettingKind`] that acts as a **strategy** for edit
//! behavior (Strategy pattern). The key handler and renderer dispatch on the
//! kind without type-switching chains, keeping them open for extension (OCP).
//!
//! Config mutations are emitted as [`PluginAction::ConfigChanged`] commands
//! (Command pattern) so the app can hot-reload and persist — the plugin never
//! touches the filesystem directly (Dependency Inversion / SRP).

pub(crate) mod renderer;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, Frame};

use crate::config::Config;
use crate::plugins::{Plugin, PluginAction};
use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

// ── Setting metadata ─────────────────────────────────────────────

/// How a setting can be edited in the TUI.
///
/// Acts as an edit **strategy**: the key handler and renderer dispatch
/// behavior based on the variant, avoiding duplicated switch logic.
#[derive(Debug, Clone)]
pub enum SettingKind {
    /// Boolean toggle — `Enter`/`Space` flips the value.
    Toggle,
    /// Cycle through a fixed set of string options — `Enter`/`Space` advances.
    Cycle(Vec<String>),
    /// Numeric input — `Enter` activates edit mode, digits modify, `Enter` confirms.
    Number {
        min: f64,
        max: f64,
        suffix: String,
        /// Whether the value should be treated as an integer.
        integer: bool,
    },
    /// Free-text input — `Enter` activates edit mode, any printable char accepted.
    Text {
        /// Maximum character length.
        max_length: usize,
        /// If true, display `[****]` when not editing (for passwords).
        masked: bool,
    },
    /// Display-only — not editable in TUI, directs user to config.toml.
    ReadOnly,
}

/// A single setting with its edit metadata and config key.
#[derive(Debug, Clone)]
pub struct SettingItem {
    /// Dot-separated config field identifier (e.g., `"thermal.warning_threshold"`).
    /// Used to map edits back to [`Config`] fields without fragile index matching.
    pub key: String,
    /// Human-readable label.
    pub label: String,
    /// Current display value.
    pub value: String,
    /// Help text.
    pub description: String,
    /// Edit behavior.
    pub kind: SettingKind,
}

/// Setting categories for navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCategory {
    General,
    Market,
    Thermal,
    Alerts,
    Notifications,
}

impl SettingsCategory {
    pub fn all() -> &'static [SettingsCategory] {
        &[
            SettingsCategory::General,
            SettingsCategory::Market,
            SettingsCategory::Thermal,
            SettingsCategory::Alerts,
            SettingsCategory::Notifications,
        ]
    }

    pub fn label(&self) -> &str {
        match self {
            SettingsCategory::General => "General",
            SettingsCategory::Market => "Market",
            SettingsCategory::Thermal => "Thermal",
            SettingsCategory::Alerts => "Alerts",
            SettingsCategory::Notifications => "Notifications",
        }
    }
}

// ── Plugin state ─────────────────────────────────────────────────

/// Settings plugin state.
pub struct SettingsPlugin {
    enabled: bool,
    pub selected_category: usize,
    pub selected_item: usize,
    pub scroll_offset: usize,
    /// Cached settings for display, grouped by category.
    pub settings: Vec<(SettingsCategory, Vec<SettingItem>)>,
    /// Whether we're in number-edit mode for the selected item.
    pub editing: bool,
    /// Text buffer while editing a numeric value.
    pub edit_buffer: String,
    /// Live config snapshot — mutated on edits, emitted via `ConfigChanged`.
    config: Config,
}

impl SettingsPlugin {
    /// Create a new settings plugin with config loaded from disk.
    pub fn new(enabled: bool) -> Self {
        let config = Config::load();
        let settings = Self::build_settings_list(&config);
        Self {
            enabled,
            selected_category: 0,
            selected_item: 0,
            scroll_offset: 0,
            settings,
            editing: false,
            edit_buffer: String::new(),
            config,
        }
    }

    /// Create a settings plugin with an explicit config (for testing).
    #[cfg(test)]
    pub fn new_with_config(enabled: bool, config: Config) -> Self {
        let settings = Self::build_settings_list(&config);
        Self {
            enabled,
            selected_category: 0,
            selected_item: 0,
            scroll_offset: 0,
            settings,
            editing: false,
            edit_buffer: String::new(),
            config,
        }
    }

    /// Build the full settings list from a config snapshot.
    fn build_settings_list(config: &Config) -> Vec<(SettingsCategory, Vec<SettingItem>)> {
        let theme_names: Vec<String> = crate::ui::theme::BUILTIN_THEME_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let lang_options: Vec<String> = vec!["en", "ja", "es", "de", "zh"]
            .into_iter()
            .map(String::from)
            .collect();
        let chart_ranges: Vec<String> = vec!["1h", "4h", "1d", "7d", "30d"]
            .into_iter()
            .map(String::from)
            .collect();

        vec![
            (
                SettingsCategory::General,
                vec![
                    SettingItem {
                        key: "refresh_interval_ms".into(),
                        label: "Refresh Interval".into(),
                        value: format!("{}", config.refresh_interval_ms),
                        description: "How often to refresh system data (ms)".into(),
                        kind: SettingKind::Number {
                            min: 200.0,
                            max: 10000.0,
                            suffix: " ms".into(),
                            integer: true,
                        },
                    },
                    SettingItem {
                        key: "theme".into(),
                        label: "Theme".into(),
                        value: config.theme.clone(),
                        description: "Color theme (cycles through built-in themes)".into(),
                        kind: SettingKind::Cycle(theme_names),
                    },
                    SettingItem {
                        key: "lang".into(),
                        label: "Language".into(),
                        value: config.lang.clone(),
                        description: "UI language (en, ja, es, de, zh)".into(),
                        kind: SettingKind::Cycle(lang_options),
                    },
                    SettingItem {
                        key: "auto_analysis_interval_secs".into(),
                        label: "Auto-Analysis".into(),
                        value: format!("{}", config.auto_analysis_interval_secs),
                        description: "AI insight refresh interval in seconds (0 = disabled)".into(),
                        kind: SettingKind::Number {
                            min: 0.0,
                            max: 3600.0,
                            suffix: " sec".into(),
                            integer: true,
                        },
                    },
                ],
            ),
            (
                SettingsCategory::Market,
                vec![
                    SettingItem {
                        key: "market.enabled".into(),
                        label: "Enabled".into(),
                        value: format!("{}", config.market.enabled),
                        description: "Enable market data plugin".into(),
                        kind: SettingKind::Toggle,
                    },
                    SettingItem {
                        key: "market.poll_interval_secs".into(),
                        label: "Poll Interval".into(),
                        value: format!("{}", config.market.poll_interval_secs),
                        description: "How often to refresh market data (seconds)".into(),
                        kind: SettingKind::Number {
                            min: 10.0,
                            max: 600.0,
                            suffix: " sec".into(),
                            integer: true,
                        },
                    },
                    SettingItem {
                        key: "market.tickers".into(),
                        label: "Watchlist".into(),
                        value: format!("{} tickers", config.market.tickers.len()),
                        description: "Edit in Market tab (+/d) or config.toml".into(),
                        kind: SettingKind::ReadOnly,
                    },
                    SettingItem {
                        key: "market.default_chart_range".into(),
                        label: "Default Chart".into(),
                        value: config.market.default_chart_range.clone(),
                        description: "Default chart time range".into(),
                        kind: SettingKind::Cycle(chart_ranges),
                    },
                ],
            ),
            (
                SettingsCategory::Thermal,
                vec![
                    SettingItem {
                        key: "thermal.lhm_url".into(),
                        label: "LHM URL".into(),
                        value: config.thermal.lhm_url.clone(),
                        description: "LibreHardwareMonitor HTTP JSON endpoint".into(),
                        kind: SettingKind::Text {
                            max_length: 200,
                            masked: false,
                        },
                    },
                    SettingItem {
                        key: "thermal.lhm_username".into(),
                        label: "LHM Username".into(),
                        value: config.thermal.lhm_username.clone().unwrap_or_default(),
                        description: "HTTP Basic Auth username (empty = no auth)".into(),
                        kind: SettingKind::Text {
                            max_length: 100,
                            masked: false,
                        },
                    },
                    SettingItem {
                        key: "thermal.lhm_password".into(),
                        label: "LHM Password".into(),
                        value: config.thermal.lhm_password.clone().unwrap_or_default(),
                        description: "HTTP Basic Auth password".into(),
                        kind: SettingKind::Text {
                            max_length: 100,
                            masked: true,
                        },
                    },
                    SettingItem {
                        key: "thermal.poll_interval_secs".into(),
                        label: "Poll Interval".into(),
                        value: format!("{}", config.thermal.poll_interval_secs),
                        description: "Temperature polling interval (seconds)".into(),
                        kind: SettingKind::Number {
                            min: 1.0,
                            max: 300.0,
                            suffix: " sec".into(),
                            integer: true,
                        },
                    },
                    SettingItem {
                        key: "thermal.warning_threshold".into(),
                        label: "Warning Temp".into(),
                        value: format!("{:.0}", config.thermal.warning_threshold),
                        description: "Warning temperature threshold (Celsius)".into(),
                        kind: SettingKind::Number {
                            min: 30.0,
                            max: 150.0,
                            suffix: " C".into(),
                            integer: false,
                        },
                    },
                    SettingItem {
                        key: "thermal.critical_threshold".into(),
                        label: "Critical Temp".into(),
                        value: format!("{:.0}", config.thermal.critical_threshold),
                        description: "Critical temperature threshold (Celsius)".into(),
                        kind: SettingKind::Number {
                            min: 30.0,
                            max: 150.0,
                            suffix: " C".into(),
                            integer: false,
                        },
                    },
                    SettingItem {
                        key: "thermal.emergency_threshold".into(),
                        label: "Emergency Temp".into(),
                        value: format!("{:.0}", config.thermal.emergency_threshold),
                        description: "Emergency shutdown threshold (Celsius)".into(),
                        kind: SettingKind::Number {
                            min: 30.0,
                            max: 150.0,
                            suffix: " C".into(),
                            integer: false,
                        },
                    },
                    SettingItem {
                        key: "thermal.auto_shutdown_enabled".into(),
                        label: "Auto-Shutdown".into(),
                        value: format!("{}", config.thermal.auto_shutdown_enabled),
                        description: "Enable thermal auto-shutdown".into(),
                        kind: SettingKind::Toggle,
                    },
                ],
            ),
            (
                SettingsCategory::Alerts,
                vec![
                    SettingItem {
                        key: "cpu_warning_threshold".into(),
                        label: "CPU Warning".into(),
                        value: format!("{:.0}", config.cpu_warning_threshold),
                        description: "CPU usage warning threshold (%)".into(),
                        kind: SettingKind::Number {
                            min: 1.0,
                            max: 100.0,
                            suffix: "%".into(),
                            integer: false,
                        },
                    },
                    SettingItem {
                        key: "cpu_critical_threshold".into(),
                        label: "CPU Critical".into(),
                        value: format!("{:.0}", config.cpu_critical_threshold),
                        description: "CPU usage critical threshold (%)".into(),
                        kind: SettingKind::Number {
                            min: 1.0,
                            max: 100.0,
                            suffix: "%".into(),
                            integer: false,
                        },
                    },
                    SettingItem {
                        key: "mem_warning_threshold_mib".into(),
                        label: "Mem Warning".into(),
                        value: format!("{}", config.mem_warning_threshold_bytes / (1024 * 1024)),
                        description: "Per-process memory warning (MiB)".into(),
                        kind: SettingKind::Number {
                            min: 64.0,
                            max: 65536.0,
                            suffix: " MiB".into(),
                            integer: true,
                        },
                    },
                    SettingItem {
                        key: "max_alerts".into(),
                        label: "Max Alerts".into(),
                        value: format!("{}", config.max_alerts),
                        description: "Maximum alerts in history".into(),
                        kind: SettingKind::Number {
                            min: 10.0,
                            max: 10000.0,
                            suffix: "".into(),
                            integer: true,
                        },
                    },
                ],
            ),
            (
                SettingsCategory::Notifications,
                vec![SettingItem {
                    key: "notifications.email_enabled".into(),
                    label: "Email Enabled".into(),
                    value: format!("{}", config.notifications.email_enabled),
                    description: "Enable email notifications (requires .env credentials)".into(),
                    kind: SettingKind::Toggle,
                }],
            ),
        ]
    }

    /// Number of items in the currently selected category.
    fn current_items_len(&self) -> usize {
        self.settings
            .get(self.selected_category)
            .map(|(_, items)| items.len())
            .unwrap_or(0)
    }

    /// Get the currently selected setting item (if any).
    fn current_item(&self) -> Option<&SettingItem> {
        self.settings
            .get(self.selected_category)
            .and_then(|(_, items)| items.get(self.selected_item))
    }

    // ── Edit actions (Strategy dispatch) ─────────────────────────

    /// Activate editing for the currently selected setting.
    /// Dispatches based on `SettingKind` (Strategy pattern).
    fn activate_edit(&mut self) -> PluginAction {
        let item = match self.current_item() {
            Some(item) => item.clone(),
            None => return PluginAction::Consumed,
        };

        match &item.kind {
            SettingKind::Toggle => {
                let current = item.value == "true";
                let new_val = (!current).to_string();
                self.apply_and_emit(&item.key, &new_val)
            }
            SettingKind::Cycle(options) => {
                let current_idx = options.iter().position(|o| o == &item.value).unwrap_or(0);
                let next_idx = (current_idx + 1) % options.len();
                let new_val = options[next_idx].clone();
                self.apply_and_emit(&item.key, &new_val)
            }
            SettingKind::Number { .. } | SettingKind::Text { .. } => {
                self.editing = true;
                self.edit_buffer = item.value.clone();
                PluginAction::Consumed
            }
            SettingKind::ReadOnly => PluginAction::SetStatus(
                "This setting is read-only. Edit ~/.config/sentinel/config.toml for advanced settings.".into(),
            ),
        }
    }

    /// Handle key events while in edit mode (Number or Text).
    fn handle_edit_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                self.edit_buffer.clear();
                PluginAction::Consumed
            }
            KeyCode::Enter => {
                let buffer = self.edit_buffer.clone();
                self.editing = false;
                self.edit_buffer.clear();

                if let Some(item) = self.current_item() {
                    let key = item.key.clone();
                    let kind = item.kind.clone();
                    match kind {
                        SettingKind::Number { .. } => {
                            let clamped = self.clamp_edit_value(&key, &buffer);
                            self.apply_and_emit(&key, &clamped)
                        }
                        SettingKind::Text { .. } => {
                            // Text values are applied as-is (trimmed)
                            self.apply_and_emit(&key, buffer.trim())
                        }
                        _ => PluginAction::Consumed,
                    }
                } else {
                    PluginAction::Consumed
                }
            }
            KeyCode::Backspace => {
                self.edit_buffer.pop();
                PluginAction::Consumed
            }
            KeyCode::Char(c) => {
                // Dispatch char acceptance based on current item's kind
                if let Some(item) = self.current_item() {
                    match &item.kind {
                        SettingKind::Number { .. } => {
                            if c.is_ascii_digit() || c == '.' {
                                // Only allow one decimal point
                                if c == '.' && self.edit_buffer.contains('.') {
                                    return PluginAction::Consumed;
                                }
                                self.edit_buffer.push(c);
                            }
                        }
                        SettingKind::Text { max_length, .. } => {
                            if !c.is_control() && self.edit_buffer.len() < *max_length {
                                self.edit_buffer.push(c);
                            }
                        }
                        _ => {}
                    }
                }
                PluginAction::Consumed
            }
            _ => PluginAction::Consumed,
        }
    }

    /// Clamp an edited numeric value to the bounds defined in its `SettingKind`.
    fn clamp_edit_value(&self, key: &str, raw: &str) -> String {
        let item = match self.current_item() {
            Some(i) if i.key == key => i,
            _ => return raw.to_string(),
        };

        if let SettingKind::Number {
            min, max, integer, ..
        } = &item.kind
        {
            if *integer {
                let v: f64 = raw.parse().unwrap_or(*min);
                let clamped = v.clamp(*min, *max) as u64;
                return clamped.to_string();
            } else {
                let v: f64 = raw.parse().unwrap_or(*min);
                let clamped = v.clamp(*min, *max);
                // Format without trailing zeros but keep it parseable
                if clamped.fract() == 0.0 {
                    return format!("{:.0}", clamped);
                }
                return format!("{}", clamped);
            }
        }

        raw.to_string()
    }

    /// Apply a value change to the config and emit `ConfigChanged`.
    fn apply_and_emit(&mut self, key: &str, value: &str) -> PluginAction {
        if Self::apply_to_config(&mut self.config, key, value) {
            self.settings = Self::build_settings_list(&self.config);
            PluginAction::ConfigChanged(Box::new(self.config.clone()))
        } else {
            PluginAction::SetStatus(format!("Invalid value for {}", key))
        }
    }

    /// Map a setting key + string value onto the correct [`Config`] field.
    ///
    /// Returns `true` if the value was applied, `false` if the key is unknown
    /// or the value couldn't be parsed. Validation/clamping follows the same
    /// rules as `Config::load()` to stay consistent.
    pub fn apply_to_config(config: &mut Config, key: &str, value: &str) -> bool {
        match key {
            // ── General ──────────────────────────────────────────
            "refresh_interval_ms" => {
                if let Ok(v) = value.parse::<u64>() {
                    config.refresh_interval_ms = v.max(200);
                    true
                } else {
                    false
                }
            }
            "theme" => {
                config.theme = value.to_string();
                true
            }
            "lang" => {
                config.lang = value.to_string();
                true
            }
            "auto_analysis_interval_secs" => {
                if let Ok(v) = value.parse::<u64>() {
                    config.auto_analysis_interval_secs = v;
                    true
                } else {
                    false
                }
            }
            // ── Market ───────────────────────────────────────────
            "market.enabled" => {
                config.market.enabled = value == "true";
                true
            }
            "market.poll_interval_secs" => {
                if let Ok(v) = value.parse::<u64>() {
                    config.market.poll_interval_secs = v.max(10);
                    true
                } else {
                    false
                }
            }
            "market.default_chart_range" => {
                config.market.default_chart_range = value.to_string();
                true
            }
            // ── Thermal ──────────────────────────────────────────
            "thermal.lhm_url" => {
                if !value.is_empty() {
                    config.thermal.lhm_url = value.to_string();
                }
                true
            }
            "thermal.lhm_username" => {
                config.thermal.lhm_username = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                true
            }
            "thermal.lhm_password" => {
                config.thermal.lhm_password = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                true
            }
            "thermal.poll_interval_secs" => {
                if let Ok(v) = value.parse::<u64>() {
                    config.thermal.poll_interval_secs = v.max(1);
                    true
                } else {
                    false
                }
            }
            "thermal.warning_threshold" => {
                if let Ok(v) = value.parse::<f32>() {
                    config.thermal.warning_threshold = v.clamp(30.0, 150.0);
                    true
                } else {
                    false
                }
            }
            "thermal.critical_threshold" => {
                if let Ok(v) = value.parse::<f32>() {
                    config.thermal.critical_threshold = v.clamp(30.0, 150.0);
                    true
                } else {
                    false
                }
            }
            "thermal.emergency_threshold" => {
                if let Ok(v) = value.parse::<f32>() {
                    config.thermal.emergency_threshold = v.clamp(30.0, 150.0);
                    true
                } else {
                    false
                }
            }
            "thermal.auto_shutdown_enabled" => {
                config.thermal.auto_shutdown_enabled = value == "true";
                true
            }
            // ── Alerts ───────────────────────────────────────────
            "cpu_warning_threshold" => {
                if let Ok(v) = value.parse::<f32>() {
                    config.cpu_warning_threshold = v.clamp(1.0, 100.0);
                    true
                } else {
                    false
                }
            }
            "cpu_critical_threshold" => {
                if let Ok(v) = value.parse::<f32>() {
                    config.cpu_critical_threshold = v.clamp(1.0, 100.0);
                    true
                } else {
                    false
                }
            }
            "mem_warning_threshold_mib" => {
                if let Ok(v) = value.parse::<u64>() {
                    config.mem_warning_threshold_bytes = v.max(64) * 1024 * 1024;
                    true
                } else {
                    false
                }
            }
            "max_alerts" => {
                if let Ok(v) = value.parse::<usize>() {
                    config.max_alerts = v.max(10);
                    true
                } else {
                    false
                }
            }
            // ── Notifications ────────────────────────────────────
            "notifications.email_enabled" => {
                config.notifications.email_enabled = value == "true";
                true
            }
            _ => false,
        }
    }
}

// ── Plugin trait implementation ──────────────────────────────────

impl Plugin for SettingsPlugin {
    fn id(&self) -> &str {
        "settings"
    }

    fn tab_label(&self) -> &str {
        "Settings"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn handle_key(&mut self, key: KeyEvent) -> PluginAction {
        // Number-edit mode intercepts all keys
        if self.editing {
            return self.handle_edit_key(key);
        }

        match key.code {
            // Navigate categories
            KeyCode::Left | KeyCode::Char('h') => {
                if self.selected_category > 0 {
                    self.selected_category -= 1;
                    self.selected_item = 0;
                    self.scroll_offset = 0;
                }
                PluginAction::Consumed
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.selected_category < SettingsCategory::all().len() - 1 {
                    self.selected_category += 1;
                    self.selected_item = 0;
                    self.scroll_offset = 0;
                }
                PluginAction::Consumed
            }
            // Navigate items
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_item > 0 {
                    self.selected_item -= 1;
                }
                PluginAction::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.current_items_len().saturating_sub(1);
                if self.selected_item < max {
                    self.selected_item += 1;
                }
                PluginAction::Consumed
            }
            // Activate editing for the selected setting
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_edit(),
            _ => PluginAction::Ignored,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme, glyphs: &Glyphs) {
        renderer::render_settings(frame, area, self, theme, glyphs);
    }

    fn status_bar_hints(&self) -> Vec<(&str, &str)> {
        if self.editing {
            vec![("Enter", "Confirm"), ("Esc", "Cancel")]
        } else {
            vec![
                ("\u{2190}\u{2192}", "Category"),
                ("\u{2191}\u{2193}", "Setting"),
                ("Enter", "Edit"),
            ]
        }
    }

    fn help_entries(&self) -> Vec<(&str, &str)> {
        vec![
            ("Left/Right", "Switch settings category"),
            ("Up/Down", "Navigate settings"),
            ("Enter/Space", "Edit selected setting"),
            ("Esc", "Cancel number edit"),
        ]
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_plugin() -> SettingsPlugin {
        SettingsPlugin::new_with_config(true, Config::default())
    }

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    // ── apply_to_config ──────────────────────────────────────

    #[test]
    fn apply_unknown_key_returns_false() {
        let mut config = Config::default();
        assert!(!SettingsPlugin::apply_to_config(
            &mut config,
            "nonexistent",
            "42"
        ));
    }

    #[test]
    fn apply_invalid_number_returns_false() {
        let mut config = Config::default();
        assert!(!SettingsPlugin::apply_to_config(
            &mut config,
            "refresh_interval_ms",
            "not_a_number"
        ));
    }

    #[test]
    fn apply_theme_sets_value() {
        let mut config = Config::default();
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "theme",
            "nord"
        ));
        assert_eq!(config.theme, "nord");
    }

    #[test]
    fn apply_toggle_flips_boolean() {
        let mut config = Config::default();
        let original = config.market.enabled;
        SettingsPlugin::apply_to_config(&mut config, "market.enabled", &(!original).to_string());
        assert_eq!(config.market.enabled, !original);
    }

    #[test]
    fn apply_clamps_cpu_threshold() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "cpu_warning_threshold", "200");
        assert_eq!(config.cpu_warning_threshold, 100.0);

        SettingsPlugin::apply_to_config(&mut config, "cpu_warning_threshold", "-5");
        assert_eq!(config.cpu_warning_threshold, 1.0);
    }

    #[test]
    fn apply_clamps_thermal_threshold() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "thermal.warning_threshold", "10");
        assert_eq!(config.thermal.warning_threshold, 30.0);

        SettingsPlugin::apply_to_config(&mut config, "thermal.warning_threshold", "200");
        assert_eq!(config.thermal.warning_threshold, 150.0);
    }

    #[test]
    fn apply_mem_warning_converts_mib_to_bytes() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "mem_warning_threshold_mib", "512");
        assert_eq!(config.mem_warning_threshold_bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn apply_refresh_interval_enforces_minimum() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "refresh_interval_ms", "50");
        assert_eq!(config.refresh_interval_ms, 200); // min is 200
    }

    #[test]
    fn apply_max_alerts_enforces_minimum() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "max_alerts", "1");
        assert_eq!(config.max_alerts, 10); // min is 10
    }

    #[test]
    fn apply_market_poll_enforces_minimum() {
        let mut config = Config::default();
        SettingsPlugin::apply_to_config(&mut config, "market.poll_interval_secs", "3");
        assert_eq!(config.market.poll_interval_secs, 10); // min is 10
    }

    #[test]
    fn apply_all_editable_keys_are_handled() {
        // Ensure every key used in build_settings_list is handled by apply_to_config
        let config = Config::default();
        let settings = SettingsPlugin::build_settings_list(&config);

        for (_cat, items) in &settings {
            for item in items {
                if matches!(item.kind, SettingKind::ReadOnly) {
                    continue;
                }
                let mut test_config = Config::default();
                let applied =
                    SettingsPlugin::apply_to_config(&mut test_config, &item.key, &item.value);
                assert!(
                    applied,
                    "apply_to_config should handle key '{}' but returned false",
                    item.key
                );
            }
        }
    }

    // ── Toggle editing ───────────────────────────────────────

    #[test]
    fn toggle_produces_config_changed() {
        let mut plugin = default_plugin();
        // Navigate to Market > Enabled (category 1, item 0)
        plugin.selected_category = 1;
        plugin.selected_item = 0;

        let was_enabled = plugin.config.market.enabled;
        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.market.enabled, !was_enabled);
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    #[test]
    fn toggle_notification_produces_config_changed() {
        let mut plugin = default_plugin();
        // Navigate to Notifications > Email Enabled (category 4, item 0)
        plugin.selected_category = 4;
        plugin.selected_item = 0;

        let was_enabled = plugin.config.notifications.email_enabled;
        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.notifications.email_enabled, !was_enabled);
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    // ── Cycle editing ────────────────────────────────────────

    #[test]
    fn cycle_advances_theme() {
        let config = Config {
            theme: "dracula".into(),
            ..Config::default()
        };
        let mut plugin = SettingsPlugin::new_with_config(true, config);
        // General > Theme (category 0, item 1)
        plugin.selected_category = 0;
        plugin.selected_item = 1;

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                // Should advance past "dracula" to the next theme
                assert_ne!(cfg.theme, "dracula");
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    #[test]
    fn cycle_wraps_around() {
        let config = Config {
            theme: "solarized".into(), // last in the list
            ..Config::default()
        };
        let mut plugin = SettingsPlugin::new_with_config(true, config);
        plugin.selected_category = 0;
        plugin.selected_item = 1;

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                // Should wrap to first theme
                assert_eq!(cfg.theme, "dracula");
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    // ── Number editing ───────────────────────────────────────

    #[test]
    fn number_enter_activates_edit_mode() {
        let mut plugin = default_plugin();
        // General > Refresh Interval (category 0, item 0) — Number type
        plugin.selected_category = 0;
        plugin.selected_item = 0;

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        assert!(matches!(action, PluginAction::Consumed));
        assert!(plugin.editing);
        assert!(!plugin.edit_buffer.is_empty());
    }

    #[test]
    fn number_edit_confirm_valid() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0; // Refresh Interval
        plugin.handle_key(make_key(KeyCode::Enter)); // enter edit mode
        plugin.edit_buffer = "500".to_string();

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.refresh_interval_ms, 500);
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
        assert!(!plugin.editing);
    }

    #[test]
    fn number_edit_clamps_below_min() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "50".to_string(); // below min (200)

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.refresh_interval_ms, 200); // clamped
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    #[test]
    fn number_edit_clamps_above_max() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "99999".to_string(); // above max (10000)

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.refresh_interval_ms, 10000); // clamped
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
    }

    #[test]
    fn number_edit_esc_cancels() {
        let mut plugin = default_plugin();
        let original_ms = plugin.config.refresh_interval_ms;
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "9999".to_string();

        let action = plugin.handle_key(make_key(KeyCode::Esc));
        assert!(matches!(action, PluginAction::Consumed));
        assert!(!plugin.editing);
        assert_eq!(plugin.config.refresh_interval_ms, original_ms);
    }

    #[test]
    fn number_edit_only_accepts_digits_and_dot() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer.clear();

        // Type valid chars
        plugin.handle_key(make_key(KeyCode::Char('1')));
        plugin.handle_key(make_key(KeyCode::Char('0')));
        plugin.handle_key(make_key(KeyCode::Char('0')));
        assert_eq!(plugin.edit_buffer, "100");

        // Type invalid char — ignored
        plugin.handle_key(make_key(KeyCode::Char('a')));
        assert_eq!(plugin.edit_buffer, "100");
    }

    #[test]
    fn number_edit_rejects_double_dot() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "1.5".to_string();

        plugin.handle_key(make_key(KeyCode::Char('.')));
        assert_eq!(plugin.edit_buffer, "1.5"); // second dot rejected
    }

    // ── ReadOnly ─────────────────────────────────────────────

    #[test]
    fn readonly_shows_status() {
        let mut plugin = default_plugin();
        // Market > Watchlist (category 1, item 2) — ReadOnly
        plugin.selected_category = 1;
        plugin.selected_item = 2;

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::SetStatus(msg) => {
                assert!(
                    msg.contains("config.toml"),
                    "ReadOnly message should mention config.toml, got: {}",
                    msg
                );
            }
            other => panic!("Expected SetStatus, got {:?}", other),
        }
    }

    // ── Settings list integrity ──────────────────────────────

    #[test]
    fn settings_list_reflects_config_values() {
        let mut config = Config::default();
        config.theme = "nord".to_string();
        config.cpu_warning_threshold = 42.0;

        let plugin = SettingsPlugin::new_with_config(true, config);

        let general_items = &plugin.settings[0].1;
        let theme_item = general_items.iter().find(|i| i.key == "theme").unwrap();
        assert_eq!(theme_item.value, "nord");

        let alerts_items = &plugin.settings[3].1;
        let cpu_item = alerts_items
            .iter()
            .find(|i| i.key == "cpu_warning_threshold")
            .unwrap();
        assert_eq!(cpu_item.value, "42");
    }

    #[test]
    fn settings_rebuild_after_edit() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0; // Refresh Interval
        plugin.handle_key(make_key(KeyCode::Enter)); // edit mode
        plugin.edit_buffer = "750".to_string();
        plugin.handle_key(make_key(KeyCode::Enter)); // confirm

        // Verify the settings list was rebuilt with the new value
        let general_items = &plugin.settings[0].1;
        let item = general_items
            .iter()
            .find(|i| i.key == "refresh_interval_ms")
            .unwrap();
        assert_eq!(item.value, "750");
    }

    // ── Navigation ───────────────────────────────────────────

    #[test]
    fn navigation_does_not_overflow() {
        let mut plugin = default_plugin();
        // Try to go past last category
        plugin.selected_category = SettingsCategory::all().len() - 1;
        plugin.handle_key(make_key(KeyCode::Right));
        assert_eq!(plugin.selected_category, SettingsCategory::all().len() - 1);

        // Try to go before first
        plugin.selected_category = 0;
        plugin.handle_key(make_key(KeyCode::Left));
        assert_eq!(plugin.selected_category, 0);
    }

    #[test]
    fn category_change_resets_item_selection() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 2;
        plugin.handle_key(make_key(KeyCode::Right));
        assert_eq!(plugin.selected_item, 0);
    }

    #[test]
    fn edit_mode_blocks_navigation() {
        let mut plugin = default_plugin();
        plugin.selected_category = 0;
        plugin.selected_item = 0;
        plugin.handle_key(make_key(KeyCode::Enter)); // enter edit mode
        assert!(plugin.editing);

        // Navigation keys should be consumed by edit mode, not move selection
        plugin.handle_key(make_key(KeyCode::Down));
        assert!(plugin.editing); // still in edit mode
    }

    // ── Text editing ─────────────────────────────────────────

    #[test]
    fn text_enter_activates_edit_mode() {
        let mut plugin = default_plugin();
        // Thermal > LHM URL (category 2, item 0) — Text type
        plugin.selected_category = 2;
        plugin.selected_item = 0;

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        assert!(matches!(action, PluginAction::Consumed));
        assert!(plugin.editing);
        // Buffer should be pre-filled with current value
        assert!(!plugin.edit_buffer.is_empty());
    }

    #[test]
    fn text_edit_accepts_printable_chars() {
        let mut plugin = default_plugin();
        // Thermal > LHM Username (category 2, item 1) — Text, non-masked
        plugin.selected_category = 2;
        plugin.selected_item = 1;
        plugin.handle_key(make_key(KeyCode::Enter)); // activate edit
        plugin.edit_buffer.clear();

        // Should accept letters, digits, symbols
        plugin.handle_key(make_key(KeyCode::Char('H')));
        plugin.handle_key(make_key(KeyCode::Char('i')));
        plugin.handle_key(make_key(KeyCode::Char('!')));
        plugin.handle_key(make_key(KeyCode::Char('3')));
        assert_eq!(plugin.edit_buffer, "Hi!3");
    }

    #[test]
    fn text_edit_respects_max_length() {
        let mut config = Config::default();
        config.thermal.lhm_username = Some("x".repeat(99));
        let mut plugin = SettingsPlugin::new_with_config(true, config);
        // Thermal > LHM Username (max_length: 100)
        plugin.selected_category = 2;
        plugin.selected_item = 1;
        plugin.handle_key(make_key(KeyCode::Enter));
        // Buffer is now 99 chars

        // One more should be accepted (100 total)
        plugin.handle_key(make_key(KeyCode::Char('Y')));
        assert_eq!(plugin.edit_buffer.len(), 100);

        // 101st char should be rejected
        plugin.handle_key(make_key(KeyCode::Char('Z')));
        assert_eq!(plugin.edit_buffer.len(), 100);
    }

    #[test]
    fn text_edit_confirm_applies_value() {
        let mut plugin = default_plugin();
        // Thermal > LHM Username (category 2, item 1)
        plugin.selected_category = 2;
        plugin.selected_item = 1;
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "NewUser".to_string();

        let action = plugin.handle_key(make_key(KeyCode::Enter));
        match action {
            PluginAction::ConfigChanged(cfg) => {
                assert_eq!(cfg.thermal.lhm_username, Some("NewUser".into()));
            }
            other => panic!("Expected ConfigChanged, got {:?}", other),
        }
        assert!(!plugin.editing);
    }

    #[test]
    fn text_edit_backspace_removes_char() {
        let mut plugin = default_plugin();
        plugin.selected_category = 2;
        plugin.selected_item = 1; // LHM Username
        plugin.handle_key(make_key(KeyCode::Enter));
        plugin.edit_buffer = "abc".to_string();

        plugin.handle_key(make_key(KeyCode::Backspace));
        assert_eq!(plugin.edit_buffer, "ab");
    }

    // ── Thermal credential apply ─────────────────────────────

    #[test]
    fn apply_lhm_username_sets_some() {
        let mut config = Config::default();
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_username",
            "MyUser"
        ));
        assert_eq!(config.thermal.lhm_username, Some("MyUser".into()));
    }

    #[test]
    fn apply_lhm_username_empty_clears_to_none() {
        let mut config = Config::default();
        config.thermal.lhm_username = Some("OldUser".into());
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_username",
            ""
        ));
        assert_eq!(config.thermal.lhm_username, None);
    }

    #[test]
    fn apply_lhm_password_sets_some() {
        let mut config = Config::default();
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_password",
            "Secret123"
        ));
        assert_eq!(config.thermal.lhm_password, Some("Secret123".into()));
    }

    #[test]
    fn apply_lhm_password_empty_clears_to_none() {
        let mut config = Config::default();
        config.thermal.lhm_password = Some("OldPass".into());
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_password",
            ""
        ));
        assert_eq!(config.thermal.lhm_password, None);
    }

    #[test]
    fn apply_lhm_url_rejects_empty() {
        let mut config = Config::default();
        let original_url = config.thermal.lhm_url.clone();
        // Empty value should be accepted (returns true) but URL stays unchanged
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_url",
            ""
        ));
        assert_eq!(config.thermal.lhm_url, original_url);
    }

    #[test]
    fn apply_lhm_url_sets_value() {
        let mut config = Config::default();
        assert!(SettingsPlugin::apply_to_config(
            &mut config,
            "thermal.lhm_url",
            "http://192.168.1.100:8085/data.json"
        ));
        assert_eq!(
            config.thermal.lhm_url,
            "http://192.168.1.100:8085/data.json"
        );
    }
}
