//! Thermal monitoring: dashboard panel + full-screen tab.
//!
//! The dashboard panel (`render_thermal_panel`) is a compact inline widget.
//! The full tab (`render_thermal_tab`) provides a comprehensive view with
//! temperature sparkline, per-sensor breakdowns, thresholds, and shutdown status.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Frame,
};

use crate::ui::state::AppState;

// ─────────────────────────────────────────────────────────────────────────────
// Dashboard panel (compact — used inline on the Dashboard tab)
// ─────────────────────────────────────────────────────────────────────────────

/// Render the thermal monitoring panel (compact, for dashboard).
pub fn render_thermal_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(" Thermal Monitor (LHM) ", t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snap) = &state.thermal else { return };

    let mut lines: Vec<Line> = Vec::new();

    // CPU temperatures
    if snap.cpu_package.is_some() || !snap.cpu_cores.is_empty() {
        lines.push(section_header("CPU ", t));
        if let Some(pkg) = snap.cpu_package {
            lines.push(make_temp_line("  Package", pkg, 20, t));
        }
        for core in &snap.cpu_cores {
            let label = format!("  {}", core.name);
            lines.push(make_temp_line(&label, core.value, 20, t));
        }
    }

    // GPU temperatures
    if snap.gpu_temp.is_some() || snap.gpu_hotspot.is_some() {
        lines.push(section_header("GPU ", t));
        if let Some(temp) = snap.gpu_temp {
            lines.push(make_temp_line("  Core", temp, 20, t));
        }
        if let Some(temp) = snap.gpu_hotspot {
            lines.push(make_temp_line("  Hot Spot", temp, 20, t));
        }
    }

    // SSD temperatures
    if !snap.ssd_temps.is_empty() {
        lines.push(section_header("Storage ", t));
        for s in snap.ssd_temps.iter().take(4) {
            let label = format!("  {}", s.name);
            lines.push(make_temp_line(&label, s.value, 20, t));
        }
    }

    // Motherboard temperatures
    if !snap.motherboard_temps.is_empty() {
        lines.push(section_header("Board ", t));
        for s in snap.motherboard_temps.iter().take(4) {
            let label = format!("  {}", s.name);
            lines.push(make_temp_line(&label, s.value, 20, t));
        }
    }

    // Fan RPMs
    if !snap.fan_rpms.is_empty() {
        lines.push(section_header("Fans ", t));
        for s in snap.fan_rpms.iter().take(4) {
            lines.push(make_fan_line(&s.name, s.value, t));
        }
    }

    // Temperature history sparkline hint
    if !state.temp_history.is_empty() && inner.height > lines.len() as u16 + 2 {
        lines.push(Line::from(""));
        let max_label = format!("Peak: {:.1}°C", snap.max_temp);
        lines.push(Line::from(vec![
            Span::styled(
                "History ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(max_label, Style::default().fg(temp_color(snap.max_temp))),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ─────────────────────────────────────────────────────────────────────────────
// Full-screen Thermal tab
// ─────────────────────────────────────────────────────────────────────────────

/// Render the full Thermal tab (Tab 5).
pub fn render_thermal_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;

    if state.thermal.is_none() {
        render_no_data(frame, area, state);
        return;
    }

    let snap = state.thermal.as_ref().unwrap();

    // Layout: left column (sensors) | right column (sparkline + info)
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // ── Left column: sensor readings ─────────────────────────────
    let left_sections = {
        let mut constraints = Vec::new();
        // CPU section
        let cpu_rows = 1 + snap.cpu_package.is_some() as u16 + snap.cpu_cores.len() as u16;
        constraints.push(Constraint::Length((cpu_rows + 2).min(14)));
        // GPU section
        let gpu_rows = snap.gpu_temp.is_some() as u16 + snap.gpu_hotspot.is_some() as u16;
        if gpu_rows > 0 {
            constraints.push(Constraint::Length(gpu_rows + 3));
        }
        // Storage section
        if !snap.ssd_temps.is_empty() {
            constraints.push(Constraint::Length(snap.ssd_temps.len() as u16 + 3));
        }
        // Board section
        if !snap.motherboard_temps.is_empty() {
            constraints.push(Constraint::Length(snap.motherboard_temps.len() as u16 + 3));
        }
        // Fans section
        if !snap.fan_rpms.is_empty() {
            constraints.push(Constraint::Length(snap.fan_rpms.len() as u16 + 3));
        }
        // Fill remaining space
        constraints.push(Constraint::Min(0));

        Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(columns[0])
    };

    let bar_width = (columns[0].width.saturating_sub(38) as usize)
        .max(10)
        .min(30);
    let mut section_idx = 0;

    // CPU section
    {
        let block = Block::default()
            .title(Span::styled(
                format!(
                    " CPU Temperatures ({}) ",
                    snap.cpu_package
                        .map(|p| format!("{:.1}°C", p))
                        .unwrap_or_else(|| "--".to_string())
                ),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(t.border_style());
        let inner = block.inner(left_sections[section_idx]);
        frame.render_widget(block, left_sections[section_idx]);
        section_idx += 1;

        let mut lines = Vec::new();
        if let Some(pkg) = snap.cpu_package {
            lines.push(make_temp_line("  Package", pkg, bar_width, t));
        }
        for core in &snap.cpu_cores {
            let label = format!("  {}", core.name);
            lines.push(make_temp_line(&label, core.value, bar_width, t));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // GPU section
    if snap.gpu_temp.is_some() || snap.gpu_hotspot.is_some() {
        let gpu_title = snap
            .gpu_temp
            .map(|t| format!(" GPU Temperatures ({:.1}°C) ", t))
            .unwrap_or_else(|| " GPU Temperatures ".to_string());
        let block = Block::default()
            .title(Span::styled(
                gpu_title,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(t.border_style());
        let inner = block.inner(left_sections[section_idx]);
        frame.render_widget(block, left_sections[section_idx]);
        section_idx += 1;

        let mut lines = Vec::new();
        if let Some(temp) = snap.gpu_temp {
            lines.push(make_temp_line("  Core", temp, bar_width, t));
        }
        if let Some(temp) = snap.gpu_hotspot {
            lines.push(make_temp_line("  Hot Spot", temp, bar_width, t));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // Storage section
    if !snap.ssd_temps.is_empty() {
        let block = Block::default()
            .title(Span::styled(
                " Storage Temperatures ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(t.border_style());
        let inner = block.inner(left_sections[section_idx]);
        frame.render_widget(block, left_sections[section_idx]);
        section_idx += 1;

        let lines: Vec<Line> = snap
            .ssd_temps
            .iter()
            .map(|s| make_temp_line(&format!("  {}", s.name), s.value, bar_width, t))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // Motherboard section
    if !snap.motherboard_temps.is_empty() {
        let block = Block::default()
            .title(Span::styled(
                " Motherboard Temperatures ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(t.border_style());
        let inner = block.inner(left_sections[section_idx]);
        frame.render_widget(block, left_sections[section_idx]);
        section_idx += 1;

        let lines: Vec<Line> = snap
            .motherboard_temps
            .iter()
            .map(|s| make_temp_line(&format!("  {}", s.name), s.value, bar_width, t))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // Fan section
    if !snap.fan_rpms.is_empty() {
        let block = Block::default()
            .title(Span::styled(
                " Fan Speeds ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(t.border_style());
        let inner = block.inner(left_sections[section_idx]);
        frame.render_widget(block, left_sections[section_idx]);
        #[allow(unused_assignments)]
        {
            section_idx += 1;
        }

        let lines: Vec<Line> = snap
            .fan_rpms
            .iter()
            .map(|s| make_fan_line(&s.name, s.value, t))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // ── Right column: sparkline + config + shutdown status ────────
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),  // Temperature sparkline
            Constraint::Length(8),  // Max temps summary
            Constraint::Length(10), // Thresholds & config
            Constraint::Min(3),     // Shutdown status
        ])
        .split(columns[1]);

    // Temperature sparkline
    render_temp_sparkline(frame, right_chunks[0], state);

    // Max temps summary card
    render_max_temps(frame, right_chunks[1], state);

    // Thresholds / config
    render_thresholds(frame, right_chunks[2], state);

    // Shutdown status
    render_shutdown_status(frame, right_chunks[3], state);
}

/// Render when no thermal data is available.
fn render_no_data(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(" Thermal Guardian ", t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  No thermal data available",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  LibreHardwareMonitor is not reachable or not running.",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Setup instructions:",
            Style::default().fg(t.accent),
        )),
        Line::from(Span::styled(
            "  1. Install LibreHardwareMonitor on Windows",
            Style::default().fg(t.text_dim),
        )),
        Line::from(Span::styled(
            "  2. Options -> Web Server -> Enable",
            Style::default().fg(t.text_dim),
        )),
        Line::from(Span::styled(
            "  3. Default URL: http://localhost:8085/data.json",
            Style::default().fg(t.text_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Configure in ~/.config/sentinel/config.toml:",
            Style::default().fg(t.accent),
        )),
        Line::from(Span::styled(
            "  [thermal]",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "  lhm_url = \"http://localhost:8085/data.json\"",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "  poll_interval_secs = 5",
            Style::default().fg(t.text_muted),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render a temperature history sparkline.
fn render_temp_sparkline(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;

    let current = state.temp_history.back().copied().unwrap_or(0.0);
    let peak = state.temp_history.iter().copied().fold(0.0f32, f32::max);
    let title = format!(
        " Temperature History (now: {:.1}°C, peak: {:.1}°C) ",
        current, peak
    );

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(temp_color(current))
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.temp_history.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "  Waiting for data...",
            Style::default().fg(t.text_dim),
        )));
        frame.render_widget(msg, inner);
        return;
    }

    // Convert f32 temps to u64 for sparkline (scale by 10 for precision)
    let data: Vec<u64> = state
        .temp_history
        .iter()
        .copied()
        .map(|t| (t * 10.0) as u64)
        .collect();

    let spark = Sparkline::default()
        .data(&data)
        .max(1200) // 120°C * 10
        .style(Style::default().fg(temp_color(current)))
        .bar_set(symbols::bar::NINE_LEVELS);

    frame.render_widget(spark, inner);
}

/// Render max temperature summary card.
fn render_max_temps(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(" Peak Temperatures ", t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let snap = match &state.thermal {
        Some(s) => s,
        None => return,
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Overall Max:  ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.1}°C", snap.max_temp),
                Style::default()
                    .fg(temp_color(snap.max_temp))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  CPU Max:      ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.1}°C", snap.max_cpu_temp),
                Style::default().fg(temp_color(snap.max_cpu_temp)),
            ),
        ]),
        Line::from(vec![
            Span::styled("  GPU Max:      ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.1}°C", snap.max_gpu_temp),
                Style::default().fg(temp_color(snap.max_gpu_temp)),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Sensors:      ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!(
                    "{} CPU, {} GPU, {} storage, {} fans",
                    snap.cpu_cores.len() + snap.cpu_package.is_some() as usize,
                    snap.gpu_temp.is_some() as usize + snap.gpu_hotspot.is_some() as usize,
                    snap.ssd_temps.len(),
                    snap.fan_rpms.len(),
                ),
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled("  History:      ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{} samples", state.temp_history.len()),
                Style::default().fg(t.text_primary),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render alert thresholds and config info.
fn render_thresholds(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(" Alert Thresholds ", t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("  Warning:    ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.0}°C", crate::constants::DEFAULT_THERMAL_WARNING_C),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("  (alert generated)", Style::default().fg(t.text_muted)),
        ]),
        Line::from(vec![
            Span::styled("  Critical:   ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.0}°C", crate::constants::DEFAULT_THERMAL_CRITICAL_C),
                Style::default().fg(Color::Rgb(255, 140, 0)),
            ),
            Span::styled(
                "  (email sent if configured)",
                Style::default().fg(t.text_muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Emergency:  ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.0}°C", crate::constants::DEFAULT_THERMAL_EMERGENCY_C),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (shutdown if enabled)", Style::default().fg(t.text_muted)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Configure in [thermal] section of config.toml",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "  Email: set SMTP credentials in .env file",
            Style::default().fg(t.text_muted),
        )),
        Line::from(Span::styled(
            "  Run ':thermal' for live status, ':email-test' to test",
            Style::default().fg(t.text_muted),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render shutdown state machine status.
fn render_shutdown_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let sm = &state.shutdown_manager;

    let (status_text, status_color) = if sm.is_enabled() {
        if sm.state.is_active() {
            (
                format!("ACTIVE — {} (Ctrl+X to abort)", sm.state.label()),
                t.danger,
            )
        } else {
            ("Armed — monitoring temperatures".to_string(), t.warning)
        }
    } else {
        (
            "Disabled (double-gate: config + .env)".to_string(),
            t.text_dim,
        )
    };

    let block = Block::default()
        .title(Span::styled(
            " Auto-Shutdown ",
            Style::default()
                .fg(if sm.is_enabled() {
                    t.warning
                } else {
                    t.text_dim
                })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(if sm.state.is_active() {
            Style::default().fg(t.danger)
        } else {
            t.border_style()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![Line::from(vec![
        Span::styled("  Status: ", Style::default().fg(t.text_dim)),
        Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    if let Some(remaining) = sm.state.seconds_remaining() {
        lines.push(Line::from(vec![
            Span::styled("  Countdown: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}s remaining", remaining),
                Style::default().fg(t.danger).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Section header line (e.g. "CPU ", "GPU ").
fn section_header<'a>(label: &str, t: &crate::ui::theme::Theme) -> Line<'a> {
    Line::from(vec![Span::styled(
        label.to_string(),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )])
}

/// Create a temperature line with a color-coded value and visual bar.
fn make_temp_line<'a>(
    label: &str,
    temp: f32,
    bar_width: usize,
    t: &crate::ui::theme::Theme,
) -> Line<'a> {
    let color = temp_color(temp);
    let bar = temp_bar(temp, bar_width);
    let flashing = temp >= 95.0;

    let mut style = Style::default().fg(color);
    if flashing {
        style = style.add_modifier(Modifier::SLOW_BLINK | Modifier::BOLD);
    }

    Line::from(vec![
        Span::styled(
            format!("{:.<22}", format!("{} ", label)),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(format!("{:5.1}°C ", temp), style),
        Span::styled(bar, Style::default().fg(color)),
    ])
}

/// Fan RPM line.
fn make_fan_line<'a>(name: &str, value: f32, t: &crate::ui::theme::Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {:.<22}", format!("{} ", name)),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            format!("{:.0} RPM", value),
            Style::default().fg(if value > 0.0 { t.success } else { t.text_dim }),
        ),
    ])
}

/// Color gradient for temperature: green -> yellow -> orange -> red.
fn temp_color(temp: f32) -> Color {
    if temp >= 95.0 {
        Color::Red
    } else if temp >= 85.0 {
        Color::Rgb(255, 140, 0) // Orange
    } else if temp >= 70.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Visual temperature bar using block characters.
fn temp_bar(temp: f32, max_width: usize) -> String {
    // Scale: 0-120°C mapped to 0-max_width
    let fill = ((temp / 120.0) * max_width as f32).round() as usize;
    let fill = fill.min(max_width);
    let empty = max_width - fill;
    format!("{}{}", "█".repeat(fill), "░".repeat(empty))
}
