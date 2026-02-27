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
    pub price: f64,  // Close price
    pub high: f64,
    pub low: f64,
    pub open: f64,
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
            ChartRange::Hour1 => 60,    // 60 minutes
            ChartRange::Hour4 => 48,    // 48 x 5min
            ChartRange::Day1 => 24,     // 24 hours
            ChartRange::Week1 => 42,    // 7 days x 6 (4h intervals)
            ChartRange::Month1 => 30,   // 30 days
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
            open: arr.get(1).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            high: arr.get(2).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            low: arr.get(3).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            close: arr.get(4).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            volume: arr.get(5).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            close_time: arr.get(6).and_then(|v| v.as_i64()).unwrap_or(0),
            quote_volume: arr.get(7).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            trades: arr.get(8).and_then(|v| v.as_u64()).unwrap_or(0),
            taker_buy_base: arr.get(9).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
            taker_buy_quote: arr.get(10).and_then(|v| v.as_str()).unwrap_or("0").to_string(),
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
}
