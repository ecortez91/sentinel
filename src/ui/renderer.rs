use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Sparkline, Table, TableState, Wrap,
    },
    Frame,
};

use super::{
    state::{AppState, SortColumn, Tab},
    theme::Theme,
    widgets::{CpuMiniChart, GradientGauge},
};
use crate::ai::MessageRole;
use crate::models::{format_bytes, AlertSeverity};

/// Top-level render function. Delegates to sub-renderers per tab.
pub fn render(frame: &mut Frame, state: &AppState) {
    let size = frame.area();

    // ── Main Layout ────────────────────────────────────────────
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header bar
            Constraint::Min(10),   // Content area
            Constraint::Length(1), // Status bar
        ])
        .split(size);

    render_header(frame, main_chunks[0], state);
    render_status_bar(frame, main_chunks[2], state);

    match state.active_tab {
        Tab::Dashboard => render_dashboard(frame, main_chunks[1], state),
        Tab::Processes => render_processes(frame, main_chunks[1], state),
        Tab::Alerts => render_alerts(frame, main_chunks[1], state),
        Tab::AskAi => render_ask_ai(frame, main_chunks[1], state),
    }

    if state.show_process_detail {
        render_process_detail(frame, size, state);
    }

    if state.show_signal_picker {
        render_signal_picker(frame, size, state);
    }

    if state.show_renice_dialog {
        render_renice_dialog(frame, size, state);
    }

    if state.show_help {
        render_help_overlay(frame, size, state);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Header
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Logo
            Constraint::Min(20),    // Tabs
            Constraint::Length(30), // System summary
        ])
        .split(area);

    // Logo
    let pulse = if state.tick_count % 2 == 0 {
        "●"
    } else {
        "○"
    };
    let logo = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(pulse, Style::default().fg(t.success)),
        Span::styled(t!("app.name").to_string(), t.header_style()),
        Span::styled(
            t!("app.version").to_string(),
            Style::default().fg(t.text_muted),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.border_style()),
    );
    frame.render_widget(logo, chunks[0]);

    // Tabs
    let tabs: Vec<Span> = Tab::all()
        .iter()
        .map(|tab| {
            if *tab == state.active_tab {
                Span::styled(tab.label(), t.tab_active_style())
            } else if *tab == Tab::AskAi {
                Span::styled(tab.label(), Style::default().fg(t.ai_accent))
            } else {
                Span::styled(tab.label(), t.tab_inactive_style())
            }
        })
        .collect();

    let mut tab_spans = vec![Span::raw(" ")];
    for (i, tab) in tabs.into_iter().enumerate() {
        tab_spans.push(tab);
        if i < Tab::all().len() - 1 {
            tab_spans.push(Span::styled(" │ ", Style::default().fg(t.text_muted)));
        }
    }

    // Alert badge
    let danger_count = state.danger_alert_count();
    if danger_count > 0 {
        tab_spans.push(Span::raw("  "));
        tab_spans.push(Span::styled(
            t!("alert.count", count = danger_count).to_string(),
            Style::default()
                .fg(t.bg_dark)
                .bg(t.danger)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // AI loading indicator
    if state.ai_loading {
        tab_spans.push(Span::raw(" "));
        let spinner = match state.tick_count % 4 {
            0 => "◐",
            1 => "◓",
            2 => "◑",
            _ => "◒",
        };
        tab_spans.push(Span::styled(
            format!(" {} AI ", spinner),
            Style::default()
                .fg(t.bg_dark)
                .bg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let tab_line = Paragraph::new(Line::from(tab_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.border_style()),
    );
    frame.render_widget(tab_line, chunks[1]);

    // Quick system summary
    let sys_text = if let Some(sys) = &state.system {
        t!(
            "summary.cpu_ram_procs",
            cpu = format!("{:.0}", sys.global_cpu_usage),
            ram = format!("{:.0}", sys.memory_percent()),
            procs = sys.total_processes
        )
        .to_string()
    } else {
        t!("summary.loading").to_string()
    };
    let sys_summary = Paragraph::new(Line::from(vec![Span::styled(
        sys_text,
        Style::default().fg(t.text_dim),
    )]))
    .alignment(Alignment::Right)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.border_style()),
    );
    frame.render_widget(sys_summary, chunks[2]);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Dashboard Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_dashboard(frame: &mut Frame, area: Rect, state: &AppState) {
    use super::state::FocusedWidget;

    // ── Focus/expand mode: render only the focused widget ──
    if let Some(focused) = state.focused_widget {
        let t = &state.theme;
        // Render a hint bar at the bottom
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        match focused {
            FocusedWidget::SystemGauges => render_system_gauges(frame, chunks[0], state),
            FocusedWidget::CpuCores => render_cpu_cores(frame, chunks[0], state),
            FocusedWidget::Sparklines => render_sparklines(frame, chunks[0], state),
            FocusedWidget::Gpu => render_gpu_panel(frame, chunks[0], state),
            FocusedWidget::Network => render_network_panel(frame, chunks[0], state),
            FocusedWidget::Disk => render_disk_panel(frame, chunks[0], state),
            FocusedWidget::AiInsight => render_ai_insight(frame, chunks[0], state),
            FocusedWidget::TopProcesses => render_top_processes(frame, chunks[0], state),
            FocusedWidget::Alerts => render_recent_alerts(frame, chunks[0], state),
        }

        let hint = Paragraph::new(Line::from(vec![
            Span::styled(
                " f ",
                Style::default()
                    .fg(t.bg_dark)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                t!("focus.unfocus").to_string(),
                Style::default().fg(t.text_dim),
            ),
            Span::styled(
                " F ",
                Style::default()
                    .fg(t.bg_dark)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                t!("focus.next").to_string(),
                Style::default().fg(t.text_dim),
            ),
        ]));
        frame.render_widget(hint, chunks[1]);
        return;
    }

    // ── Normal dashboard layout ──
    let has_insight = state.ai_has_key;
    let has_gpu = state.system.as_ref().and_then(|s| s.gpu.as_ref()).is_some();
    let has_docker = state.docker_available && !state.containers.is_empty();

    let insight_height: u16 = if has_insight {
        if state.ai_insight_expanded {
            20
        } else {
            8
        }
    } else {
        0
    };
    let gpu_height: u16 = if has_gpu { 5 } else { 0 };
    let docker_height: u16 = if has_docker {
        (state.containers.len() as u16 + 3).min(8)
    } else {
        0
    };
    let battery_row: u16 = if state
        .system
        .as_ref()
        .and_then(|s| s.battery.as_ref())
        .is_some()
    {
        1 // extra row in system gauges
    } else {
        0
    };

    let constraints = vec![
        Constraint::Length(8 + battery_row), // System gauges (+ battery row if present)
        Constraint::Length(5),               // CPU per-core (+ temp)
        Constraint::Length(5),               // Sparkline history charts
        Constraint::Length(gpu_height),      // GPU panel (0 if no GPU)
        Constraint::Length(5),               // Network panel
        Constraint::Length(5),               // Disk/filesystem panel
        Constraint::Length(docker_height),   // Docker containers (0 if none)
        Constraint::Length(insight_height),  // AI insight card
        Constraint::Min(8),                  // Top processes
        Constraint::Length(8),               // Recent alerts
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_system_gauges(frame, chunks[0], state);
    render_cpu_cores(frame, chunks[1], state);
    render_sparklines(frame, chunks[2], state);
    if has_gpu {
        render_gpu_panel(frame, chunks[3], state);
    }
    render_network_panel(frame, chunks[4], state);
    render_disk_panel(frame, chunks[5], state);
    if has_docker {
        render_docker_panel(frame, chunks[6], state);
    }
    if has_insight {
        render_ai_insight(frame, chunks[7], state);
    }
    render_top_processes(frame, chunks[8], state);
    render_recent_alerts(frame, chunks[9], state);
}

fn render_system_gauges(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(
            t!("title.system_resources").to_string(),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(sys) = &state.system else { return };

    let has_battery = sys.battery.is_some();
    let mut constraints = vec![
        Constraint::Length(1), // CPU
        Constraint::Length(1), // RAM
        Constraint::Length(1), // SWAP
        Constraint::Length(1), // Load
    ];
    if has_battery {
        constraints.push(Constraint::Length(1)); // Battery
    }

    let gauge_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(inner);

    // CPU gauge — append temp if available
    let cpu_label = if let Some(ref temp) = sys.cpu_temp {
        if let Some(pkg) = temp.package_temp {
            format!("CPU  {:.0}°C  ", pkg)
        } else {
            "CPU  ".to_string()
        }
    } else {
        "CPU  ".to_string()
    };
    let cpu_gauge = GradientGauge::new(sys.global_cpu_usage, &cpu_label, t);
    frame.render_widget(cpu_gauge, gauge_chunks[0]);

    // Memory gauge
    let mem_pct = sys.memory_percent();
    let mem_label = format!(
        "RAM  {} / {}  ",
        format_bytes(sys.used_memory),
        format_bytes(sys.total_memory)
    );
    let mem_label_short = if mem_label.len() > 28 {
        "RAM  ".to_string()
    } else {
        mem_label
    };
    let mem_gauge = GradientGauge::new(mem_pct, &mem_label_short, t);
    frame.render_widget(mem_gauge, gauge_chunks[1]);

    // Swap gauge
    let swap_pct = sys.swap_percent();
    let swap_gauge = GradientGauge::new(swap_pct, "SWAP ", t);
    frame.render_widget(swap_gauge, gauge_chunks[2]);

    // Load average + hostname + uptime
    let mut load_spans = vec![
        Span::styled("LOAD ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("1m: {:.2}", sys.load_avg_1),
            Style::default()
                .fg(t.usage_color((sys.load_avg_1 as f32 / sys.cpu_count as f32) * 100.0)),
        ),
        Span::styled("  5m: ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("{:.2}", sys.load_avg_5),
            Style::default().fg(t.text_primary),
        ),
        Span::styled("  15m: ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("{:.2}", sys.load_avg_15),
            Style::default().fg(t.text_primary),
        ),
        Span::styled("  │  ", Style::default().fg(t.text_muted)),
        Span::styled(
            sys.hostname.clone(),
            Style::default().fg(t.accent_secondary),
        ),
        Span::styled("  │  ", Style::default().fg(t.text_muted)),
        Span::styled(
            format!(
                "uptime {}h {}m",
                sys.uptime / 3600,
                (sys.uptime % 3600) / 60
            ),
            Style::default().fg(t.text_dim),
        ),
    ];

    // Append CPU temp to load line if available
    if let Some(ref temp) = sys.cpu_temp {
        if let Some(pkg) = temp.package_temp {
            load_spans.push(Span::styled("  │  ", Style::default().fg(t.text_muted)));
            load_spans.push(Span::styled(
                format!("{:.0}°C", pkg),
                Style::default().fg(t.temp_color(pkg)),
            ));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(load_spans)), gauge_chunks[3]);

    // Battery gauge (if present)
    if let Some(ref bat) = sys.battery {
        if has_battery {
            let bat_label = match &bat.status {
                crate::models::BatteryStatus::Charging => {
                    if let Some(ref tr) = bat.time_remaining {
                        format!("BAT  ⚡ {}  ", tr)
                    } else {
                        "BAT  ⚡ Charging  ".to_string()
                    }
                }
                crate::models::BatteryStatus::Discharging => {
                    if let Some(ref tr) = bat.time_remaining {
                        format!("BAT  🔋 {}  ", tr)
                    } else {
                        "BAT  🔋 Discharging  ".to_string()
                    }
                }
                crate::models::BatteryStatus::Full => "BAT  Full  ".to_string(),
                _ => "BAT  ".to_string(),
            };
            let bat_gauge = GradientGauge::new(bat.percent, &bat_label, t);
            frame.render_widget(bat_gauge, gauge_chunks[4]);
        }
    }
}

fn render_cpu_cores(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let Some(sys) = &state.system else { return };

    // Build title with temp info if available
    let title = if let Some(ref temp) = sys.cpu_temp {
        if let Some(pkg) = temp.package_temp {
            format!(" CPU Cores ({:.0}°C) ", pkg)
        } else {
            " CPU Cores ".to_string()
        }
    } else {
        " CPU Cores ".to_string()
    };

    let block = Block::default()
        .title(Span::styled(title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content_area = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    if content_area.height >= 1 {
        let bar_area = Rect {
            height: 1,
            ..content_area
        };
        let chart = CpuMiniChart::new(&sys.cpu_usages, t);
        frame.render_widget(chart, bar_area);
    }

    if content_area.height >= 3 {
        let label_area = Rect {
            x: content_area.x,
            y: content_area.y + 2,
            width: content_area.width,
            height: 1,
        };

        let mut spans = Vec::new();
        let cores_to_show = (content_area.width as usize / 8).min(sys.cpu_usages.len());
        for (i, &usage) in sys.cpu_usages.iter().take(cores_to_show).enumerate() {
            let color = t.usage_color(usage);
            spans.push(Span::styled(
                format!("C{:<2}", i),
                Style::default().fg(t.text_muted),
            ));
            spans.push(Span::styled(
                format!("{:>4.0}% ", usage),
                Style::default().fg(color),
            ));
        }
        if sys.cpu_usages.len() > cores_to_show {
            spans.push(Span::styled(
                format!("(+{} more)", sys.cpu_usages.len() - cores_to_show),
                Style::default().fg(t.text_muted),
            ));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), label_area);
    }
}

fn render_sparklines(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let window = state.history_window;
    let points = window.points();
    let title = format!(" History ({}) [+/-: zoom] ", window.label());

    let block = Block::default()
        .title(Span::styled(title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split horizontally: CPU left, RAM right
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Take last N points from history based on window
    let cpu_data: Vec<u64> = state
        .cpu_history
        .iter()
        .copied()
        .rev()
        .take(points)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let cpu_label = if let Some(&last) = cpu_data.last() {
        format!("CPU {}%", last)
    } else {
        "CPU --%".to_string()
    };
    let cpu_spark = Sparkline::default()
        .block(
            Block::default()
                .title(Span::styled(cpu_label, Style::default().fg(t.accent)))
                .borders(Borders::NONE),
        )
        .data(&cpu_data)
        .max(100)
        .style(Style::default().fg(t.accent))
        .bar_set(symbols::bar::NINE_LEVELS);
    frame.render_widget(cpu_spark, halves[0]);

    // RAM sparkline
    let mem_data: Vec<u64> = state
        .mem_history
        .iter()
        .copied()
        .rev()
        .take(points)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mem_label = if let Some(&last) = mem_data.last() {
        format!("RAM {}%", last)
    } else {
        "RAM --%".to_string()
    };
    let mem_spark = Sparkline::default()
        .block(
            Block::default()
                .title(Span::styled(
                    mem_label,
                    Style::default().fg(t.accent_secondary),
                ))
                .borders(Borders::NONE),
        )
        .data(&mem_data)
        .max(100)
        .style(Style::default().fg(t.accent_secondary))
        .bar_set(symbols::bar::NINE_LEVELS);
    frame.render_widget(mem_spark, halves[1]);
}

fn render_gpu_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let Some(sys) = &state.system else { return };
    let Some(gpu) = &sys.gpu else { return };

    let title = format!(
        " GPU: {} ({}°C, {:.0}W) ",
        truncate_str(&gpu.name, 24),
        gpu.temperature,
        gpu.power_draw
    );

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(t.gpu_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split into two rows: GPU util gauge + VRAM gauge, then details
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // GPU utilization gauge
    let gpu_gauge = GradientGauge::new(gpu.utilization as f32, "GPU  ", t);
    frame.render_widget(gpu_gauge, rows[0]);

    // VRAM gauge
    let vram_pct = gpu.memory_percent();
    let vram_label = format!(
        "VRAM {} / {}  ",
        crate::models::format_bytes(gpu.memory_used),
        crate::models::format_bytes(gpu.memory_total),
    );
    let vram_gauge = GradientGauge::new(vram_pct, &vram_label, t);
    frame.render_widget(vram_gauge, rows[1]);

    // Detail line (temp, power, fan)
    if rows[2].height >= 1 {
        let temp_color = t.temp_color(gpu.temperature as f32);
        let mut detail_spans = vec![
            Span::styled(" Temp: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{}°C", gpu.temperature),
                Style::default().fg(temp_color),
            ),
            Span::styled("  Power: ", Style::default().fg(t.text_dim)),
            Span::styled(
                format!("{:.0}W", gpu.power_draw),
                Style::default().fg(t.text_primary),
            ),
        ];
        if let Some(fan) = gpu.fan_speed {
            detail_spans.push(Span::styled("  Fan: ", Style::default().fg(t.text_dim)));
            detail_spans.push(Span::styled(
                format!("{}%", fan),
                Style::default().fg(t.text_primary),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(detail_spans)), rows[2]);
    }
}

fn render_network_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(
            t!("title.network").to_string(),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(sys) = &state.system else { return };

    if sys.networks.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            t!("network.none").to_string(),
            Style::default().fg(t.text_muted),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    // Show interfaces with nonzero totals, sorted by total traffic desc
    let mut nets: Vec<_> = sys
        .networks
        .iter()
        .filter(|n| n.total_rx + n.total_tx > 0)
        .collect();
    nets.sort_by(|a, b| (b.total_rx + b.total_tx).cmp(&(a.total_rx + a.total_tx)));

    let lines: Vec<Line> = nets
        .iter()
        .take(inner.height as usize)
        .map(|n| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<12}", truncate_str(&n.name, 12)),
                    Style::default().fg(t.text_primary),
                ),
                Span::styled("RX: ", Style::default().fg(t.text_dim)),
                Span::styled(
                    format!("{}/s ", format_rate(n.rx_bytes)),
                    Style::default().fg(t.success),
                ),
                Span::styled(
                    format!("({}) ", crate::models::format_bytes(n.total_rx)),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled("TX: ", Style::default().fg(t.text_dim)),
                Span::styled(
                    format!("{}/s ", format_rate(n.tx_bytes)),
                    Style::default().fg(t.warning),
                ),
                Span::styled(
                    format!("({})", crate::models::format_bytes(n.total_tx)),
                    Style::default().fg(t.text_muted),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_disk_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(
            t!("title.filesystems").to_string(),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(sys) = &state.system else { return };

    if sys.disks.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            t!("disk.none").to_string(),
            Style::default().fg(t.text_muted),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = sys
        .disks
        .iter()
        .take(inner.height as usize)
        .map(|d| {
            let used = d.total_space - d.available_space;
            let pct = if d.total_space > 0 {
                (used as f64 / d.total_space as f64) * 100.0
            } else {
                0.0
            };
            let pct_color = t.usage_color(pct as f32);

            // Build a mini gauge: [████████░░░░] 72%
            let bar_width = 16;
            let filled = ((pct / 100.0) * bar_width as f64) as usize;
            let empty = bar_width - filled;
            let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

            Line::from(vec![
                Span::styled(
                    format!(" {:<14}", truncate_str(&d.mount_point, 14)),
                    Style::default().fg(t.text_primary),
                ),
                Span::styled(bar, Style::default().fg(pct_color)),
                Span::styled(format!(" {:>5.1}%", pct), Style::default().fg(pct_color)),
                Span::styled(
                    format!(
                        "  {} / {}",
                        crate::models::format_bytes(used),
                        crate::models::format_bytes(d.total_space)
                    ),
                    Style::default().fg(t.text_dim),
                ),
                Span::styled(
                    format!("  [{}]", d.fs_type),
                    Style::default().fg(t.text_muted),
                ),
                // Disk I/O rates (if nonzero)
                if d.read_bytes_per_sec > 0 || d.write_bytes_per_sec > 0 {
                    Span::styled(
                        format!(
                            "  R:{}/s W:{}/s",
                            format_rate(d.read_bytes_per_sec),
                            format_rate(d.write_bytes_per_sec)
                        ),
                        Style::default().fg(t.accent_secondary),
                    )
                } else {
                    Span::styled("", Style::default())
                },
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_docker_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let running = state
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    let total = state.containers.len();
    let title = t!("title.docker", running = running, total = total).to_string();

    let block = Block::default()
        .title(Span::styled(title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.containers.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            t!("docker.none").to_string(),
            Style::default().fg(t.text_muted),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = state
        .containers
        .iter()
        .skip(state.container_scroll)
        .take(inner.height as usize)
        .map(|c| {
            let state_color = match c.state.as_str() {
                "running" => t.success,
                "exited" => t.text_muted,
                "paused" => t.warning,
                "restarting" => t.warning,
                _ => t.text_dim,
            };

            let mut spans = vec![
                Span::styled(
                    format!(" {:<12}", truncate_str(&c.name, 12)),
                    Style::default().fg(t.text_primary),
                ),
                Span::styled(
                    format!("{:<8}", truncate_str(&c.state, 8)),
                    Style::default().fg(state_color),
                ),
            ];

            if c.state == "running" {
                let cpu_color = t.usage_color(c.cpu_percent as f32);
                let mem_pct = c.memory_percent();
                let mem_color = t.usage_color(mem_pct as f32);

                spans.push(Span::styled(
                    format!("CPU:{:>5.1}% ", c.cpu_percent),
                    Style::default().fg(cpu_color),
                ));
                spans.push(Span::styled(
                    format!(
                        "MEM:{}/{} ",
                        crate::models::format_bytes(c.memory_usage),
                        crate::models::format_bytes(c.memory_limit),
                    ),
                    Style::default().fg(mem_color),
                ));
                spans.push(Span::styled(
                    format!("PIDs:{} ", c.pids),
                    Style::default().fg(t.text_dim),
                ));
            }

            spans.push(Span::styled(
                truncate_str(&c.image, 24),
                Style::default().fg(t.text_muted),
            ));

            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Format bytes-per-tick as a human-readable rate.
/// Since sysinfo `received()`/`transmitted()` return bytes since last refresh (~1s), treat as /s.
fn format_rate(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1_073_741_824 {
        format!("{:.1} GiB", bytes_per_sec as f64 / 1_073_741_824.0)
    } else if bytes_per_sec >= 1_048_576 {
        format!("{:.1} MiB", bytes_per_sec as f64 / 1_048_576.0)
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KiB", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{} B", bytes_per_sec)
    }
}

fn render_ai_insight(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let expand_hint = if state.ai_insight_expanded {
        " e:collapse ↑↓:scroll "
    } else {
        " e:expand ↑↓:scroll "
    };

    let title = if state.ai_insight_loading {
        let spinner = match state.tick_count % 4 {
            0 => "◐",
            1 => "◓",
            2 => "◑",
            _ => "◒",
        };
        format!(" {} AI Analysis ", spinner)
    } else {
        let age = state
            .ai_insight_updated
            .map(|when| {
                let secs = when.elapsed().as_secs();
                if secs < 60 {
                    format!("{}s ago", secs)
                } else {
                    format!("{}m ago", secs / 60)
                }
            })
            .unwrap_or_else(|| "pending".to_string());
        format!(" AI Analysis ({}) ", age)
    };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            expand_hint,
            Style::default().fg(t.text_muted),
        )))
        .borders(Borders::ALL)
        .border_style(if state.ai_insight_loading {
            Style::default().fg(t.ai_accent)
        } else {
            t.border_style()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.ai_insight_loading && state.ai_insight.is_none() {
        let dots = match state.tick_count % 4 {
            0 => ".",
            1 => "..",
            2 => "...",
            _ => "",
        };
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            format!("  Analyzing your system{}", dots),
            Style::default().fg(t.ai_accent),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    if let Some(ref insight) = state.ai_insight {
        let wrap_width = inner.width.saturating_sub(2) as usize;
        let all_lines: Vec<Line> = insight
            .lines()
            .flat_map(|line| {
                if line.trim().is_empty() {
                    vec![Line::raw("")]
                } else {
                    textwrap::wrap(line, wrap_width.max(20))
                        .into_iter()
                        .map(|wrapped| {
                            Line::from(vec![
                                Span::styled(" ", Style::default()),
                                Span::styled(
                                    wrapped.to_string(),
                                    Style::default().fg(t.ai_response),
                                ),
                            ])
                        })
                        .collect()
                }
            })
            .collect();

        let visible_height = inner.height as usize;
        let total_lines = all_lines.len();
        let scroll = state
            .ai_insight_scroll
            .min(total_lines.saturating_sub(visible_height));

        let visible_lines: Vec<Line> = all_lines
            .into_iter()
            .skip(scroll)
            .take(visible_height)
            .collect();

        frame.render_widget(Paragraph::new(visible_lines), inner);

        // Scrollbar when content overflows
        if total_lines > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"));
            let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll);
            frame.render_stateful_widget(
                scrollbar,
                inner.inner(Margin {
                    vertical: 0,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    } else if !state.ai_has_key {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            "  No API credentials - insight unavailable",
            Style::default().fg(t.text_muted),
        )]));
        frame.render_widget(msg, inner);
    }
}

fn render_top_processes(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(
            t!("title.top_processes").to_string(),
            t.header_style(),
        ))
        .borders(Borders::ALL)
        .border_style(t.border_style());

    let header = Row::new(vec![
        Cell::from("PID").style(t.table_header_style()),
        Cell::from("NAME").style(t.table_header_style()),
        Cell::from("CPU %").style(t.table_header_style()),
        Cell::from("MEMORY").style(t.table_header_style()),
        Cell::from("MEM %").style(t.table_header_style()),
        Cell::from("STATUS").style(t.table_header_style()),
    ]);

    let rows: Vec<Row> = state
        .processes
        .iter()
        .take(area.height.saturating_sub(4) as usize)
        .map(|p| {
            let cpu_color = t.usage_color(p.cpu_usage);
            let mem_color = t.usage_color(p.memory_percent);

            Row::new(vec![
                Cell::from(format!("{}", p.pid)).style(Style::default().fg(t.text_dim)),
                Cell::from(truncate_str(&p.name, 20)).style(Style::default().fg(t.text_primary)),
                Cell::from(format!("{:.1}", p.cpu_usage)).style(Style::default().fg(cpu_color)),
                Cell::from(p.memory_display()).style(Style::default().fg(t.text_primary)),
                Cell::from(format!("{:.1}", p.memory_percent))
                    .style(Style::default().fg(mem_color)),
                Cell::from(status_badge(&p.status, t)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(15),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(block);

    frame.render_widget(table, area);
}

fn render_recent_alerts(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let alert_count = state.alerts.len();
    let title = format!(" Recent Alerts ({}) ", alert_count);
    let block = Block::default()
        .title(Span::styled(&title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(if state.danger_alert_count() > 0 {
            t.border_highlight_style()
        } else {
            t.border_style()
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.alerts.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            t!("alert.none_healthy").to_string(),
            Style::default().fg(t.success),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = state
        .alerts
        .iter()
        .take(inner.height as usize)
        .map(|a| {
            Line::from(vec![
                Span::styled(
                    format!(" {:>6} ", a.severity),
                    t.severity_badge_style(a.severity),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("[{}] ", a.category),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(&a.message, t.alert_style(a.severity)),
                Span::styled(
                    format!("  {}", a.age_display()),
                    Style::default().fg(t.text_muted),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Processes Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_processes(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter / info bar
            Constraint::Min(10),   // Process table
        ])
        .split(area);

    let filter_text = if state.filter_text.is_empty() {
        "Type / to filter processes...".to_string()
    } else {
        format!("Filter: {}_", state.filter_text)
    };

    let tree_indicator = if state.tree_view { " [TREE] │" } else { "" };
    let filtered = state.filtered_processes();
    let info = format!(
        " {} processes shown │{} Sort: {:?} {:?} │ {} ",
        filtered.len(),
        tree_indicator,
        state.sort_column,
        state.sort_direction,
        filter_text
    );

    let filter_bar = Paragraph::new(Line::from(vec![Span::styled(
        info,
        Style::default().fg(t.text_dim),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(t.border_style()),
    );
    frame.render_widget(filter_bar, chunks[0]);

    let sort_indicator = |col: SortColumn| -> &str {
        if col == state.sort_column {
            match state.sort_direction {
                super::state::SortDirection::Asc => " ▲",
                super::state::SortDirection::Desc => " ▼",
            }
        } else {
            ""
        }
    };

    let title = if state.tree_view {
        t!("title.process_tree").to_string()
    } else {
        t!("title.process_list").to_string()
    };

    if state.tree_view {
        // ── Tree view rendering ─────────────────────────────
        let tree_data = state.tree_processes();

        let header = Row::new(vec![
            Cell::from("PID").style(t.table_header_style()),
            Cell::from("TREE / NAME").style(t.table_header_style()),
            Cell::from("CPU %").style(t.table_header_style()),
            Cell::from("MEMORY").style(t.table_header_style()),
            Cell::from("MEM %").style(t.table_header_style()),
            Cell::from("STATUS").style(t.table_header_style()),
            Cell::from("USER").style(t.table_header_style()),
        ])
        .height(1);

        let rows: Vec<Row> = tree_data
            .iter()
            .enumerate()
            .map(|(i, (prefix, p))| {
                let cpu_color = t.usage_color(p.cpu_usage);
                let mem_color = t.usage_color(p.memory_percent);
                let style = if i == state.selected_process {
                    t.table_row_selected()
                } else {
                    t.table_row_normal()
                };

                let tree_name = format!("{}{}", prefix, p.name);

                Row::new(vec![
                    Cell::from(format!("{}", p.pid)).style(Style::default().fg(t.text_dim)),
                    Cell::from(truncate_str(&tree_name, 40))
                        .style(Style::default().fg(t.text_primary)),
                    Cell::from(format!("{:.1}", p.cpu_usage)).style(Style::default().fg(cpu_color)),
                    Cell::from(p.memory_display()),
                    Cell::from(format!("{:.1}", p.memory_percent))
                        .style(Style::default().fg(mem_color)),
                    Cell::from(status_badge(&p.status, t)),
                    Cell::from(truncate_str(&p.user, 10)).style(Style::default().fg(t.text_dim)),
                ])
                .style(style)
            })
            .collect();

        let total = tree_data.len();

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Min(30),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(7),
                Constraint::Length(10),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .title(Span::styled(title, t.header_style()))
                .borders(Borders::ALL)
                .border_style(t.border_style()),
        )
        .row_highlight_style(t.table_row_selected());

        let mut table_state = TableState::default();
        table_state.select(Some(state.selected_process));
        frame.render_stateful_widget(table, chunks[1], &mut table_state);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = ScrollbarState::new(total).position(state.selected_process);
        frame.render_stateful_widget(
            scrollbar,
            chunks[1].inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    } else {
        // ── Flat table view ─────────────────────────────────
        let header = Row::new(vec![
            Cell::from(format!("PID{}", sort_indicator(SortColumn::Pid)))
                .style(t.table_header_style()),
            Cell::from(format!("NAME{}", sort_indicator(SortColumn::Name)))
                .style(t.table_header_style()),
            Cell::from(format!("CPU %{}", sort_indicator(SortColumn::Cpu)))
                .style(t.table_header_style()),
            Cell::from(format!("MEMORY{}", sort_indicator(SortColumn::Memory)))
                .style(t.table_header_style()),
            Cell::from("MEM %").style(t.table_header_style()),
            Cell::from("DISK R").style(t.table_header_style()),
            Cell::from("DISK W").style(t.table_header_style()),
            Cell::from(format!("STATUS{}", sort_indicator(SortColumn::Status)))
                .style(t.table_header_style()),
            Cell::from("USER").style(t.table_header_style()),
            Cell::from("CMD").style(t.table_header_style()),
        ])
        .height(1);

        let rows: Vec<Row> = filtered
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let cpu_color = t.usage_color(p.cpu_usage);
                let mem_color = t.usage_color(p.memory_percent);
                let style = if i == state.selected_process {
                    t.table_row_selected()
                } else {
                    t.table_row_normal()
                };

                Row::new(vec![
                    Cell::from(format!("{}", p.pid)).style(Style::default().fg(t.text_dim)),
                    Cell::from(truncate_str(&p.name, 22)),
                    Cell::from(format!("{:.1}", p.cpu_usage)).style(Style::default().fg(cpu_color)),
                    Cell::from(p.memory_display()),
                    Cell::from(format!("{:.1}", p.memory_percent))
                        .style(Style::default().fg(mem_color)),
                    Cell::from(p.disk_read_display()).style(Style::default().fg(t.text_dim)),
                    Cell::from(p.disk_write_display()).style(Style::default().fg(t.text_dim)),
                    Cell::from(status_badge(&p.status, t)),
                    Cell::from(truncate_str(&p.user, 10)).style(Style::default().fg(t.text_dim)),
                    Cell::from(truncate_str(&p.cmd, 40)).style(Style::default().fg(t.text_muted)),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(24),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(7),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(12),
                Constraint::Min(20),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .title(Span::styled(title, t.header_style()))
                .borders(Borders::ALL)
                .border_style(t.border_style()),
        )
        .row_highlight_style(t.table_row_selected());

        let mut table_state = TableState::default();
        table_state.select(Some(state.selected_process));
        frame.render_stateful_widget(table, chunks[1], &mut table_state);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let total = state.filtered_processes().len();
        let mut scrollbar_state = ScrollbarState::new(total).position(state.selected_process);
        frame.render_stateful_widget(
            scrollbar,
            chunks[1].inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Alerts Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_alerts(frame: &mut Frame, area: Rect, state: &AppState) {
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

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"));
    let mut scrollbar_state = ScrollbarState::new(state.alerts.len()).position(state.alert_scroll);
    frame.render_stateful_widget(
        scrollbar,
        inner.inner(Margin {
            vertical: 0,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Ask AI Tab - Chat Interface
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_ask_ai(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // Chat history
            Constraint::Length(4), // Input box
        ])
        .split(area);

    render_chat_history(frame, chunks[0], state);
    render_chat_input(frame, chunks[1], state);
}

fn render_chat_history(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let border_style = if state.ai_loading {
        Style::default().fg(t.ai_accent)
    } else {
        t.border_style()
    };

    let title = if state.ai_loading {
        let spinner = match state.tick_count % 4 {
            0 => "◐",
            1 => "◓",
            2 => "◑",
            _ => "◒",
        };
        t!("chat.thinking", spinner = spinner).to_string()
    } else {
        t!("title.ask_ai_full").to_string()
    };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !state.ai_has_key {
        let msg = Paragraph::new(vec![
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.no_key_title").to_string(),
                Style::default().fg(t.danger).add_modifier(Modifier::BOLD),
            )]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.no_key_hint").to_string(),
                Style::default().fg(t.text_dim),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.no_key_opt1").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.no_key_opt2").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.no_key_opt3").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.no_key_restart").to_string(),
                Style::default().fg(t.text_dim),
            )]),
        ]);
        frame.render_widget(msg, inner);
        return;
    }

    if state.ai_conversation.messages.is_empty() {
        // Welcome screen
        let auth_info = if state.ai_auth_method.is_empty() {
            t!("ai.authenticated").to_string()
        } else {
            t!("ai.auth_method", method = &state.ai_auth_method).to_string()
        };
        let msg = Paragraph::new(vec![
            Line::raw(""),
            Line::from(vec![
                Span::styled(
                    t!("ai.welcome_prefix").to_string(),
                    Style::default().fg(t.text_dim),
                ),
                Span::styled(
                    t!("ai.welcome_name").to_string(),
                    Style::default()
                        .fg(t.ai_accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  ({})", auth_info), Style::default().fg(t.success)),
            ]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.welcome_desc1").to_string(),
                Style::default().fg(t.text_dim),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.welcome_desc2").to_string(),
                Style::default().fg(t.text_dim),
            )]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.try_asking").to_string(),
                Style::default().fg(t.text_muted),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.example1").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.example2").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.example3").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.example4").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::from(vec![Span::styled(
                t!("ai.example5").to_string(),
                Style::default().fg(t.accent),
            )]),
            Line::raw(""),
            Line::from(vec![Span::styled(
                t!("ai.type_hint").to_string(),
                Style::default().fg(t.text_muted),
            )]),
        ]);
        frame.render_widget(msg, inner);
        return;
    }

    // Render conversation messages
    let wrap_width = inner.width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for msg in &state.ai_conversation.messages {
        match msg.role {
            MessageRole::User => {
                lines.push(Line::from(vec![
                    Span::styled(
                        t!("chat.you").to_string(),
                        Style::default()
                            .fg(t.bg_dark)
                            .bg(t.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", msg.timestamp.format("%H:%M:%S")),
                        Style::default().fg(t.text_muted),
                    ),
                ]));
                let wrapped = textwrap::wrap(&msg.content, wrap_width);
                for line in wrapped {
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(line.to_string(), Style::default().fg(t.text_primary)),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            MessageRole::Assistant => {
                lines.push(Line::from(vec![
                    Span::styled(
                        t!("chat.ai").to_string(),
                        Style::default()
                            .fg(t.bg_dark)
                            .bg(t.ai_accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", msg.timestamp.format("%H:%M:%S")),
                        Style::default().fg(t.text_muted),
                    ),
                ]));
                let wrapped = textwrap::wrap(&msg.content, wrap_width);
                for line in wrapped {
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(line.to_string(), Style::default().fg(t.ai_response)),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            MessageRole::System => {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {}", msg.content),
                    Style::default().fg(t.text_muted),
                )]));
                lines.push(Line::raw(""));
            }
        }
    }

    // Loading indicator at the bottom
    if state.ai_loading {
        let dots = match state.tick_count % 4 {
            0 => ".",
            1 => "..",
            2 => "...",
            _ => "",
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  Analyzing your system{}", dots),
            Style::default().fg(t.ai_accent),
        )]));
    }

    // Apply scrolling
    let visible_height = inner.height as usize;
    let total_lines = lines.len();
    let scroll = if state.ai_scroll > 0 {
        state
            .ai_scroll
            .min(total_lines.saturating_sub(visible_height))
    } else {
        // Auto-scroll to bottom
        total_lines.saturating_sub(visible_height)
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    frame.render_widget(Paragraph::new(visible_lines), inner);

    // Scrollbar
    if total_lines > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll);
        frame.render_stateful_widget(
            scrollbar,
            inner.inner(Margin {
                vertical: 0,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_chat_input(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let is_ai_tab = state.active_tab == Tab::AskAi;

    let border_style = if is_ai_tab && !state.ai_loading {
        t.border_highlight_style()
    } else {
        t.border_style()
    };

    let prompt_hint = if state.ai_loading {
        t!("chat.waiting").to_string()
    } else if !state.ai_has_key {
        t!("chat.no_key").to_string()
    } else {
        t!("chat.placeholder").to_string()
    };

    let display_text = if state.ai_input.is_empty() {
        prompt_hint.to_string()
    } else {
        format!("  {}", state.ai_input)
    };

    let input_style = if state.ai_input.is_empty() {
        Style::default().fg(t.text_muted)
    } else {
        Style::default().fg(t.text_primary)
    };

    // Show cursor position
    let cursor_line = if !state.ai_input.is_empty() {
        let before_cursor = &state.ai_input[..state.ai_cursor_pos];
        let after_cursor = &state.ai_input[state.ai_cursor_pos..];
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                before_cursor.to_string(),
                Style::default().fg(t.text_primary),
            ),
            Span::styled(
                if after_cursor.is_empty() {
                    " "
                } else {
                    &after_cursor[..after_cursor
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| i)
                        .unwrap_or(after_cursor.len())]
                }
                .to_string(),
                Style::default().fg(t.bg_dark).bg(t.accent),
            ),
            Span::styled(
                if after_cursor.len() > 1 {
                    after_cursor[after_cursor
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| i)
                        .unwrap_or(after_cursor.len())..]
                        .to_string()
                } else {
                    String::new()
                },
                Style::default().fg(t.text_primary),
            ),
        ])
    } else {
        Line::from(vec![Span::styled(display_text, input_style)])
    };

    let mut lines = vec![cursor_line];
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            "Enter",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("key.send").to_string(),
            Style::default().fg(t.text_muted),
        ),
        Span::styled(
            "Ctrl+L",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("key.clear_chat").to_string(),
            Style::default().fg(t.text_muted),
        ),
        Span::styled(
            "Esc",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("key.back").to_string(),
            Style::default().fg(t.text_muted),
        ),
    ]));

    let input = Paragraph::new(lines).block(
        Block::default()
            .title(Span::styled(
                t!("title.message").to_string(),
                Style::default().fg(t.ai_accent),
            ))
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(input, area);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Status Bar
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let mut spans = vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            " q ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.quit").to_string(),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " Tab ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.switch").to_string(),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " ↑↓ ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.scroll").to_string(),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " s ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.sort").to_string(),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " T ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {} ", t.name), Style::default().fg(t.text_dim)),
        Span::styled(
            " L ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", state.current_lang.to_uppercase()),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " 4 ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.ask_ai").to_string(),
            Style::default().fg(t.text_dim),
        ),
        Span::styled(
            " ? ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            t!("status.help").to_string(),
            Style::default().fg(t.text_dim),
        ),
    ];

    // Show process-specific shortcuts on Processes tab
    if state.active_tab == crate::ui::Tab::Processes {
        spans.push(Span::styled(
            " t ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.tree").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            " Enter ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.detail").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            " a ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.ask_ai").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            " x ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.signal").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            " n ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.renice").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            " k ",
            Style::default()
                .fg(t.bg_dark)
                .bg(t.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            t!("status.kill").to_string(),
            Style::default().fg(t.text_dim),
        ));
        spans.push(Span::styled(
            t!("status.kill").to_string(),
            Style::default().fg(t.text_dim),
        ));
    }

    // Show status message (e.g., kill confirmation) — auto-expires after 5 seconds
    if let Some((msg, when)) = &state.status_message {
        if when.elapsed().as_secs() < 5 {
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Help Overlay
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_process_detail(frame: &mut Frame, area: Rect, state: &AppState) {
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

    // ── Basic info section ──
    lines.push(Line::from(Span::styled(
        " Process Info",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("  PID:      ", Style::default().fg(t.text_dim)),
        Span::styled(
            format!("{}", detail.pid),
            Style::default().fg(t.text_primary),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Name:     ", Style::default().fg(t.text_dim)),
        Span::styled(&detail.name, Style::default().fg(t.text_primary)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  User:     ", Style::default().fg(t.text_dim)),
        Span::styled(&detail.user, Style::default().fg(t.text_primary)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Status:   ", Style::default().fg(t.text_dim)),
        Span::styled(&detail.status, Style::default().fg(t.success)),
    ]));
    if let Some(ppid) = detail.parent_pid {
        lines.push(Line::from(vec![
            Span::styled("  Parent:   ", Style::default().fg(t.text_dim)),
            Span::styled(format!("PID {}", ppid), Style::default().fg(t.text_primary)),
        ]));
    }
    if let Some(tc) = detail.thread_count {
        lines.push(Line::from(vec![
            Span::styled("  Threads:  ", Style::default().fg(t.text_dim)),
            Span::styled(format!("{}", tc), Style::default().fg(t.text_primary)),
        ]));
    }
    lines.push(Line::raw(""));

    // ── Resource usage ──
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

    // ── Command ──
    lines.push(Line::from(Span::styled(
        " Full Command",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )));
    // Wrap long command lines
    let cmd_width = (inner.width as usize).saturating_sub(4);
    let cmd_wrapped = textwrap::wrap(&detail.cmd, cmd_width);
    for line in cmd_wrapped {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(line.to_string(), Style::default().fg(t.text_primary)),
        ]));
    }
    lines.push(Line::raw(""));

    // ── Open File Descriptors ──
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

    // ── Environment Variables ──
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

    // Scrollbar
    if total_lines > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll);
        frame.render_stateful_widget(
            scrollbar,
            inner.inner(Margin {
                vertical: 0,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_help_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let popup_width = 55;
    let popup_height = 38;
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled(
            "  SENTINEL - Keyboard Shortcuts",
            t.header_style(),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "  Tab / Shift+Tab  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Switch tabs", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  1 / 2 / 3 / 4    ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Jump to tab (4 = Ask AI)",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Up/Down / j / k   ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Scroll up/down", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  PgUp / PgDn       ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Page up/down", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  s                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cycle sort column", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  r                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Reverse sort direction",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  /                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Filter processes", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  k                  ",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "SIGTERM selected process",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  K (shift)          ",
                Style::default().fg(t.danger).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "SIGKILL selected process",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Enter              ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Process detail popup", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  t                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Toggle process tree view",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  T                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cycle color theme", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  x                  ",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Signal picker (process)",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  n                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Renice process", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  T                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cycle color theme", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  L                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Cycle UI language", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  +/- (Dashboard)    ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Zoom history charts", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  f (Dashboard)      ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Focus/expand widget", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  a                  ",
                Style::default()
                    .fg(t.ai_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Ask AI about process", Style::default().fg(t.text_primary)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc                ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Clear filter / close help",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Esc                ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Clear filter / close help",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  q                  ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Quit", Style::default().fg(t.text_primary)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  Ask AI Tab:",
            Style::default()
                .fg(t.ai_accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "  Enter              ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Send question to Claude",
                Style::default().fg(t.text_primary),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Ctrl+L             ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Clear conversation", Style::default().fg(t.text_primary)),
        ]),
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Signal Picker Popup
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_signal_picker(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height =
        (super::state::SIGNAL_LIST.len() as u16 + 6).min(area.height.saturating_sub(4));
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

    for (i, (num, name, desc)) in super::state::SIGNAL_LIST.iter().enumerate() {
        let is_selected = i == state.signal_picker_selected;
        let prefix = if is_selected { " > " } else { "   " };

        let style = if is_selected {
            t.table_row_selected()
        } else {
            Style::default().fg(t.text_primary)
        };

        let danger_style = if *num == 9 {
            // SIGKILL - red
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Renice Dialog
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn render_renice_dialog(frame: &mut Frame, area: Rect, state: &AppState) {
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

    // Nice value visualization
    let nice = state.renice_value;
    let nice_color = if nice < 0 {
        t.danger // Higher priority
    } else if nice == 0 {
        t.success // Normal
    } else {
        t.text_dim // Lower priority
    };

    let bar_width = 40.min(inner.width.saturating_sub(4)) as usize;
    // Map nice -20..19 to bar position 0..bar_width
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

fn status_badge<'a>(status: &crate::models::ProcessStatus, t: &Theme) -> Span<'a> {
    use crate::models::ProcessStatus;
    match status {
        ProcessStatus::Running => Span::styled(
            t!("status.running").to_string(),
            Style::default().fg(t.success),
        ),
        ProcessStatus::Sleeping => Span::styled(
            t!("status.sleeping").to_string(),
            Style::default().fg(t.text_dim),
        ),
        ProcessStatus::Stopped => Span::styled(
            t!("status.stopped").to_string(),
            Style::default().fg(t.warning),
        ),
        ProcessStatus::Zombie => Span::styled(
            t!("status.zombie").to_string(),
            Style::default().fg(t.danger),
        ),
        ProcessStatus::Dead => {
            Span::styled(t!("status.dead").to_string(), Style::default().fg(t.danger))
        }
        ProcessStatus::Unknown => Span::styled(
            t!("status.unknown").to_string(),
            Style::default().fg(t.text_muted),
        ),
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}
