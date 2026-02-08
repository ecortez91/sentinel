use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::models::AlertSeverity;

/// All available built-in theme names.
pub const BUILTIN_THEME_NAMES: &[&str] = &[
    "default",
    "gruvbox",
    "nord",
    "catppuccin",
    "dracula",
    "solarized",
];

/// Data-driven theme: every color in one struct.
/// Constructed from built-in presets or loaded from TOML files.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,

    // ── Brand / Primary ──────────────────────────────────────
    pub accent: Color,
    pub accent_secondary: Color,
    pub bg_dark: Color,
    pub bg_panel: Color,

    // ── Text ─────────────────────────────────────────────────
    pub text_primary: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    // ── Semantic ─────────────────────────────────────────────
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub critical: Color,
    pub info: Color,

    // ── Gauges ───────────────────────────────────────────────
    pub gauge_low: Color,
    pub gauge_mid: Color,
    pub gauge_high: Color,
    pub gauge_critical: Color,
    pub gauge_bg: Color,

    // ── Table selection ──────────────────────────────────────
    pub table_row_selected_bg: Color,

    // ── Borders ──────────────────────────────────────────────
    pub border: Color,

    // ── AI ───────────────────────────────────────────────────
    pub ai_accent: Color,
    pub ai_response: Color,

    // ── GPU (NVIDIA green) ───────────────────────────────────
    pub gpu_accent: Color,
}

impl Theme {
    // ── Constructors ─────────────────────────────────────────

    /// Default dark theme (the original Sentinel palette).
    pub fn default_dark() -> Self {
        Self {
            name: "default".to_string(),
            accent: Color::Rgb(99, 179, 237),
            accent_secondary: Color::Rgb(129, 230, 217),
            bg_dark: Color::Rgb(22, 22, 30),
            bg_panel: Color::Rgb(30, 30, 42),
            text_primary: Color::Rgb(220, 220, 235),
            text_dim: Color::Rgb(120, 120, 145),
            text_muted: Color::Rgb(80, 80, 100),
            success: Color::Rgb(72, 199, 142),
            warning: Color::Rgb(255, 193, 69),
            danger: Color::Rgb(255, 85, 85),
            critical: Color::Rgb(255, 136, 0),
            info: Color::Rgb(99, 179, 237),
            gauge_low: Color::Rgb(72, 199, 142),
            gauge_mid: Color::Rgb(255, 193, 69),
            gauge_high: Color::Rgb(255, 136, 0),
            gauge_critical: Color::Rgb(255, 85, 85),
            gauge_bg: Color::Rgb(45, 45, 58),
            table_row_selected_bg: Color::Rgb(40, 40, 60),
            border: Color::Rgb(55, 55, 75),
            ai_accent: Color::Rgb(217, 143, 255),
            ai_response: Color::Rgb(200, 210, 230),
            gpu_accent: Color::Rgb(118, 185, 0),
        }
    }

    /// Gruvbox dark palette.
    pub fn gruvbox() -> Self {
        Self {
            name: "gruvbox".to_string(),
            accent: Color::Rgb(215, 153, 33),            // yellow
            accent_secondary: Color::Rgb(142, 192, 124), // green
            bg_dark: Color::Rgb(40, 40, 40),             // bg0
            bg_panel: Color::Rgb(50, 48, 47),            // bg0_s
            text_primary: Color::Rgb(235, 219, 178),     // fg
            text_dim: Color::Rgb(168, 153, 132),         // fg4
            text_muted: Color::Rgb(102, 92, 84),         // bg4
            success: Color::Rgb(142, 192, 124),          // green
            warning: Color::Rgb(250, 189, 47),           // yellow bright
            danger: Color::Rgb(251, 73, 52),             // red
            critical: Color::Rgb(254, 128, 25),          // orange
            info: Color::Rgb(131, 165, 152),             // blue
            gauge_low: Color::Rgb(142, 192, 124),
            gauge_mid: Color::Rgb(250, 189, 47),
            gauge_high: Color::Rgb(254, 128, 25),
            gauge_critical: Color::Rgb(251, 73, 52),
            gauge_bg: Color::Rgb(60, 56, 54),
            table_row_selected_bg: Color::Rgb(60, 56, 54),
            border: Color::Rgb(80, 73, 69),
            ai_accent: Color::Rgb(211, 134, 155), // purple
            ai_response: Color::Rgb(235, 219, 178),
            gpu_accent: Color::Rgb(142, 192, 124),
        }
    }

    /// Nord palette.
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            accent: Color::Rgb(136, 192, 208), // nord8 frost
            accent_secondary: Color::Rgb(143, 188, 187), // nord7
            bg_dark: Color::Rgb(46, 52, 64),   // nord0
            bg_panel: Color::Rgb(59, 66, 82),  // nord1
            text_primary: Color::Rgb(229, 233, 240), // nord5
            text_dim: Color::Rgb(182, 191, 204), // custom
            text_muted: Color::Rgb(107, 112, 127), // custom
            success: Color::Rgb(163, 190, 140), // nord14 green
            warning: Color::Rgb(235, 203, 139), // nord13 yellow
            danger: Color::Rgb(191, 97, 106),  // nord11 red
            critical: Color::Rgb(208, 135, 112), // nord12 orange
            info: Color::Rgb(129, 161, 193),   // nord9
            gauge_low: Color::Rgb(163, 190, 140),
            gauge_mid: Color::Rgb(235, 203, 139),
            gauge_high: Color::Rgb(208, 135, 112),
            gauge_critical: Color::Rgb(191, 97, 106),
            gauge_bg: Color::Rgb(67, 76, 94), // nord2
            table_row_selected_bg: Color::Rgb(67, 76, 94),
            border: Color::Rgb(76, 86, 106),        // nord3
            ai_accent: Color::Rgb(180, 142, 173),   // nord15 purple
            ai_response: Color::Rgb(216, 222, 233), // nord4
            gpu_accent: Color::Rgb(163, 190, 140),
        }
    }

    /// Catppuccin Mocha palette.
    pub fn catppuccin() -> Self {
        Self {
            name: "catppuccin".to_string(),
            accent: Color::Rgb(137, 180, 250),           // blue
            accent_secondary: Color::Rgb(148, 226, 213), // teal
            bg_dark: Color::Rgb(30, 30, 46),             // base
            bg_panel: Color::Rgb(36, 39, 58),            // mantle
            text_primary: Color::Rgb(205, 214, 244),     // text
            text_dim: Color::Rgb(166, 173, 200),         // subtext0
            text_muted: Color::Rgb(108, 112, 134),       // overlay0
            success: Color::Rgb(166, 227, 161),          // green
            warning: Color::Rgb(249, 226, 175),          // yellow
            danger: Color::Rgb(243, 139, 168),           // red
            critical: Color::Rgb(250, 179, 135),         // peach
            info: Color::Rgb(137, 180, 250),             // blue
            gauge_low: Color::Rgb(166, 227, 161),
            gauge_mid: Color::Rgb(249, 226, 175),
            gauge_high: Color::Rgb(250, 179, 135),
            gauge_critical: Color::Rgb(243, 139, 168),
            gauge_bg: Color::Rgb(49, 50, 68), // surface0
            table_row_selected_bg: Color::Rgb(49, 50, 68),
            border: Color::Rgb(69, 71, 90),         // surface1
            ai_accent: Color::Rgb(203, 166, 247),   // mauve
            ai_response: Color::Rgb(186, 194, 222), // subtext1
            gpu_accent: Color::Rgb(166, 227, 161),
        }
    }

    /// Dracula palette.
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            accent: Color::Rgb(139, 233, 253),          // cyan
            accent_secondary: Color::Rgb(80, 250, 123), // green
            bg_dark: Color::Rgb(40, 42, 54),            // background
            bg_panel: Color::Rgb(48, 51, 65),           // current line
            text_primary: Color::Rgb(248, 248, 242),    // foreground
            text_dim: Color::Rgb(188, 188, 172),        // custom
            text_muted: Color::Rgb(98, 114, 164),       // comment
            success: Color::Rgb(80, 250, 123),          // green
            warning: Color::Rgb(241, 250, 140),         // yellow
            danger: Color::Rgb(255, 85, 85),            // red
            critical: Color::Rgb(255, 184, 108),        // orange
            info: Color::Rgb(139, 233, 253),            // cyan
            gauge_low: Color::Rgb(80, 250, 123),
            gauge_mid: Color::Rgb(241, 250, 140),
            gauge_high: Color::Rgb(255, 184, 108),
            gauge_critical: Color::Rgb(255, 85, 85),
            gauge_bg: Color::Rgb(68, 71, 90), // selection
            table_row_selected_bg: Color::Rgb(68, 71, 90),
            border: Color::Rgb(98, 114, 164),     // comment
            ai_accent: Color::Rgb(189, 147, 249), // purple
            ai_response: Color::Rgb(248, 248, 242),
            gpu_accent: Color::Rgb(80, 250, 123),
        }
    }

    /// Solarized dark palette.
    pub fn solarized() -> Self {
        Self {
            name: "solarized".to_string(),
            accent: Color::Rgb(38, 139, 210),           // blue
            accent_secondary: Color::Rgb(42, 161, 152), // cyan
            bg_dark: Color::Rgb(0, 43, 54),             // base03
            bg_panel: Color::Rgb(7, 54, 66),            // base02
            text_primary: Color::Rgb(147, 161, 161),    // base1
            text_dim: Color::Rgb(101, 123, 131),        // base00
            text_muted: Color::Rgb(88, 110, 117),       // base01
            success: Color::Rgb(133, 153, 0),           // green
            warning: Color::Rgb(181, 137, 0),           // yellow
            danger: Color::Rgb(220, 50, 47),            // red
            critical: Color::Rgb(203, 75, 22),          // orange
            info: Color::Rgb(38, 139, 210),             // blue
            gauge_low: Color::Rgb(133, 153, 0),
            gauge_mid: Color::Rgb(181, 137, 0),
            gauge_high: Color::Rgb(203, 75, 22),
            gauge_critical: Color::Rgb(220, 50, 47),
            gauge_bg: Color::Rgb(7, 54, 66),
            table_row_selected_bg: Color::Rgb(7, 54, 66),
            border: Color::Rgb(88, 110, 117),
            ai_accent: Color::Rgb(108, 113, 196), // violet
            ai_response: Color::Rgb(147, 161, 161),
            gpu_accent: Color::Rgb(133, 153, 0),
        }
    }

    /// Look up a built-in theme by name (case-insensitive).
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "default" => Some(Self::default_dark()),
            "gruvbox" => Some(Self::gruvbox()),
            "nord" => Some(Self::nord()),
            "catppuccin" => Some(Self::catppuccin()),
            "dracula" => Some(Self::dracula()),
            "solarized" => Some(Self::solarized()),
            _ => None,
        }
    }

    /// Cycle to the next built-in theme.
    pub fn next_builtin(&self) -> Self {
        let idx = BUILTIN_THEME_NAMES
            .iter()
            .position(|&n| n == self.name)
            .unwrap_or(0);
        let next_idx = (idx + 1) % BUILTIN_THEME_NAMES.len();
        Self::by_name(BUILTIN_THEME_NAMES[next_idx]).unwrap()
    }

    /// Load a custom theme from a TOML file, falling back to default for missing fields.
    pub fn from_toml_file(path: &std::path::Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let file: ThemeFile = toml::from_str(&content).ok()?;
        Some(
            file.into_theme(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("custom"),
            ),
        )
    }

    /// Discover custom themes from ~/.config/sentinel/themes/*.toml
    #[allow(dead_code)]
    pub fn load_custom_themes() -> Vec<Self> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let themes_dir = std::path::PathBuf::from(home)
            .join(".config")
            .join("sentinel")
            .join("themes");

        let mut themes = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&themes_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Some(theme) = Self::from_toml_file(&path) {
                        themes.push(theme);
                    }
                }
            }
        }
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        themes
    }

    // ── Computed Styles ──────────────────────────────────────

    pub fn header_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_style(&self) -> Style {
        Style::default()
            .fg(self.bg_dark)
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive_style(&self) -> Style {
        Style::default().fg(self.text_dim)
    }

    pub fn table_header_style(&self) -> Style {
        Style::default()
            .fg(self.accent_secondary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn table_row_normal(&self) -> Style {
        Style::default().fg(self.text_primary)
    }

    pub fn table_row_selected(&self) -> Style {
        Style::default()
            .fg(self.text_primary)
            .bg(self.table_row_selected_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn alert_style(&self, severity: AlertSeverity) -> Style {
        let color = match severity {
            AlertSeverity::Info => self.info,
            AlertSeverity::Warning => self.warning,
            AlertSeverity::Critical => self.critical,
            AlertSeverity::Danger => self.danger,
        };
        Style::default().fg(color)
    }

    pub fn severity_badge_style(&self, severity: AlertSeverity) -> Style {
        let (fg, bg) = match severity {
            AlertSeverity::Info => (self.bg_dark, self.info),
            AlertSeverity::Warning => (self.bg_dark, self.warning),
            AlertSeverity::Critical => (self.bg_dark, self.critical),
            AlertSeverity::Danger => (Color::White, self.danger),
        };
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
    }

    /// Returns a color for a usage percentage gauge.
    pub fn usage_color(&self, percent: f32) -> Color {
        if percent >= 90.0 {
            self.gauge_critical
        } else if percent >= 70.0 {
            self.gauge_high
        } else if percent >= 40.0 {
            self.gauge_mid
        } else {
            self.gauge_low
        }
    }

    /// Returns a color for temperature in Celsius.
    pub fn temp_color(&self, celsius: f32) -> Color {
        if celsius >= 90.0 {
            self.gauge_critical
        } else if celsius >= 75.0 {
            self.gauge_high
        } else if celsius >= 60.0 {
            self.gauge_mid
        } else {
            self.gauge_low
        }
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn border_highlight_style(&self) -> Style {
        Style::default().fg(self.accent)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_dark()
    }
}

// ── TOML deserialization for custom themes ──────────────────

/// Intermediate struct for parsing theme TOML files.
/// All fields are optional — missing fields inherit from the default theme.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ThemeFile {
    accent: Option<String>,
    accent_secondary: Option<String>,
    bg_dark: Option<String>,
    bg_panel: Option<String>,
    text_primary: Option<String>,
    text_dim: Option<String>,
    text_muted: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    danger: Option<String>,
    critical: Option<String>,
    info: Option<String>,
    gauge_low: Option<String>,
    gauge_mid: Option<String>,
    gauge_high: Option<String>,
    gauge_critical: Option<String>,
    gauge_bg: Option<String>,
    table_row_selected_bg: Option<String>,
    border: Option<String>,
    ai_accent: Option<String>,
    ai_response: Option<String>,
    gpu_accent: Option<String>,
}

impl ThemeFile {
    fn into_theme(self, name: &str) -> Theme {
        let base = Theme::default_dark();
        Theme {
            name: name.to_string(),
            accent: parse_color(&self.accent).unwrap_or(base.accent),
            accent_secondary: parse_color(&self.accent_secondary).unwrap_or(base.accent_secondary),
            bg_dark: parse_color(&self.bg_dark).unwrap_or(base.bg_dark),
            bg_panel: parse_color(&self.bg_panel).unwrap_or(base.bg_panel),
            text_primary: parse_color(&self.text_primary).unwrap_or(base.text_primary),
            text_dim: parse_color(&self.text_dim).unwrap_or(base.text_dim),
            text_muted: parse_color(&self.text_muted).unwrap_or(base.text_muted),
            success: parse_color(&self.success).unwrap_or(base.success),
            warning: parse_color(&self.warning).unwrap_or(base.warning),
            danger: parse_color(&self.danger).unwrap_or(base.danger),
            critical: parse_color(&self.critical).unwrap_or(base.critical),
            info: parse_color(&self.info).unwrap_or(base.info),
            gauge_low: parse_color(&self.gauge_low).unwrap_or(base.gauge_low),
            gauge_mid: parse_color(&self.gauge_mid).unwrap_or(base.gauge_mid),
            gauge_high: parse_color(&self.gauge_high).unwrap_or(base.gauge_high),
            gauge_critical: parse_color(&self.gauge_critical).unwrap_or(base.gauge_critical),
            gauge_bg: parse_color(&self.gauge_bg).unwrap_or(base.gauge_bg),
            table_row_selected_bg: parse_color(&self.table_row_selected_bg)
                .unwrap_or(base.table_row_selected_bg),
            border: parse_color(&self.border).unwrap_or(base.border),
            ai_accent: parse_color(&self.ai_accent).unwrap_or(base.ai_accent),
            ai_response: parse_color(&self.ai_response).unwrap_or(base.ai_response),
            gpu_accent: parse_color(&self.gpu_accent).unwrap_or(base.gpu_accent),
        }
    }
}

/// Parse a hex color string like "#FF8800" or "FF8800" into a ratatui Color.
fn parse_color(opt: &Option<String>) -> Option<Color> {
    let s = opt.as_ref()?;
    let hex = s.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}
