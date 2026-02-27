//! Settings plugin: in-app configuration editor.
//!
//! Provides a tab for viewing and editing non-credential settings
//! without manually editing config.toml.

mod renderer;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, Frame};

use crate::plugins::{Plugin, PluginAction};
use crate::ui::theme::Theme;

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

/// Individual setting item for display.
#[derive(Debug, Clone)]
pub struct SettingItem {
    pub label: String,
    pub value: String,
    pub description: String,
}

/// Settings plugin state.
pub struct SettingsPlugin {
    enabled: bool,
    pub selected_category: usize,
    pub selected_item: usize,
    pub scroll_offset: usize,
    /// Cached settings for display.
    pub settings: Vec<(SettingsCategory, Vec<SettingItem>)>,
}

impl SettingsPlugin {
    pub fn new(enabled: bool) -> Self {
        let settings = Self::build_settings_list();
        Self {
            enabled,
            selected_category: 0,
            selected_item: 0,
            scroll_offset: 0,
            settings,
        }
    }

    fn build_settings_list() -> Vec<(SettingsCategory, Vec<SettingItem>)> {
        // Load current config values for display
        let config = crate::config::Config::load();

        vec![
            (
                SettingsCategory::General,
                vec![
                    SettingItem {
                        label: "Refresh Interval".into(),
                        value: format!("{} ms", config.refresh_interval_ms),
                        description: "How often to refresh system data".into(),
                    },
                    SettingItem {
                        label: "Theme".into(),
                        value: config.theme.clone(),
                        description: "Color theme name".into(),
                    },
                    SettingItem {
                        label: "Language".into(),
                        value: config.lang.clone(),
                        description: "UI language (en, ja, es, de, zh)".into(),
                    },
                    SettingItem {
                        label: "Auto-Analysis".into(),
                        value: if config.auto_analysis_interval_secs == 0 {
                            "Disabled".into()
                        } else {
                            format!("{} sec", config.auto_analysis_interval_secs)
                        },
                        description: "AI insight refresh interval".into(),
                    },
                ],
            ),
            (
                SettingsCategory::Market,
                vec![
                    SettingItem {
                        label: "Enabled".into(),
                        value: format!("{}", config.market.enabled),
                        description: "Enable market data plugin".into(),
                    },
                    SettingItem {
                        label: "Poll Interval".into(),
                        value: format!("{} sec", config.market.poll_interval_secs),
                        description: "How often to refresh market data".into(),
                    },
                    SettingItem {
                        label: "Watchlist".into(),
                        value: format!("{} tickers", config.market.tickers.len()),
                        description: "Trading pairs to track (e.g., BTCUSDT)".into(),
                    },
                    SettingItem {
                        label: "Default Chart".into(),
                        value: config.market.default_chart_range.clone(),
                        description: "Default chart time range".into(),
                    },
                ],
            ),
            (
                SettingsCategory::Thermal,
                vec![
                    SettingItem {
                        label: "LHM URL".into(),
                        value: config.thermal.lhm_url.clone(),
                        description: "LibreHardwareMonitor endpoint".into(),
                    },
                    SettingItem {
                        label: "Poll Interval".into(),
                        value: format!("{} sec", config.thermal.poll_interval_secs),
                        description: "Temperature polling interval".into(),
                    },
                    SettingItem {
                        label: "Warning Temp".into(),
                        value: format!("{:.0} C", config.thermal.warning_threshold),
                        description: "Warning temperature threshold".into(),
                    },
                    SettingItem {
                        label: "Critical Temp".into(),
                        value: format!("{:.0} C", config.thermal.critical_threshold),
                        description: "Critical temperature threshold".into(),
                    },
                    SettingItem {
                        label: "Emergency Temp".into(),
                        value: format!("{:.0} C", config.thermal.emergency_threshold),
                        description: "Emergency shutdown threshold".into(),
                    },
                    SettingItem {
                        label: "Auto-Shutdown".into(),
                        value: format!("{}", config.thermal.auto_shutdown_enabled),
                        description: "Enable thermal auto-shutdown".into(),
                    },
                ],
            ),
            (
                SettingsCategory::Alerts,
                vec![
                    SettingItem {
                        label: "CPU Warning".into(),
                        value: format!("{:.0}%", config.cpu_warning_threshold),
                        description: "CPU usage warning threshold".into(),
                    },
                    SettingItem {
                        label: "CPU Critical".into(),
                        value: format!("{:.0}%", config.cpu_critical_threshold),
                        description: "CPU usage critical threshold".into(),
                    },
                    SettingItem {
                        label: "Mem Warning".into(),
                        value: format!(
                            "{} MiB",
                            config.mem_warning_threshold_bytes / (1024 * 1024)
                        ),
                        description: "Per-process memory warning".into(),
                    },
                    SettingItem {
                        label: "Max Alerts".into(),
                        value: format!("{}", config.max_alerts),
                        description: "Maximum alerts in history".into(),
                    },
                ],
            ),
            (
                SettingsCategory::Notifications,
                vec![SettingItem {
                    label: "Email Enabled".into(),
                    value: format!("{}", config.notifications.email_enabled),
                    description: "Enable email notifications".into(),
                }],
            ),
        ]
    }

    fn current_items_len(&self) -> usize {
        self.settings
            .get(self.selected_category)
            .map(|(_, items)| items.len())
            .unwrap_or(0)
    }
}

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
            _ => PluginAction::Ignored,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        renderer::render_settings(frame, area, self, theme);
    }

    fn status_bar_hints(&self) -> Vec<(&str, &str)> {
        vec![
            ("\u{2190}\u{2192}", "Category"),
            ("\u{2191}\u{2193}", "Setting"),
        ]
    }

    fn help_entries(&self) -> Vec<(&str, &str)> {
        vec![
            ("Left/Right", "Switch settings category"),
            ("Up/Down", "Navigate settings"),
        ]
    }
}
