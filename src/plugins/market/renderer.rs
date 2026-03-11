//! Market plugin renderer: list view, detail view, and overlays.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::models::{format_change, format_large_number, format_price, ChartRange, PricePoint};
use super::state::{MarketState, MarketView};
use crate::constants::{
    CANDLE_BODY_CHAR, CANDLE_COL_WIDTH, CANDLE_HALF_BOTTOM, CANDLE_HALF_TOP, CANDLE_MIN_CHART_ROWS,
    CANDLE_PRICE_LABEL_WIDTH, CANDLE_WICK_CHAR,
};
use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Top-level render dispatcher.
pub fn render_market(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    match state.view {
        MarketView::List => render_list(frame, area, state, theme, glyphs),
        MarketView::Detail => render_detail(frame, area, state, theme, glyphs),
    }
}

/// Render overlays (add ticker input modal).
pub fn render_market_overlay(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    if state.add_ticker_mode {
        render_add_ticker_modal(frame, area, state, theme, glyphs);
    }
}

fn render_add_ticker_modal(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
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
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(Span::styled(
            "Enter symbol (e.g., BTCUSDT):",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(vec![
            Span::styled(
                &state.add_ticker_text,
                Style::default().fg(theme.text_primary),
            ),
            Span::styled(
                glyphs.cursor,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]),
        Line::from(Span::styled(
            "[Enter] Add  [Esc] Cancel",
            Style::default().fg(theme.text_muted),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

// ── List View ────────────────────────────────────────────────────

fn render_list(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme, glyphs: &Glyphs) {
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
    render_info_bar(frame, chunks[chunk_idx], state, theme, glyphs);
    chunk_idx += 1;

    // ── Search bar (conditional) ─────────────────────────────
    if has_search {
        render_search_bar(frame, chunks[chunk_idx], state, theme, glyphs);
        chunk_idx += 1;
    }

    // ── Column headers ───────────────────────────────────────
    render_column_headers(frame, chunks[chunk_idx], state, theme);
    chunk_idx += 1;

    // ── Table rows ───────────────────────────────────────────
    render_table_rows(frame, chunks[chunk_idx], state, theme, glyphs);
}

fn render_info_bar(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
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

    spans.push(Span::styled("│ ", Style::default().fg(theme.text_muted)));

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
            format!(
                "{}{} {} favorites",
                glyphs.separator, glyphs.star, fav_count
            ),
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
            "{}Sort: {}{}",
            glyphs.separator,
            state.sort_column.label(),
            if state.sort_ascending {
                glyphs.sort_asc
            } else {
                glyphs.sort_desc
            }
        ),
        Style::default().fg(theme.text_muted),
    ));

    // Add ticker hint
    spans.push(Span::styled(
        format!("{}[+] add ticker  [d] remove", glyphs.separator),
        Style::default().fg(theme.text_muted),
    ));

    // Error
    if let Some(ref err) = state.error {
        spans.push(Span::styled(
            format!("{}Error: {}", glyphs.separator, truncate(err, 30)),
            Style::default().fg(theme.danger),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_search_bar(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let spans = vec![
        Span::styled(
            " /",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&state.search_text, Style::default().fg(theme.text_primary)),
        Span::styled(
            glyphs.cursor,
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

fn render_table_rows(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
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

        let fav = if coin.is_favorite { glyphs.star } else { " " };
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
            Span::styled(
                format!(" {:<4} {:<10} {:>14} ", rank_str, symbol, price),
                base_style,
            ),
            styled_change_span(
                &change_24h,
                coin.price_change_pct_24h,
                is_selected,
                theme,
                glyphs,
            ),
            Span::styled(format!(" {:>14} {:>12} ", vol, trades), base_style),
        ];

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Create a colored span for a percentage change.
fn styled_change_span(
    text: &str,
    pct: f64,
    selected: bool,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Span<'static> {
    let color = if pct > 0.5 {
        theme.success
    } else if pct < -0.5 {
        theme.danger
    } else {
        theme.text_dim
    };

    let arrow = if pct > 0.5 {
        glyphs.price_up
    } else if pct < -0.5 {
        glyphs.price_down
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

fn render_detail(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    let coin = match state.detail_coin {
        Some(ref c) => c,
        None => return,
    };

    let fav_icon = if coin.is_favorite {
        format!(" {}", glyphs.star)
    } else {
        String::new()
    };
    let title = format!(" {} ({}){}", coin.name, coin.symbol, fav_icon);

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
    render_chart_column(frame, columns[0], state, coin, theme, glyphs);

    // Right: stats + AI
    render_stats_column(frame, columns[1], state, coin, theme);
}

fn render_chart_column(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    _coin: &super::models::CoinMarket,
    theme: &Theme,
    _glyphs: &Glyphs,
) {
    let has_news = !state.news_items.is_empty() || state.news_loading;
    let chunks = if has_news {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),    // chart
                Constraint::Length(1), // range selector
                Constraint::Length(8), // news feed (#6)
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),    // chart
                Constraint::Length(1), // range selector
            ])
            .split(area)
    };

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
        render_candlestick_chart(frame, chart_inner, history, theme);
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

    // ── News feed (#6) ───────────────────────────────────────
    if has_news {
        render_news_panel(frame, chunks[2], state, theme);
    }
}

/// Render the news feed panel (#6).
fn render_news_panel(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let news_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " News ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let news_inner = news_block.inner(area);
    frame.render_widget(news_block, area);

    if state.news_loading && state.news_items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Fetching news...",
                Style::default().fg(theme.text_muted),
            )),
            news_inner,
        );
        return;
    }

    if state.news_items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No news available",
                Style::default().fg(theme.text_muted),
            )),
            news_inner,
        );
        return;
    }

    let max_items = news_inner.height as usize;
    let mut lines = Vec::with_capacity(max_items);

    for item in state.news_items.iter().take(max_items) {
        let sentiment_icon = match item.sentiment.as_deref() {
            Some("positive") => Span::styled("+", Style::default().fg(theme.success)),
            Some("negative") => Span::styled("-", Style::default().fg(theme.danger)),
            _ => Span::styled(" ", Style::default().fg(theme.text_muted)),
        };

        let time_ago = format_time_ago(item.published_at);

        lines.push(Line::from(vec![
            Span::styled(" ", Style::default()),
            sentiment_icon,
            Span::styled(" ", Style::default()),
            Span::styled(
                truncate(&item.title, news_inner.width.saturating_sub(20) as usize),
                Style::default().fg(theme.text_primary),
            ),
            Span::styled(
                format!("  {} | {}", item.source, time_ago),
                Style::default().fg(theme.text_muted),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), news_inner);
}

/// Format epoch seconds to a human-readable "X ago" string.
fn format_time_ago(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = (now - epoch_secs).max(0);

    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// A single OHLC candle bucketed for rendering.
#[derive(Debug, Clone)]
struct RenderCandle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

impl RenderCandle {
    /// Whether this candle is bullish (close >= open).
    fn is_bullish(&self) -> bool {
        self.close >= self.open
    }

    /// Body top (higher of open/close).
    fn body_top(&self) -> f64 {
        self.open.max(self.close)
    }

    /// Body bottom (lower of open/close).
    fn body_bottom(&self) -> f64 {
        self.open.min(self.close)
    }
}

/// Bucket `history` into `n` candles by aggregating OHLC across buckets.
fn bucket_candles(history: &[PricePoint], n: usize) -> Vec<RenderCandle> {
    if history.is_empty() || n == 0 {
        return Vec::new();
    }

    let step = history.len() as f64 / n as f64;
    let mut candles = Vec::with_capacity(n);

    for i in 0..n {
        let start = (i as f64 * step) as usize;
        let end = (((i + 1) as f64 * step) as usize).min(history.len());
        if start >= end {
            continue;
        }

        let slice = &history[start..end];
        let open = slice[0].open;
        let close = slice[slice.len() - 1].price;
        let high = slice.iter().map(|p| p.high).fold(f64::MIN, f64::max);
        let low = slice.iter().map(|p| p.low).fold(f64::MAX, f64::min);

        candles.push(RenderCandle {
            open,
            high,
            low,
            close,
        });
    }

    candles
}

/// Map a price value to a row index (0 = top row, max = bottom row).
/// Returns None if the price is outside the range.
fn price_to_row(price: f64, min_price: f64, price_range: f64, chart_height: u16) -> u16 {
    if price_range <= 0.0 || chart_height == 0 {
        return chart_height / 2;
    }
    let ratio = (price - min_price) / price_range;
    // Invert: high prices → low row numbers (top of terminal)
    let row = ((1.0 - ratio) * (chart_height.saturating_sub(1) as f64)).round() as u16;
    row.min(chart_height.saturating_sub(1))
}

/// Format a price for the Y-axis label (compact).
fn format_axis_price(price: f64) -> String {
    if price >= 10000.0 {
        format!("{:.0}", price)
    } else if price >= 100.0 {
        format!("{:.1}", price)
    } else if price >= 1.0 {
        format!("{:.2}", price)
    } else {
        format!("{:.4}", price)
    }
}

/// Render an OHLC candlestick chart into the given area.
///
/// Layout: `[chart area][price labels]`
/// Each candle occupies `CANDLE_COL_WIDTH` columns. Bullish candles are
/// green, bearish candles are red. Wicks are drawn with `│`, bodies
/// with `█`.
fn render_candlestick_chart(frame: &mut Frame, area: Rect, history: &[PricePoint], theme: &Theme) {
    if history.len() < 2
        || area.height < CANDLE_MIN_CHART_ROWS
        || area.width < CANDLE_PRICE_LABEL_WIDTH + 4
    {
        return;
    }

    // Split: chart area | price label column
    let label_width = CANDLE_PRICE_LABEL_WIDTH;
    let chart_width = area.width.saturating_sub(label_width);
    let chart_height = area.height;

    if chart_width < CANDLE_COL_WIDTH * 2 {
        return;
    }

    let chart_area = Rect::new(area.x, area.y, chart_width, chart_height);
    let label_area = Rect::new(area.x + chart_width, area.y, label_width, chart_height);

    // Determine how many candles fit
    let max_candles = (chart_width / CANDLE_COL_WIDTH) as usize;
    let n_candles = max_candles.min(history.len());

    let candles = bucket_candles(history, n_candles);
    if candles.is_empty() {
        return;
    }

    // Find global min/max across all candles
    let global_high = candles.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let global_low = candles.iter().map(|c| c.low).fold(f64::MAX, f64::min);

    // Add 2% padding so wicks don't touch edges
    let price_range = global_high - global_low;
    let padding = if price_range > 0.0 {
        price_range * 0.02
    } else {
        global_high * 0.01
    };
    let min_price = global_low - padding;
    let max_price = global_high + padding;
    let padded_range = max_price - min_price;

    // Draw candles directly into the frame buffer
    let buf = frame.buffer_mut();
    draw_candles_to_buffer(buf, &chart_area, &candles, min_price, padded_range, theme);

    // Draw price labels (top, mid, bottom)
    let labels = build_price_labels(min_price, max_price, chart_height, theme);
    frame.render_widget(Paragraph::new(labels), label_area);
}

/// Draw individual candle sticks directly into the terminal buffer.
fn draw_candles_to_buffer(
    buf: &mut Buffer,
    area: &Rect,
    candles: &[RenderCandle],
    min_price: f64,
    price_range: f64,
    theme: &Theme,
) {
    let chart_height = area.height;

    for (i, candle) in candles.iter().enumerate() {
        let col = area.x + (i as u16) * CANDLE_COL_WIDTH;
        if col >= area.x + area.width {
            break;
        }

        let color = if candle.is_bullish() {
            theme.success
        } else {
            theme.danger
        };

        let wick_style = Style::default().fg(color);
        let body_style = Style::default().fg(color);

        // Map prices to rows
        let high_row = price_to_row(candle.high, min_price, price_range, chart_height);
        let low_row = price_to_row(candle.low, min_price, price_range, chart_height);
        let body_top_row = price_to_row(candle.body_top(), min_price, price_range, chart_height);
        let body_bot_row = price_to_row(candle.body_bottom(), min_price, price_range, chart_height);

        // Draw upper wick (from high_row to body_top_row, exclusive of body)
        for row in high_row..body_top_row {
            let y = area.y + row;
            if y < area.y + area.height && col < area.x + area.width {
                buf[(col, y)]
                    .set_char(CANDLE_WICK_CHAR)
                    .set_style(wick_style);
            }
        }

        // Draw body (from body_top_row to body_bot_row, inclusive)
        let body_end = body_bot_row.max(body_top_row); // handle zero-height doji
        for row in body_top_row..=body_end {
            let y = area.y + row;
            if y < area.y + area.height && col < area.x + area.width {
                buf[(col, y)]
                    .set_char(CANDLE_BODY_CHAR)
                    .set_style(body_style);
            }
        }

        // If body is zero-height (doji), use a half-block
        if body_top_row == body_bot_row {
            let y = area.y + body_top_row;
            if y < area.y + area.height && col < area.x + area.width {
                let doji_char = if candle.is_bullish() {
                    CANDLE_HALF_TOP
                } else {
                    CANDLE_HALF_BOTTOM
                };
                buf[(col, y)].set_char(doji_char).set_style(body_style);
            }
        }

        // Draw lower wick (from body_bot_row+1 to low_row, inclusive)
        for row in (body_bot_row + 1)..=low_row {
            let y = area.y + row;
            if y < area.y + area.height && col < area.x + area.width {
                buf[(col, y)]
                    .set_char(CANDLE_WICK_CHAR)
                    .set_style(wick_style);
            }
        }
    }
}

/// Build Y-axis price labels for the chart.
fn build_price_labels<'a>(
    min_price: f64,
    max_price: f64,
    chart_height: u16,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::with_capacity(chart_height as usize);
    let label_style = Style::default().fg(theme.text_muted);

    // Show labels at top, middle, and bottom
    let mid_price = (min_price + max_price) / 2.0;
    let mid_row = chart_height / 2;

    for row in 0..chart_height {
        let label = if row == 0 {
            format!(" ${}", format_axis_price(max_price))
        } else if row == mid_row {
            format!(" ${}", format_axis_price(mid_price))
        } else if row == chart_height - 1 {
            format!(" ${}", format_axis_price(min_price))
        } else {
            String::new()
        };

        lines.push(Line::from(Span::styled(label, label_style)));
    }

    lines
}

fn render_stats_column(
    frame: &mut Frame,
    area: Rect,
    state: &MarketState,
    coin: &super::models::CoinMarket,
    theme: &Theme,
) {
    let has_range_stats = state.range_stats.is_some();
    let chunks = if has_range_stats {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10), // market stats
                Constraint::Length(10), // range analysis (#7)
                Constraint::Min(4),     // AI analysis
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10), // market stats
                Constraint::Min(6),     // AI analysis
            ])
            .split(area)
    };

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

    // ── Range Analysis (#7) ──────────────────────────────────
    let ai_chunk_idx = if has_range_stats {
        render_range_analysis(frame, chunks[1], state, theme);
        2
    } else {
        1
    };

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

    let ai_inner = ai_block.inner(chunks[ai_chunk_idx]);
    frame.render_widget(ai_block, chunks[ai_chunk_idx]);

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

/// Render the Range Analysis panel (#7).
fn render_range_analysis(frame: &mut Frame, area: Rect, state: &MarketState, theme: &Theme) {
    let range_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            format!(" Range Analysis ({}) ", state.chart_range.label()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));

    let range_inner = range_block.inner(area);
    frame.render_widget(range_block, area);

    if let Some(ref stats) = state.range_stats {
        let trend_color = if stats.trend_pct > 0.5 {
            theme.success
        } else if stats.trend_pct < -0.5 {
            theme.danger
        } else {
            theme.text_dim
        };

        let total_candles = stats.bullish_count + stats.bearish_count;
        let bull_pct = if total_candles > 0 {
            (stats.bullish_count as f64 / total_candles as f64) * 100.0
        } else {
            0.0
        };

        let mut lines = Vec::new();
        lines.push(stat_line(
            "Range High",
            &format_price(stats.range_high),
            theme.success,
            theme,
        ));
        lines.push(stat_line(
            "Range Low",
            &format_price(stats.range_low),
            theme.danger,
            theme,
        ));
        lines.push(stat_line(
            "Range %",
            &format!("{:.2}%", stats.range_pct),
            theme.text_primary,
            theme,
        ));
        lines.push(stat_line(
            "Volatility",
            &format!("{:.3}%", stats.volatility_pct),
            theme.warning,
            theme,
        ));
        lines.push(stat_line(
            "Trend",
            &format_change(stats.trend_pct),
            trend_color,
            theme,
        ));
        lines.push(stat_line(
            "Avg Close",
            &format_price(stats.avg_close),
            theme.text_dim,
            theme,
        ));
        lines.push(stat_line(
            "Bull/Bear",
            &format!(
                "{}/{} ({:.0}%)",
                stats.bullish_count, stats.bearish_count, bull_pct
            ),
            theme.text_primary,
            theme,
        ));

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            range_inner,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_point(open: f64, high: f64, low: f64, close: f64) -> PricePoint {
        PricePoint {
            timestamp: 0,
            open,
            high,
            low,
            price: close,
        }
    }

    // ── bucket_candles tests ─────────────────────────────────

    #[test]
    fn bucket_candles_empty_history() {
        let result = bucket_candles(&[], 5);
        assert!(result.is_empty());
    }

    #[test]
    fn bucket_candles_zero_buckets() {
        let history = vec![make_point(100.0, 110.0, 90.0, 105.0)];
        let result = bucket_candles(&history, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn bucket_candles_one_to_one() {
        let history = vec![
            make_point(100.0, 110.0, 90.0, 105.0),
            make_point(105.0, 115.0, 95.0, 112.0),
            make_point(112.0, 120.0, 100.0, 108.0),
        ];
        let candles = bucket_candles(&history, 3);
        assert_eq!(candles.len(), 3);
        assert!((candles[0].open - 100.0).abs() < 0.01);
        assert!((candles[0].close - 105.0).abs() < 0.01);
        assert!((candles[2].open - 112.0).abs() < 0.01);
        assert!((candles[2].close - 108.0).abs() < 0.01);
    }

    #[test]
    fn bucket_candles_aggregates_ohlc() {
        // 4 points bucketed into 2 candles
        let history = vec![
            make_point(100.0, 115.0, 95.0, 110.0),
            make_point(110.0, 120.0, 105.0, 108.0),
            make_point(108.0, 125.0, 100.0, 118.0),
            make_point(118.0, 130.0, 110.0, 122.0),
        ];
        let candles = bucket_candles(&history, 2);
        assert_eq!(candles.len(), 2);

        // First bucket: points 0-1
        assert!((candles[0].open - 100.0).abs() < 0.01); // open of first point
        assert!((candles[0].close - 108.0).abs() < 0.01); // close of last point in bucket
        assert!((candles[0].high - 120.0).abs() < 0.01); // max high
        assert!((candles[0].low - 95.0).abs() < 0.01); // min low

        // Second bucket: points 2-3
        assert!((candles[1].open - 108.0).abs() < 0.01);
        assert!((candles[1].close - 122.0).abs() < 0.01);
        assert!((candles[1].high - 130.0).abs() < 0.01);
        assert!((candles[1].low - 100.0).abs() < 0.01);
    }

    // ── RenderCandle tests ───────────────────────────────────

    #[test]
    fn render_candle_bullish() {
        let candle = RenderCandle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
        };
        assert!(candle.is_bullish());
        assert!((candle.body_top() - 105.0).abs() < 0.01);
        assert!((candle.body_bottom() - 100.0).abs() < 0.01);
    }

    #[test]
    fn render_candle_bearish() {
        let candle = RenderCandle {
            open: 105.0,
            high: 110.0,
            low: 90.0,
            close: 100.0,
        };
        assert!(!candle.is_bullish());
        assert!((candle.body_top() - 105.0).abs() < 0.01);
        assert!((candle.body_bottom() - 100.0).abs() < 0.01);
    }

    #[test]
    fn render_candle_doji() {
        let candle = RenderCandle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 100.0,
        };
        assert!(candle.is_bullish()); // close >= open
        assert!((candle.body_top() - candle.body_bottom()).abs() < 0.01);
    }

    // ── price_to_row tests ───────────────────────────────────

    #[test]
    fn price_to_row_maps_max_to_top() {
        let row = price_to_row(200.0, 100.0, 100.0, 20);
        assert_eq!(row, 0);
    }

    #[test]
    fn price_to_row_maps_min_to_bottom() {
        let row = price_to_row(100.0, 100.0, 100.0, 20);
        assert_eq!(row, 19);
    }

    #[test]
    fn price_to_row_maps_mid_to_middle() {
        let row = price_to_row(150.0, 100.0, 100.0, 20);
        // ratio = 0.5, inverted = 0.5 * 19 = ~10
        assert_eq!(row, 10);
    }

    #[test]
    fn price_to_row_zero_range() {
        let row = price_to_row(100.0, 100.0, 0.0, 20);
        assert_eq!(row, 10); // midpoint
    }

    // ── format_axis_price tests ──────────────────────────────

    #[test]
    fn format_axis_price_large() {
        assert_eq!(format_axis_price(67000.0), "67000");
    }

    #[test]
    fn format_axis_price_medium() {
        assert_eq!(format_axis_price(350.5), "350.5");
    }

    #[test]
    fn format_axis_price_small() {
        assert_eq!(format_axis_price(1.2345), "1.23");
    }

    #[test]
    fn format_axis_price_tiny() {
        assert_eq!(format_axis_price(0.1234), "0.1234");
    }

    // ── format_time_ago tests ────────────────────────────────

    #[test]
    fn format_time_ago_recent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(format_time_ago(now), "just now");
    }

    #[test]
    fn format_time_ago_minutes() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(format_time_ago(now - 300), "5m ago");
    }

    #[test]
    fn format_time_ago_hours() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(format_time_ago(now - 7200), "2h ago");
    }

    #[test]
    fn format_time_ago_days() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(format_time_ago(now - 172800), "2d ago");
    }

    // ── truncate tests ───────────────────────────────────────

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("this is a very long headline", 15);
        assert!(result.len() <= 15);
        assert!(result.ends_with("..."));
    }
}
