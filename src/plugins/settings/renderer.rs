//! Settings plugin renderer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{SettingsCategory, SettingsPlugin};
use crate::ui::theme::Theme;

/// Render the settings tab.
pub fn render_settings(frame: &mut Frame, area: Rect, state: &SettingsPlugin, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Settings ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 || inner.width < 30 {
        return;
    }

    // Two-column layout: categories on left, items on right
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Min(30)])
        .split(inner);

    // ── Category list ────────────────────────────────────────
    render_categories(frame, columns[0], state, theme);

    // ── Settings items ───────────────────────────────────────
    render_items(frame, columns[1], state, theme);
}

fn render_categories(frame: &mut Frame, area: Rect, state: &SettingsPlugin, theme: &Theme) {
    let cat_block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Category ",
            Style::default().fg(theme.text_dim),
        ));

    let cat_inner = cat_block.inner(area);
    frame.render_widget(cat_block, area);

    let mut lines = Vec::new();
    for (i, cat) in SettingsCategory::all().iter().enumerate() {
        let is_selected = i == state.selected_category;
        let prefix = if is_selected { "\u{25B8} " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, cat.label()),
            style,
        )));
    }

    frame.render_widget(Paragraph::new(lines), cat_inner);
}

fn render_items(frame: &mut Frame, area: Rect, state: &SettingsPlugin, theme: &Theme) {
    let (category, items) = match state.settings.get(state.selected_category) {
        Some(pair) => pair,
        None => return,
    };

    let items_block = Block::default().borders(Borders::NONE).title(Span::styled(
        format!(" {} ", category.label()),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));

    let items_inner = items_block.inner(area);
    frame.render_widget(items_block, area);

    let mut lines = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let is_selected = i == state.selected_item;

        let style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.text_primary)
        } else {
            Style::default().fg(theme.text_primary)
        };

        let value_style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accent)
        };

        let desc_style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.text_muted)
        } else {
            Style::default().fg(theme.text_muted)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:.<20} ", item.label), style),
            Span::styled(format!("[{}]", item.value), value_style),
        ]));
        lines.push(Line::from(Span::styled(
            format!("    {}", item.description),
            desc_style,
        )));
        lines.push(Line::raw(""));
    }

    // Footer hint
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  Edit config.toml to change values. Restart to apply.",
        Style::default()
            .fg(theme.text_muted)
            .add_modifier(Modifier::ITALIC),
    )));

    frame.render_widget(Paragraph::new(lines), items_inner);
}
