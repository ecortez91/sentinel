//! Popup overlays: process detail, help, signal picker, renice dialog,
//! command palette, command result.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::state::AppState;

use super::helpers::{centered_rect, render_scrollbar, truncate_str};

pub fn render_process_detail(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let Some(detail) = &state.process_detail else {
        return;
    };

    let popup_width = 80.min(area.width.saturating_sub(4));
    let popup_height = (area.height - 4).min(40);
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(Span::styled(
            format!(
                " Process {} - {} (Esc to close, ↑↓ scroll) ",
                detail.pid, detail.name
            ),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_highlight_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    let cmd_width = (inner.width as usize).saturating_sub(4);

    // Basic info section
    lines.push(Line::from(Span::styled(
        " Process Info",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    lines.push(detail_line("PID:      ", &format!("{}", detail.pid), t));
    lines.push(detail_line("Name:     ", &detail.name, t));
    lines.push(detail_line("User:     ", &detail.user, t));
    lines.push(Line::from(vec![
        Span::styled("  Status:   ", Style::default().fg(t.text_dim)),
        Span::styled(&detail.status, Style::default().fg(t.success)),
    ]));
    if let Some(ppid) = detail.parent_pid {
        lines.push(detail_line("Parent:   ", &format!("PID {}", ppid), t));
    }
    if let Some(tc) = detail.thread_count {
        lines.push(detail_line("Threads:  ", &format!("{}", tc), t));
    }
    lines.push(Line::raw(""));

    // Resource usage
    lines.push(Line::from(Span::styled(
        " Resource Usage",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    let cpu_color = t.usage_color(detail.cpu_usage);
    lines.push(Line::from(vec![
        Span::styled("  CPU:      ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("{:.1}%", detail.cpu_usage),
            Style::default().fg(cpu_color),
        ),
    ]));
    let mem_color = t.usage_color(detail.memory_percent);
    lines.push(Line::from(vec![
        Span::styled("  Memory:   ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!(
                "{} ({:.1}%)",
                crate::models::format_bytes(detail.memory_bytes),
                detail.memory_percent
            ),
            Style::default().fg(mem_color),
        ),
    ]));
    lines.push(Line::raw(""));

    // Full command
    lines.push(Line::from(Span::styled(
        " Full Command",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    for line in textwrap::wrap(&detail.cmd, cmd_width) {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(line.to_string(), Style::default().fg(t.text_primary)),
        ]));
    }
    lines.push(Line::raw(""));

    // Open file descriptors
    lines.push(Line::from(Span::styled(
        format!(" Open File Descriptors ({})", detail.open_fds),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    for fd in &detail.fd_sample {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(truncate_str(fd, cmd_width), Style::default().fg(t.text_dim)),
        ]));
    }
    if detail.open_fds > detail.fd_sample.len() {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!(
                    "  ... and {} more",
                    detail.open_fds - detail.fd_sample.len()
                ),
                Style::default().fg(t.text_muted),
            ),
        ]));
    }
    lines.push(Line::raw(""));

    // Environment variables
    lines.push(Line::from(Span::styled(
        format!(" Environment Variables ({})", detail.environ.len()),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    for var in &detail.environ {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                truncate_str(var, cmd_width),
                Style::default().fg(t.text_dim),
            ),
        ]));
    }

    // Apply scrolling
    let visible_height = inner.height as usize;
    let total_lines = lines.len();
    let scroll = state
        .detail_scroll
        .min(total_lines.saturating_sub(visible_height));

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    frame.render_widget(Paragraph::new(visible_lines), inner);

    render_scrollbar(frame, inner, total_lines, scroll);
}

/// Helper: create a simple "  label: value" detail line.
fn detail_line<'a>(label: &str, value: &str, t: &crate::ui::theme::Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {}", label), Style::default().fg(t.text_dim)),
        Span::styled(value.to_string(), Style::default().fg(t.text_primary)),
    ])
}

pub fn render_help_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let popup_width = 55;
    let popup_height = 40;
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let help_entry = |key: &str, desc: &str, color: ratatui::style::Color| -> Line {
        Line::from(vec![
            Span::styled(
                format!("  {:<20}", key),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc.to_string(), Style::default().fg(t.text_primary)),
        ])
    };

    let help_text = vec![
        Line::from(Span::styled(
            "  SENTINEL - Keyboard Shortcuts",
            t.header_style(),
        )),
        Line::raw(""),
        help_entry("Tab / Shift+Tab", "Switch tabs", t.accent),
        help_entry("1 / 2 / 3 / 4", "Jump to tab (4 = Ask AI)", t.accent),
        help_entry("Up/Down / j / k", "Scroll up/down", t.accent),
        help_entry("PgUp / PgDn", "Page up/down", t.accent),
        help_entry("s", "Cycle sort column", t.accent),
        help_entry("r", "Reverse sort direction", t.accent),
        help_entry("/", "Filter processes", t.accent),
        help_entry("k", "SIGTERM selected process", t.warning),
        help_entry("K (shift)", "SIGKILL selected process", t.danger),
        help_entry("Enter", "Process detail popup", t.accent),
        help_entry("t", "Toggle process tree view", t.accent),
        help_entry("x", "Signal picker (process)", t.warning),
        help_entry("n", "Renice process", t.accent),
        help_entry("T", "Cycle color theme", t.accent),
        help_entry("L", "Cycle UI language", t.accent),
        help_entry("+/- (Dashboard)", "Zoom history charts", t.accent),
        help_entry("f (Dashboard)", "Focus/expand widget", t.accent),
        help_entry("a", "Ask AI about process", t.ai_accent),
        help_entry(":", "Command palette", t.accent_secondary),
        help_entry("  Tab (in results)", "Cycle actions", t.accent_secondary),
        help_entry(
            "  1-9 (in results)",
            "Quick-select action",
            t.accent_secondary,
        ),
        help_entry("Esc", "Clear filter / close help", t.accent),
        help_entry("q", "Quit", t.accent),
        Line::raw(""),
        Line::from(Span::styled(
            "  Ask AI Tab:",
            Style::default()
                .fg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        )),
        help_entry("Enter", "Send question to Claude", t.accent),
        help_entry("Ctrl+L", "Clear conversation", t.accent),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Thresholds: ", Style::default().fg(t.text_dim)),
            Span::styled(
                "CPU >50% warn, >90% crit",
                Style::default().fg(t.text_muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("              ", Style::default().fg(t.text_dim)),
            Span::styled(
                "RAM >1GiB warn, >2GiB crit/proc",
                Style::default().fg(t.text_muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("              ", Style::default().fg(t.text_dim)),
            Span::styled(
                "System RAM >75% warn, >90% crit",
                Style::default().fg(t.text_muted),
            ),
        ]),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(Span::styled(t!("title.help").to_string(), t.header_style()))
                .borders(Borders::ALL)
                .border_style(t.border_highlight_style()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(help, popup_area);
}

pub fn render_signal_picker(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height =
        (super::super::state::SIGNAL_LIST.len() as u16 + 6).min(area.height.saturating_sub(4));
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let title = format!(
        " Send Signal to PID {} ({}) ",
        state.signal_picker_pid.unwrap_or(0),
        truncate_str(&state.signal_picker_name, 16),
    );

    let block = Block::default()
        .title(Span::styled(title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_highlight_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        " Select a signal (Enter to send, Esc to cancel):",
        Style::default().fg(t.text_dim),
    )));
    lines.push(Line::raw(""));

    for (i, (num, name, desc)) in super::super::state::SIGNAL_LIST.iter().enumerate() {
        let is_selected = i == state.signal_picker_selected;
        let prefix = if is_selected { " > " } else { "   " };

        let style = if is_selected {
            t.table_row_selected()
        } else {
            Style::default().fg(t.text_primary)
        };

        let danger_style = if *num == 9 {
            if is_selected {
                Style::default()
                    .fg(t.danger)
                    .bg(t.table_row_selected_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.danger)
            }
        } else {
            style
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(format!("{:>2} ", num), Style::default().fg(t.text_muted)),
            Span::styled(format!("{:<12}", name), danger_style),
            Span::styled(*desc, Style::default().fg(t.text_dim)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn render_renice_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = 10.min(area.height.saturating_sub(4));
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let title = format!(
        " Renice PID {} ({}) ",
        state.renice_pid.unwrap_or(0),
        truncate_str(&state.renice_name, 16),
    );

    let block = Block::default()
        .title(Span::styled(title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_highlight_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let nice = state.renice_value;
    let nice_color = if nice < 0 {
        t.danger
    } else if nice == 0 {
        t.success
    } else {
        t.text_dim
    };

    let bar_width = 40.min(inner.width.saturating_sub(4)) as usize;
    let pos = ((nice + 20) as f64 / 39.0 * bar_width as f64) as usize;
    let bar: String = (0..bar_width)
        .map(|i| if i == pos { '█' } else { '░' })
        .collect();

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Nice value: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:+}", nice),
                Style::default().fg(nice_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if nice < 0 {
                    "  (higher priority)"
                } else if nice == 0 {
                    "  (normal priority)"
                } else {
                    "  (lower priority)"
                },
                Style::default().fg(t.text_muted),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  -20 ", Style::default().fg(t.danger)),
            Span::styled(bar, Style::default().fg(nice_color)),
            Span::styled(" +19", Style::default().fg(t.text_dim)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "  ←/→ ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Adjust  ", Style::default().fg(t.text_dim)),
            Span::styled(
                "Enter ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Apply  ", Style::default().fg(t.text_dim)),
            Span::styled(
                "Esc ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cancel", Style::default().fg(t.text_dim)),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn render_command_palette(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    // Render at the bottom of the screen, like a vim command line
    let width = area.width.min(80);
    let height = 3;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height.saturating_sub(height + 1);
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(Span::styled(
            " Command (type 'help' for list) ",
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_highlight_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let input_line = Line::from(vec![
        Span::styled(
            ":",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state.command_input.clone(),
            Style::default().fg(t.text_primary),
        ),
        Span::styled("█", Style::default().fg(t.accent)),
    ]);

    frame.render_widget(Paragraph::new(vec![input_line]), inner);
}

pub fn render_command_result(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let Some(ref result) = state.command_result else {
        return;
    };

    let popup_width = 76.min(area.width.saturating_sub(4));
    let popup_height = 30.min(area.height.saturating_sub(4));
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let has_actions = result.has_executable_actions();
    let is_ai_loading = state.command_ai_loading;
    let title = if is_ai_loading {
        let dots = crate::utils::loading_dots(state.tick_count);
        format!(" AI Thinking{} (Esc to close) ", dots)
    } else if has_actions {
        " Diagnostic Result (Tab: select action, 1-9: quick select, Esc: close) ".to_string()
    } else {
        " Diagnostic Result (Esc to close, Up/Down scroll) ".to_string()
    };

    let block = Block::default()
        .title(Span::styled(&title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(if is_ai_loading {
            Style::default().fg(t.ai_accent)
        } else {
            t.border_highlight_style()
        });
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let wrap_width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Build a map from action arrow lines to their action index
    // We number only executable (non-Info) actions
    let mut executable_index = 0usize;
    let action_numbers: Vec<Option<usize>> = result
        .actions
        .iter()
        .map(|(_, a)| {
            if matches!(a, crate::diagnostics::SuggestedAction::Info(_)) {
                None
            } else {
                executable_index += 1;
                Some(executable_index)
            }
        })
        .collect();

    // Track which action arrow lines map to which action index
    let mut action_line_map: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    let mut action_arrow_count = 0usize;

    for raw_line in result.text.lines() {
        let line_idx = lines.len();

        if raw_line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )));
        } else if raw_line.starts_with('\u{2716}') {
            // ✖
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(t.danger),
            )));
        } else if raw_line.starts_with('\u{26A0}') {
            // ⚠
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(t.warning),
            )));
        } else if raw_line.starts_with('\u{2139}') {
            // ℹ
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(t.info),
            )));
        } else if raw_line.contains('\u{2192}') {
            // → action line
            // Determine which action this is
            let action_idx = action_arrow_count;
            action_arrow_count += 1;

            if action_idx < result.actions.len() {
                action_line_map.insert(line_idx, action_idx);

                let is_selected = action_idx == state.command_result_selected_action && has_actions;
                let num = action_numbers.get(action_idx).copied().flatten();

                let prefix = if let Some(n) = num {
                    format!("[{}] ", n)
                } else {
                    String::new()
                };

                if is_selected {
                    // Highlighted action
                    lines.push(Line::from(vec![
                        Span::styled(
                            prefix,
                            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            raw_line.to_string(),
                            Style::default()
                                .fg(t.accent_secondary)
                                .bg(t.table_row_selected_bg)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else if num.is_some() {
                    // Numbered but not selected
                    lines.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(t.text_muted)),
                        Span::styled(
                            raw_line.to_string(),
                            Style::default().fg(t.accent_secondary),
                        ),
                    ]));
                } else {
                    // Info action (not executable)
                    lines.push(Line::from(Span::styled(
                        raw_line.to_string(),
                        Style::default().fg(t.text_dim),
                    )));
                }
            } else {
                lines.push(Line::from(Span::styled(
                    raw_line.to_string(),
                    Style::default().fg(t.accent_secondary),
                )));
            }
        } else if raw_line.len() > wrap_width {
            for wrapped in textwrap::wrap(raw_line, wrap_width) {
                lines.push(Line::from(Span::styled(
                    wrapped.to_string(),
                    Style::default().fg(t.text_primary),
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(t.text_primary),
            )));
        }
    }

    // Add action hint at the bottom if there are executable actions
    if has_actions {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  Tab",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cycle actions  ", Style::default().fg(t.text_dim)),
            Span::styled(
                "Enter",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" execute  ", Style::default().fg(t.text_dim)),
            Span::styled(
                "1-9",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" quick select", Style::default().fg(t.text_dim)),
        ]));
    }

    let visible_height = inner.height as usize;
    let total_lines = lines.len();
    let scroll = state
        .command_result_scroll
        .min(total_lines.saturating_sub(visible_height));

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    frame.render_widget(Paragraph::new(visible_lines), inner);
    render_scrollbar(frame, inner, total_lines, scroll);

    // Render confirmation dialog on top if active
    if state.show_action_confirm {
        render_action_confirm(frame, area, state);
    }
}

/// Render a confirmation dialog for executing a diagnostic action.
fn render_action_confirm(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let Some(ref cr) = state.command_result else {
        return;
    };
    let sel = state.command_result_selected_action;
    if sel >= cr.actions.len() {
        return;
    }
    let (label, action) = &cr.actions[sel];

    let (title_text, danger_level) = match action {
        crate::diagnostics::SuggestedAction::KillProcess { signal, .. } => {
            let is_kill = *signal == "SIGKILL";
            (
                format!(
                    " Confirm: {} ",
                    if is_kill {
                        "FORCE KILL"
                    } else {
                        "Kill Process"
                    }
                ),
                if is_kill { 2 } else { 1 },
            )
        }
        crate::diagnostics::SuggestedAction::FreePort { .. } => {
            (" Confirm: Free Port ".to_string(), 1)
        }
        crate::diagnostics::SuggestedAction::ReniceProcess { .. } => {
            (" Confirm: Renice Process ".to_string(), 0)
        }
        crate::diagnostics::SuggestedAction::CleanDirectory { .. } => {
            (" Confirm: Clean Directory ".to_string(), 2)
        }
        crate::diagnostics::SuggestedAction::Info(_) => return,
    };

    let border_color = match danger_level {
        2 => t.danger,
        1 => t.warning,
        _ => t.accent,
    };

    let popup_width = 60.min(area.width.saturating_sub(4));
    let popup_height = 8;
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(Span::styled(
            title_text,
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            format!("  {}", label),
            Style::default().fg(t.text_primary),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Are you sure? ", Style::default().fg(t.text_dim)),
            Span::styled(
                "[y]",
                Style::default().fg(t.success).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Yes  ", Style::default().fg(t.text_dim)),
            Span::styled(
                "[n]",
                Style::default().fg(t.danger).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" No", Style::default().fg(t.text_dim)),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
