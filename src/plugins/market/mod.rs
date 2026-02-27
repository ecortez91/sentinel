//! Market data plugin: crypto prices from Binance.
//!
//! Provides a full-screen tab with configurable watchlist,
//! price charts, and AI-powered sentiment analysis.

pub mod client;
pub mod models;
pub mod renderer;
pub mod state;

use std::collections::HashSet;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, Frame};
use tokio::sync::{mpsc, watch};

use crate::plugins::{Plugin, PluginAction};
use crate::ui::theme::Theme;

use client::BinanceClient;
use models::CoinMarket;
use state::{ChartRange, MarketState, MarketView};

/// Market data plugin implementing the Plugin trait.
pub struct MarketPlugin {
    state: MarketState,
    client: BinanceClient,
    /// Receiver for background market data polling.
    market_rx: mpsc::UnboundedReceiver<MarketPollResult>,
    /// Sender cloned into the background polling task.
    market_tx: mpsc::UnboundedSender<MarketPollResult>,
    /// Receiver for on-demand chart data fetches.
    chart_rx: mpsc::UnboundedReceiver<ChartResult>,
    /// Sender for chart data fetches.
    chart_tx: mpsc::UnboundedSender<ChartResult>,
    /// Whether the background poller has been spawned.
    poller_spawned: bool,
    /// Sends updated watchlist to the background poller.
    watchlist_tx: watch::Sender<Vec<String>>,
    /// Polling interval in seconds.
    poll_interval_secs: u64,
    /// Whether the plugin is enabled.
    enabled: bool,
    /// Callback to save watchlist changes.
    on_watchlist_change: Option<Box<dyn Fn(&[String]) + Send + Sync>>,
}

/// Result from a background market data poll.
enum MarketPollResult {
    Data(Vec<CoinMarket>),
    Error(String),
}

/// Result from an on-demand chart fetch.
enum ChartResult {
    Data(Vec<models::PricePoint>),
    Error(String),
}

impl MarketPlugin {
    /// Create a new market plugin with the given configuration.
    pub fn new(
        enabled: bool,
        poll_interval_secs: u64,
        watchlist: Vec<String>,
        favorites: HashSet<String>,
    ) -> Self {
        let (market_tx, market_rx) = mpsc::unbounded_channel();
        let (chart_tx, chart_rx) = mpsc::unbounded_channel();
        let (watchlist_tx, _) = watch::channel(watchlist.clone());

        Self {
            state: MarketState::new(favorites, watchlist),
            client: BinanceClient::new(),
            market_rx,
            market_tx,
            chart_rx,
            chart_tx,
            poller_spawned: false,
            watchlist_tx,
            poll_interval_secs,
            enabled,
            on_watchlist_change: None,
        }
    }

    /// Set a callback for when the watchlist changes.
    #[allow(dead_code)]
    pub fn on_watchlist_change<F>(&mut self, callback: F)
    where
        F: Fn(&[String]) + Send + Sync + 'static,
    {
        self.on_watchlist_change = Some(Box::new(callback));
    }

    /// Get the current watchlist.
    #[allow(dead_code)]
    pub fn watchlist(&self) -> &[String] {
        &self.state.watchlist
    }

    /// Spawn the background polling task. Called once on first tick.
    pub fn spawn_poller(&mut self) {
        if self.poller_spawned || !self.enabled {
            return;
        }
        self.poller_spawned = true;

        let tx = self.market_tx.clone();
        let client = self.client.clone();
        let interval = self.poll_interval_secs;
        let mut watchlist_rx = self.watchlist_tx.subscribe();

        tokio::spawn(async move {
            // Initial fetch using current watchlist
            let watchlist = watchlist_rx.borrow_and_update().clone();
            Self::fetch_and_send(&client, &watchlist, &tx).await;

            // Polling loop — re-read watchlist each cycle
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                let watchlist = watchlist_rx.borrow_and_update().clone();
                Self::fetch_and_send(&client, &watchlist, &tx).await;
            }
        });
    }

    async fn fetch_and_send(
        client: &BinanceClient,
        watchlist: &[String],
        tx: &mpsc::UnboundedSender<MarketPollResult>,
    ) {
        if watchlist.is_empty() {
            return;
        }

        match client.fetch_tickers(watchlist).await {
            Ok(coins) => {
                if tx.send(MarketPollResult::Data(coins)).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(MarketPollResult::Error(e.to_string()));
            }
        }
    }

    /// Fetch data for a single new ticker.
    fn fetch_single_ticker(&self, symbol: &str) {
        let tx = self.market_tx.clone();
        let client = self.client.clone();
        let symbol = symbol.to_string();

        tokio::spawn(async move {
            match client.fetch_ticker(&symbol).await {
                Ok(Some(coin)) => {
                    let _ = tx.send(MarketPollResult::Data(vec![coin]));
                }
                Ok(None) => {
                    let _ = tx.send(MarketPollResult::Error(format!(
                        "Symbol '{}' not found",
                        symbol
                    )));
                }
                Err(e) => {
                    let _ = tx.send(MarketPollResult::Error(e.to_string()));
                }
            }
        });
    }

    /// Fetch chart data for a coin on demand.
    fn fetch_chart(&self, symbol: &str, range: &ChartRange) {
        let tx = self.chart_tx.clone();
        let client = self.client.clone();
        let symbol = symbol.to_string();
        let interval = range.interval().to_string();
        let limit = range.limit();

        tokio::spawn(async move {
            match client.fetch_klines(&symbol, &interval, limit).await {
                Ok(points) => {
                    let _ = tx.send(ChartResult::Data(points));
                }
                Err(e) => {
                    let _ = tx.send(ChartResult::Error(e.to_string()));
                }
            }
        });
    }

    /// Build the AI analysis context for the currently selected coin.
    fn build_ai_context(&self) -> Option<String> {
        let coin = self.state.detail_coin.as_ref()?;
        let mut ctx = String::with_capacity(1024);
        ctx.push_str(&format!(
            "Analyze the current market data for {} ({}).\n\n",
            coin.name,
            coin.symbol
        ));
        ctx.push_str("=== BINANCE 24H DATA ===\n");
        ctx.push_str(&format!("Price: ${:.8}\n", coin.current_price));
        ctx.push_str(&format!("24h Change: {:.2}%\n", coin.price_change_pct_24h));
        ctx.push_str(&format!("24h High: ${:.8}\n", coin.high_24h));
        ctx.push_str(&format!("24h Low: ${:.8}\n", coin.low_24h));
        ctx.push_str(&format!("24h Open: ${:.8}\n", coin.open_24h));
        ctx.push_str(&format!("24h Volume (Quote): ${:.2}\n", coin.quote_volume));
        ctx.push_str(&format!("24h Trades: {}\n", coin.trade_count));
        ctx.push_str(&format!("Weighted Avg Price: ${:.8}\n", coin.weighted_avg_price));

        // Calculate some derived metrics
        let range_pct = if coin.low_24h > 0.0 {
            ((coin.high_24h - coin.low_24h) / coin.low_24h) * 100.0
        } else {
            0.0
        };
        ctx.push_str(&format!("24h Range: {:.2}%\n", range_pct));

        ctx.push_str("\nProvide a brief, objective sentiment analysis covering:\n");
        ctx.push_str("1. Price action assessment (bullish/bearish/neutral signals)\n");
        ctx.push_str("2. Volume analysis (is volume supporting the price movement?)\n");
        ctx.push_str("3. Key levels to watch (based on 24h high/low)\n");
        ctx.push_str("4. Risk factors\n");
        ctx.push_str("Keep it concise (under 200 words). Be data-driven, not speculative.\n");

        Some(ctx)
    }

    /// Notify callback and background poller about watchlist changes.
    fn notify_watchlist_change(&self) {
        // Update the background poller with the current watchlist
        let _ = self.watchlist_tx.send(self.state.watchlist.clone());
        if let Some(ref callback) = self.on_watchlist_change {
            callback(&self.state.watchlist);
        }
    }
}

impl Plugin for MarketPlugin {
    fn id(&self) -> &str {
        "market"
    }

    fn tab_label(&self) -> &str {
        "Market"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn tick(&mut self) {
        // Spawn poller on first tick
        self.spawn_poller();

        // Drain market data channel
        while let Ok(result) = self.market_rx.try_recv() {
            match result {
                MarketPollResult::Data(coins) => {
                    // Merge favorites into the new data
                    let favorites = &self.state.favorites;
                    let mut coins = coins;
                    for coin in &mut coins {
                        coin.is_favorite = favorites.contains(&coin.symbol);
                    }
                    // If this is a single ticker add, merge into existing
                    if coins.len() == 1 {
                        let coin = coins.remove(0);
                        if !self.state.coins.iter().any(|c| c.symbol == coin.symbol) {
                            // Update rank based on position in watchlist
                            let mut coin = coin;
                            if let Some(pos) = self.state.watchlist.iter().position(|t| *t == coin.symbol) {
                                coin.rank = pos as u32 + 1;
                            }
                            self.state.coins.push(coin);
                        }
                    } else {
                        self.state.coins = coins;
                    }
                    self.state.loading = false;
                    self.state.error = None;
                    self.state.last_updated = Some(Instant::now());
                }
                MarketPollResult::Error(e) => {
                    self.state.loading = false;
                    self.state.error = Some(e);
                }
            }
        }

        // Drain chart data channel
        while let Ok(result) = self.chart_rx.try_recv() {
            match result {
                ChartResult::Data(points) => {
                    self.state.price_history = Some(points);
                    self.state.chart_loading = false;
                }
                ChartResult::Error(_e) => {
                    self.state.chart_loading = false;
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PluginAction {
        // Add ticker mode has highest priority
        if self.state.add_ticker_mode {
            return self.handle_add_ticker_key(key);
        }

        // Search mode has priority
        if self.state.search_mode {
            return self.handle_search_key(key);
        }

        match self.state.view {
            MarketView::List => self.handle_list_key(key),
            MarketView::Detail => self.handle_detail_key(key),
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        renderer::render_market(frame, area, &self.state, theme);
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        renderer::render_market_overlay(frame, area, &self.state, theme);
    }

    fn status_bar_hints(&self) -> Vec<(&str, &str)> {
        match self.state.view {
            MarketView::List => {
                vec![
                    ("Enter", "Detail"),
                    ("/", "Search"),
                    ("f", "Favorite"),
                    ("s", "Sort"),
                    ("+", "Add ticker"),
                    ("d", "Remove"),
                ]
            }
            MarketView::Detail => {
                vec![
                    ("Esc", "Back"),
                    ("1-5", "Chart range"),
                    ("f", "Favorite"),
                    ("a", "AI Analysis"),
                ]
            }
        }
    }

    fn help_entries(&self) -> Vec<(&str, &str)> {
        vec![
            ("Enter", "Open coin detail view"),
            ("Esc", "Back to list / close modal"),
            ("/", "Filter coins by name/symbol"),
            ("f / F", "Toggle favorite"),
            ("*", "Show favorites only"),
            ("s", "Cycle sort column"),
            ("r", "Reverse sort direction"),
            ("+", "Add new ticker (e.g., BTCUSDT)"),
            ("d", "Remove selected ticker from watchlist"),
            ("1-5", "Chart range (1H/4H/1D/7D/30D)"),
            ("a", "AI sentiment analysis (detail view)"),
        ]
    }

    fn receive_ai_chunk(&mut self, chunk: &str) {
        if let Some(ref mut analysis) = self.state.ai_analysis {
            analysis.push_str(chunk);
        } else {
            self.state.ai_analysis = Some(chunk.to_string());
        }
    }

    fn ai_analysis_done(&mut self) {
        self.state.ai_loading = false;
    }

    fn ai_analysis_error(&mut self, error: &str) {
        self.state.ai_loading = false;
        self.state.ai_analysis = Some(format!("Error: {}", error));
    }

    fn favorites(&self) -> Option<&std::collections::HashSet<String>> {
        Some(&self.state.favorites)
    }

    fn commands(&self) -> Vec<(&str, &str)> {
        vec![
            ("market", "Show market overview"),
            ("market add <TICKER>", "Add ticker to watchlist"),
            ("market remove <TICKER>", "Remove ticker from watchlist"),
        ]
    }

    fn execute_command(&mut self, cmd: &str, args: &str) -> Option<String> {
        match cmd {
            "market" => {
                let args = args.trim();
                if args.starts_with("add ") {
                    let ticker = args.strip_prefix("add ").unwrap().trim().to_uppercase();
                    if self.state.add_ticker(&ticker) {
                        self.fetch_single_ticker(&ticker);
                        self.notify_watchlist_change();
                        Some(format!("Added {} to watchlist", ticker))
                    } else {
                        Some(format!("{} is already in watchlist", ticker))
                    }
                } else if args.starts_with("remove ") {
                    let ticker = args.strip_prefix("remove ").unwrap().trim().to_uppercase();
                    if let Some(pos) = self.state.watchlist.iter().position(|t| t == &ticker) {
                        self.state.watchlist.remove(pos);
                        self.state.coins.retain(|c| c.symbol != ticker);
                        self.notify_watchlist_change();
                        Some(format!("Removed {} from watchlist", ticker))
                    } else {
                        Some(format!("{} not in watchlist", ticker))
                    }
                } else {
                    // Show overview
                    let mut lines = vec!["# Market Overview (Binance)".to_string()];
                    if self.state.coins.is_empty() {
                        lines.push("No market data loaded yet.".to_string());
                        lines.push(format!("Watchlist: {:?}", self.state.watchlist));
                    } else {
                        for coin in &self.state.coins {
                            let change = coin.price_change_pct_24h;
                            let arrow = if change >= 0.0 { "+" } else { "" };
                            lines.push(format!(
                                "{:>8} ${:<14.8} {}{:.2}%",
                                coin.name,
                                coin.current_price,
                                arrow,
                                change,
                            ));
                        }
                    }
                    Some(lines.join("\n"))
                }
            }
            _ => None,
        }
    }
}

// ── Key handling methods ─────────────────────────────────────────

impl MarketPlugin {
    fn handle_list_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.move_selection_up();
                PluginAction::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.move_selection_down();
                PluginAction::Consumed
            }
            KeyCode::PageUp => {
                self.state.page_up();
                PluginAction::Consumed
            }
            KeyCode::PageDown => {
                self.state.page_down();
                PluginAction::Consumed
            }
            KeyCode::Home => {
                self.state.selected_index = 0;
                self.state.scroll_offset = 0;
                PluginAction::Consumed
            }
            KeyCode::End => {
                let len = self.state.visible_coins().len();
                self.state.selected_index = len.saturating_sub(1);
                PluginAction::Consumed
            }
            KeyCode::Enter => {
                // Open detail view for selected coin
                if let Some(coin) = self.state.selected_coin().cloned() {
                    self.state.detail_coin = Some(coin.clone());
                    self.state.view = MarketView::Detail;
                    self.state.ai_analysis = None;
                    self.state.ai_loading = false;
                    self.state.detail_scroll = 0;
                    self.state.chart_loading = true;
                    self.state.price_history = None;
                    self.fetch_chart(&coin.symbol, &self.state.chart_range.clone());
                }
                PluginAction::Consumed
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.state.toggle_favorite();
                PluginAction::Consumed
            }
            KeyCode::Char('/') => {
                self.state.search_mode = true;
                self.state.search_text.clear();
                self.state.search_cursor = 0;
                PluginAction::Consumed
            }
            KeyCode::Char('*') => {
                self.state.show_favorites_only = !self.state.show_favorites_only;
                self.state.selected_index = 0;
                self.state.scroll_offset = 0;
                PluginAction::Consumed
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.state.cycle_sort();
                PluginAction::Consumed
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.state.sort_ascending = !self.state.sort_ascending;
                PluginAction::Consumed
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                // Open add ticker modal
                self.state.add_ticker_mode = true;
                self.state.add_ticker_text.clear();
                PluginAction::Consumed
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                // Remove selected ticker
                if let Some(removed) = self.state.remove_selected_ticker() {
                    self.notify_watchlist_change();
                    self.state.error = Some(format!("Removed {}", removed));
                }
                PluginAction::Consumed
            }
            _ => PluginAction::Ignored,
        }
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.state.view = MarketView::List;
                self.state.detail_coin = None;
                self.state.price_history = None;
                self.state.ai_analysis = None;
                PluginAction::Consumed
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                // Toggle favorite for detail coin
                if let Some(ref coin) = self.state.detail_coin {
                    let symbol = coin.symbol.clone();
                    if self.state.favorites.contains(&symbol) {
                        self.state.favorites.remove(&symbol);
                    } else {
                        self.state.favorites.insert(symbol.clone());
                    }
                    // Update the detail coin's favorite status
                    if let Some(ref mut dc) = self.state.detail_coin {
                        dc.is_favorite = self.state.favorites.contains(&dc.symbol);
                    }
                    // Update in list too
                    for c in &mut self.state.coins {
                        if c.symbol == symbol {
                            c.is_favorite = self.state.favorites.contains(&symbol);
                        }
                    }
                }
                PluginAction::Consumed
            }
            KeyCode::Char('1') => {
                self.switch_chart_range(ChartRange::Hour1);
                PluginAction::Consumed
            }
            KeyCode::Char('2') => {
                self.switch_chart_range(ChartRange::Hour4);
                PluginAction::Consumed
            }
            KeyCode::Char('3') => {
                self.switch_chart_range(ChartRange::Day1);
                PluginAction::Consumed
            }
            KeyCode::Char('4') => {
                self.switch_chart_range(ChartRange::Week1);
                PluginAction::Consumed
            }
            KeyCode::Char('5') => {
                self.switch_chart_range(ChartRange::Month1);
                PluginAction::Consumed
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Request AI analysis
                if !self.state.ai_loading {
                    self.state.ai_loading = true;
                    self.state.ai_analysis = None;
                    if let Some(ctx) = self.build_ai_context() {
                        return PluginAction::RequestAiAnalysis(ctx);
                    }
                }
                PluginAction::Consumed
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.detail_scroll > 0 {
                    self.state.detail_scroll -= 1;
                }
                PluginAction::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.detail_scroll += 1;
                PluginAction::Consumed
            }
            _ => PluginAction::Ignored,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            KeyCode::Esc => {
                self.state.search_mode = false;
                self.state.search_text.clear();
                PluginAction::Consumed
            }
            KeyCode::Enter => {
                self.state.search_mode = false;
                PluginAction::Consumed
            }
            KeyCode::Backspace => {
                if self.state.search_cursor > 0 {
                    let pos = self.state.search_cursor - 1;
                    self.state.search_text.remove(pos);
                    self.state.search_cursor = pos;
                }
                self.state.selected_index = 0;
                self.state.scroll_offset = 0;
                PluginAction::Consumed
            }
            KeyCode::Left => {
                if self.state.search_cursor > 0 {
                    self.state.search_cursor -= 1;
                }
                PluginAction::Consumed
            }
            KeyCode::Right => {
                if self.state.search_cursor < self.state.search_text.len() {
                    self.state.search_cursor += 1;
                }
                PluginAction::Consumed
            }
            KeyCode::Char(c) => {
                let pos = self.state.search_cursor;
                self.state.search_text.insert(pos, c);
                self.state.search_cursor += 1;
                self.state.selected_index = 0;
                self.state.scroll_offset = 0;
                PluginAction::Consumed
            }
            _ => PluginAction::Consumed,
        }
    }

    fn handle_add_ticker_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            KeyCode::Esc => {
                self.state.add_ticker_mode = false;
                self.state.add_ticker_text.clear();
                PluginAction::Consumed
            }
            KeyCode::Enter => {
                let ticker = self.state.add_ticker_text.trim().to_uppercase();
                if !ticker.is_empty() {
                    if self.state.add_ticker(&ticker) {
                        self.fetch_single_ticker(&ticker);
                        self.notify_watchlist_change();
                        self.state.error = None;
                    } else {
                        self.state.error = Some(format!("{} already in watchlist", ticker));
                    }
                }
                self.state.add_ticker_mode = false;
                self.state.add_ticker_text.clear();
                PluginAction::Consumed
            }
            KeyCode::Backspace => {
                self.state.add_ticker_text.pop();
                PluginAction::Consumed
            }
            KeyCode::Char(c) => {
                // Only allow alphanumeric chars
                if c.is_alphanumeric() {
                    self.state.add_ticker_text.push(c.to_ascii_uppercase());
                }
                PluginAction::Consumed
            }
            _ => PluginAction::Consumed,
        }
    }

    fn switch_chart_range(&mut self, range: ChartRange) {
        self.state.chart_range = range;
        self.state.chart_loading = true;
        self.state.price_history = None;
        if let Some(ref coin) = self.state.detail_coin {
            self.fetch_chart(&coin.symbol.clone(), &self.state.chart_range.clone());
        }
    }
}
