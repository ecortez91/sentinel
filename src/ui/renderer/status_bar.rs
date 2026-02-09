//! Status bar at the bottom of the screen.

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

    let mut spans = vec![
        Span::styled(" ", Style::default()),
        badge("q", t.accent),
        dim(&t!("status.quit").to_string()),
        badge("Tab", t.accent),
        dim(&t!("status.switch").to_string()),
        badge("↑↓", t.accent),
        dim(&t!("status.scroll").to_string()),
        badge("s", t.accent),
        dim(&t!("status.sort").to_string()),
        badge("T", t.accent),
        dim(&format!(" Theme: {} ", t.name)),
        badge("L", t.accent),
        dim(&format!(" Lang: {} ", state.current_lang.to_uppercase())),
        badge("4", t.ai_accent),
        dim(&t!("status.ask_ai").to_string()),
        badge("?", t.accent),
        dim(&t!("status.help").to_string()),
    ];

    // Show process-specific shortcuts on Processes tab
    if state.active_tab == Tab::Processes {
        spans.push(badge("t", t.accent));
        spans.push(dim(&t!("status.tree").to_string()));
        spans.push(badge("Enter", t.accent));
        spans.push(dim(&t!("status.detail").to_string()));
        spans.push(badge("a", t.ai_accent));
        spans.push(dim(&t!("status.ask_ai").to_string()));
        spans.push(badge("x", t.warning));
        spans.push(dim(&t!("status.signal").to_string()));
        spans.push(badge("n", t.accent));
        spans.push(dim(&t!("status.renice").to_string()));
        spans.push(badge("k", t.warning));
        spans.push(dim(&t!("status.kill").to_string()));
        // NOTE: Removed duplicate "Kill" label that existed in the original
    }

    // Show status message (e.g., kill confirmation) -- auto-expires
    if let Some((msg, when)) = &state.status_message {
        if when.elapsed().as_secs() < STATUS_MESSAGE_TIMEOUT_SECS {
            spans.push(Span::styled(
                format!("  {} ", msg),
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ));
        }
    }

    // System health indicator
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
