//! Dashboard tab: system gauges, CPU cores, sparklines, GPU, network, disk,
//! Docker, AI insight, top processes, recent alerts.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::models::format_bytes;
use crate::ui::state::{AppState, FocusedWidget};
use crate::ui::widgets::{CpuMiniChart, GradientGauge};
use crate::utils::{loading_dots, spinner_char};

use super::helpers::{format_rate, render_scrollbar, status_badge, truncate_str};

pub fn render_dashboard(frame: &mut Frame, area: Rect, state: &AppState) {
    // Focus/expand mode: render only the focused widget
    if let Some(focused) = state.focused_widget {
        let t = &state.theme;
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

    // Normal dashboard layout
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
        1
    } else {
        0
    };

    let constraints = vec![
        Constraint::Length(8 + battery_row),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(gpu_height),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(docker_height),
        Constraint::Length(insight_height),
        Constraint::Min(8),
        Constraint::Length(8),
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

    // Bottom row: split between recent alerts and event ticker
    if !state.recent_events.is_empty() {
        let bottom_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(chunks[9]);
        render_recent_alerts(frame, bottom_split[0], state);
        render_event_ticker(frame, bottom_split[1], state);
    } else {
        render_recent_alerts(frame, chunks[9], state);
    }
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
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ];
    if has_battery {
        constraints.push(Constraint::Length(1));
    }

    let gauge_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(inner);

    // CPU gauge
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

    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

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

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let gpu_gauge = GradientGauge::new(gpu.utilization as f32, "GPU  ", t);
    frame.render_widget(gpu_gauge, rows[0]);

    let vram_pct = gpu.memory_percent();
    let vram_label = format!(
        "VRAM {} / {}  ",
        crate::models::format_bytes(gpu.memory_used),
        crate::models::format_bytes(gpu.memory_total),
    );
    let vram_gauge = GradientGauge::new(vram_pct, &vram_label, t);
    frame.render_widget(vram_gauge, rows[1]);

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

fn render_event_ticker(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let event_count = state.recent_events.len();
    let title = format!(" Events (last 5m: {}) ", event_count);
    let block = Block::default()
        .title(Span::styled(&title, t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.recent_events.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            " No recent events",
            Style::default().fg(t.text_dim),
        )]));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = state
        .recent_events
        .iter()
        .take(inner.height as usize)
        .map(|event_str| {
            let color = if event_str.contains("! ") {
                t.warning
            } else if event_str.contains("+ ") {
                t.success
            } else if event_str.contains("- ") {
                t.danger
            } else if event_str.contains("> ") || event_str.contains("< ") {
                t.info
            } else if event_str.contains("X ") {
                t.danger
            } else {
                t.text_dim
            };
            Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_str(event_str, inner.width.saturating_sub(2) as usize)
                ),
                Style::default().fg(color),
            ))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_disk_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let block = Block::default()
        .title(Span::styled(t!("title.disk").to_string(), t.header_style()))
        .borders(Borders::ALL)
        .border_style(t.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(sys) = &state.system else { return };

    if sys.disks.is_empty() {
        let msg = Paragraph::new(Line::from(vec![Span::styled(
            " No disks found",
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

            let bar_width = 16;
            let filled = ((pct / 100.0) * bar_width as f64) as usize;
            let empty = bar_width - filled;
            let bar = format!("{}{}", "\u{2588}".repeat(filled), "\u{2591}".repeat(empty));

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
                "paused" | "restarting" => t.warning,
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

fn render_ai_insight(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let expand_hint = if state.ai_insight_expanded {
        " e:collapse ↑↓:scroll "
    } else {
        " e:expand ↑↓:scroll "
    };

    let title = if state.ai_insight_loading {
        let spinner = spinner_char(state.tick_count);
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
        let dots = loading_dots(state.tick_count);
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
        render_scrollbar(frame, inner, total_lines, scroll);
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
