//! Ask AI tab: chat history and input box.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ai::MessageRole;
use crate::ui::state::{AppState, Tab};
use crate::utils::{loading_dots, spinner_char};

use super::helpers::render_scrollbar;

pub fn render_ask_ai(frame: &mut Frame, area: Rect, state: &AppState) {
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
        let spinner = spinner_char(state.tick_count);
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
        render_no_key_message(frame, inner, state);
        return;
    }

    if state.ai_conversation.messages.is_empty() {
        render_welcome_message(frame, inner, state);
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
                for line in textwrap::wrap(&msg.content, wrap_width) {
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
                for line in textwrap::wrap(&msg.content, wrap_width) {
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

    // Loading indicator
    if state.ai_loading {
        let dots = loading_dots(state.tick_count);
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

    render_scrollbar(frame, inner, total_lines, scroll);
}

fn render_no_key_message(frame: &mut Frame, inner: Rect, state: &AppState) {
    let t = &state.theme;
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
}

fn render_welcome_message(frame: &mut Frame, inner: Rect, state: &AppState) {
    let t = &state.theme;
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
