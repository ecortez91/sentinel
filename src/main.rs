//! # Sentinel - Terminal Process Monitor with AI
//!
//! A beautiful real-time process monitor that tracks CPU, RAM, disk I/O,
//! detects suspicious processes, memory leaks, and security threats.
//! Now with Claude Opus 4 integration for AI-powered system analysis.

#[macro_use]
extern crate rust_i18n;

// Load locale files from `locales/` directory, default to English
i18n!("locales", fallback = "en");

mod ai;
mod alerts;
mod app;
mod config;
pub mod constants;
#[allow(dead_code)]
mod diagnostics;
mod metrics;
mod models;
mod monitor;
mod store;
mod ui;
mod utils;

use anyhow::Result;
use clap::Parser;

use config::Config;
use constants::MIN_REFRESH_MS;

/// Sentinel - AI-Powered Terminal System Monitor
#[derive(Parser, Debug)]
#[command(name = "sentinel", version, about = "A beautiful terminal process monitor with AI-powered analysis")]
struct Cli {
    /// Disable all AI features (no API calls)
    #[arg(long)]
    no_ai: bool,

    /// Color theme (default, gruvbox, nord, catppuccin, dracula, solarized)
    #[arg(long, short = 't')]
    theme: Option<String>,

    /// Refresh rate in milliseconds
    #[arg(long, short = 'r')]
    refresh_rate: Option<u64>,

    /// Disable auto-analysis on the dashboard
    #[arg(long)]
    no_auto_analysis: bool,

    /// Enable Prometheus metrics endpoint on the given address (e.g. "0.0.0.0:9100")
    #[arg(long, value_name = "ADDR")]
    prometheus: Option<String>,

    /// UI language (en, ja, es, de, zh)
    #[arg(long, short = 'l', value_name = "LANG")]
    lang: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load and apply CLI overrides to config
    let mut config = Config::load();
    if let Some(rate) = cli.refresh_rate {
        config.refresh_interval_ms = rate.max(MIN_REFRESH_MS);
    }
    if cli.no_auto_analysis {
        config.auto_analysis_interval_secs = 0;
    }
    if let Some(ref theme_name) = cli.theme {
        config.theme = theme_name.clone();
    }
    if let Some(ref lang) = cli.lang {
        config.lang = lang.clone();
    }

    // Set UI language (CLI > config > default "en")
    rust_i18n::set_locale(&config.lang);

    // Build and run the application
    let mut app = app::App::new(
        &config,
        cli.no_ai,
        cli.prometheus.as_deref(),
    )
    .await?;

    app.run().await
}
