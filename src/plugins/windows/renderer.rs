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
use crate::constants::STANDARD_PORTS;
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
            WindowsPanel::Security => {
                render_security_status(frame, chunks[0], snapshot, theme);
            }
            WindowsPanel::Connections => {
                render_connections(frame, chunks[0], snapshot, theme);
            }
            WindowsPanel::Network => {
                render_network_info(frame, chunks[0], snapshot, theme);
            }
            WindowsPanel::StartupPrograms => {
                render_startup_programs(frame, chunks[0], snapshot, theme);
            }
            WindowsPanel::AiAnalysis => {
                render_ai_panel(frame, chunks[0], state, theme);
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
    let has_security = snapshot.security.is_some();
    let has_connections =
        !snapshot.tcp_connections.is_empty() || !snapshot.listening_ports.is_empty();
    let has_startup = !snapshot.startup_programs.is_empty();
    let has_ai = state.ai_analysis.is_some() || state.ai_loading;

    let security_height: u16 = if has_security { 3 } else { 0 };
    let connections_height: u16 = if has_connections { 6 } else { 0 };
    let startup_height: u16 = if has_startup { 4 } else { 0 };
    let ai_height: u16 = if has_ai { 6 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                  // 0: header info bar
            Constraint::Length(security_height),    // 1: security status
            Constraint::Length(5),                  // 2: system overview (CPU + RAM)
            Constraint::Min(6),                     // 3: process list + sidebar
            Constraint::Length(connections_height), // 4: connections
            Constraint::Length(startup_height),     // 5: startup programs
            Constraint::Length(ai_height),          // 6: AI analysis
        ])
        .split(area);

    render_header(frame, chunks[0], snapshot, state, theme);
    if has_security {
        render_security_status(frame, chunks[1], snapshot, theme);
    }
    render_system_overview(frame, chunks[2], snapshot, theme, glyphs);

    // Middle: process list | sidebar (disks + GPU + network)
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[3]);

    render_process_list(frame, middle[0], snapshot, state, theme);
    render_sidebar(frame, middle[1], snapshot, theme, glyphs);

    if has_connections {
        render_connections(frame, chunks[4], snapshot, theme);
    }
    if has_startup {
        render_startup_programs(frame, chunks[5], snapshot, theme);
    }
    if has_ai {
        render_ai_panel(frame, chunks[6], state, theme);
    }
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
    let has_networks = !snapshot.networks.is_empty();

    let mut constraints: Vec<Constraint> = vec![Constraint::Min(4)]; // disks always
    if has_gpu {
        constraints.push(Constraint::Length(6));
    }
    if has_networks {
        constraints.push(Constraint::Length(4));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;

    // Disk info
    render_disks(frame, chunks[idx], &snapshot.disks, theme);
    idx += 1;

    // GPU info
    if has_gpu {
        render_gpu(frame, chunks[idx], snapshot.gpu.as_ref().unwrap(), theme);
        idx += 1;
    }

    // Network interfaces
    if has_networks {
        render_network_info(frame, chunks[idx], snapshot, theme);
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

// ── New panels for expanded agent data ───────────────────────────

/// Security status panel — firewall, defender, updates, users in one compact row.
fn render_security_status(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Security ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(ref sec) = snapshot.security else {
        return;
    };

    let mut spans: Vec<Span> = vec![Span::styled(" ", Style::default())];

    // Firewall per profile
    spans.push(Span::styled(
        "Firewall: ",
        Style::default().fg(theme.text_dim),
    ));
    for (i, profile) in sec.firewall_profiles.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("/", Style::default().fg(theme.text_muted)));
        }
        let (label, color) = if profile.enabled {
            ("ON", theme.success)
        } else {
            ("OFF", theme.danger)
        };
        spans.push(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::styled(" | ", Style::default().fg(theme.text_muted)));

    // Defender
    spans.push(Span::styled(
        "Defender: ",
        Style::default().fg(theme.text_dim),
    ));
    match sec.defender_enabled {
        Some(true) => {
            spans.push(Span::styled(
                "ON",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        Some(false) => {
            spans.push(Span::styled(
                "OFF",
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        None => {
            spans.push(Span::styled("N/A", Style::default().fg(theme.text_muted)));
        }
    }

    // Real-time protection
    if let Some(rt) = sec.defender_realtime {
        spans.push(Span::styled(
            if rt { " RT:ON" } else { " RT:OFF" },
            Style::default().fg(if rt { theme.success } else { theme.danger }),
        ));
    }

    spans.push(Span::styled(" | ", Style::default().fg(theme.text_muted)));

    // Windows Update age
    spans.push(Span::styled(
        "Updates: ",
        Style::default().fg(theme.text_dim),
    ));
    match sec.last_update_days {
        Some(days) => {
            let color = if days > 30 {
                theme.danger
            } else if days > 14 {
                theme.warning
            } else {
                theme.success
            };
            spans.push(Span::styled(
                format!("{}d ago", days),
                Style::default().fg(color),
            ));
        }
        None => {
            spans.push(Span::styled("N/A", Style::default().fg(theme.text_muted)));
        }
    }

    // Logged-in users summary
    if !snapshot.logged_in_users.is_empty() {
        spans.push(Span::styled(" | ", Style::default().fg(theme.text_muted)));
        let rdp_count = snapshot
            .logged_in_users
            .iter()
            .filter(|u| u.session_type == "RDP")
            .count();
        let user_label = if rdp_count > 0 {
            format!(
                "Users:{} ({}xRDP)",
                snapshot.logged_in_users.len(),
                rdp_count
            )
        } else {
            format!("Users:{}", snapshot.logged_in_users.len())
        };
        let user_color = if rdp_count > 0 {
            theme.warning
        } else {
            theme.text_primary
        };
        spans.push(Span::styled(user_label, Style::default().fg(user_color)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

/// Connections panel — active TCP connections + listening ports.
fn render_connections(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    // Left: active connections
    render_active_connections(frame, halves[0], snapshot, theme);
    // Right: listening ports
    render_listening_ports(frame, halves[1], snapshot, theme);
}

/// Whether a TCP connection looks suspicious (non-standard port, unknown process).
fn is_suspicious_port(port: u16) -> bool {
    !STANDARD_PORTS.contains(&port)
}

fn render_active_connections(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let suspicious_count = snapshot
        .tcp_connections
        .iter()
        .filter(|c| c.state == "ESTABLISHED" && is_suspicious_port(c.remote_port))
        .count();

    let title = if suspicious_count > 0 {
        format!(
            " Connections ({}, {} suspicious) ",
            snapshot.tcp_connections.len(),
            suspicious_count
        )
    } else {
        format!(" Connections ({}) ", snapshot.tcp_connections.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if suspicious_count > 0 {
            theme.warning
        } else {
            theme.border
        }))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snapshot.tcp_connections.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No active connections",
                Style::default().fg(theme.text_muted),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = snapshot
        .tcp_connections
        .iter()
        .take(inner.height as usize)
        .map(|c| {
            let suspicious = c.state == "ESTABLISHED" && is_suspicious_port(c.remote_port);
            let color = if suspicious {
                theme.warning
            } else {
                theme.text_primary
            };
            let indicator = if suspicious { "!" } else { " " };
            Line::from(Span::styled(
                format!(
                    "{}{}:{} -> {}:{} [{}] {}",
                    indicator,
                    truncate_str(&c.local_addr, 15),
                    c.local_port,
                    truncate_str(&c.remote_addr, 15),
                    c.remote_port,
                    truncate_str(&c.state, 6),
                    truncate_str(&c.process_name, 15),
                ),
                Style::default().fg(color),
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_listening_ports(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            format!(" Listening ({}) ", snapshot.listening_ports.len()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snapshot.listening_ports.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No listening ports",
                Style::default().fg(theme.text_muted),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = snapshot
        .listening_ports
        .iter()
        .take(inner.height as usize)
        .map(|p| {
            Line::from(Span::styled(
                format!(
                    " {:>5} {:>4} {} ({})",
                    p.port,
                    p.protocol,
                    truncate_str(&p.process_name, 20),
                    p.pid,
                ),
                Style::default().fg(theme.text_primary),
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Network interfaces panel.
fn render_network_info(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Network ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snapshot.networks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No network data",
                Style::default().fg(theme.text_muted),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = snapshot
        .networks
        .iter()
        .take(inner.height as usize)
        .map(|n| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<14}", truncate_str(&n.name, 14)),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled("RX:", Style::default().fg(theme.text_dim)),
                Span::styled(
                    format!("{} ", format_bytes(n.rx_bytes)),
                    Style::default().fg(theme.success),
                ),
                Span::styled("TX:", Style::default().fg(theme.text_dim)),
                Span::styled(format_bytes(n.tx_bytes), Style::default().fg(theme.warning)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// AI security analysis panel.
fn render_ai_panel(frame: &mut Frame, area: Rect, state: &WindowsState, theme: &Theme) {
    let is_focused = state.focused_panel == Some(WindowsPanel::AiAnalysis);

    if let Some(ref analysis) = state.ai_analysis {
        let total_lines = analysis.lines().count();
        let title = if state.ai_loading {
            " AI Security Analysis (streaming...) ".to_string()
        } else if is_focused {
            format!(
                " AI Security Analysis [j/k:scroll a:refresh] ({}/{}) ",
                state.ai_scroll + 1,
                total_lines
            )
        } else {
            " AI Security Analysis [f:expand a:refresh] ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if is_focused {
                theme.accent
            } else {
                theme.border
            }))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = analysis
            .lines()
            .skip(state.ai_scroll)
            .take(inner.height as usize)
            .map(|line| {
                Line::from(Span::styled(
                    format!(" {}", line),
                    Style::default().fg(theme.text_primary),
                ))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    } else {
        let title = if state.ai_loading {
            " AI Security Analysis (streaming...) "
        } else {
            " AI Security Analysis [a: analyze] "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if state.ai_loading {
                theme.accent
            } else {
                theme.border
            }))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let msg = if state.ai_loading {
            " Analyzing Windows host security..."
        } else {
            " Press 'a' to analyze host security with AI"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                msg,
                Style::default().fg(if state.ai_loading {
                    theme.accent
                } else {
                    theme.text_muted
                }),
            )),
            inner,
        );
    }
}

/// Startup programs panel.
fn render_startup_programs(
    frame: &mut Frame,
    area: Rect,
    snapshot: &WindowsHostSnapshot,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            format!(" Startup Programs ({}) ", snapshot.startup_programs.len()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snapshot.startup_programs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No startup programs detected",
                Style::default().fg(theme.text_muted),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = snapshot
        .startup_programs
        .iter()
        .take(inner.height as usize)
        .map(|s| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<20} ", truncate_str(&s.name, 20)),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled(
                    truncate_str(&s.command, 40),
                    Style::default().fg(theme.text_dim),
                ),
                Span::styled(
                    format!("  [{}]", truncate_str(&s.location, 15)),
                    Style::default().fg(theme.text_muted),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
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
