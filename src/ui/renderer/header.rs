//! Header bar: logo, tab strip, system summary.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::plugins::registry::PluginRegistry;
use crate::ui::state::{AppState, Tab};

pub fn render_header_with_plugins(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    plugins: Option<&PluginRegistry>,
) {
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
        state.glyphs.pulse_on
    } else {
        state.glyphs.pulse_off
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

    // Tabs — build dynamic list including plugin tabs
    let all_tabs = Tab::all_with_plugins(state.plugin_count);
    let tabs: Vec<Span> = all_tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let label = if let Some(reg) = plugins {
                tab.label_with_plugins(reg)
            } else {
                tab.label()
            };
            // Add tab number prefix for quick-switch hint
            let numbered = format!("{} {}", i + 1, label);
            if *tab == state.active_tab {
                Span::styled(numbered, t.tab_active_style())
            } else if *tab == Tab::AskAi {
                Span::styled(numbered, Style::default().fg(t.ai_accent))
            } else if matches!(tab, Tab::Plugin(_)) {
                Span::styled(numbered, t.tab_inactive_style())
            } else {
                Span::styled(numbered, t.tab_inactive_style())
            }
        })
        .collect();

    let tab_count = tabs.len();
    let mut tab_spans = vec![Span::raw(" ")];
    for (i, tab) in tabs.into_iter().enumerate() {
        tab_spans.push(tab);
        if i < tab_count - 1 {
            tab_spans.push(Span::styled(
                state.glyphs.separator,
                Style::default().fg(t.text_muted),
            ));
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
        let spinner = state.glyphs.spinner_char(state.tick_count);
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
