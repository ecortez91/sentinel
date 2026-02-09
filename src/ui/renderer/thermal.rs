//! Thermal monitoring panel for the dashboard.
//!
//! Renders color-coded temperature bars, fan RPMs, SSD temps, and
//! motherboard temps from the LibreHardwareMonitor snapshot.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ui::state::AppState;

/// Render the thermal monitoring panel.
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
        lines.push(Line::from(vec![Span::styled(
            "CPU ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )]));

        if let Some(pkg) = snap.cpu_package {
            lines.push(make_temp_line("  Package", pkg, t));
        }

        for core in &snap.cpu_cores {
            let label = format!("  {}", core.name);
            lines.push(make_temp_line(&label, core.value, t));
        }
    }

    // GPU temperatures
    if snap.gpu_temp.is_some() || snap.gpu_hotspot.is_some() {
        lines.push(Line::from(vec![Span::styled(
            "GPU ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )]));

        if let Some(temp) = snap.gpu_temp {
            lines.push(make_temp_line("  Core", temp, t));
        }
        if let Some(temp) = snap.gpu_hotspot {
            lines.push(make_temp_line("  Hot Spot", temp, t));
        }
    }

    // SSD temperatures
    if !snap.ssd_temps.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Storage ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )]));
        for s in snap.ssd_temps.iter().take(4) {
            let label = format!("  {}", s.name);
            lines.push(make_temp_line(&label, s.value, t));
        }
    }

    // Motherboard temperatures
    if !snap.motherboard_temps.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Board ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )]));
        for s in snap.motherboard_temps.iter().take(4) {
            let label = format!("  {}", s.name);
            lines.push(make_temp_line(&label, s.value, t));
        }
    }

    // Fan RPMs
    if !snap.fan_rpms.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Fans ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )]));
        for s in snap.fan_rpms.iter().take(4) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:.<22}", format!("{} ", s.name)),
                    Style::default().fg(t.text_dim),
                ),
                Span::styled(
                    format!("{:.0} RPM", s.value),
                    Style::default().fg(if s.value > 0.0 { t.success } else { t.text_dim }),
                ),
            ]));
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

/// Create a temperature line with a color-coded value and visual bar.
fn make_temp_line<'a>(label: &str, temp: f32, t: &crate::ui::theme::Theme) -> Line<'a> {
    let color = temp_color(temp);
    let bar = temp_bar(temp, 20);
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
