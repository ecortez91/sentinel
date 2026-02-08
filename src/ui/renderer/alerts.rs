//! Alerts tab: full alert history with scrolling.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::models::AlertSeverity;
use crate::ui::state::AppState;

use super::helpers::render_scrollbar;

pub fn render_alerts(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(
            format!(" Alert History ({}) ", state.alerts.len()),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.alerts.is_empty() {
        let msg = Paragraph::new(vec![
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("alert.none_normal").to_string(),
                Style::default().fg(t.success),
            )]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("alert.monitoring").to_string(),
                Style::default().fg(t.text_dim),
            )]),
            Line::from(vec![Span::styled(
                t!("alert.threshold_note").to_string(),
                Style::default().fg(t.text_dim),
            )]),
        ]);
        frame.render_widget(msg, inner);
        return;
    }

    let visible_start = state.alert_scroll;
    let visible_count = inner.height as usize;

    let lines: Vec<Line> = state
        .alerts
        .iter()
        .skip(visible_start)
        .take(visible_count)
        .map(|a| {
            let severity_symbol = match a.severity {
                AlertSeverity::Info => "i",
                AlertSeverity::Warning => "!",
                AlertSeverity::Critical => "*",
                AlertSeverity::Danger => "X",
            };

            Line::from(vec![
                Span::styled(format!(" {} ", severity_symbol), t.alert_style(a.severity)),
                Span::styled(
                    format!("{:>6} ", a.severity),
                    t.severity_badge_style(a.severity),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("[{:>8}] ", a.category),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(&a.message, t.alert_style(a.severity)),
                Span::styled(
                    format!("  PID:{}", a.pid),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(
                    format!("  {}", a.timestamp.format("%H:%M:%S")),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(
                    format!("  ({})", a.age_display()),
                    Style::default().fg(t.text_muted),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);

    render_scrollbar(frame, inner, state.alerts.len(), state.alert_scroll);
}
