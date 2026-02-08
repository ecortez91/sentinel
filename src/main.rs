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
mod config;
mod metrics;
mod models;
mod monitor;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use clap::Parser;

use ai::client::AiEvent;
use ai::{ClaudeClient, ContextBuilder};
use alerts::AlertDetector;
use config::Config;
use monitor::{DockerMonitor, SystemCollector};
use sysinfo::{Pid, Signal};
use ui::{AppState, Tab};

extern crate libc;

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

/// System prompt for auto-analysis (Dashboard insight card).
/// Asks for a brief, scannable system health summary.
const AUTO_ANALYSIS_PROMPT: &str = r#"You are Sentinel AI, a system analyst embedded in a terminal monitor.
Analyze the live system data below and provide a brief health summary (4-6 bullet points max).

Focus on:
- Overall system health (CPU, RAM, swap pressure)
- Top resource consumers and whether they're normal
- Any concerning patterns (memory leaks, zombies, high CPU)
- Actionable recommendations if any

Format: Use bullet points. Be concise - this appears in a small dashboard card.
Do NOT use markdown headers. Start directly with bullet points."#;

/// Build the system prompt dynamically using detected OS and hardware info.
fn build_system_prompt(system: Option<&models::SystemSnapshot>) -> String {
    let os_name = system
        .map(|s| s.os_name.clone())
        .unwrap_or_else(|| "Unknown OS".to_string());
    let total_ram = system
        .map(|s| models::format_bytes(s.total_memory))
        .unwrap_or_else(|| "unknown".to_string());
    let cpu_count = system.map(|s| s.cpu_count).unwrap_or(0);

    format!(
        r#"You are Sentinel AI, an expert system analyst embedded in a terminal process monitor.

System: {} | {} RAM | {} CPU cores

You have LIVE access to the user's system data including:
- All running processes with CPU, memory, disk I/O, status, and command lines
- System-wide CPU, RAM, swap usage and load averages
- Network interfaces and filesystem usage
- Active alerts and security warnings
- Process groupings and aggregations

Your role:
1. Answer questions about what processes are doing, why they exist, and whether they're normal
2. Identify resource hogs, memory leaks, and suspicious activity
3. Explain technical concepts clearly (like what tokio-runtime-w, kworker, node workers are)
4. Give actionable advice for managing system resources based on the actual hardware detected above
5. Flag genuine security concerns vs benign processes

Guidelines:
- Be concise but thorough. This is a terminal - keep responses scannable.
- Use bullet points and short paragraphs.
- When explaining processes, mention what spawned them and whether they're safe.
- If something looks genuinely dangerous, say so clearly.
- If memory is tight, suggest what can be safely killed.
- Reference specific PIDs and process names from the live data when relevant."#,
        os_name, total_ram, cpu_count,
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── CLI args ────────────────────────────────────────────────
    let cli = Cli::parse();

    // ── Setup ──────────────────────────────────────────────────
    let mut config = Config::load();

    // CLI overrides
    if let Some(rate) = cli.refresh_rate {
        config.refresh_interval_ms = rate.max(100);
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

    let mut collector = SystemCollector::new();
    let mut detector = AlertDetector::new(config.clone());

    // Auto-discover auth: env var -> OpenCode OAuth (with auto-refresh) -> Claude Code
    let (auth, has_key) = if cli.no_ai {
        (None, false)
    } else {
        let a = ClaudeClient::discover_auth().await;
        let has = a.is_some();
        (a, has)
    };
    let auth_display = auth.as_ref().map(|a| a.display_name().to_string());
    let claude_client = auth.map(ClaudeClient::new);

    // Resolve theme: try config name as built-in, then custom file, fallback to default
    let initial_theme = ui::Theme::by_name(&config.theme)
        .or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            let custom_path = std::path::PathBuf::from(home)
                .join(".config")
                .join("sentinel")
                .join("themes")
                .join(format!("{}.toml", config.theme));
            ui::Theme::from_toml_file(&custom_path)
        })
        .unwrap_or_default();

    let mut state = AppState::new(config.max_alerts, has_key, initial_theme);
    if let Some(method) = &auth_display {
        state.ai_auth_method = method.clone();
    }

    // Channel for receiving AI streaming responses (chat)
    let (ai_tx, mut ai_rx) = mpsc::unbounded_channel::<AiEvent>();
    // Separate channel for auto-analysis insight card
    let (insight_tx, mut insight_rx) = mpsc::unbounded_channel::<AiEvent>();

    // ── Docker monitoring ────────────────────────────────────────
    let docker = DockerMonitor::new();
    state.docker_available = docker.is_available();
    let (docker_tx, mut docker_rx) =
        mpsc::unbounded_channel::<Vec<monitor::ContainerInfo>>();

    // Spawn a background task to periodically fetch Docker container info
    if docker.is_available() {
        let tx = docker_tx.clone();
        tokio::spawn(async move {
            let docker = DockerMonitor::new();
            loop {
                let containers = docker.list_containers().await;
                if tx.send(containers).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    // ── Prometheus metrics endpoint (optional) ─────────────────
    let shared_metrics = if let Some(ref addr) = cli.prometheus {
        match metrics::start_server(addr) {
            Ok(m) => {
                eprintln!("Prometheus metrics available at http://{}/metrics", addr);
                Some(m)
            }
            Err(e) => {
                eprintln!("Warning: {}", e);
                None
            }
        }
    } else {
        None
    };

    // ── Terminal Init ──────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // ── Initial data collection ────────────────────────────────
    let (system, processes) = collector.collect();
    let alerts_vec = detector.analyze(&system, &processes);
    state.update(system, processes, alerts_vec);

    // ── Main Loop ──────────────────────────────────────────────
    let _tick_rate = Duration::from_millis(config.refresh_interval_ms);
    let mut filtering = false;
    let mut ai_typing = false; // Whether user is typing in AI input
    let mut last_insight_time: Option<std::time::Instant> = None;
    let insight_interval = Duration::from_secs(config.auto_analysis_interval_secs);
    let auto_analysis_enabled = config.auto_analysis_interval_secs > 0;

    loop {
        // Render
        terminal.draw(|frame| ui::render(frame, &state))?;

        // ── Drain AI response chunks (non-blocking) ────────────
        while let Ok(event) = ai_rx.try_recv() {
            match event {
                AiEvent::Chunk(text) => {
                    state.ai_conversation.append_to_last_assistant(&text);
                }
                AiEvent::Done => {
                    state.ai_loading = false;
                }
                AiEvent::Error(err) => {
                    state.ai_loading = false;
                    state.ai_conversation.add_system_message(&format!("Error: {}", err));
                }
            }
        }

        // ── Drain auto-analysis insight chunks ──────────────────
        while let Ok(event) = insight_rx.try_recv() {
            match event {
                AiEvent::Chunk(text) => {
                    if let Some(ref mut insight) = state.ai_insight {
                        insight.push_str(&text);
                    } else {
                        state.ai_insight = Some(text);
                    }
                }
                AiEvent::Done => {
                    state.ai_insight_loading = false;
                    state.ai_insight_updated = Some(std::time::Instant::now());
                }
                AiEvent::Error(err) => {
                    state.ai_insight_loading = false;
                    state.ai_insight = Some(format!("Analysis failed: {}", err));
                }
            }
        }

        // ── Drain Docker container updates ──────────────────────
        while let Ok(containers) = docker_rx.try_recv() {
            state.containers = containers;
        }

        // ── Event handling ─────────────────────────────────────
        if event::poll(Duration::from_millis(50))? {
            let terminal_event = event::read()?;

            // ── Mouse events ────────────────────────────────
            if let Event::Mouse(mouse) = terminal_event {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        if state.show_process_detail {
                            if state.detail_scroll > 0 {
                                state.detail_scroll -= 1;
                            }
                        } else {
                            state.scroll_up();
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if state.show_process_detail {
                            state.detail_scroll += 1;
                        } else {
                            state.scroll_down();
                        }
                    }
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        let x = mouse.column;
                        let y = mouse.row;

                        // Close popup overlays on click outside
                        if state.show_process_detail {
                            state.close_process_detail();
                        } else if state.show_help {
                            state.show_help = false;
                        }
                        // Tab bar click (header area, row 0-2, after logo at x>=22)
                        else if y <= 2 && x >= 22 {
                            let tab_x = (x - 22) as usize;
                            // Each tab is roughly: " Label " (varies) with " │ " separators
                            // Tab labels: " Dashboard " (12), " │ " (3), " Processes " (12), " │ " (3), " Alerts " (9), " │ " (3), " Ask AI " (8)
                            // Cumulative: 0-11 = Dashboard, 15-26 = Processes, 30-38 = Alerts, 42-49 = Ask AI
                            if tab_x < 13 {
                                state.active_tab = Tab::Dashboard;
                            } else if tab_x < 28 {
                                state.active_tab = Tab::Processes;
                            } else if tab_x < 40 {
                                state.active_tab = Tab::Alerts;
                            } else {
                                state.active_tab = Tab::AskAi;
                                ai_typing = true;
                            }
                        }
                        // Process table row click (Processes tab)
                        else if state.active_tab == Tab::Processes && y >= 7 {
                            // Header bar = 3 rows, filter bar = 3 rows, table header = 1 row
                            // Content rows start at row 7
                            let row_index = (y - 7) as usize;
                            let max = if state.tree_view {
                                state.tree_processes().len()
                            } else {
                                state.filtered_processes().len()
                            };
                            if row_index < max {
                                state.selected_process = row_index;
                            }
                        }
                    }
                    MouseEventKind::Down(crossterm::event::MouseButton::Right) => {
                        // Right-click on Processes tab: open detail popup
                        if state.active_tab == Tab::Processes && mouse.row >= 7 {
                            let row_index = (mouse.row - 7) as usize;
                            let proc_clone = {
                                let filtered = state.filtered_processes();
                                if row_index < filtered.len() {
                                    Some((*filtered[row_index]).clone())
                                } else {
                                    None
                                }
                            };
                            if let Some(proc) = proc_clone {
                                state.selected_process = row_index;
                                state.open_process_detail(&proc);
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if let Event::Key(key) = terminal_event {

                // ── Process detail popup mode ────────────────
                if state.show_process_detail {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                            state.close_process_detail();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if state.detail_scroll > 0 {
                                state.detail_scroll -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            state.detail_scroll += 1;
                        }
                        KeyCode::PageUp => {
                            state.detail_scroll = state.detail_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            state.detail_scroll += 10;
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── Signal picker popup mode ──────────────────
                if state.show_signal_picker {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            state.close_signal_picker();
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if state.signal_picker_selected > 0 {
                                state.signal_picker_selected -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if state.signal_picker_selected < ui::SIGNAL_LIST.len() - 1 {
                                state.signal_picker_selected += 1;
                            }
                        }
                        KeyCode::Enter => {
                            // Send the selected signal
                            if let Some(pid) = state.signal_picker_pid {
                                let (sig_num, sig_name, _) = ui::SIGNAL_LIST[state.signal_picker_selected];
                                let name = state.signal_picker_name.clone();
                                // Use libc::kill to send arbitrary signals
                                let result = unsafe { libc::kill(pid as i32, sig_num) };
                                if result == 0 {
                                    state.status_message = Some((
                                        format!("Sent {} to PID {} ({})", sig_name, pid, name),
                                        std::time::Instant::now(),
                                    ));
                                } else {
                                    state.status_message = Some((
                                        format!("Failed to send {} to PID {} ({})", sig_name, pid, name),
                                        std::time::Instant::now(),
                                    ));
                                }
                            }
                            state.close_signal_picker();
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── Renice dialog mode ───────────────────────────
                if state.show_renice_dialog {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            state.close_renice_dialog();
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            state.renice_value = (state.renice_value - 1).max(-20);
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            state.renice_value = (state.renice_value + 1).min(19);
                        }
                        KeyCode::Up => {
                            state.renice_value = (state.renice_value - 5).max(-20);
                        }
                        KeyCode::Down => {
                            state.renice_value = (state.renice_value + 5).min(19);
                        }
                        KeyCode::Enter => {
                            // Apply renice via libc::setpriority
                            if let Some(pid) = state.renice_pid {
                                let name = state.renice_name.clone();
                                let nice = state.renice_value;
                                let result = unsafe {
                                    libc::setpriority(libc::PRIO_PROCESS, pid, nice)
                                };
                                if result == 0 {
                                    state.status_message = Some((
                                        format!("Set nice {} for PID {} ({})", nice, pid, name),
                                        std::time::Instant::now(),
                                    ));
                                } else {
                                    let err = std::io::Error::last_os_error();
                                    state.status_message = Some((
                                        format!("Renice failed for PID {}: {}", pid, err),
                                        std::time::Instant::now(),
                                    ));
                                }
                            }
                            state.close_renice_dialog();
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── AI typing mode ─────────────────────────────
                if ai_typing {
                    match key.code {
                        KeyCode::Esc => {
                            ai_typing = false;
                            state.ai_input.clear();
                            state.ai_cursor_pos = 0;
                        }
                        KeyCode::Enter => {
                            if !state.ai_loading {
                                if let Some(_question) = state.ai_submit() {
                                    // Fire off AI request
                                    if let Some(ref _client) = claude_client {
                                        state.ai_loading = true;

                                        // Build context from live system data
                                        let context = ContextBuilder::build(
                                            state.system.as_ref(),
                                            &state.processes,
                                            &state.alerts,
                                        );

                                        let system_prompt = build_system_prompt(state.system.as_ref());
                                        let full_system = format!(
                                            "{}\n\n--- LIVE SYSTEM DATA (captured at this moment) ---\n\n{}",
                                            system_prompt, context
                                        );

                                        let messages = state.ai_conversation.to_api_messages();
                                        let tx = ai_tx.clone();

                                        // Re-discover auth for each request (with auto token refresh)
                                        tokio::spawn(async move {
                                            let auth = ClaudeClient::discover_auth().await;
                                            if let Some(auth) = auth {
                                                let client = ClaudeClient::new(auth);
                                                let _ = client
                                                    .ask_streaming(&full_system, messages, tx)
                                                    .await;
                                            }
                                        });
                                    }
                                }
                            }
                            ai_typing = false;
                        }
                        KeyCode::Backspace => {
                            state.ai_input_backspace();
                        }
                        KeyCode::Left => {
                            state.ai_cursor_left();
                        }
                        KeyCode::Right => {
                            state.ai_cursor_right();
                        }
                        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            state.ai_conversation.clear();
                            state.ai_input.clear();
                            state.ai_cursor_pos = 0;
                            ai_typing = false;
                        }
                        KeyCode::Char(c) => {
                            state.ai_input_char(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── Filter mode ────────────────────────────────
                if filtering {
                    match key.code {
                        KeyCode::Esc => {
                            state.filter_text.clear();
                            filtering = false;
                        }
                        KeyCode::Enter => {
                            filtering = false;
                        }
                        KeyCode::Backspace => {
                            state.filter_text.pop();
                            if state.filter_text.is_empty() {
                                filtering = false;
                            }
                        }
                        KeyCode::Char(c) => {
                            state.filter_text.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // ── Normal mode ────────────────────────────────
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        if state.active_tab != Tab::AskAi {
                            break;
                        } else {
                            // On AI tab, q starts typing
                            ai_typing = true;
                            state.ai_input_char('q');
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,

                    // Tab navigation
                    KeyCode::Tab => state.next_tab(),
                    KeyCode::BackTab => state.prev_tab(),
                    KeyCode::Char('1') => state.active_tab = Tab::Dashboard,
                    KeyCode::Char('2') => state.active_tab = Tab::Processes,
                    KeyCode::Char('3') => state.active_tab = Tab::Alerts,
                    KeyCode::Char('4') => {
                        state.active_tab = Tab::AskAi;
                        ai_typing = true; // Auto-focus input
                    }

                    // Kill process (Processes tab only) — must be before scroll
                    // handlers since 'k' is also used for vim-style scroll
                    KeyCode::Char('k') if state.active_tab == Tab::Processes => {
                        // SIGTERM (graceful)
                        let filtered = state.filtered_processes();
                        if let Some(proc) = filtered.get(state.selected_process) {
                            let pid = proc.pid;
                            let name = proc.name.clone();
                            let sys = collector.system();
                            if let Some(process) = sys.process(Pid::from_u32(pid)) {
                                if process.kill_with(Signal::Term).unwrap_or(false) {
                                    state.status_message = Some((
                                        format!("Sent SIGTERM to PID {} ({})", pid, name),
                                        std::time::Instant::now(),
                                    ));
                                } else {
                                    state.status_message = Some((
                                        format!("Failed to send SIGTERM to PID {} ({})", pid, name),
                                        std::time::Instant::now(),
                                    ));
                                }
                            }
                        }
                    }
                    KeyCode::Char('K') if state.active_tab == Tab::Processes => {
                        // SIGKILL (force)
                        let filtered = state.filtered_processes();
                        if let Some(proc) = filtered.get(state.selected_process) {
                            let pid = proc.pid;
                            let name = proc.name.clone();
                            let sys = collector.system();
                            if let Some(process) = sys.process(Pid::from_u32(pid)) {
                                if process.kill() {
                                    state.status_message = Some((
                                        format!("Sent SIGKILL to PID {} ({})", pid, name),
                                        std::time::Instant::now(),
                                    ));
                                } else {
                                    state.status_message = Some((
                                        format!("Failed to send SIGKILL to PID {} ({})", pid, name),
                                        std::time::Instant::now(),
                                    ));
                                }
                            }
                        }
                    }

                    // Scrolling
                    KeyCode::Up | KeyCode::Char('k') => state.scroll_up(),
                    KeyCode::Down | KeyCode::Char('j') => state.scroll_down(),
                    KeyCode::PageUp => state.page_up(),
                    KeyCode::PageDown => state.page_down(),
                    KeyCode::Home => {
                        state.selected_process = 0;
                        state.alert_scroll = 0;
                        state.ai_scroll = 0;
                    }
                    KeyCode::End => {
                        let max = state.filtered_processes().len().saturating_sub(1);
                        state.selected_process = max;
                        state.alert_scroll = state.alerts.len().saturating_sub(1);
                    }

                    // Sort
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        if state.active_tab != Tab::AskAi {
                            state.cycle_sort();
                        } else {
                            ai_typing = true;
                            state.ai_input_char('s');
                        }
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        if state.active_tab != Tab::AskAi {
                            state.toggle_sort_direction();
                        } else {
                            ai_typing = true;
                            state.ai_input_char('r');
                        }
                    }

                    // AI insight expand/collapse (Dashboard tab only)
                    KeyCode::Char('e') if state.active_tab == Tab::Dashboard => {
                        state.ai_insight_expanded = !state.ai_insight_expanded;
                    }

                    // Signal picker (Processes tab, x = open picker)
                    KeyCode::Char('x') if state.active_tab == Tab::Processes => {
                        state.open_signal_picker();
                    }

                    // Renice dialog (Processes tab, n = open renice)
                    KeyCode::Char('n') if state.active_tab == Tab::Processes => {
                        state.open_renice_dialog();
                    }

                    // Zoom history charts (Dashboard tab)
                    KeyCode::Char('+') | KeyCode::Char('=') if state.active_tab == Tab::Dashboard => {
                        state.history_window = state.history_window.prev(); // Zoom in = shorter window
                    }
                    KeyCode::Char('-') if state.active_tab == Tab::Dashboard => {
                        state.history_window = state.history_window.next(); // Zoom out = longer window
                    }

                    // Widget focus/expand (Dashboard tab)
                    KeyCode::Char('f') if state.active_tab == Tab::Dashboard => {
                        state.toggle_focus();
                    }
                    KeyCode::Char('F') if state.active_tab == Tab::Dashboard && state.focused_widget.is_some() => {
                        state.cycle_focus_forward();
                    }

                    // Theme cycling (T uppercase, from any tab except AI typing)
                    KeyCode::Char('T') => {
                        if state.active_tab != Tab::AskAi {
                            state.cycle_theme();
                            state.status_message = Some((
                                format!("Theme: {}", state.theme.name),
                                std::time::Instant::now(),
                            ));
                        } else {
                            ai_typing = true;
                            state.ai_input_char('T');
                        }
                    }

                    // Language cycling (L uppercase, from any tab except AI typing)
                    KeyCode::Char('L') => {
                        if state.active_tab != Tab::AskAi {
                            state.cycle_lang();
                            state.status_message = Some((
                                format!("Language: {}", state.current_lang),
                                std::time::Instant::now(),
                            ));
                        } else {
                            ai_typing = true;
                            state.ai_input_char('L');
                        }
                    }

                    // Tree view toggle (Processes tab only)
                    KeyCode::Char('t') if state.active_tab == Tab::Processes => {
                        state.tree_view = !state.tree_view;
                        state.selected_process = 0; // Reset selection on toggle
                    }

                    // Ask AI about selected process (Processes tab only)
                    KeyCode::Char('a') if state.active_tab == Tab::Processes => {
                        if has_key && !state.ai_loading {
                            let proc_clone = {
                                let filtered = state.filtered_processes();
                                filtered.get(state.selected_process).map(|p| (*p).clone())
                            };
                            if let Some(proc) = proc_clone {
                                // Build a targeted question
                                let question = format!(
                                    "Tell me about this process: PID {} ({}) - \
                                     CPU: {:.1}%, Memory: {}, Status: {}, User: {}, \
                                     Command: {}. \
                                     What is it, is it normal, should I be concerned?",
                                    proc.pid,
                                    proc.name,
                                    proc.cpu_usage,
                                    crate::models::format_bytes(proc.memory_bytes),
                                    proc.status,
                                    proc.user,
                                    proc.cmd,
                                );

                                // Switch to AI tab and submit
                                state.active_tab = Tab::AskAi;
                                state.ai_conversation.add_user_message(&question);
                                state.ai_loading = true;

                                let context = ContextBuilder::build(
                                    state.system.as_ref(),
                                    &state.processes,
                                    &state.alerts,
                                );
                                let system_prompt = build_system_prompt(state.system.as_ref());
                                let full_system = format!(
                                    "{}\n\n--- LIVE SYSTEM DATA (captured at this moment) ---\n\n{}",
                                    system_prompt, context
                                );
                                let messages = state.ai_conversation.to_api_messages();
                                let tx = ai_tx.clone();

                                tokio::spawn(async move {
                                    let auth = ClaudeClient::discover_auth().await;
                                    if let Some(auth) = auth {
                                        let client = ClaudeClient::new(auth);
                                        let _ = client
                                            .ask_streaming(&full_system, messages, tx)
                                            .await;
                                    }
                                });
                            }
                        }
                    }

                    // Filter
                    KeyCode::Char('/') => {
                        if state.active_tab != Tab::AskAi {
                            filtering = true;
                            state.filter_text.clear();
                        } else {
                            ai_typing = true;
                            state.ai_input_char('/');
                        }
                    }

                    // Process detail popup (Enter on Processes tab)
                    KeyCode::Enter if state.active_tab == Tab::Processes => {
                        let proc_clone = {
                            let filtered = state.filtered_processes();
                            filtered.get(state.selected_process).map(|p| (*p).clone())
                        };
                        if let Some(proc) = proc_clone {
                            state.open_process_detail(&proc);
                        }
                    }

                    // On AI tab, Enter focuses the input
                    KeyCode::Enter if state.active_tab == Tab::AskAi => {
                        ai_typing = true;
                    }

                    // Any character on AI tab starts typing
                    KeyCode::Char(c) if state.active_tab == Tab::AskAi => {
                        if c == '?' {
                            state.show_help = !state.show_help;
                        } else {
                            ai_typing = true;
                            state.ai_input_char(c);
                        }
                    }

                    // Ctrl+L to clear AI chat from any mode
                    KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if state.active_tab == Tab::AskAi {
                            state.ai_conversation.clear();
                        }
                    }

                    // Help
                    KeyCode::Char('?') => state.show_help = !state.show_help,
                    KeyCode::Esc => {
                        if state.show_help {
                            state.show_help = false;
                        } else if state.active_tab == Tab::AskAi {
                            state.active_tab = Tab::Dashboard;
                        } else {
                            state.filter_text.clear();
                        }
                    }

                    _ => {}
                }
            }
        }

        // ── Tick: Refresh data (throttled) ─────────────────────
        // Only refresh system data every ~1 second (based on tick_count timing)
        // but keep the UI responsive at 50ms polling
        if state.tick_count == 0 || {
            // Simple time-based throttle using tick_count
            let should_refresh = state.tick_count % 20 == 0; // ~1s at 50ms poll
            should_refresh
        } {
            let (system, processes) = collector.collect();
            let new_alerts = detector.analyze(&system, &processes);
            state.update(system, processes, new_alerts);

            // Update Prometheus metrics snapshot (if enabled)
            if let Some(ref metrics_handle) = shared_metrics {
                if let Ok(mut snap) = metrics_handle.lock() {
                    snap.system = state.system.clone();
                    snap.process_count = state.processes.len();
                    snap.alerts = state.alerts.clone();
                    snap.containers = state.containers.clone();
                }
            }
        } else {
            state.tick_count += 1;
        }

        // ── Auto-analysis trigger ───────────────────────────────
        // Fire on startup (after a few ticks so data settles) and periodically
        if auto_analysis_enabled && has_key && !state.ai_insight_loading {
            let should_analyze = match last_insight_time {
                None => state.tick_count >= 5, // Wait ~5 seconds after startup
                Some(t) => t.elapsed() >= insight_interval,
            };
            if should_analyze {
                state.ai_insight_loading = true;
                state.ai_insight = None; // Clear old insight, will stream in fresh
                state.ai_insight_scroll = 0;
                last_insight_time = Some(std::time::Instant::now());

                let context = ContextBuilder::build(
                    state.system.as_ref(),
                    &state.processes,
                    &state.alerts,
                );
                let full_system = format!(
                    "{}\n\n--- LIVE SYSTEM DATA ---\n\n{}",
                    AUTO_ANALYSIS_PROMPT, context
                );
                let messages = vec![serde_json::json!({
                    "role": "user",
                    "content": "Analyze my system now. Give me a quick health check."
                })];
                let tx = insight_tx.clone();

                tokio::spawn(async move {
                    let auth = ClaudeClient::discover_auth().await;
                    if let Some(auth) = auth {
                        let client = ClaudeClient::new(auth);
                        let _ = client.ask_streaming(&full_system, messages, tx).await;
                    }
                });
            }
        }
    }

    // ── Cleanup ────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    println!("\n{}\n", t!("app.stopped"));
    Ok(())
}
