//! Processes tab: flat table and tree view.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::ui::state::{AppState, SortColumn, SortDirection};

use super::helpers::{render_scrollbar_bordered, status_badge, truncate_str};

pub fn render_processes(frame: &mut Frame, area: Rect, state: &AppState) {
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
                SortDirection::Asc => " ▲",
                SortDirection::Desc => " ▼",
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
        render_tree_view(frame, chunks[1], state, &title);
    } else {
        render_flat_view(frame, chunks[1], state, &title, sort_indicator);
    }
}

fn render_tree_view(frame: &mut Frame, area: Rect, state: &AppState, title: &str) {
    let t = &state.theme;
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
                Cell::from(truncate_str(&tree_name, 40)).style(Style::default().fg(t.text_primary)),
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
    frame.render_stateful_widget(table, area, &mut table_state);

    render_scrollbar_bordered(frame, area, total, state.selected_process);
}

fn render_flat_view(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    title: &str,
    sort_indicator: impl Fn(SortColumn) -> &'static str,
) {
    let t = &state.theme;
    let filtered = state.filtered_processes();

    let header = Row::new(vec![
        Cell::from(format!("PID{}", sort_indicator(SortColumn::Pid))).style(t.table_header_style()),
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

    let total = filtered.len();

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
    frame.render_stateful_widget(table, area, &mut table_state);

    render_scrollbar_bordered(frame, area, total, state.selected_process);
}
