//! Centralized glyph set for all UI rendering.
//!
//! Provides Unicode (rich) and ASCII (safe) character sets so the entire
//! application can gracefully degrade on terminals that don't render
//! Unicode block-drawing characters correctly (e.g., Windows Terminal
//! with certain fonts).
//!
//! Usage: `state.glyphs.filled` instead of hardcoded `'█'`.

use ratatui::symbols;

/// Which character set to use for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphMode {
    /// Full Unicode block elements, shade chars, geometric shapes.
    Unicode,
    /// ASCII-only safe characters that render on any terminal/font.
    Ascii,
}

impl GlyphMode {
    /// Parse from config string. Returns `None` for "auto".
    pub fn from_config(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "unicode" => Some(GlyphMode::Unicode),
            "ascii" => Some(GlyphMode::Ascii),
            _ => None, // "auto" or unknown → caller does detection
        }
    }
}

/// All glyphs used across the application, in one place.
///
/// Some fields (e.g. `icon_info`, `nav_left_right`) are intentionally
/// defined but not yet consumed — they exist for completeness and will
/// be used as renderers migrate to the glyph system.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Glyphs {
    pub mode: GlyphMode,

    // ── Gauge / bar characters ────────────────────────────────
    /// Fully filled bar cell.
    pub filled: char,
    /// Dark shade (>75% partial fill).
    pub shade_dark: char,
    /// Medium shade (>50% partial fill).
    pub shade_medium: char,
    /// Light shade / unfilled bar background.
    pub shade_light: char,

    // ── Vertical bar chart (8-level, for CpuMiniChart) ────────
    pub bar_chars: [char; 8],

    // ── Sparkline bar set (for ratatui Sparkline widget) ──────
    pub bar_set: symbols::bar::Set,

    // ── Scrollbar arrows ──────────────────────────────────────
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,

    // ── Sort indicators ───────────────────────────────────────
    pub sort_asc: &'static str,
    pub sort_desc: &'static str,

    // ── Heartbeat pulse ───────────────────────────────────────
    pub pulse_on: &'static str,
    pub pulse_off: &'static str,

    // ── Separators ────────────────────────────────────────────
    pub separator: &'static str,

    // ── Spinner animation frames ──────────────────────────────
    pub spinner: &'static [&'static str],

    // ── Process tree connectors ───────────────────────────────
    pub tree_branch: &'static str,
    pub tree_last: &'static str,
    pub tree_pipe: &'static str,
    pub tree_space: &'static str,

    // ── Favorites / icons ─────────────────────────────────────
    pub star: &'static str,
    pub pointer: &'static str,

    // ── Price direction arrows ─────────────────────────────────
    pub price_up: &'static str,
    pub price_down: &'static str,

    // ── Text cursor ───────────────────────────────────────────
    pub cursor: &'static str,

    // ── Battery status ────────────────────────────────────────
    pub battery_charging: &'static str,
    pub battery_discharging: &'static str,

    // ── Diagnostic severity icons ─────────────────────────────
    pub icon_info: &'static str,
    pub icon_warning: &'static str,
    pub icon_critical: &'static str,
    pub icon_action: &'static str,

    // ── Navigation hint arrows ────────────────────────────────
    pub nav_up_down: &'static str,
    pub nav_left_right: &'static str,
}

/// Unicode spinner frames.
const SPINNER_UNICODE: &[&str] = &["◐", "◓", "◑", "◒"];
/// ASCII spinner frames.
const SPINNER_ASCII: &[&str] = &["|", "/", "-", "\\"];

impl Glyphs {
    /// Build the glyph set for the given mode.
    pub fn new(mode: GlyphMode) -> Self {
        match mode {
            GlyphMode::Unicode => Self::unicode(),
            GlyphMode::Ascii => Self::ascii(),
        }
    }

    /// Full Unicode glyph set (default).
    fn unicode() -> Self {
        Self {
            mode: GlyphMode::Unicode,

            filled: '█',
            shade_dark: '▓',
            shade_medium: '▒',
            shade_light: '░',

            bar_chars: ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'],
            bar_set: symbols::bar::NINE_LEVELS,

            arrow_up: "▲",
            arrow_down: "▼",

            sort_asc: " ▲",
            sort_desc: " ▼",

            pulse_on: "●",
            pulse_off: "○",

            separator: " │ ",

            spinner: SPINNER_UNICODE,

            tree_branch: "├── ",
            tree_last: "└── ",
            tree_pipe: "│   ",
            tree_space: "    ",

            star: "★",
            pointer: "▸",

            price_up: "▲",
            price_down: "▼",

            cursor: "█",

            battery_charging: "⚡",
            battery_discharging: "🔋",

            icon_info: "ℹ",
            icon_warning: "⚠",
            icon_critical: "✖",
            icon_action: "→",

            nav_up_down: "\u{2191}\u{2193}",
            nav_left_right: "\u{2190}\u{2192}",
        }
    }

    /// ASCII-safe glyph set for terminals with poor Unicode support.
    fn ascii() -> Self {
        Self {
            mode: GlyphMode::Ascii,

            filled: '#',
            shade_dark: '#',
            shade_medium: '=',
            shade_light: '-',

            bar_chars: ['_', '.', ':', '-', '=', '+', '#', '#'],
            bar_set: symbols::bar::THREE_LEVELS,

            arrow_up: "^",
            arrow_down: "v",

            sort_asc: " ^",
            sort_desc: " v",

            pulse_on: "*",
            pulse_off: ".",

            separator: " | ",

            spinner: SPINNER_ASCII,

            tree_branch: "|-- ",
            tree_last: "`-- ",
            tree_pipe: "|   ",
            tree_space: "    ",

            star: "*",
            pointer: ">",

            price_up: "^",
            price_down: "v",

            cursor: "_",

            battery_charging: "+",
            battery_discharging: "-",

            icon_info: "[i]",
            icon_warning: "[!]",
            icon_critical: "[X]",
            icon_action: "->",

            nav_up_down: "Up/Dn",
            nav_left_right: "Lt/Rt",
        }
    }

    /// Get spinner character for the given tick.
    pub fn spinner_char(&self, tick: u64) -> &str {
        self.spinner[(tick % self.spinner.len() as u64) as usize]
    }

    /// Build a filled/empty bar string of given width and fill ratio.
    pub fn bar(&self, fill_ratio: f64, width: usize) -> String {
        let filled = (fill_ratio * width as f64).round() as usize;
        let filled = filled.min(width);
        let empty = width - filled;
        format!(
            "{}{}",
            self.filled.to_string().repeat(filled),
            self.shade_light.to_string().repeat(empty)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_mode_has_block_chars() {
        let g = Glyphs::new(GlyphMode::Unicode);
        assert_eq!(g.filled, '█');
        assert_eq!(g.shade_light, '░');
        assert_eq!(g.bar_chars[7], '█');
        assert_eq!(g.bar_chars[0], '▁');
    }

    #[test]
    fn ascii_mode_has_safe_chars() {
        let g = Glyphs::new(GlyphMode::Ascii);
        assert_eq!(g.filled, '#');
        assert_eq!(g.shade_light, '-');
        assert!(g.star.is_ascii());
        assert!(g.separator.is_ascii());
    }

    #[test]
    fn spinner_cycles() {
        let g = Glyphs::new(GlyphMode::Unicode);
        assert_eq!(g.spinner_char(0), "◐");
        assert_eq!(g.spinner_char(4), "◐");
    }

    #[test]
    fn ascii_spinner_cycles() {
        let g = Glyphs::new(GlyphMode::Ascii);
        assert_eq!(g.spinner_char(0), "|");
        assert_eq!(g.spinner_char(1), "/");
        assert_eq!(g.spinner_char(4), "|");
    }

    #[test]
    fn bar_helper_full() {
        let g = Glyphs::new(GlyphMode::Unicode);
        let bar = g.bar(1.0, 10);
        assert_eq!(bar, "██████████");
    }

    #[test]
    fn bar_helper_empty() {
        let g = Glyphs::new(GlyphMode::Unicode);
        let bar = g.bar(0.0, 10);
        assert_eq!(bar, "░░░░░░░░░░");
    }

    #[test]
    fn bar_helper_half() {
        let g = Glyphs::new(GlyphMode::Ascii);
        let bar = g.bar(0.5, 10);
        assert_eq!(bar, "#####-----");
    }

    #[test]
    fn glyph_mode_from_config() {
        assert_eq!(GlyphMode::from_config("unicode"), Some(GlyphMode::Unicode));
        assert_eq!(GlyphMode::from_config("ascii"), Some(GlyphMode::Ascii));
        assert_eq!(GlyphMode::from_config("auto"), None);
        assert_eq!(GlyphMode::from_config("UNICODE"), Some(GlyphMode::Unicode));
    }
}
