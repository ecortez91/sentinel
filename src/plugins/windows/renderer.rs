//! Windows host monitoring tab renderer (#4).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
    Frame,
};

use super::models::{WindowsDiskInfo, WindowsHostSnapshot, WindowsProcessInfo};
use super::state::{WindowsPanel, WindowsSortField, WindowsState};
use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Top-level render function for the Windows tab.
pub fn render_windows(
    frame: &mut Frame,
    area: Rect,
    state: &WindowsState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Windows Host ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 || inner.width < 30 {
        return;
    }

    match &state.snapshot {
        None if state.loading => {
            render_loading(frame, inner, theme);
        }
        None => {
            render_disconnected(frame, inner, state, theme);
        }
        Some(snapshot) => {
            render_dashboard(frame, inner, snapshot, state, theme, glyphs);
        }
    }
}

fn render_loading(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Connecting to sentinel-agent...",
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC),
        )),
        area,
    );
}

fn render_disconnected(frame: &mut Frame, area: Rect, state: &WindowsState, theme: &Theme) {
    let mut lines = vec![
        Line::from(Span::styled(
            " Agent not connected",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
    ];

    if let Some(ref err) = state.error {
        lines.push(Line::from(Span::styled(
            format!(" Error: {}", err),
            Style::default().fg(theme.danger),
        )));
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled(
        " To monitor your Windows host:",
        Style::default().fg(theme.text_dim),
    )));
    lines.push(Line::from(Span::styled(
        " 1. Run sentinel-agent.exe on Windows",
        Style::default().fg(theme.text_primary),
    )));
    lines.push(Line::from(Span::styled(
        " 2. Configure agent URL in Settings > General",
        Style::default().fg(theme.text_primary),
    )));

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    state: &WindowsState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    // Focus/expand mode: render only the focused panel full-screen
    if let Some(panel) = state.focused_panel {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        match panel {
            WindowsPanel::SystemOverview => {
                render_system_overview(frame, chunks[0], snapshot, theme, glyphs);
            }
            WindowsPanel::ProcessList => {
                render_process_list(frame, chunks[0], snapshot, state, theme);
            }
            WindowsPanel::Disks => {
                render_disks(frame, chunks[0], &snapshot.disks, theme);
            }
            // Panels that will be implemented in later phases — show placeholder
            _ => {
                let msg = Paragraph::new(Span::styled(
                    format!(" {:?} panel (coming soon)", panel),
                    Style::default().fg(theme.text_muted),
                ));
                frame.render_widget(msg, chunks[0]);
            }
        }

        // Hint bar at bottom
        let hint = Paragraph::new(Line::from(vec![
            Span::styled(
                " f ",
                Style::default()
                    .fg(theme.bg_dark)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" unfocus  ", Style::default().fg(theme.text_dim)),
            Span::styled(
                " F ",
                Style::default()
                    .fg(theme.bg_dark)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" next panel ", Style::default().fg(theme.text_dim)),
        ]));
        frame.render_widget(hint, chunks[1]);
        return;
    }

    // Normal dashboard layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header info bar
            Constraint::Length(5), // system overview (CPU + RAM gauges)
            Constraint::Min(6),    // process list + disk/GPU sidebar
        ])
        .split(area);

    render_header(frame, chunks[0], snapshot, state, theme);
    render_system_overview(frame, chunks[1], snapshot, theme, glyphs);

    // Bottom: process list | disk + GPU info
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[2]);

    render_process_list(frame, bottom[0], snapshot, state, theme);
    render_sidebar(frame, bottom[1], snapshot, theme, glyphs);
}

fn render_header(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    state: &WindowsState,
    theme: &Theme,
) {
    let uptime_str = format_uptime(snapshot.uptime_secs);
    let updated = state
        .last_updated
        .map(|t| {
            let ago = t.elapsed().as_secs();
            if ago < 60 {
                format!("{}s ago", ago)
            } else {
                format!("{}m ago", ago / 60)
            }
        })
        .unwrap_or_else(|| "...".to_string());

    let spans = vec![
        Span::styled(
            format!(" {} ", snapshot.hostname),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({}) ", snapshot.os_version),
            Style::default().fg(theme.text_dim),
        ),
        Span::styled(
            format!("| Up: {} ", uptime_str),
            Style::default().fg(theme.text_muted),
        ),
        Span::styled(
            format!("| Updated: {}", updated),
            Style::default().fg(theme.text_muted),
        ),
    ];

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_system_overview(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
    _glyphs: &Glyphs,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // CPU gauge
            Constraint::Length(2), // RAM gauge
        ])
        .split(area);

    // CPU gauge
    let cpu_color = usage_color(snapshot.cpu_usage_pct, theme);
    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE).title(Span::styled(
            format!(
                " CPU: {:.1}% ({} cores)",
                snapshot.cpu_usage_pct, snapshot.cpu_cores
            ),
            Style::default().fg(theme.text_primary),
        )))
        .gauge_style(Style::default().fg(cpu_color).bg(theme.bg_dark))
        .ratio((snapshot.cpu_usage_pct as f64 / 100.0).clamp(0.0, 1.0));
    frame.render_widget(cpu_gauge, chunks[0]);

    // RAM gauge
    let mem_pct = snapshot.memory_usage_pct();
    let mem_color = usage_color(mem_pct, theme);
    let total_gb = snapshot.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let used_gb = snapshot.used_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let ram_gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE).title(Span::styled(
            format!(" RAM: {:.1}/{:.1} GB ({:.0}%)", used_gb, total_gb, mem_pct),
            Style::default().fg(theme.text_primary),
        )))
        .gauge_style(Style::default().fg(mem_color).bg(theme.bg_dark))
        .ratio((mem_pct as f64 / 100.0).clamp(0.0, 1.0));
    frame.render_widget(ram_gauge, chunks[1]);
}

/// Sort indicator arrow for column headers.
fn sort_arrow(ascending: bool) -> &'static str {
    if ascending {
        "▲"
    } else {
        "▼"
    }
}

/// Build a column header label, appending a sort arrow if this column is active.
fn column_label(
    label: &str,
    field: WindowsSortField,
    active: WindowsSortField,
    ascending: bool,
) -> String {
    if field == active {
        format!("{}{}", label, sort_arrow(ascending))
    } else {
        label.to_string()
    }
}

/// Sort processes according to the current sort field and direction.
fn sort_processes(
    procs: &[WindowsProcessInfo],
    field: WindowsSortField,
    ascending: bool,
) -> Vec<WindowsProcessInfo> {
    let mut sorted = procs.to_vec();
    sorted.sort_by(|a, b| {
        let cmp = match field {
            WindowsSortField::Cpu => a
                .cpu_pct
                .partial_cmp(&b.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal),
            WindowsSortField::Memory => a.memory_bytes.cmp(&b.memory_bytes),
            WindowsSortField::Pid => a.pid.cmp(&b.pid),
            WindowsSortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
    sorted
}

fn render_process_list(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    state: &WindowsState,
    theme: &Theme,
) {
    let arrow = sort_arrow(state.sort_ascending);
    let title = format!(
        " Processes ({}) [s:sort by {} {}] ",
        snapshot.top_processes.len(),
        state.sort_field.label(),
        arrow,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    // Build column headers with sort indicator on active column
    let pid_hdr = column_label(
        "PID",
        WindowsSortField::Pid,
        state.sort_field,
        state.sort_ascending,
    );
    let name_hdr = column_label(
        "Name",
        WindowsSortField::Name,
        state.sort_field,
        state.sort_ascending,
    );
    let cpu_hdr = column_label(
        "CPU %",
        WindowsSortField::Cpu,
        state.sort_field,
        state.sort_ascending,
    );
    let mem_hdr = column_label(
        "Memory",
        WindowsSortField::Memory,
        state.sort_field,
        state.sort_ascending,
    );

    let header = Line::from(Span::styled(
        format!(
            " {:<7} {:<25} {:>8} {:>10}",
            pid_hdr, name_hdr, cpu_hdr, mem_hdr
        ),
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ));

    // Sort processes
    let sorted = sort_processes(
        &snapshot.top_processes,
        state.sort_field,
        state.sort_ascending,
    );

    let max_rows = (inner.height as usize).saturating_sub(1);
    let mut lines = vec![header];

    for (i, proc) in sorted
        .iter()
        .enumerate()
        .skip(state.scroll_offset)
        .take(max_rows)
    {
        let is_selected = i == state.selected_process;
        let mem_str = format_bytes(proc.memory_bytes);

        let style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.text_primary)
        } else {
            Style::default().fg(theme.text_primary)
        };

        lines.push(Line::from(Span::styled(
            format!(
                " {:<7} {:<25} {:>7.1}% {:>10}",
                proc.pid,
                truncate_str(&proc.name, 25),
                proc.cpu_pct,
                mem_str,
            ),
            style,
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
    _glyphs: &Glyphs,
) {
    let has_gpu = snapshot.gpu.is_some();
    let chunks = if has_gpu {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    // disks
                Constraint::Length(6), // GPU
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4)])
            .split(area)
    };

    // Disk info
    render_disks(frame, chunks[0], &snapshot.disks, theme);

    // GPU info
    if has_gpu {
        render_gpu(frame, chunks[1], snapshot.gpu.as_ref().unwrap(), theme);
    }
}

fn render_disks(frame: &mut Frame, area: Rect, disks: &[WindowsDiskInfo], theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Disks ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if disks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No disk data",
                Style::default().fg(theme.text_muted),
            )),
            inner,
        );
        return;
    }

    let mut lines = Vec::new();
    for disk in disks {
        let pct = disk.usage_pct();
        let total_gb = disk.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let used_gb = disk.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let color = usage_color(pct, theme);

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", disk.mount),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:.0}/{:.0} GB ({:.0}%)", used_gb, total_gb, pct),
                Style::default().fg(color),
            ),
            Span::styled(
                format!(" [{}]", disk.fs_type),
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_gpu(frame: &mut Frame, area: Rect, gpu: &super::models::WindowsGpuInfo, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " GPU ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let vram_total_gb = gpu.vram_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let vram_used_gb = gpu.vram_used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    let lines = vec![
        Line::from(Span::styled(
            format!(" {}", gpu.name),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" Load: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("{:.0}%", gpu.usage_pct),
                Style::default().fg(usage_color(gpu.usage_pct, theme)),
            ),
            Span::styled("  Temp: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("{:.0}C", gpu.temp_celsius),
                Style::default().fg(temp_color(gpu.temp_celsius, theme)),
            ),
        ]),
        Line::from(vec![
            Span::styled(" VRAM: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("{:.1}/{:.1} GB", vram_used_gb, vram_total_gb),
                Style::default().fg(theme.text_primary),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Helpers ──────────────────────────────────────────────────────

fn usage_color(pct: f32, theme: &Theme) -> Color {
    if pct >= 90.0 {
        theme.danger
    } else if pct >= 70.0 {
        theme.warning
    } else {
        theme.success
    }
}

fn temp_color(temp: f32, theme: &Theme) -> Color {
    if temp >= 90.0 {
        theme.danger
    } else if temp >= 75.0 {
        theme.warning
    } else {
        theme.success
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_days() {
        assert_eq!(format_uptime(90061), "1d 1h 1m");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(3660), "1h 1m");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(format_uptime(120), "2m");
    }

    #[test]
    fn format_bytes_gb() {
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn format_bytes_mb() {
        assert_eq!(format_bytes(512 * 1024 * 1024), "512.0 MB");
    }

    #[test]
    fn truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long() {
        let result = truncate_str("very long process name here", 15);
        assert!(result.len() <= 15);
        assert!(result.ends_with("..."));
    }
}
