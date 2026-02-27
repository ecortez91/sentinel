//! Binance API client.
//!
//! Fetches market data using Binance's free public API.
//! No API key required for public endpoints.

use super::models::{BinanceKline, BinanceTicker24hr, CoinMarket, PricePoint};

const BINANCE_BASE_URL: &str = "https://api.binance.com/api/v3";

/// HTTP client for the Binance API.
#[derive(Clone)]
pub struct BinanceClient {
    client: reqwest::Client,
    base_url: String,
}

impl BinanceClient {
    /// Create a new client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            base_url: BINANCE_BASE_URL.to_string(),
        }
    }

    /// Fetch 24hr ticker data for multiple symbols.
    /// Symbols should be trading pairs like "BTCUSDT", "ETHUSDT".
    pub async fn fetch_tickers(
        &self,
        symbols: &[String],
    ) -> Result<Vec<CoinMarket>, reqwest::Error> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/ticker/24hr", self.base_url);
        
        // Format symbols for Binance API: ["BTCUSDT","ETHUSDT"]
        let symbols_json = serde_json::to_string(symbols).unwrap_or_default();
        
        let resp = self
            .client
            .get(&url)
            .query(&[("symbols", &symbols_json)])
            .send()
            .await?;

        let tickers: Vec<BinanceTicker24hr> = resp.json().await?;
        
        Ok(tickers
            .into_iter()
            .enumerate()
            .map(|(i, t)| t.into_coin_market(i as u32 + 1))
            .collect())
    }

    /// Fetch a single ticker.
    pub async fn fetch_ticker(
        &self,
        symbol: &str,
    ) -> Result<Option<CoinMarket>, reqwest::Error> {
        let url = format!("{}/ticker/24hr", self.base_url);
        
        let resp = self
            .client
            .get(&url)
            .query(&[("symbol", symbol)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let ticker: BinanceTicker24hr = resp.json().await?;
        Ok(Some(ticker.into_coin_market(0)))
    }

    /// Fetch kline (candlestick) data for charts.
    /// Interval: 1m, 5m, 15m, 1h, 4h, 1d, 1w
    pub async fn fetch_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<PricePoint>, reqwest::Error> {
        let url = format!("{}/klines", self.base_url);
        
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol),
                ("interval", interval),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await?;

        let klines: Vec<BinanceKline> = resp.json().await?;
        
        Ok(klines
            .into_iter()
            .map(|k| PricePoint {
                timestamp: k.close_time,
                price: k.close.parse().unwrap_or(0.0),
                high: k.high.parse().unwrap_or(0.0),
                low: k.low.parse().unwrap_or(0.0),
                open: k.open.parse().unwrap_or(0.0),
            })
            .collect())
    }

    /// Get current price for a symbol (lightweight endpoint).
    #[allow(dead_code)]
    pub async fn fetch_price(&self, symbol: &str) -> Result<f64, reqwest::Error> {
        let url = format!("{}/ticker/price", self.base_url);
        
        let resp = self
            .client
            .get(&url)
            .query(&[("symbol", symbol)])
            .send()
            .await?;

        #[derive(serde::Deserialize)]
        struct PriceResponse {
            price: String,
        }

        let price_resp: PriceResponse = resp.json().await?;
        Ok(price_resp.price.parse().unwrap_or(0.0))
    }
}

impl Default for BinanceClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation() {
        let client = BinanceClient::new();
        assert_eq!(client.base_url, BINANCE_BASE_URL);
    }

    #[test]
    fn parse_ticker_response() {
        let json = r#"{
            "symbol": "BTCUSDT",
            "priceChange": "2126.88",
            "priceChangePercent": "3.12",
            "weightedAvgPrice": "68500.00",
            "prevClosePrice": "67000.00",
            "lastPrice": "69126.88",
            "lastQty": "0.001",
            "bidPrice": "69126.00",
            "bidQty": "1.5",
            "askPrice": "69127.00",
            "askQty": "2.0",
            "openPrice": "67000.00",
            "highPrice": "70000.00",
            "lowPrice": "66500.00",
            "volume": "25000.00",
            "quoteVolume": "1712500000.00",
            "openTime": 1711843200000,
            "closeTime": 1711929600000,
            "firstId": 1000000,
            "lastId": 1050000,
            "count": 50000
        }"#;

        let ticker: BinanceTicker24hr = serde_json::from_str(json).unwrap();
        let coin = ticker.into_coin_market(1);
        
        assert_eq!(coin.symbol, "BTCUSDT");
        assert!((coin.current_price - 69126.88).abs() < 0.01);
        assert!((coin.price_change_pct_24h - 3.12).abs() < 0.01);
        assert!((coin.high_24h - 70000.0).abs() < 0.01);
    }

    #[test]
    fn parse_kline_response() {
        let json = r#"[
            1711843200000,
            "67000.00",
            "70000.00",
            "66500.00",
            "69000.00",
            "25000.00",
            1711929599999,
            "1712500000.00",
            50000,
            "12500.00",
            "856250000.00",
            "0"
        ]"#;

        let kline: BinanceKline = serde_json::from_str(json).unwrap();
        assert_eq!(kline.close_time, 1711929599999);
        assert_eq!(kline.close, "69000.00");
    }
}
