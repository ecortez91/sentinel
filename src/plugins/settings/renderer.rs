//! Settings plugin renderer.
//!
//! Renders the settings tab with inline edit affordances:
//! - Toggle items show `[ON]`/`[OFF]`
//! - Cycle items show `[< value >]`
//! - Number items show `[value]` and, when editing, `[_buffer|]`
//! - ReadOnly items show `[value]` with a config.toml hint

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{SettingKind, SettingsCategory, SettingsPlugin};
use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Render the settings tab.
pub fn render_settings(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsPlugin,
    theme: &Theme,
    glyphs: &Glyphs,
) {
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

    // -- Category list --
    render_categories(frame, columns[0], state, theme, glyphs);

    // -- Settings items --
    render_items(frame, columns[1], state, theme);
}

fn render_categories(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsPlugin,
    theme: &Theme,
    glyphs: &Glyphs,
) {
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
        let prefix = if is_selected {
            format!("{} ", glyphs.pointer)
        } else {
            "  ".to_string()
        };
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
        let is_editing = is_selected && state.editing;

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

        // Format the value display based on kind and edit state
        let value_display = if is_editing {
            format!("[{}|]", state.edit_buffer)
        } else {
            format_value_for_kind(&item.value, &item.kind)
        };

        // Build the hint span shown after the value
        let hint = if is_editing {
            " Enter=save Esc=cancel"
        } else if is_selected {
            match &item.kind {
                SettingKind::Toggle => " Enter=toggle",
                SettingKind::Cycle(_) => " Enter=cycle",
                SettingKind::Number { .. } => " Enter=edit",
                SettingKind::Text { .. } => " Enter=edit",
                SettingKind::ReadOnly => " (config.toml)",
            }
        } else {
            ""
        };

        let hint_style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::ITALIC)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:.<20} ", item.label), style),
            Span::styled(value_display, value_style),
            Span::styled(hint, hint_style),
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
        "  Enter: Edit  |  Advanced settings: ~/.config/sentinel/config.toml",
        Style::default()
            .fg(theme.text_muted)
            .add_modifier(Modifier::ITALIC),
    )));

    frame.render_widget(Paragraph::new(lines), items_inner);
}

/// Format a setting value for display based on its kind.
fn format_value_for_kind(value: &str, kind: &SettingKind) -> String {
    match kind {
        SettingKind::Toggle => {
            if value == "true" {
                "[ON]".to_string()
            } else {
                "[OFF]".to_string()
            }
        }
        SettingKind::Cycle(_) => format!("[< {} >]", value),
        SettingKind::Number { suffix, .. } => format!("[{}{}]", value, suffix),
        SettingKind::Text { masked, .. } => {
            if *masked && !value.is_empty() {
                format!("[{}]", "*".repeat(value.len().min(16)))
            } else if value.is_empty() {
                "[<empty>]".to_string()
            } else {
                format!("[{}]", value)
            }
        }
        SettingKind::ReadOnly => format!("[{}]", value),
    }
}
