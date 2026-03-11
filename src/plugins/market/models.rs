//! Market data models for the Binance integration.

use serde::Deserialize;

/// A coin's market data (normalized from Binance).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CoinMarket {
    /// Trading pair symbol (e.g., "BTCUSDT")
    pub symbol: String,
    /// Display name derived from symbol (e.g., "BTC")
    pub name: String,
    /// Position in user's watchlist
    pub rank: u32,
    pub current_price: f64,
    pub total_volume: f64,
    pub quote_volume: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    pub open_24h: f64,
    pub price_change_24h: f64,
    pub price_change_pct_24h: f64,
    pub weighted_avg_price: f64,
    pub trade_count: u64,
    pub is_favorite: bool,
}

/// Price history data point for charts (OHLC).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PricePoint {
    pub timestamp: i64,
    pub price: f64, // Close price
    pub high: f64,
    pub low: f64,
    pub open: f64,
}

/// Extended range statistics computed from OHLC history (#7).
#[derive(Debug, Clone, Default)]
pub struct RangeStats {
    /// Highest price in the range.
    pub range_high: f64,
    /// Lowest price in the range.
    pub range_low: f64,
    /// Range as percentage: ((high - low) / low) * 100.
    pub range_pct: f64,
    /// Annualized volatility estimate (std dev of returns * sqrt(periods)).
    pub volatility_pct: f64,
    /// Average volume per candle (if available).
    pub avg_close: f64,
    /// Price trend: positive means upward, negative means downward.
    pub trend_pct: f64,
    /// Number of bullish candles (close > open).
    pub bullish_count: usize,
    /// Number of bearish candles (close < open).
    pub bearish_count: usize,
}

/// Compute range statistics from OHLC price history (#7).
pub fn compute_range_stats(history: &[PricePoint]) -> Option<RangeStats> {
    if history.len() < 2 {
        return None;
    }

    let range_high = history.iter().map(|p| p.high).fold(f64::MIN, f64::max);
    let range_low = history.iter().map(|p| p.low).fold(f64::MAX, f64::min);
    let range_pct = if range_low > 0.0 {
        ((range_high - range_low) / range_low) * 100.0
    } else {
        0.0
    };

    // Compute returns for volatility
    let returns: Vec<f64> = history
        .windows(2)
        .filter_map(|w| {
            if w[0].price > 0.0 {
                Some((w[1].price / w[0].price).ln())
            } else {
                None
            }
        })
        .collect();

    let volatility_pct = if returns.len() >= 2 {
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance =
            returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
        let std_dev = variance.sqrt();
        // Annualize: multiply by sqrt(periods_per_year)
        // For simplicity, just report the period std dev as percentage
        std_dev * 100.0
    } else {
        0.0
    };

    let avg_close = history.iter().map(|p| p.price).sum::<f64>() / history.len() as f64;

    let first_price = history.first().map(|p| p.price).unwrap_or(0.0);
    let last_price = history.last().map(|p| p.price).unwrap_or(0.0);
    let trend_pct = if first_price > 0.0 {
        ((last_price - first_price) / first_price) * 100.0
    } else {
        0.0
    };

    let bullish_count = history.iter().filter(|p| p.price >= p.open).count();
    let bearish_count = history.iter().filter(|p| p.price < p.open).count();

    Some(RangeStats {
        range_high,
        range_low,
        range_pct,
        volatility_pct,
        avg_close,
        trend_pct,
        bullish_count,
        bearish_count,
    })
}

/// A news item for the market feed (#6).
#[derive(Debug, Clone)]
pub struct NewsItem {
    /// Headline text.
    #[allow(dead_code)] // read by renderer
    pub title: String,
    /// Source name (e.g., "CoinDesk", "CryptoSlate").
    #[allow(dead_code)] // read by renderer
    pub source: String,
    /// Publication timestamp (epoch seconds).
    #[allow(dead_code)] // read by renderer
    pub published_at: i64,
    /// URL to the full article.
    #[allow(dead_code)] // read by renderer
    pub url: String,
    /// Sentiment label if available ("positive", "negative", "neutral").
    #[allow(dead_code)] // read by renderer
    pub sentiment: Option<String>,
}

/// Chart time range selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartRange {
    Hour1,
    Hour4,
    Day1,
    Week1,
    Month1,
}

impl ChartRange {
    /// Binance kline interval parameter.
    pub fn interval(&self) -> &str {
        match self {
            ChartRange::Hour1 => "1m",
            ChartRange::Hour4 => "5m",
            ChartRange::Day1 => "1h",
            ChartRange::Week1 => "4h",
            ChartRange::Month1 => "1d",
        }
    }

    /// Number of data points to fetch.
    pub fn limit(&self) -> u32 {
        match self {
            ChartRange::Hour1 => 60,  // 60 minutes
            ChartRange::Hour4 => 48,  // 48 x 5min
            ChartRange::Day1 => 24,   // 24 hours
            ChartRange::Week1 => 42,  // 7 days x 6 (4h intervals)
            ChartRange::Month1 => 30, // 30 days
        }
    }

    /// Label for UI display.
    pub fn label(&self) -> &str {
        match self {
            ChartRange::Hour1 => "1H",
            ChartRange::Hour4 => "4H",
            ChartRange::Day1 => "1D",
            ChartRange::Week1 => "7D",
            ChartRange::Month1 => "30D",
        }
    }

    /// All variants for iteration.
    pub fn all() -> &'static [ChartRange] {
        &[
            ChartRange::Hour1,
            ChartRange::Hour4,
            ChartRange::Day1,
            ChartRange::Week1,
            ChartRange::Month1,
        ]
    }
}

// ── Binance JSON response types ──────────────────────────────────

/// Raw JSON response from Binance `/ticker/24hr`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct BinanceTicker24hr {
    pub symbol: String,
    pub price_change: String,
    pub price_change_percent: String,
    pub weighted_avg_price: String,
    #[serde(default)]
    pub prev_close_price: String,
    pub last_price: String,
    #[serde(default)]
    pub last_qty: String,
    #[serde(default)]
    pub bid_price: String,
    #[serde(default)]
    pub bid_qty: String,
    #[serde(default)]
    pub ask_price: String,
    #[serde(default)]
    pub ask_qty: String,
    pub open_price: String,
    pub high_price: String,
    pub low_price: String,
    pub volume: String,
    pub quote_volume: String,
    pub open_time: i64,
    pub close_time: i64,
    #[serde(default)]
    pub first_id: i64,
    #[serde(default)]
    pub last_id: i64,
    pub count: u64,
}

impl BinanceTicker24hr {
    /// Convert API response to our domain model.
    pub fn into_coin_market(self, rank: u32) -> CoinMarket {
        // Extract base asset from symbol (e.g., "BTC" from "BTCUSDT")
        let name = extract_base_asset(&self.symbol);

        CoinMarket {
            symbol: self.symbol,
            name,
            rank,
            current_price: self.last_price.parse().unwrap_or(0.0),
            total_volume: self.volume.parse().unwrap_or(0.0),
            quote_volume: self.quote_volume.parse().unwrap_or(0.0),
            high_24h: self.high_price.parse().unwrap_or(0.0),
            low_24h: self.low_price.parse().unwrap_or(0.0),
            open_24h: self.open_price.parse().unwrap_or(0.0),
            price_change_24h: self.price_change.parse().unwrap_or(0.0),
            price_change_pct_24h: self.price_change_percent.parse().unwrap_or(0.0),
            weighted_avg_price: self.weighted_avg_price.parse().unwrap_or(0.0),
            trade_count: self.count,
            is_favorite: false,
        }
    }
}

/// Binance kline (candlestick) data.
/// Response is an array: [openTime, open, high, low, close, volume, closeTime, ...]
#[derive(Debug)]
#[allow(dead_code)]
pub struct BinanceKline {
    pub open_time: i64,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub close_time: i64,
    pub quote_volume: String,
    pub trades: u64,
    pub taker_buy_base: String,
    pub taker_buy_quote: String,
}

impl<'de> Deserialize<'de> for BinanceKline {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let arr: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;

        Ok(BinanceKline {
            open_time: arr.get(0).and_then(|v| v.as_i64()).unwrap_or(0),
            open: arr
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            high: arr
                .get(2)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            low: arr
                .get(3)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            close: arr
                .get(4)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            volume: arr
                .get(5)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            close_time: arr.get(6).and_then(|v| v.as_i64()).unwrap_or(0),
            quote_volume: arr
                .get(7)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            trades: arr.get(8).and_then(|v| v.as_u64()).unwrap_or(0),
            taker_buy_base: arr
                .get(9)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
            taker_buy_quote: arr
                .get(10)
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string(),
        })
    }
}

// ── Helper functions ─────────────────────────────────────────────

/// Extract base asset from trading pair.
/// "BTCUSDT" -> "BTC", "ETHUSDT" -> "ETH", "SOLUSDT" -> "SOL"
fn extract_base_asset(symbol: &str) -> String {
    // Common quote assets to strip
    let quote_assets = ["USDT", "USDC", "BUSD", "USD", "BTC", "ETH", "BNB"];

    for quote in quote_assets {
        if symbol.ends_with(quote) && symbol.len() > quote.len() {
            return symbol[..symbol.len() - quote.len()].to_string();
        }
    }

    symbol.to_string()
}

// ── Formatting helpers ───────────────────────────────────────────

/// Format a large number with K/M/B/T suffix.
pub fn format_large_number(n: f64) -> String {
    if n >= 1_000_000_000_000.0 {
        format!("${:.2}T", n / 1_000_000_000_000.0)
    } else if n >= 1_000_000_000.0 {
        format!("${:.2}B", n / 1_000_000_000.0)
    } else if n >= 1_000_000.0 {
        format!("${:.2}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("${:.2}K", n / 1_000.0)
    } else {
        format!("${:.2}", n)
    }
}

/// Format a price with appropriate precision.
pub fn format_price(price: f64) -> String {
    if price >= 1000.0 {
        format!("${:.2}", price)
    } else if price >= 1.0 {
        format!("${:.4}", price)
    } else if price >= 0.01 {
        format!("${:.6}", price)
    } else {
        format!("${:.8}", price)
    }
}

/// Format a percentage change with sign.
pub fn format_change(pct: f64) -> String {
    if pct >= 0.0 {
        format!("+{:.2}%", pct)
    } else {
        format!("{:.2}%", pct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_base_asset_btc() {
        assert_eq!(extract_base_asset("BTCUSDT"), "BTC");
    }

    #[test]
    fn extract_base_asset_eth() {
        assert_eq!(extract_base_asset("ETHUSDT"), "ETH");
    }

    #[test]
    fn extract_base_asset_sol() {
        assert_eq!(extract_base_asset("SOLUSDT"), "SOL");
    }

    #[test]
    fn extract_base_asset_btc_pair() {
        assert_eq!(extract_base_asset("ETHBTC"), "ETH");
    }

    #[test]
    fn format_large_number_trillion() {
        assert_eq!(format_large_number(1_380_000_000_000.0), "$1.38T");
    }

    #[test]
    fn format_large_number_billion() {
        assert_eq!(format_large_number(415_200_000_000.0), "$415.20B");
    }

    #[test]
    fn format_large_number_million() {
        assert_eq!(format_large_number(87_500_000.0), "$87.50M");
    }

    #[test]
    fn format_price_large() {
        assert_eq!(format_price(67187.33), "$67187.33");
    }

    #[test]
    fn format_price_small() {
        assert_eq!(format_price(0.00001234), "$0.00001234");
    }

    #[test]
    fn format_change_positive() {
        assert_eq!(format_change(3.13), "+3.13%");
    }

    #[test]
    fn format_change_negative() {
        assert_eq!(format_change(-2.15), "-2.15%");
    }

    #[test]
    fn chart_range_interval() {
        assert_eq!(ChartRange::Day1.interval(), "1h");
        assert_eq!(ChartRange::Week1.interval(), "4h");
    }

    #[test]
    fn chart_range_all_has_five() {
        assert_eq!(ChartRange::all().len(), 5);
    }

    // ── compute_range_stats tests (#7) ───────────────────────

    fn make_point(open: f64, high: f64, low: f64, close: f64) -> PricePoint {
        PricePoint {
            timestamp: 0,
            open,
            high,
            low,
            price: close,
        }
    }

    #[test]
    fn range_stats_returns_none_for_too_few_points() {
        assert!(compute_range_stats(&[]).is_none());
        assert!(compute_range_stats(&[make_point(100.0, 110.0, 90.0, 105.0)]).is_none());
    }

    #[test]
    fn range_stats_computes_high_low() {
        let history = vec![
            make_point(100.0, 120.0, 95.0, 110.0),
            make_point(110.0, 130.0, 100.0, 125.0),
            make_point(125.0, 140.0, 105.0, 115.0),
        ];
        let stats = compute_range_stats(&history).unwrap();
        assert!((stats.range_high - 140.0).abs() < 0.01);
        assert!((stats.range_low - 95.0).abs() < 0.01);
    }

    #[test]
    fn range_stats_computes_range_pct() {
        let history = vec![
            make_point(100.0, 200.0, 100.0, 150.0),
            make_point(150.0, 200.0, 100.0, 180.0),
        ];
        let stats = compute_range_stats(&history).unwrap();
        // range_pct = ((200 - 100) / 100) * 100 = 100%
        assert!((stats.range_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn range_stats_computes_trend() {
        let history = vec![
            make_point(100.0, 110.0, 90.0, 100.0), // first close = 100
            make_point(100.0, 120.0, 95.0, 110.0),
            make_point(110.0, 130.0, 105.0, 120.0), // last close = 120
        ];
        let stats = compute_range_stats(&history).unwrap();
        // trend = ((120 - 100) / 100) * 100 = 20%
        assert!((stats.trend_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn range_stats_counts_bullish_bearish() {
        let history = vec![
            make_point(100.0, 110.0, 90.0, 105.0), // bullish (close > open)
            make_point(110.0, 120.0, 95.0, 105.0), // bearish (close < open)
            make_point(105.0, 115.0, 100.0, 110.0), // bullish
            make_point(110.0, 115.0, 105.0, 110.0), // bullish (close == open)
        ];
        let stats = compute_range_stats(&history).unwrap();
        assert_eq!(stats.bullish_count, 3);
        assert_eq!(stats.bearish_count, 1);
    }

    #[test]
    fn range_stats_volatility_is_positive() {
        let history = vec![
            make_point(100.0, 110.0, 90.0, 105.0),
            make_point(105.0, 115.0, 95.0, 100.0),
            make_point(100.0, 112.0, 88.0, 108.0),
        ];
        let stats = compute_range_stats(&history).unwrap();
        assert!(stats.volatility_pct > 0.0);
    }

    #[test]
    fn range_stats_avg_close() {
        let history = vec![
            make_point(100.0, 110.0, 90.0, 100.0),
            make_point(100.0, 110.0, 90.0, 200.0),
        ];
        let stats = compute_range_stats(&history).unwrap();
        assert!((stats.avg_close - 150.0).abs() < 0.01);
    }

    // ── NewsItem tests (#6) ──────────────────────────────────

    #[test]
    fn news_item_creation() {
        let item = NewsItem {
            title: "Bitcoin hits new high".to_string(),
            source: "CoinDesk".to_string(),
            published_at: 1700000000,
            url: "https://example.com/news".to_string(),
            sentiment: Some("positive".to_string()),
        };
        assert_eq!(item.title, "Bitcoin hits new high");
        assert_eq!(item.source, "CoinDesk");
        assert_eq!(item.sentiment, Some("positive".to_string()));
    }

    #[test]
    fn news_item_no_sentiment() {
        let item = NewsItem {
            title: "Market update".to_string(),
            source: "CryptoSlate".to_string(),
            published_at: 1700000000,
            url: "https://example.com".to_string(),
            sentiment: None,
        };
        assert!(item.sentiment.is_none());
    }
}
