//! Security Dashboard module.
//!
//! Multi-panel interactive security monitoring with:
//! - Active listeners with port risk classification
//! - Established connections
//! - Unified security event timeline
//! - Threat summary counters
//! - System integrity (auth log, package integrity, logged-in users)
//! - Security score (0-100) with Telegram alerts on drop

pub mod collector;
pub mod state;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::security::state::*;
use crate::ui::AppState;

// ── Main renderer ────────────────────────────────────────────────

/// Render the full security dashboard.
pub fn render_security(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let sec = &state.security;

    // 5-section vertical layout:
    // [Score bar: 3] [Listeners+Connections: ~40%] [Timeline: ~30%] [Summary+Integrity: ~25%]
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Score bar
            Constraint::Min(8),     // Listeners + Connections (stretchy)
            Constraint::Length(10), // Security Events Timeline
            Constraint::Length(8),  // Threat Summary + System Integrity
        ])
        .split(area);

    render_score_bar(frame, main_chunks[0], sec, t);

    // Listeners (50%) | Connections (50%)
    let top_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    render_listeners(frame, top_split[0], sec, t);
    render_connections(frame, top_split[1], sec, t);

    render_timeline(frame, main_chunks[2], sec, t);

    // Threat Summary (50%) | System Integrity (50%)
    let bottom_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[3]);

    render_threat_summary(frame, bottom_split[0], sec, t);
    render_integrity(frame, bottom_split[1], sec, t);

    // Detail popup overlay
    if sec.detail_popup {
        render_detail_popup(frame, area, sec, t);
    }
}

// ── Score bar ────────────────────────────────────────────────────

fn render_score_bar(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let score_color = match sec.score {
        80..=100 => Color::Green,
        60..=79 => Color::Yellow,
        40..=59 => Color::Rgb(255, 165, 0), // orange
        _ => Color::Red,
    };

    let label = format!(" Security Score: {}/100  {} ", sec.score, sec.score_label());

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(" Security Dashboard ", t.header_style()))
                .borders(Borders::ALL)
                .border_style(t.border_style()),
        )
        .gauge_style(Style::default().fg(score_color).bg(t.bg_dark))
        .label(Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .ratio(sec.score as f64 / 100.0);

    frame.render_widget(gauge, area);
}

// ── Listeners panel ──────────────────────────────────────────────

fn render_listeners(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let is_focused = sec.focused_panel == SecurityPanel::Listeners;
    let border_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        t.border_style()
    };

    let title = format!(" Active Listeners ({}) ", sec.listeners.len());
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if is_focused { t.accent } else { t.text_primary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    if sec.listeners.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  No active listeners",
            Style::default().fg(t.text_dim),
        )))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("PORT").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("PROTO").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("PID").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("PROCESS").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("BIND").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("RISK").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = sec
        .listeners
        .iter()
        .skip(sec.listener_scroll)
        .enumerate()
        .map(|(i, l)| {
            let risk_color = match l.risk {
                PortRisk::Known => Color::Green,
                PortRisk::Suspicious => Color::Yellow,
                PortRisk::Unowned => Color::Red,
            };
            let is_selected =
                is_focused && i == sec.selected_index.saturating_sub(sec.listener_scroll);
            let row_style = if is_selected {
                Style::default().bg(t.accent).fg(t.bg_dark)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(format!("{}", l.port)).style(Style::default().fg(t.text_primary)),
                Cell::from(l.protocol.as_str()).style(Style::default().fg(t.text_dim)),
                Cell::from(l.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()))
                    .style(Style::default().fg(t.text_dim)),
                Cell::from(l.process_name.as_str()).style(Style::default().fg(t.text_primary)),
                Cell::from(l.bind_addr.as_str()).style(Style::default().fg(t.text_dim)),
                Cell::from(format!("{}", l.risk)).style(Style::default().fg(risk_color)),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),  // PORT
            Constraint::Length(6),  // PROTO
            Constraint::Length(7),  // PID
            Constraint::Min(12),    // PROCESS
            Constraint::Length(16), // BIND
            Constraint::Length(8),  // RISK
        ],
    )
    .header(header)
    .block(block);

    frame.render_widget(table, area);
}

// ── Connections panel ────────────────────────────────────────────

fn render_connections(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let is_focused = sec.focused_panel == SecurityPanel::Connections;
    let border_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        t.border_style()
    };

    let title = format!(" Connections ({}) ", sec.connections.len());
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if is_focused { t.accent } else { t.text_primary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    if sec.connections.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  No established connections",
            Style::default().fg(t.text_dim),
        )))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("LOCAL").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("REMOTE").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("PID").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
        Cell::from("PROCESS").style(Style::default().fg(t.text_dim).add_modifier(Modifier::BOLD)),
    ]);

    let rows: Vec<Row> = sec
        .connections
        .iter()
        .skip(sec.connection_scroll)
        .enumerate()
        .map(|(i, c)| {
            let is_selected =
                is_focused && i == sec.selected_index.saturating_sub(sec.connection_scroll);
            let row_style = if is_selected {
                Style::default().bg(t.accent).fg(t.bg_dark)
            } else {
                Style::default()
            };
            let local = format!(":{}", c.local_port);
            let remote = format!("{}:{}", c.remote_addr, c.remote_port);
            Row::new(vec![
                Cell::from(local).style(Style::default().fg(t.text_primary)),
                Cell::from(remote).style(Style::default().fg(t.text_dim)),
                Cell::from(c.pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into()))
                    .style(Style::default().fg(t.text_dim)),
                Cell::from(c.process_name.as_str()).style(Style::default().fg(t.text_primary)),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),  // LOCAL
            Constraint::Min(18),    // REMOTE
            Constraint::Length(7),  // PID
            Constraint::Length(15), // PROCESS
        ],
    )
    .header(header)
    .block(block);

    frame.render_widget(table, area);
}

// ── Security events timeline ─────────────────────────────────────

fn render_timeline(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let is_focused = sec.focused_panel == SecurityPanel::Timeline;
    let border_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        t.border_style()
    };

    let title = format!(" Security Events ({}) ", sec.events.len());
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if is_focused { t.accent } else { t.text_primary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    if sec.events.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  No security events in the last 30 minutes",
            Style::default().fg(t.text_dim),
        )))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let lines: Vec<Line> = sec
        .events
        .iter()
        .skip(sec.event_scroll)
        .take(visible_height)
        .enumerate()
        .map(|(i, ev)| {
            let is_selected =
                is_focused && i == sec.selected_index.saturating_sub(sec.event_scroll);

            let severity_color = match ev.severity {
                crate::models::AlertSeverity::Danger => Color::Red,
                crate::models::AlertSeverity::Critical => Color::Rgb(255, 165, 0),
                crate::models::AlertSeverity::Warning => Color::Yellow,
                crate::models::AlertSeverity::Info => Color::Green,
            };

            let age = ev.age_display();
            let icon = ev.icon();

            let style = if is_selected {
                Style::default().bg(t.accent).fg(t.bg_dark)
            } else {
                Style::default()
            };

            Line::from(vec![
                Span::styled(
                    format!(" [{:>4}] ", age),
                    if is_selected {
                        style
                    } else {
                        Style::default().fg(t.text_dim)
                    },
                ),
                Span::styled(
                    format!("{} ", icon),
                    if is_selected {
                        style
                    } else {
                        Style::default().fg(severity_color)
                    },
                ),
                Span::styled(
                    format!("{:<5} ", ev.kind),
                    if is_selected {
                        style
                    } else {
                        Style::default().fg(t.text_dim)
                    },
                ),
                Span::styled(
                    ev.message.clone(),
                    if is_selected {
                        style
                    } else {
                        Style::default().fg(t.text_primary)
                    },
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Threat summary ───────────────────────────────────────────────

fn render_threat_summary(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let is_focused = sec.focused_panel == SecurityPanel::ThreatSummary;
    let border_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        t.border_style()
    };

    let block = Block::default()
        .title(Span::styled(
            " Threat Summary ",
            Style::default()
                .fg(if is_focused { t.accent } else { t.text_primary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    let threat_color = if sec.active_threats > 0 {
        Color::Red
    } else {
        Color::Green
    };
    let suspicious_color = if sec.suspicious_count > 0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let risky_color = if !sec.risky_ports.is_empty() {
        Color::Yellow
    } else {
        Color::Green
    };
    let unowned_color = if sec.unowned_listeners > 0 {
        Color::Red
    } else {
        Color::Green
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Active threats:    ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}", sec.active_threats),
                Style::default()
                    .fg(threat_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Suspicious procs:  ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}", sec.suspicious_count),
                Style::default().fg(suspicious_color),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Risky ports:       ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}", sec.risky_ports.len()),
                Style::default().fg(risky_color),
            ),
            if !sec.risky_ports.is_empty() {
                Span::styled(
                    format!(
                        " ({})",
                        sec.risky_ports
                            .iter()
                            .take(5)
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Style::default().fg(t.text_muted),
                )
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("  Unowned listeners: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}", sec.unowned_listeners),
                Style::default().fg(unowned_color),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── System integrity ─────────────────────────────────────────────

fn render_integrity(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    let is_focused = sec.focused_panel == SecurityPanel::Integrity;
    let border_style = if is_focused {
        Style::default().fg(t.accent)
    } else {
        t.border_style()
    };

    let block = Block::default()
        .title(Span::styled(
            " System Integrity ",
            Style::default()
                .fg(if is_focused { t.accent } else { t.text_primary })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    let users_str = if sec.logged_in_users.is_empty() {
        "none".to_string()
    } else {
        sec.logged_in_users.join(", ")
    };

    let auth_str = if sec.auth_log_readable {
        format!("{} (24h)", sec.auth_event_count_24h)
    } else {
        "N/A (no permission)".to_string()
    };

    let pkg_color = if sec.modified_packages.is_empty() {
        Color::Green
    } else {
        Color::Yellow
    };

    // Compute uptime
    let uptime_str = match std::fs::read_to_string("/proc/uptime") {
        Ok(content) => {
            let secs: u64 = content
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|f| f as u64)
                .unwrap_or(0);
            let days = secs / 86400;
            let hours = (secs % 86400) / 3600;
            if days > 0 {
                format!("{}d {}h", days, hours)
            } else {
                format!("{}h", hours)
            }
        }
        Err(_) => "N/A".to_string(),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Users logged in:   ", Style::default().fg(t.text_dim)),
            Span::styled(users_str, Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled("  Auth events:       ", Style::default().fg(t.text_dim)),
            Span::styled(auth_str, Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled("  Modified packages: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}", sec.modified_packages.len()),
                Style::default().fg(pkg_color),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Uptime:            ", Style::default().fg(t.text_dim)),
            Span::styled(uptime_str, Style::default().fg(t.text_primary)),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Detail popup ─────────────────────────────────────────────────

fn render_detail_popup(frame: &mut Frame, area: Rect, sec: &SecurityState, t: &crate::ui::Theme) {
    // Center the popup
    let popup_width = 60u16.min(area.width.saturating_sub(4));
    let popup_height = 12u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.bg_dark)),
        popup_area,
    );

    let block = Block::default()
        .title(Span::styled(
            " Detail ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent));

    let mut lines = Vec::new();

    match sec.focused_panel {
        SecurityPanel::Listeners => {
            if let Some(listener) = sec.listeners.get(sec.selected_index) {
                lines.push(Line::from(vec![
                    Span::styled("  Port:     ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        format!("{}", listener.port),
                        Style::default()
                            .fg(t.text_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Protocol: ", Style::default().fg(t.text_dim)),
                    Span::styled(&listener.protocol, Style::default().fg(t.text_primary)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Process:  ", Style::default().fg(t.text_dim)),
                    Span::styled(&listener.process_name, Style::default().fg(t.text_primary)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  PID:      ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        listener
                            .pid
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "?".into()),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Bind:     ", Style::default().fg(t.text_dim)),
                    Span::styled(&listener.bind_addr, Style::default().fg(t.text_primary)),
                ]));
                let risk_color = match listener.risk {
                    PortRisk::Known => Color::Green,
                    PortRisk::Suspicious => Color::Yellow,
                    PortRisk::Unowned => Color::Red,
                };
                lines.push(Line::from(vec![
                    Span::styled("  Risk:     ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        format!("{}", listener.risk),
                        Style::default().fg(risk_color).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }
        SecurityPanel::Connections => {
            if let Some(conn) = sec.connections.get(sec.selected_index) {
                lines.push(Line::from(vec![
                    Span::styled("  Local:    ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        format!("{}:{}", conn.local_addr, conn.local_port),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Remote:   ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        format!("{}:{}", conn.remote_addr, conn.remote_port),
                        Style::default()
                            .fg(t.text_primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Process:  ", Style::default().fg(t.text_dim)),
                    Span::styled(&conn.process_name, Style::default().fg(t.text_primary)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  PID:      ", Style::default().fg(t.text_dim)),
                    Span::styled(
                        conn.pid
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "?".into()),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  State:    ", Style::default().fg(t.text_dim)),
                    Span::styled(&conn.state, Style::default().fg(t.text_primary)),
                ]));
            }
        }
        _ => {
            lines.push(Line::from(Span::styled(
                "  No detail view for this panel",
                Style::default().fg(t.text_dim),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press Esc to close",
        Style::default().fg(t.text_muted),
    )));

    frame.render_widget(Paragraph::new(lines).block(block), popup_area);
}
