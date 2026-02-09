//! Security dashboard module — Phase 2 stub.
//!
//! Currently provides a placeholder "Coming Soon" display with
//! existing port listener data from the event store.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ui::AppState;

/// Render the security tab content.
pub fn render_security(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Header
            Constraint::Min(5),    // Content
        ])
        .split(area);

    // Header block
    let header_block = Block::default()
        .title(Span::styled(" Security Dashboard ", t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let header_inner = header_block.inner(chunks[0]);
    frame.render_widget(header_block, chunks[0]);

    let header_lines = vec![
        Line::from(Span::styled(
            "  Security monitoring — Coming Soon",
            Style::default().fg(t.text_dim),
        )),
        Line::from(Span::styled(
            "  Open port monitoring available below",
            Style::default().fg(t.text_muted),
        )),
    ];
    frame.render_widget(Paragraph::new(header_lines), header_inner);

    // Open ports section (reuse data from event store/recent_events)
    let content_block = Block::default()
        .title(Span::styled(
            " Open Ports & Listeners ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let content_inner = content_block.inner(chunks[1]);
    frame.render_widget(content_block, chunks[1]);

    let mut lines = Vec::new();
    lines.push(Line::from(""));

    // Show listening port events from the event ticker
    let port_events: Vec<&String> = state
        .recent_events
        .iter()
        .filter(|e| e.contains(">") || e.contains("<"))
        .collect();

    if port_events.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No recent port activity detected",
            Style::default().fg(t.text_dim),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Use ':listeners' command for current port bindings",
            Style::default().fg(t.text_muted),
        )));
        lines.push(Line::from(Span::styled(
            "  Use ':port <number>' to investigate specific ports",
            Style::default().fg(t.text_muted),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Recent Port Activity:",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for event in port_events.iter().take(15) {
            lines.push(Line::from(Span::styled(
                format!("  {}", event),
                Style::default().fg(t.text_primary),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Planned features:",
        Style::default().fg(t.text_dim),
    )));
    lines.push(Line::from(Span::styled(
        "  - Network connection monitoring",
        Style::default().fg(t.text_muted),
    )));
    lines.push(Line::from(Span::styled(
        "  - Suspicious process detection dashboard",
        Style::default().fg(t.text_muted),
    )));
    lines.push(Line::from(Span::styled(
        "  - File integrity monitoring",
        Style::default().fg(t.text_muted),
    )));
    lines.push(Line::from(Span::styled(
        "  - Authentication log analysis",
        Style::default().fg(t.text_muted),
    )));

    frame.render_widget(Paragraph::new(lines), content_inner);
}
