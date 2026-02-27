//! Market plugin renderer: list view, detail view, and overlays.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline, Wrap},
    Frame,
};

use super::models::{format_change, format_large_number, format_price, ChartRange};
use super::state::{MarketState, MarketView};
use crate::ui::theme::Theme;

/// Top-level render dispatcher.
pub fn render_market(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    match state.view {
        MarketView::List => render_list(frame, area, state, theme),
        MarketView::Detail => render_detail(frame, area, state, theme),
    }
}

/// Render overlays (add ticker input modal).
pub fn render_market_overlay(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
) {
    if state.add_ticker_mode {
        render_add_ticker_modal(frame, area, state, theme);
    }
}

fn render_add_ticker_modal(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    // Center the modal
    let modal_width = 40;
    let modal_height = 5;
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(area.x + x, area.y + y, modal_width, modal_height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            " Add Ticker ",
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            "Enter symbol (e.g., BTCUSDT):",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(vec![
            Span::styled(&state.add_ticker_text, Style::default().fg(theme.text_primary)),
            Span::styled("█", Style::default().fg(theme.accent).add_modifier(Modifier::SLOW_BLINK)),
        ]),
        Line::from(Span::styled(
            "[Enter] Add  [Esc] Cancel",
            Style::default().fg(theme.text_muted),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── List View ────────────────────────────────────────────────────

fn render_list(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Market (Binance) ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 || inner.width < 40 {
        return;
    }

    // Layout: header info bar + search bar (conditional) + table
    let has_search = state.search_mode;
    let mut constraints = vec![Constraint::Length(1)]; // info bar
    if has_search {
        constraints.push(Constraint::Length(1)); // search bar
    }
    constraints.push(Constraint::Length(1)); // column headers
    constraints.push(Constraint::Min(1)); // table rows

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut chunk_idx = 0;

    // ── Info bar ─────────────────────────────────────────────
    render_info_bar(frame, chunks[chunk_idx], state, theme);
    chunk_idx += 1;

    // ── Search bar (conditional) ─────────────────────────────
    if has_search {
        render_search_bar(frame, chunks[chunk_idx], state, theme);
        chunk_idx += 1;
    }

    // ── Column headers ───────────────────────────────────────
    render_column_headers(frame, chunks[chunk_idx], state, theme);
    chunk_idx += 1;

    // ── Table rows ───────────────────────────────────────────
    render_table_rows(frame, chunks[chunk_idx], state, theme);
}

fn render_info_bar(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let mut spans = Vec::new();

    // Last updated
    if let Some(updated) = state.last_updated {
        let ago = updated.elapsed().as_secs();
        let time_str = if ago < 60 {
            format!("{}s ago", ago)
        } else {
            format!("{}m ago", ago / 60)
        };
        spans.push(Span::styled(
            format!(" Updated: {} ", time_str),
            Style::default().fg(theme.text_dim),
        ));
    } else if state.loading {
        spans.push(Span::styled(
            " Loading... ",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }

    spans.push(Span::styled(
        "│ ",
        Style::default().fg(theme.text_muted),
    ));

    // Coin count
    let visible = state.visible_coins();
    spans.push(Span::styled(
        format!("{} pairs", visible.len()),
        Style::default().fg(theme.text_dim),
    ));

    // Favorites count
    let fav_count = state.favorites.len();
    if fav_count > 0 {
        spans.push(Span::styled(
            format!(" │ ★ {} favorites", fav_count),
            Style::default().fg(Color::Yellow),
        ));
    }

    // Show favorites filter indicator
    if state.show_favorites_only {
        spans.push(Span::styled(
            " [FAVS ONLY]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Sort indicator
    spans.push(Span::styled(
        format!(
            " │ Sort: {} {}",
            state.sort_column.label(),
            if state.sort_ascending { "↑" } else { "↓" }
        ),
        Style::default().fg(theme.text_muted),
    ));

    // Add ticker hint
    spans.push(Span::styled(
        " │ [+] add ticker  [d] remove",
        Style::default().fg(theme.text_muted),
    ));

    // Error
    if let Some(ref err) = state.error {
        spans.push(Span::styled(
            format!(" │ Error: {}", truncate(err, 30)),
            Style::default().fg(theme.danger),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_search_bar(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let spans = vec![
        Span::styled(
            " /",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&state.search_text, Style::default().fg(theme.text_primary)),
        Span::styled(
            "█",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_column_headers(frame: &mut Frame, area: Rect, _state: &MarketState, theme: &Theme) {
    let header = format!(
        " {:<4} {:<10} {:>14} {:>10} {:>14} {:>12}",
        "#", "Symbol", "Price", "24h %", "Volume (Quote)", "Trades"
    );

    let header_line = Line::from(Span::styled(
        header,
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(header_line), area);
}

fn render_table_rows(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let visible = state.visible_coins();
    let max_rows = area.height as usize;

    if visible.is_empty() {
        let msg = if state.loading {
            "Fetching market data..."
        } else if state.show_favorites_only {
            "No favorites yet. Press 'f' to add coins."
        } else if !state.search_text.is_empty() {
            "No coins match your search."
        } else if state.watchlist.is_empty() {
            "No tickers configured. Press '+' to add one (e.g., BTCUSDT)."
        } else {
            "No market data available."
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", msg),
                Style::default().fg(theme.text_muted),
            )),
            area,
        );
        return;
    }

    // Adjust scroll offset so selected item is visible
    let scroll = if state.selected_index >= state.scroll_offset + max_rows {
        state.selected_index - max_rows + 1
    } else if state.selected_index < state.scroll_offset {
        state.selected_index
    } else {
        state.scroll_offset
    };

    let mut lines = Vec::new();
    for (i, coin) in visible.iter().enumerate().skip(scroll).take(max_rows) {
        let is_selected = i == state.selected_index;

        let fav = if coin.is_favorite { "★" } else { " " };
        let rank_str = format!("{}{:<3}", fav, coin.rank);
        let symbol = &coin.name; // Display base asset (BTC, ETH)
        let price = format_price(coin.current_price);
        let change_24h = format_change(coin.price_change_pct_24h);
        let vol = format_large_number(coin.quote_volume);
        let trades = format_trades(coin.trade_count);

        let base_style = if is_selected {
            Style::default()
                .bg(theme.table_row_selected_bg)
                .fg(theme.text_primary)
        } else {
            Style::default().fg(theme.text_primary)
        };

        let spans = vec![
            Span::styled(format!(" {:<4} {:<10} {:>14} ", rank_str, symbol, price), base_style),
            styled_change_span(&change_24h, coin.price_change_pct_24h, is_selected, theme),
            Span::styled(format!(" {:>14} {:>12} ", vol, trades), base_style),
        ];

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Create a colored span for a percentage change.
fn styled_change_span(text: &str, pct: f64, selected: bool, theme: &Theme) -> Span<'static> {
    let color = if pct > 0.5 {
        theme.success
    } else if pct < -0.5 {
        theme.danger
    } else {
        theme.text_dim
    };

    let arrow = if pct > 0.5 {
        "▲"
    } else if pct < -0.5 {
        "▼"
    } else {
        " "
    };

    let style = if selected {
        Style::default().fg(color).bg(theme.table_row_selected_bg)
    } else {
        Style::default().fg(color)
    };

    Span::styled(format!("{}{:>9}", arrow, text), style)
}

/// Format trade count with K/M suffix.
fn format_trades(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

// ── Detail View ──────────────────────────────────────────────────

fn render_detail(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let coin = match state.detail_coin {
        Some(ref c) => c,
        None => return,
    };

    let fav_icon = if coin.is_favorite { " ★" } else { "" };
    let title = format!(" {} ({}) {}", coin.name, coin.symbol, fav_icon);

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

    if inner.height < 6 || inner.width < 40 {
        return;
    }

    // Two-column layout
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(inner);

    // Left: chart + range selector
    render_chart_column(frame, columns[0], state, coin, theme);

    // Right: stats + AI
    render_stats_column(frame, columns[1], state, coin, theme);
}

fn render_chart_column(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    _coin: &super::models::CoinMarket,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // chart
            Constraint::Length(1), // range selector
        ])
        .split(area);

    // ── Price chart ──────────────────────────────────────────
    let chart_title = format!(" Price Chart ({}) ", state.chart_range.label());

    let chart_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            chart_title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let chart_inner = chart_block.inner(chunks[0]);
    frame.render_widget(chart_block, chunks[0]);

    if state.chart_loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading chart data...",
                Style::default().fg(theme.text_muted),
            )),
            chart_inner,
        );
    } else if let Some(ref history) = state.price_history {
        render_price_chart(frame, chart_inner, history, theme);
    } else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No chart data available",
                Style::default().fg(theme.text_muted),
            )),
            chart_inner,
        );
    }

    // ── Range selector ───────────────────────────────────────
    let mut range_spans = vec![Span::styled(" ", Style::default())];
    for (i, range) in ChartRange::all().iter().enumerate() {
        let is_active = *range == state.chart_range;
        let label = format!("[{}]{}", i + 1, range.label());
        let style = if is_active {
            Style::default()
                .fg(theme.bg_dark)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        range_spans.push(Span::styled(label, style));
        range_spans.push(Span::styled(" ", Style::default()));
    }
    frame.render_widget(Paragraph::new(Line::from(range_spans)), chunks[1]);
}

fn render_price_chart(
    frame: &mut Frame,
    area: Rect,
    history: &[super::models::PricePoint],
    theme: &Theme,
) {
    if history.is_empty() || area.height < 2 {
        return;
    }

    // Convert to sparkline data (using close prices)
    let prices: Vec<f64> = history.iter().map(|p| p.price).collect();
    let min = prices.iter().cloned().fold(f64::MAX, f64::min);
    let max = prices.iter().cloned().fold(f64::MIN, f64::max);
    let range = max - min;

    // Sample to fit width
    let width = area.width as usize;
    let step = prices.len() as f64 / width as f64;
    let mut sampled: Vec<u64> = Vec::with_capacity(width);
    for i in 0..width {
        let idx = ((i as f64) * step).min(prices.len() as f64 - 1.0) as usize;
        let val = prices[idx];
        let normalized = if range > 0.0 {
            ((val - min) / range * 1000.0) as u64
        } else {
            500
        };
        sampled.push(normalized);
    }

    // Determine color: green if price went up, red if down
    let color = if prices.last() >= prices.first() {
        theme.success
    } else {
        theme.danger
    };

    // Price labels on sides
    let price_label_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" ${:.2}", max),
            Style::default().fg(theme.text_muted),
        )),
        price_label_area[0],
    );

    // Sparkline in middle
    let sparkline = Sparkline::default()
        .data(&sampled)
        .max(1000)
        .style(Style::default().fg(color))
        .bar_set(symbols::bar::NINE_LEVELS);

    frame.render_widget(sparkline, price_label_area[1]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" ${:.2}", min),
            Style::default().fg(theme.text_muted),
        )),
        price_label_area[2],
    );
}

fn render_stats_column(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    coin: &super::models::CoinMarket,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // market stats
            Constraint::Min(6),     // AI analysis
        ])
        .split(area);

    // ── Market Stats ─────────────────────────────────────────
    let stats_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " 24h Stats ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let stats_inner = stats_block.inner(chunks[0]);
    frame.render_widget(stats_block, chunks[0]);

    let change_color = |v: f64| -> Color {
        if v > 0.5 {
            theme.success
        } else if v < -0.5 {
            theme.danger
        } else {
            theme.text_dim
        }
    };

    let mut stat_lines = Vec::new();
    stat_lines.push(stat_line(
        "Price",
        &format_price(coin.current_price),
        theme.text_primary,
        theme,
    ));
    stat_lines.push(stat_line(
        "24h Change",
        &format_change(coin.price_change_pct_24h),
        change_color(coin.price_change_pct_24h),
        theme,
    ));
    stat_lines.push(stat_line(
        "24h High",
        &format_price(coin.high_24h),
        theme.text_primary,
        theme,
    ));
    stat_lines.push(stat_line(
        "24h Low",
        &format_price(coin.low_24h),
        theme.text_primary,
        theme,
    ));
    stat_lines.push(stat_line(
        "24h Open",
        &format_price(coin.open_24h),
        theme.text_dim,
        theme,
    ));
    stat_lines.push(stat_line(
        "Volume",
        &format_large_number(coin.quote_volume),
        theme.text_primary,
        theme,
    ));
    stat_lines.push(stat_line(
        "Trades",
        &format_trades(coin.trade_count),
        theme.text_dim,
        theme,
    ));
    stat_lines.push(stat_line(
        "Avg Price",
        &format_price(coin.weighted_avg_price),
        theme.text_dim,
        theme,
    ));

    frame.render_widget(
        Paragraph::new(stat_lines).wrap(Wrap { trim: false }),
        stats_inner,
    );

    // ── AI Analysis ──────────────────────────────────────────
    let ai_title = if state.ai_loading {
        " AI Sentiment (analyzing...) "
    } else if state.ai_analysis.is_some() {
        " AI Sentiment "
    } else {
        " AI Sentiment [a] analyze "
    };

    let ai_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if state.ai_loading {
            theme.ai_accent
        } else {
            theme.border
        }))
        .title(Span::styled(
            ai_title,
            Style::default()
                .fg(theme.ai_accent)
                .add_modifier(Modifier::BOLD),
        ));

    let ai_inner = ai_block.inner(chunks[1]);
    frame.render_widget(ai_block, chunks[1]);

    if let Some(ref analysis) = state.ai_analysis {
        let wrapped: Vec<Line> = analysis
            .lines()
            .flat_map(|line| {
                if line.is_empty() {
                    vec![Line::raw("")]
                } else {
                    textwrap::wrap(line, ai_inner.width.saturating_sub(1) as usize)
                        .into_iter()
                        .map(|s| {
                            Line::from(Span::styled(
                                s.to_string(),
                                Style::default().fg(theme.ai_response),
                            ))
                        })
                        .collect()
                }
            })
            .collect();

        let visible_lines: Vec<Line> = wrapped.into_iter().skip(state.detail_scroll).collect();

        frame.render_widget(
            Paragraph::new(visible_lines).wrap(Wrap { trim: false }),
            ai_inner,
        );
    } else if state.ai_loading {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Analyzing market data...",
                Style::default()
                    .fg(theme.ai_accent)
                    .add_modifier(Modifier::ITALIC),
            )),
            ai_inner,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Press 'a' to request AI sentiment analysis",
                Style::default().fg(theme.text_muted),
            )),
            ai_inner,
        );
    }
}

/// Build a stat line: "  Label ........... value"
fn stat_line<'a>(label: &str, value: &str, value_color: Color, theme: &Theme) -> Line<'a> {
    let label_width = 12;
    let padded_label = format!(" {:.<label_width$}", format!("{} ", label));
    Line::from(vec![
        Span::styled(padded_label, Style::default().fg(theme.text_dim)),
        Span::styled(format!(" {}", value), Style::default().fg(value_color)),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
