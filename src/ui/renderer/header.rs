//! Header bar: logo, tab strip, system summary.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ui::state::{AppState, Tab};
use crate::utils::spinner_char;

pub fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
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
        let spinner = spinner_char(state.tick_count);
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
