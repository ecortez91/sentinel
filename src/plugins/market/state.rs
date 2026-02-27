//! Market plugin UI state management.

use std::collections::HashSet;
use std::time::Instant;

pub use super::models::ChartRange;
use super::models::{CoinMarket, PricePoint};

/// Which view is active in the market tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketView {
    List,
    Detail,
}

/// Sortable columns in the market list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketSortColumn {
    Rank,
    Symbol,
    Price,
    Change24h,
    Volume,
}

impl MarketSortColumn {
    pub fn label(&self) -> &str {
        match self {
            MarketSortColumn::Rank => "#",
            MarketSortColumn::Symbol => "Symbol",
            MarketSortColumn::Price => "Price",
            MarketSortColumn::Change24h => "24h %",
            MarketSortColumn::Volume => "Volume",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            MarketSortColumn::Rank => MarketSortColumn::Symbol,
            MarketSortColumn::Symbol => MarketSortColumn::Price,
            MarketSortColumn::Price => MarketSortColumn::Change24h,
            MarketSortColumn::Change24h => MarketSortColumn::Volume,
            MarketSortColumn::Volume => MarketSortColumn::Rank,
        }
    }
}

const PAGE_SIZE: usize = 20;

/// Central state for the market plugin.
pub struct MarketState {
    // ── Data ──────────────────────────────────────────────────
    pub coins: Vec<CoinMarket>,
    pub favorites: HashSet<String>,
    pub price_history: Option<Vec<PricePoint>>,
    /// Configurable watchlist tickers (e.g., ["BTCUSDT", "ETHUSDT"])
    pub watchlist: Vec<String>,

    // ── List view ────────────────────────────────────────────
    pub view: MarketView,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub search_mode: bool,
    pub search_text: String,
    pub search_cursor: usize,
    pub show_favorites_only: bool,
    pub sort_column: MarketSortColumn,
    pub sort_ascending: bool,
    /// Input mode for adding new tickers
    pub add_ticker_mode: bool,
    pub add_ticker_text: String,

    // ── Detail view ──────────────────────────────────────────
    pub detail_coin: Option<CoinMarket>,
    pub detail_scroll: usize,
    pub chart_range: ChartRange,
    pub chart_loading: bool,
    pub ai_analysis: Option<String>,
    pub ai_loading: bool,

    // ── Loading / error ──────────────────────────────────────
    pub loading: bool,
    pub error: Option<String>,
    pub last_updated: Option<Instant>,
}

impl MarketState {
    pub fn new(favorites: HashSet<String>, watchlist: Vec<String>) -> Self {
        Self {
            coins: Vec::new(),
            favorites,
            price_history: None,
            watchlist,
            view: MarketView::List,
            selected_index: 0,
            scroll_offset: 0,
            search_mode: false,
            search_text: String::new(),
            search_cursor: 0,
            show_favorites_only: false,
            sort_column: MarketSortColumn::Rank,
            sort_ascending: true,
            add_ticker_mode: false,
            add_ticker_text: String::new(),
            detail_coin: None,
            detail_scroll: 0,
            chart_range: ChartRange::Day1,
            chart_loading: false,
            ai_analysis: None,
            ai_loading: false,
            loading: true,
            error: None,
            last_updated: None,
        }
    }

    /// Get visible coins after filtering and sorting.
    pub fn visible_coins(&self) -> Vec<&CoinMarket> {
        let mut coins: Vec<&CoinMarket> = if self.show_favorites_only {
            self.coins.iter().filter(|c| c.is_favorite).collect()
        } else if !self.search_text.is_empty() {
            let q = self.search_text.to_lowercase();
            self.coins
                .iter()
                .filter(|c| {
                    c.name.to_lowercase().contains(&q)
                        || c.symbol.to_lowercase().contains(&q)
                })
                .collect()
        } else {
            self.coins.iter().collect()
        };

        // Sort
        let asc = self.sort_ascending;
        coins.sort_by(|a, b| {
            let cmp = match self.sort_column {
                MarketSortColumn::Rank => a.rank.cmp(&b.rank),
                MarketSortColumn::Symbol => a.symbol.to_lowercase().cmp(&b.symbol.to_lowercase()),
                MarketSortColumn::Price => a
                    .current_price
                    .partial_cmp(&b.current_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
                MarketSortColumn::Change24h => a
                    .price_change_pct_24h
                    .partial_cmp(&b.price_change_pct_24h)
                    .unwrap_or(std::cmp::Ordering::Equal),
                MarketSortColumn::Volume => a
                    .quote_volume
                    .partial_cmp(&b.quote_volume)
                    .unwrap_or(std::cmp::Ordering::Equal),
            };
            if asc {
                cmp
            } else {
                cmp.reverse()
            }
        });

        coins
    }

    /// Get the currently selected coin.
    pub fn selected_coin(&self) -> Option<&CoinMarket> {
        let visible = self.visible_coins();
        visible.get(self.selected_index).copied()
    }

    /// Move selection up.
    pub fn move_selection_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
    }

    /// Move selection down.
    pub fn move_selection_down(&mut self) {
        let len = self.visible_coins().len();
        if self.selected_index < len.saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    /// Page up.
    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(PAGE_SIZE);
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
    }

    /// Page down.
    pub fn page_down(&mut self) {
        let len = self.visible_coins().len();
        self.selected_index = (self.selected_index + PAGE_SIZE).min(len.saturating_sub(1));
    }

    /// Toggle favorite for the selected coin.
    pub fn toggle_favorite(&mut self) {
        let visible = self.visible_coins();
        if let Some(coin) = visible.get(self.selected_index) {
            let symbol = coin.symbol.clone();
            if self.favorites.contains(&symbol) {
                self.favorites.remove(&symbol);
            } else {
                self.favorites.insert(symbol.clone());
            }
            for c in &mut self.coins {
                if c.symbol == symbol {
                    c.is_favorite = self.favorites.contains(&symbol);
                }
            }
        }
    }

    /// Cycle to the next sort column.
    pub fn cycle_sort(&mut self) {
        self.sort_column = self.sort_column.next();
    }

    /// Add a ticker to the watchlist.
    pub fn add_ticker(&mut self, ticker: &str) -> bool {
        let ticker = ticker.to_uppercase();
        if !ticker.is_empty() && !self.watchlist.contains(&ticker) {
            self.watchlist.push(ticker);
            true
        } else {
            false
        }
    }

    /// Remove selected ticker from watchlist.
    pub fn remove_selected_ticker(&mut self) -> Option<String> {
        if let Some(coin) = self.selected_coin() {
            let symbol = coin.symbol.clone();
            if let Some(pos) = self.watchlist.iter().position(|t| t == &symbol) {
                self.watchlist.remove(pos);
                self.coins.retain(|c| c.symbol != symbol);
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
                return Some(symbol);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_coin(symbol: &str, rank: u32, price: f64, change_24h: f64) -> CoinMarket {
        CoinMarket {
            symbol: symbol.to_string(),
            name: symbol[..symbol.len().min(3)].to_string(),
            rank,
            current_price: price,
            total_volume: price * 1000.0,
            quote_volume: price * 1_000_000.0,
            high_24h: price * 1.05,
            low_24h: price * 0.95,
            open_24h: price * 0.98,
            price_change_24h: price * change_24h / 100.0,
            price_change_pct_24h: change_24h,
            weighted_avg_price: price,
            trade_count: 50000,
            is_favorite: false,
        }
    }

    #[test]
    fn visible_coins_default_sorted_by_rank() {
        let mut state = MarketState::new(HashSet::new(), vec![]);
        state.coins = vec![
            make_coin("BTCUSDT", 1, 67000.0, 3.0),
            make_coin("ETHUSDT", 2, 3400.0, -1.0),
            make_coin("SOLUSDT", 3, 100.0, 0.0),
        ];
        let visible = state.visible_coins();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].symbol, "BTCUSDT");
        assert_eq!(visible[2].symbol, "SOLUSDT");
    }

    #[test]
    fn favorites_filter_works() {
        let mut favs = HashSet::new();
        favs.insert("ETHUSDT".to_string());
        let mut state = MarketState::new(favs, vec![]);
        state.coins = vec![make_coin("BTCUSDT", 1, 67000.0, 3.0), {
            let mut c = make_coin("ETHUSDT", 2, 3400.0, -1.0);
            c.is_favorite = true;
            c
        }];
        state.show_favorites_only = true;
        let visible = state.visible_coins();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].symbol, "ETHUSDT");
    }

    #[test]
    fn search_filter_works() {
        let mut state = MarketState::new(HashSet::new(), vec![]);
        state.coins = vec![
            make_coin("BTCUSDT", 1, 67000.0, 3.0),
            make_coin("ETHUSDT", 2, 3400.0, -1.0),
        ];
        state.search_text = "eth".to_string();
        let visible = state.visible_coins();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].symbol, "ETHUSDT");
    }

    #[test]
    fn add_ticker_works() {
        let mut state = MarketState::new(HashSet::new(), vec!["BTCUSDT".to_string()]);
        assert!(state.add_ticker("ethusdt"));
        assert_eq!(state.watchlist.len(), 2);
        assert_eq!(state.watchlist[1], "ETHUSDT");
        // Duplicate shouldn't add
        assert!(!state.add_ticker("ETHUSDT"));
        assert_eq!(state.watchlist.len(), 2);
    }

    #[test]
    fn cycle_sort_goes_through_all() {
        let mut col = MarketSortColumn::Rank;
        let mut visited = vec![col];
        for _ in 0..5 {
            col = col.next();
            visited.push(col);
        }
        assert_eq!(visited.len(), 6);
        assert_eq!(visited.last(), Some(&MarketSortColumn::Rank));
    }
}
