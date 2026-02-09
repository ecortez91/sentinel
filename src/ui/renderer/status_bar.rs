//! Status bar at the bottom of the screen.
//!
//! Context-sensitive: shows global hints + tab-specific hints.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::constants::STATUS_MESSAGE_TIMEOUT_SECS;
use crate::ui::state::{AppState, Tab};

pub fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;

    // Helper to create a keybind badge
    let badge = |key: &str, color: ratatui::style::Color| -> Span {
        Span::styled(
            format!(" {} ", key),
            Style::default()
                .fg(t.bg_dark)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )
    };
    let dim =
        |text: &str| -> Span { Span::styled(text.to_string(), Style::default().fg(t.text_dim)) };
    let sep = || -> Span { Span::styled(" \u{2502} ", Style::default().fg(t.text_muted)) };

    let mut spans = vec![Span::styled(" ", Style::default())];

    // ── Global hints (always shown) ──────────────────────────
    spans.push(badge("?", t.accent));
    spans.push(dim(" Help "));
    spans.push(badge(":", t.accent_secondary));
    spans.push(dim(" Cmd "));
    spans.push(badge("Tab", t.accent));
    spans.push(dim(" Tabs "));

    spans.push(sep());

    // ── Tab-specific hints ───────────────────────────────────
    match state.active_tab {
        Tab::Dashboard => {
            spans.push(badge("+/-", t.accent));
            spans.push(dim(" Zoom "));
            spans.push(badge("f", t.accent));
            spans.push(dim(" Focus "));
            if state.ai_has_key {
                spans.push(badge("e", t.ai_accent));
                spans.push(dim(" AI Insight "));
            }
        }
        Tab::Processes => {
            spans.push(badge("Enter", t.accent));
            spans.push(dim(" Detail "));
            spans.push(badge("/", t.accent));
            spans.push(dim(" Filter "));
            spans.push(badge("k", t.warning));
            spans.push(dim(" Kill "));
            spans.push(badge("x", t.warning));
            spans.push(dim(" Signal "));
            spans.push(badge("n", t.accent));
            spans.push(dim(" Nice "));
            spans.push(badge("t", t.accent));
            spans.push(dim(" Tree "));
            if state.ai_has_key {
                spans.push(badge("a", t.ai_accent));
                spans.push(dim(" AI "));
            }
        }
        Tab::Alerts => {
            spans.push(badge("\u{2191}\u{2193}", t.accent));
            spans.push(dim(" Scroll "));
        }
        Tab::AskAi => {
            spans.push(badge("Enter", t.ai_accent));
            spans.push(dim(" Send "));
            spans.push(badge("Ctrl+L", t.ai_accent));
            spans.push(dim(" Clear "));
        }
        Tab::Thermal => {
            spans.push(badge(":", t.accent_secondary));
            spans.push(dim(" thermal "));
            spans.push(badge("Ctrl+X", t.warning));
            spans.push(dim(" Abort shutdown "));
        }
        Tab::Security => {
            spans.push(badge(":", t.accent_secondary));
            spans.push(dim(" listeners "));
            spans.push(badge(":", t.accent_secondary));
            spans.push(dim(" port <n> "));
        }
    }

    spans.push(sep());

    // ── Appearance (compact) ─────────────────────────────────
    spans.push(badge("T", t.accent));
    spans.push(dim(&format!(" {} ", t.name)));
    spans.push(badge("L", t.accent));
    spans.push(dim(&format!(" {} ", state.current_lang.to_uppercase())));

    // ── Status message (e.g., kill confirmation) -- auto-expires ──
    if let Some((msg, when)) = &state.status_message {
        if when.elapsed().as_secs() < STATUS_MESSAGE_TIMEOUT_SECS {
            spans.push(Span::styled(
                format!("  {} ", msg),
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ));
        }
    }

    // ── System health indicator (right side) ─────────────────
    if let Some(sys) = &state.system {
        let health_color = if sys.memory_percent() > 90.0 || sys.global_cpu_usage > 90.0 {
            t.danger
        } else if sys.memory_percent() > 75.0 || sys.global_cpu_usage > 75.0 {
            t.warning
        } else {
            t.success
        };
        let health = if sys.memory_percent() > 90.0 {
            t!("health.critical").to_string()
        } else if sys.memory_percent() > 75.0 {
            t!("health.warning").to_string()
        } else {
            t!("health.healthy").to_string()
        };
        spans.push(Span::styled(
            t!("health.label").to_string(),
            Style::default().fg(t.text_muted),
        ));
        spans.push(Span::styled(
            health,
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let status = Paragraph::new(Line::from(spans));
    frame.render_widget(status, area);
}
