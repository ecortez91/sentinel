//! Application struct and event loop.
//!
//! Owns the terminal, state, collectors, and AI channels.
//! Extracts the event loop from `main()` into a testable, well-structured unit.

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
use sysinfo::{Pid, Signal};
use tokio::sync::mpsc;

use crate::ai::client::AiEvent;
use crate::ai::{ClaudeClient, ContextBuilder};
use crate::alerts::AlertDetector;
use crate::config::Config;
use crate::constants::*;
use crate::diagnostics::{DiagnosticEngine, SuggestedAction};
use crate::notifications::{self, EmailNotifier, NotifyEvent};
use crate::thermal::LhmClient;
use crate::thermal::shutdown::{ShutdownEvent, ShutdownManager};
use crate::ui::CommandResult;
use crate::monitor::{ContainerInfo, DockerMonitor, SystemCollector};
use crate::store::EventStore;
use crate::ui::{self, AppState, Tab};

/// System prompt for auto-analysis (Dashboard insight card).
const AUTO_ANALYSIS_PROMPT: &str = r#"You are Sentinel AI, a system analyst embedded in a terminal monitor.
Analyze the live system data below and provide a brief health summary (4-6 bullet points max).

Focus on
- Overall system health (CPU, RAM, swap pressure)
- Top resource consumers and whether they're normal
- Any concerning patterns (memory leaks, zombies, high CPU)
- Actionable recommendations if any

Format: Use bullet points. Be concise - this appears in a small dashboard card.
Do NOT use markdown headers. Start directly with bullet points."#;

/// Build the system prompt dynamically using detected OS and hardware info.
fn build_system_prompt(system: Option<&crate::models::SystemSnapshot>) -> String {
    let os_name = system
        .map(|s| s.os_name.clone())
        .unwrap_or_else(|| "Unknown OS".to_string());
    let total_ram = system
        .map(|s| crate::models::format_bytes(s.total_memory))
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

/// Main application struct.
///
/// Owns all runtime resources: terminal, state, data collectors, AI channels.
pub struct App {
    state: AppState,
    collector: SystemCollector,
    detector: AlertDetector,
    claude_client: Option<ClaudeClient>,
    has_key: bool,

    // Channels
    ai_tx: mpsc::UnboundedSender<AiEvent>,
    ai_rx: mpsc::UnboundedReceiver<AiEvent>,
    insight_tx: mpsc::UnboundedSender<AiEvent>,
    insight_rx: mpsc::UnboundedReceiver<AiEvent>,
    command_ai_tx: mpsc::UnboundedSender<AiEvent>,
    command_ai_rx: mpsc::UnboundedReceiver<AiEvent>,
    docker_rx: mpsc::UnboundedReceiver<Vec<ContainerInfo>>,

    // Prometheus
    shared_metrics: Option<crate::metrics::SharedMetrics>,

    // Event store (persistent timeline)
    event_store: Option<EventStore>,
    /// Ticks between network socket scans (every ~10s at 1s tick = 10).
    net_scan_interval: u64,

    // Thermal monitoring (LHM)
    thermal_rx: mpsc::UnboundedReceiver<Option<crate::thermal::ThermalSnapshot>>,

    // Email notifications
    email_notifier: Option<EmailNotifier>,

    // Local loop state
    filtering: bool,
    ai_typing: bool,
    last_insight_time: Option<std::time::Instant>,
    insight_interval: Duration,
    auto_analysis_enabled: bool,
}

impl App {
    /// Create a new App, initializing all subsystems.
    ///
    /// This performs auth discovery, theme resolution, Docker setup,
    /// and optional Prometheus server startup.
    pub async fn new(
        config: &Config,
        no_ai: bool,
        prometheus_addr: Option<&str>,
    ) -> Result<Self> {
        let collector = SystemCollector::new();
        let detector = AlertDetector::new(config.clone());

        // Auto-discover auth
        let (auth, has_key) = if no_ai {
            (None, false)
        } else {
            let a = ClaudeClient::discover_auth().await;
            let has = a.is_some();
            (a, has)
        };
        let auth_display = auth.as_ref().map(|a| a.display_name().to_string());
        let claude_client = auth.map(ClaudeClient::new);

        // Resolve theme
        let initial_theme = ui::Theme::by_name(&config.theme)
            .or_else(|| ui::Theme::from_toml_file(&custom_theme_path(&config.theme)))
            .unwrap_or_default();

        // Detect CJK font support before entering alternate screen
        let cjk_supported = crate::utils::detect_cjk_support();

        // Load .env for SMTP/shutdown credentials (optional, never committed)
        let env_path = crate::constants::env_file_path();
        let _ = dotenvy::from_path(&env_path);

        // Create shutdown manager (double-gated: config + .env)
        let shutdown_manager = ShutdownManager::new(
            config.thermal.auto_shutdown_enabled,
            config.thermal.emergency_threshold,
            config.thermal.critical_threshold,
            config.thermal.sustained_seconds,
            SHUTDOWN_GRACE_PERIOD_SECS,
            config.thermal.shutdown_schedule_start,
            config.thermal.shutdown_schedule_end,
        );

        // Initialize email notifier (requires .env SMTP credentials)
        let email_notifier = if config.notifications.email_enabled {
            EmailNotifier::from_env()
        } else {
            None
        };

        let mut state = AppState::new(config.max_alerts, has_key, initial_theme, cjk_supported, shutdown_manager);
        if let Some(method) = &auth_display {
            state.ai_auth_method = method.clone();
        }

        // AI channels
        let (ai_tx, ai_rx) = mpsc::unbounded_channel::<AiEvent>();
        let (insight_tx, insight_rx) = mpsc::unbounded_channel::<AiEvent>();
        let (command_ai_tx, command_ai_rx) = mpsc::unbounded_channel::<AiEvent>();

        // Docker monitoring
        let docker = DockerMonitor::new();
        state.docker_available = docker.is_available();
        let (docker_tx, docker_rx) = mpsc::unbounded_channel::<Vec<ContainerInfo>>();

        if docker.is_available() {
            let tx = docker_tx.clone();
            tokio::spawn(async move {
                let docker = DockerMonitor::new();
                loop {
                    let containers = docker.list_containers().await;
                    if tx.send(containers).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(DOCKER_POLL_SECS)).await;
                }
            });
        }

        // Thermal monitoring (LHM HTTP polling)
        let (thermal_tx, thermal_rx) = mpsc::unbounded_channel();
        {
            let tx = thermal_tx;
            let lhm_url = config.thermal.lhm_url.clone();
            let poll_secs = config.thermal.poll_interval_secs;
            tokio::spawn(async move {
                let client = LhmClient::new(&lhm_url);
                loop {
                    let snapshot = client.poll().await;
                    if tx.send(snapshot).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(poll_secs)).await;
                }
            });
        }

        // Prometheus metrics endpoint
        let shared_metrics = if let Some(addr) = prometheus_addr {
            match crate::metrics::start_server(addr) {
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

        let insight_interval = Duration::from_secs(config.auto_analysis_interval_secs);
        let auto_analysis_enabled = config.auto_analysis_interval_secs > 0;

        // Initialize event store (persistent timeline)
        let event_store = match EventStore::open(Some(&EventStore::default_path())) {
            Ok(store) => {
                eprintln!(
                    "Event store: {}",
                    EventStore::default_path().display()
                );
                Some(store)
            }
            Err(e) => {
                eprintln!("Warning: could not open event store: {}", e);
                None
            }
        };

        Ok(Self {
            state,
            collector,
            detector,
            claude_client,
            has_key,
            ai_tx,
            ai_rx,
            insight_tx,
            insight_rx,
            command_ai_tx,
            command_ai_rx,
            docker_rx,
            shared_metrics,
            event_store,
            net_scan_interval: 10, // scan network sockets every ~10 ticks
            thermal_rx,
            email_notifier,
            filtering: false,
            ai_typing: false,
            last_insight_time: None,
            insight_interval,
            auto_analysis_enabled,
        })
    }

    /// Run the main event loop. Returns when the user quits.
    pub async fn run(&mut self) -> Result<()> {
        // Terminal init
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Initial data collection
        let (system, processes) = self.collector.collect();
        let alerts_vec = self.detector.analyze(&system, &processes);
        self.state.update(system, processes, alerts_vec);

        // Main loop
        loop {
            terminal.draw(|frame| ui::render(frame, &self.state))?;

            self.drain_ai_events();
            self.drain_insight_events();
            self.drain_docker_events();
            self.drain_thermal_events();
            self.drain_command_ai_events();

            if event::poll(Duration::from_millis(EVENT_POLL_MS))? {
                let terminal_event = event::read()?;

                if let Event::Mouse(mouse) = terminal_event {
                    self.handle_mouse(mouse);
                    continue;
                }

                if let Event::Key(key) = terminal_event {
                    if self.handle_key(key) {
                        break; // quit requested
                    }
                }
            }

            self.tick_refresh();
            self.tick_auto_analysis();
            self.tick_shutdown();
        }

        // Cleanup
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

    // ── Channel draining ─────────────────────────────────────────

    fn drain_ai_events(&mut self) {
        while let Ok(event) = self.ai_rx.try_recv() {
            match event {
                AiEvent::Chunk(text) => {
                    self.state.ai_conversation.append_to_last_assistant(&text);
                }
                AiEvent::Done => {
                    self.state.ai_loading = false;
                }
                AiEvent::Error(err) => {
                    self.state.ai_loading = false;
                    self.state
                        .ai_conversation
                        .add_system_message(&format!("Error: {}", err));
                }
            }
        }
    }

    fn drain_insight_events(&mut self) {
        while let Ok(event) = self.insight_rx.try_recv() {
            match event {
                AiEvent::Chunk(text) => {
                    if let Some(ref mut insight) = self.state.ai_insight {
                        insight.push_str(&text);
                    } else {
                        self.state.ai_insight = Some(text);
                    }
                }
                AiEvent::Done => {
                    self.state.ai_insight_loading = false;
                    self.state.ai_insight_updated = Some(std::time::Instant::now());
                }
                AiEvent::Error(err) => {
                    self.state.ai_insight_loading = false;
                    self.state.ai_insight = Some(format!("Analysis failed: {}", err));
                }
            }
        }
    }

    fn drain_docker_events(&mut self) {
        while let Ok(containers) = self.docker_rx.try_recv() {
            self.state.containers = containers;
        }
    }

    fn drain_thermal_events(&mut self) {
        while let Ok(snapshot) = self.thermal_rx.try_recv() {
            if let Some(ref snap) = snapshot {
                // Push CPU package temp (or max CPU temp) to history ring buffer
                let temp = snap.cpu_package.unwrap_or(snap.max_cpu_temp);
                if self.state.temp_history.len() >= THERMAL_HISTORY_CAPACITY {
                    self.state.temp_history.pop_front();
                }
                self.state.temp_history.push_back(temp);
            }
            self.state.thermal = snapshot;
        }
    }

    fn drain_command_ai_events(&mut self) {
        while let Ok(event) = self.command_ai_rx.try_recv() {
            match event {
                AiEvent::Chunk(text) => {
                    if let Some(ref mut cr) = self.state.command_result {
                        cr.text.push_str(&text);
                    }
                }
                AiEvent::Done => {
                    self.state.command_ai_loading = false;
                }
                AiEvent::Error(err) => {
                    self.state.command_ai_loading = false;
                    if let Some(ref mut cr) = self.state.command_result {
                        cr.text.push_str(&format!("\n\nError: {}", err));
                    }
                }
            }
        }
    }

    // ── AI dispatch (deduplicated) ───────────────────────────────

    /// Build diagnostic context from the event store for AI enrichment.
    fn build_diagnostic_context(&self) -> String {
        if let (Some(system), Some(ref store)) = (self.state.system.as_ref(), &self.event_store) {
            DiagnosticEngine::full_context_report(
                system,
                &self.state.processes,
                &self.state.alerts,
                store,
            )
        } else {
            String::new()
        }
    }

    /// Dispatch an AI streaming request on the chat channel.
    ///
    /// Builds context from live system data + diagnostics, prepares the system
    /// prompt, and spawns an async task to stream the response.
    fn dispatch_ai_chat(&self) {
        let context = ContextBuilder::build(
            self.state.system.as_ref(),
            &self.state.processes,
            &self.state.alerts,
        );
        let diagnostic_context = self.build_diagnostic_context();
        let system_prompt = build_system_prompt(self.state.system.as_ref());
        let full_system = if diagnostic_context.is_empty() {
            format!("{}{}{}", system_prompt, AI_CONTEXT_SEPARATOR, context)
        } else {
            format!(
                "{}{}{}\n\n--- DIAGNOSTIC FINDINGS ---\n\n{}",
                system_prompt, AI_CONTEXT_SEPARATOR, context, diagnostic_context
            )
        };
        let messages = self.state.ai_conversation.to_api_messages();
        let tx = self.ai_tx.clone();

        tokio::spawn(async move {
            let auth = ClaudeClient::discover_auth().await;
            if let Some(auth) = auth {
                let client = ClaudeClient::new(auth);
                let _ = client.ask_streaming(&full_system, messages, tx).await;
            }
        });
    }

    /// Dispatch an auto-analysis request on the insight channel.
    fn dispatch_insight(&self) {
        let context = ContextBuilder::build(
            self.state.system.as_ref(),
            &self.state.processes,
            &self.state.alerts,
        );
        let diagnostic_context = self.build_diagnostic_context();
        let full_system = if diagnostic_context.is_empty() {
            format!("{}{}{}", AUTO_ANALYSIS_PROMPT, AI_CONTEXT_SEPARATOR_SHORT, context)
        } else {
            format!(
                "{}{}{}\n\n--- DIAGNOSTIC FINDINGS ---\n\n{}",
                AUTO_ANALYSIS_PROMPT, AI_CONTEXT_SEPARATOR_SHORT, context, diagnostic_context
            )
        };
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "Analyze my system now. Give me a quick health check."
        })];
        let tx = self.insight_tx.clone();

        tokio::spawn(async move {
            let auth = ClaudeClient::discover_auth().await;
            if let Some(auth) = auth {
                let client = ClaudeClient::new(auth);
                let _ = client.ask_streaming(&full_system, messages, tx).await;
            }
        });
    }

    /// Dispatch a natural language query from the command palette to the AI.
    fn dispatch_command_ai(&mut self, query: &str) {
        self.state.command_ai_loading = true;
        let context = ContextBuilder::build(
            self.state.system.as_ref(),
            &self.state.processes,
            &self.state.alerts,
        );
        let diagnostic_context = self.build_diagnostic_context();
        let system_prompt = format!(
            "{}\n\n\
             The user asked this via the command palette. Give a concise, actionable answer.\n\
             Focus on their specific question. Use bullet points. Be brief — this appears in a popup.\n\
             If their question relates to system diagnostics, use the live data and diagnostic findings below.",
            build_system_prompt(self.state.system.as_ref()),
        );
        let full_system = if diagnostic_context.is_empty() {
            format!("{}{}{}", system_prompt, AI_CONTEXT_SEPARATOR, context)
        } else {
            format!(
                "{}{}{}\n\n--- DIAGNOSTIC FINDINGS ---\n\n{}",
                system_prompt, AI_CONTEXT_SEPARATOR, context, diagnostic_context
            )
        };
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": query
        })];
        let tx = self.command_ai_tx.clone();

        tokio::spawn(async move {
            let auth = ClaudeClient::discover_auth().await;
            if let Some(auth) = auth {
                let client = ClaudeClient::new(auth);
                let _ = client.ask_streaming(&full_system, messages, tx).await;
            }
        });
    }

    // ── Signal sending (deduplicated) ────────────────────────────

    /// Send SIGTERM to the currently selected process.
    fn send_sigterm(&mut self) {
        let filtered = self.state.filtered_processes();
        if let Some(proc) = filtered.get(self.state.selected_process) {
            let pid = proc.pid;
            let name = proc.name.clone();
            let sys = self.collector.system();
            if let Some(process) = sys.process(Pid::from_u32(pid)) {
                if process.kill_with(Signal::Term).unwrap_or(false) {
                    self.state
                        .set_status(format!("Sent SIGTERM to PID {} ({})", pid, name));
                } else {
                    self.state
                        .set_status(format!("Failed to send SIGTERM to PID {} ({})", pid, name));
                }
            }
        }
    }

    /// Send SIGKILL to the currently selected process.
    fn send_sigkill(&mut self) {
        let filtered = self.state.filtered_processes();
        if let Some(proc) = filtered.get(self.state.selected_process) {
            let pid = proc.pid;
            let name = proc.name.clone();
            let sys = self.collector.system();
            if let Some(process) = sys.process(Pid::from_u32(pid)) {
                if process.kill() {
                    self.state
                        .set_status(format!("Sent SIGKILL to PID {} ({})", pid, name));
                } else {
                    self.state
                        .set_status(format!("Failed to send SIGKILL to PID {} ({})", pid, name));
                }
            }
        }
    }

    // ── Mouse handling ───────────────────────────────────────────

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.state.show_process_detail {
                    if self.state.detail_scroll > 0 {
                        self.state.detail_scroll -= 1;
                    }
                } else {
                    self.state.scroll_up();
                }
            }
            MouseEventKind::ScrollDown => {
                if self.state.show_process_detail {
                    self.state.detail_scroll += 1;
                } else {
                    self.state.scroll_down();
                }
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                let x = mouse.column;
                let y = mouse.row;

                if self.state.show_process_detail {
                    self.state.close_process_detail();
                } else if self.state.show_help {
                    self.state.show_help = false;
                } else if y <= 2 && x >= TAB_BAR_X_OFFSET {
                    let tab_x = (x - TAB_BAR_X_OFFSET) as usize;
                    if tab_x < 13 {
                        self.state.active_tab = Tab::Dashboard;
                    } else if tab_x < 28 {
                        self.state.active_tab = Tab::Processes;
                    } else if tab_x < 40 {
                        self.state.active_tab = Tab::Alerts;
                    } else {
                        self.state.active_tab = Tab::AskAi;
                        self.ai_typing = true;
                    }
                } else if self.state.active_tab == Tab::Processes && y >= PROCESS_TABLE_ROW_START {
                    let row_index = (y - PROCESS_TABLE_ROW_START) as usize;
                    let max = if self.state.tree_view {
                        self.state.tree_processes().len()
                    } else {
                        self.state.filtered_processes().len()
                    };
                    if row_index < max {
                        self.state.selected_process = row_index;
                    }
                }
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Right) => {
                if self.state.active_tab == Tab::Processes
                    && mouse.row >= PROCESS_TABLE_ROW_START
                {
                    let row_index = (mouse.row - PROCESS_TABLE_ROW_START) as usize;
                    let proc_clone = {
                        let filtered = self.state.filtered_processes();
                        if row_index < filtered.len() {
                            Some((*filtered[row_index]).clone())
                        } else {
                            None
                        }
                    };
                    if let Some(proc) = proc_clone {
                        self.state.selected_process = row_index;
                        self.state.open_process_detail(&proc);
                    }
                }
            }
            _ => {}
        }
    }

    // ── Keyboard handling ────────────────────────────────────────

    /// Handle a key event. Returns `true` if the app should quit.
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        // Ctrl+X: abort thermal shutdown from ANY mode
        if key.code == KeyCode::Char('x') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.state.shutdown_manager.abort() {
                self.state.set_status("Thermal shutdown ABORTED".to_string());
            }
            return false;
        }

        // Command palette mode
        if self.state.show_command_palette {
            return self.handle_key_command_palette(key);
        }

        // Command result popup (scrollable)
        if self.state.command_result.is_some() {
            return self.handle_key_command_result(key);
        }

        // Help overlay mode (scrollable)
        if self.state.show_help {
            return self.handle_key_help(key);
        }

        // Process detail popup mode
        if self.state.show_process_detail {
            return self.handle_key_detail_popup(key);
        }

        // Signal picker popup mode
        if self.state.show_signal_picker {
            return self.handle_key_signal_picker(key);
        }

        // Renice dialog mode
        if self.state.show_renice_dialog {
            return self.handle_key_renice_dialog(key);
        }

        // AI typing mode
        if self.ai_typing {
            return self.handle_key_ai_typing(key);
        }

        // Filter mode
        if self.filtering {
            return self.handle_key_filter(key);
        }

        // Normal mode
        self.handle_key_normal(key)
    }

    fn handle_key_detail_popup(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.state.close_process_detail();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.detail_scroll > 0 {
                    self.state.detail_scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.detail_scroll += 1;
            }
            KeyCode::PageUp => {
                self.state.detail_scroll =
                    self.state.detail_scroll.saturating_sub(DETAIL_PAGE_STEP);
            }
            KeyCode::PageDown => {
                self.state.detail_scroll += DETAIL_PAGE_STEP;
            }
            _ => {}
        }
        false
    }

    fn handle_key_help(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.state.show_help = false;
                self.state.help_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.help_scroll > 0 {
                    self.state.help_scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.help_scroll += 1;
            }
            KeyCode::PageUp => {
                self.state.help_scroll = self.state.help_scroll.saturating_sub(PAGE_SIZE);
            }
            KeyCode::PageDown => {
                self.state.help_scroll += PAGE_SIZE;
            }
            KeyCode::Home => {
                self.state.help_scroll = 0;
            }
            KeyCode::End => {
                self.state.help_scroll = usize::MAX; // clamped at render time
            }
            _ => {}
        }
        false
    }

    fn handle_key_signal_picker(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.close_signal_picker();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.signal_picker_selected > 0 {
                    self.state.signal_picker_selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.state.signal_picker_selected < ui::SIGNAL_LIST.len() - 1 {
                    self.state.signal_picker_selected += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(pid) = self.state.signal_picker_pid {
                    let (sig_num, sig_name, _) =
                        ui::SIGNAL_LIST[self.state.signal_picker_selected];
                    let name = self.state.signal_picker_name.clone();
                    let result = unsafe { libc::kill(pid as i32, sig_num) };
                    if result == 0 {
                        self.state
                            .set_status(format!("Sent {} to PID {} ({})", sig_name, pid, name));
                    } else {
                        self.state.set_status(format!(
                            "Failed to send {} to PID {} ({})",
                            sig_name, pid, name
                        ));
                    }
                }
                self.state.close_signal_picker();
            }
            _ => {}
        }
        false
    }

    fn handle_key_renice_dialog(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.close_renice_dialog();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.state.renice_value = (self.state.renice_value - 1).max(NICE_MIN);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.state.renice_value = (self.state.renice_value + 1).min(NICE_MAX);
            }
            KeyCode::Up => {
                self.state.renice_value = (self.state.renice_value - NICE_STEP).max(NICE_MIN);
            }
            KeyCode::Down => {
                self.state.renice_value = (self.state.renice_value + NICE_STEP).min(NICE_MAX);
            }
            KeyCode::Enter => {
                if let Some(pid) = self.state.renice_pid {
                    let name = self.state.renice_name.clone();
                    let nice = self.state.renice_value;
                    let result =
                        unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, nice) };
                    if result == 0 {
                        self.state
                            .set_status(format!("Set nice {} for PID {} ({})", nice, pid, name));
                    } else {
                        let err = std::io::Error::last_os_error();
                        self.state
                            .set_status(format!("Renice failed for PID {}: {}", pid, err));
                    }
                }
                self.state.close_renice_dialog();
            }
            _ => {}
        }
        false
    }

    fn handle_key_ai_typing(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.ai_typing = false;
                self.state.ai_input.clear();
                self.state.ai_cursor_pos = 0;
            }
            KeyCode::Enter => {
                if !self.state.ai_loading {
                    if self.state.ai_submit().is_some() {
                        if self.claude_client.is_some() {
                            self.state.ai_loading = true;
                            self.dispatch_ai_chat();
                        }
                    }
                }
                self.ai_typing = false;
            }
            KeyCode::Backspace => {
                self.state.ai_input_backspace();
            }
            KeyCode::Left => {
                self.state.ai_cursor_left();
            }
            KeyCode::Right => {
                self.state.ai_cursor_right();
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.ai_conversation.clear();
                self.state.ai_input.clear();
                self.state.ai_cursor_pos = 0;
                self.ai_typing = false;
            }
            KeyCode::Char(c) => {
                self.state.ai_input_char(c);
            }
            _ => {}
        }
        false
    }

    fn handle_key_filter(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.state.filter_text.clear();
                self.filtering = false;
            }
            KeyCode::Enter => {
                self.filtering = false;
            }
            KeyCode::Backspace => {
                self.state.filter_text.pop();
                if self.state.filter_text.is_empty() {
                    self.filtering = false;
                }
            }
            KeyCode::Char(c) => {
                self.state.filter_text.push(c);
            }
            _ => {}
        }
        false
    }

    /// Handle keys in normal mode. Returns `true` if the app should quit.
    fn handle_key_normal(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                if self.state.active_tab != Tab::AskAi {
                    return true;
                }
                self.ai_typing = true;
                self.state.ai_input_char(if key.code == KeyCode::Char('Q') { 'Q' } else { 'q' });
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,

            // Tab navigation
            KeyCode::Tab => self.state.next_tab(),
            KeyCode::BackTab => self.state.prev_tab(),
            KeyCode::Char('1') => self.state.active_tab = Tab::Dashboard,
            KeyCode::Char('2') => self.state.active_tab = Tab::Processes,
            KeyCode::Char('3') => self.state.active_tab = Tab::Alerts,
            KeyCode::Char('4') => {
                self.state.active_tab = Tab::AskAi;
                self.ai_typing = true;
            }
            KeyCode::Char('5') => self.state.active_tab = Tab::Security,

            // Kill process (Processes tab only) -- must be before scroll
            KeyCode::Char('k') if self.state.active_tab == Tab::Processes => {
                self.send_sigterm();
            }
            KeyCode::Char('K') if self.state.active_tab == Tab::Processes => {
                self.send_sigkill();
            }

            // Scrolling
            KeyCode::Up | KeyCode::Char('k') => self.state.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.state.scroll_down(),
            KeyCode::PageUp => self.state.page_up(),
            KeyCode::PageDown => self.state.page_down(),
            KeyCode::Home => {
                self.state.selected_process = 0;
                self.state.alert_scroll = 0;
                self.state.ai_scroll = 0;
            }
            KeyCode::End => {
                let max = self.state.filtered_processes().len().saturating_sub(1);
                self.state.selected_process = max;
                self.state.alert_scroll = self.state.alerts.len().saturating_sub(1);
            }

            // Sort
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if self.state.active_tab != Tab::AskAi {
                    self.state.cycle_sort();
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char('s');
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if self.state.active_tab != Tab::AskAi {
                    self.state.toggle_sort_direction();
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char('r');
                }
            }

            // AI insight expand/collapse (Dashboard tab only)
            KeyCode::Char('e') if self.state.active_tab == Tab::Dashboard => {
                self.state.ai_insight_expanded = !self.state.ai_insight_expanded;
            }

            // Signal picker
            KeyCode::Char('x') if self.state.active_tab == Tab::Processes => {
                self.state.open_signal_picker();
            }

            // Renice dialog
            KeyCode::Char('n') if self.state.active_tab == Tab::Processes => {
                self.state.open_renice_dialog();
            }

            // Zoom history charts (Dashboard tab)
            KeyCode::Char('+') | KeyCode::Char('=')
                if self.state.active_tab == Tab::Dashboard =>
            {
                self.state.history_window = self.state.history_window.prev();
            }
            KeyCode::Char('-') if self.state.active_tab == Tab::Dashboard => {
                self.state.history_window = self.state.history_window.next();
            }

            // Widget focus/expand (Dashboard tab)
            KeyCode::Char('f') if self.state.active_tab == Tab::Dashboard => {
                self.state.toggle_focus();
            }
            KeyCode::Char('F')
                if self.state.active_tab == Tab::Dashboard
                    && self.state.focused_widget.is_some() =>
            {
                self.state.cycle_focus_forward();
            }

            // Theme cycling
            KeyCode::Char('T') => {
                if self.state.active_tab != Tab::AskAi {
                    self.state.cycle_theme();
                    self.state
                        .set_status(format!("Theme: {}", self.state.theme.name));
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char('T');
                }
            }

            // Language cycling
            KeyCode::Char('L') => {
                if self.state.active_tab != Tab::AskAi {
                    self.state.cycle_lang();
                    self.state
                        .set_status(format!("Language: {}", self.state.current_lang));
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char('L');
                }
            }

            // Tree view toggle (Processes tab only)
            KeyCode::Char('t') if self.state.active_tab == Tab::Processes => {
                self.state.tree_view = !self.state.tree_view;
                self.state.selected_process = 0;
            }

            // Ask AI about selected process (Processes tab only)
            KeyCode::Char('a') if self.state.active_tab == Tab::Processes => {
                self.ask_ai_about_selected_process();
            }

            // Filter
            KeyCode::Char('/') => {
                if self.state.active_tab != Tab::AskAi {
                    self.filtering = true;
                    self.state.filter_text.clear();
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char('/');
                }
            }

            // Process detail popup
            KeyCode::Enter if self.state.active_tab == Tab::Processes => {
                let proc_clone = {
                    let filtered = self.state.filtered_processes();
                    filtered
                        .get(self.state.selected_process)
                        .map(|p| (*p).clone())
                };
                if let Some(proc) = proc_clone {
                    self.state.open_process_detail(&proc);
                }
            }

            // On AI tab, Enter focuses the input
            KeyCode::Enter if self.state.active_tab == Tab::AskAi => {
                self.ai_typing = true;
            }

            // Any character on AI tab starts typing
            KeyCode::Char(c) if self.state.active_tab == Tab::AskAi => {
                if c == '?' {
                    self.state.show_help = !self.state.show_help;
                    self.state.help_scroll = 0;
                } else {
                    self.ai_typing = true;
                    self.state.ai_input_char(c);
                }
            }

            // Ctrl+L to clear AI chat from any mode
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.active_tab == Tab::AskAi {
                    self.state.ai_conversation.clear();
                }
            }

            // Command palette
            KeyCode::Char(':') if self.state.active_tab != Tab::AskAi => {
                self.state.show_command_palette = true;
                self.state.command_input.clear();
                self.state.command_cursor_pos = 0;
            }

            // Help
            KeyCode::Char('?') => {
                self.state.show_help = !self.state.show_help;
                self.state.help_scroll = 0;
            }
            KeyCode::Esc => {
                if self.state.active_tab == Tab::AskAi {
                    self.state.active_tab = Tab::Dashboard;
                } else {
                    self.state.filter_text.clear();
                }
            }

            _ => {}
        }
        false
    }

    // ── Command palette ─────────────────────────────────────────

    fn handle_key_command_palette(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.state.show_command_palette = false;
                self.state.command_input.clear();
                self.state.command_cursor_pos = 0;
            }
            KeyCode::Enter => {
                let input = self.state.command_input.trim().to_string();
                self.state.show_command_palette = false;
                self.state.command_input.clear();
                self.state.command_cursor_pos = 0;
                if !input.is_empty() {
                    self.execute_command(&input);
                }
            }
            KeyCode::Backspace => {
                if self.state.command_cursor_pos > 0 {
                    let prev = self.state.command_input[..self.state.command_cursor_pos]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.state.command_input.remove(prev);
                    self.state.command_cursor_pos = prev;
                }
                if self.state.command_input.is_empty() {
                    self.state.show_command_palette = false;
                }
            }
            KeyCode::Left => {
                if self.state.command_cursor_pos > 0 {
                    self.state.command_cursor_pos = self.state.command_input
                        [..self.state.command_cursor_pos]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if self.state.command_cursor_pos < self.state.command_input.len() {
                    self.state.command_cursor_pos = self.state.command_input
                        [self.state.command_cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.state.command_cursor_pos + i)
                        .unwrap_or(self.state.command_input.len());
                }
            }
            KeyCode::Char(c) => {
                self.state.command_input.insert(self.state.command_cursor_pos, c);
                self.state.command_cursor_pos += c.len_utf8();
            }
            _ => {}
        }
        false
    }

    fn handle_key_command_result(&mut self, key: crossterm::event::KeyEvent) -> bool {
        // Confirmation dialog takes priority
        if self.state.show_action_confirm {
            return self.handle_key_action_confirm(key);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.command_result = None;
                self.state.command_result_scroll = 0;
                self.state.command_result_selected_action = 0;
                self.state.command_ai_loading = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.state.command_result_scroll > 0 {
                    self.state.command_result_scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.command_result_scroll += 1;
            }
            KeyCode::PageUp => {
                self.state.command_result_scroll =
                    self.state.command_result_scroll.saturating_sub(PAGE_SIZE);
            }
            KeyCode::PageDown => {
                self.state.command_result_scroll += PAGE_SIZE;
            }
            // Tab / Shift+Tab to cycle through actions
            KeyCode::Tab => {
                if let Some(ref cr) = self.state.command_result {
                    let executable: Vec<usize> = cr
                        .actions
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, a))| !matches!(a, SuggestedAction::Info(_)))
                        .map(|(i, _)| i)
                        .collect();
                    if !executable.is_empty() {
                        let cur = self.state.command_result_selected_action;
                        let next_pos = executable
                            .iter()
                            .position(|&i| i > cur)
                            .unwrap_or(0);
                        self.state.command_result_selected_action = executable[next_pos];
                    }
                }
            }
            KeyCode::BackTab => {
                if let Some(ref cr) = self.state.command_result {
                    let executable: Vec<usize> = cr
                        .actions
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, a))| !matches!(a, SuggestedAction::Info(_)))
                        .map(|(i, _)| i)
                        .collect();
                    if !executable.is_empty() {
                        let cur = self.state.command_result_selected_action;
                        let prev_pos = executable
                            .iter()
                            .rposition(|&i| i < cur)
                            .unwrap_or(executable.len() - 1);
                        self.state.command_result_selected_action = executable[prev_pos];
                    }
                }
            }
            // Number keys 1-9 to select action directly
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if let Some(ref cr) = self.state.command_result {
                    // Map to executable actions only
                    let executable: Vec<usize> = cr
                        .actions
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, a))| !matches!(a, SuggestedAction::Info(_)))
                        .map(|(i, _)| i)
                        .collect();
                    if idx < executable.len() {
                        self.state.command_result_selected_action = executable[idx];
                        self.state.show_action_confirm = true;
                    }
                }
            }
            // Enter triggers confirmation for selected action
            KeyCode::Enter => {
                if let Some(ref cr) = self.state.command_result {
                    let sel = self.state.command_result_selected_action;
                    if sel < cr.actions.len() {
                        let is_executable =
                            !matches!(cr.actions[sel].1, SuggestedAction::Info(_));
                        if is_executable {
                            self.state.show_action_confirm = true;
                        }
                    } else {
                        // No action selected — close
                        self.state.command_result = None;
                        self.state.command_result_scroll = 0;
                        self.state.command_result_selected_action = 0;
                    }
                } else {
                    self.state.command_result = None;
                }
            }
            _ => {}
        }
        false
    }

    /// Handle keys in the action confirmation dialog.
    fn handle_key_action_confirm(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                // Execute the action
                self.execute_selected_action();
                self.state.show_action_confirm = false;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.state.show_action_confirm = false;
            }
            _ => {}
        }
        false
    }

    /// Execute the currently selected action from the command result.
    fn execute_selected_action(&mut self) {
        let (action_label, action) = {
            let cr = match self.state.command_result.as_ref() {
                Some(cr) => cr,
                None => return,
            };
            let sel = self.state.command_result_selected_action;
            if sel >= cr.actions.len() {
                return;
            }
            cr.actions[sel].clone()
        };

        let status = match &action {
            SuggestedAction::KillProcess { pid, name, signal } => {
                let sig_num = match *signal {
                    "SIGTERM" => libc::SIGTERM,
                    "SIGKILL" => libc::SIGKILL,
                    "SIGHUP" => libc::SIGHUP,
                    _ => libc::SIGTERM,
                };
                let result = unsafe { libc::kill(*pid as i32, sig_num) };
                if result == 0 {
                    format!("Sent {} to PID {} ({})", signal, pid, name)
                } else {
                    let err = std::io::Error::last_os_error();
                    format!("Failed to send {} to PID {} ({}): {}", signal, pid, name, err)
                }
            }
            SuggestedAction::ReniceProcess { pid, name, nice } => {
                let result =
                    unsafe { libc::setpriority(libc::PRIO_PROCESS, *pid, *nice) };
                if result == 0 {
                    format!("Set nice {} for PID {} ({})", nice, pid, name)
                } else {
                    let err = std::io::Error::last_os_error();
                    format!("Renice failed for PID {} ({}): {}", pid, name, err)
                }
            }
            SuggestedAction::FreePort { port, pid, name } => {
                let result = unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
                if result == 0 {
                    format!("Sent SIGTERM to PID {} ({}) to free port {}", pid, name, port)
                } else {
                    let err = std::io::Error::last_os_error();
                    format!("Failed to kill PID {} ({}): {}", pid, name, err)
                }
            }
            SuggestedAction::CleanDirectory { path, size_bytes } => {
                match std::fs::remove_dir_all(path) {
                    Ok(_) => {
                        // Recreate the directory so it exists but is empty
                        let _ = std::fs::create_dir_all(path);
                        format!(
                            "Cleaned {} ({:.1} GB freed)",
                            path,
                            *size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                        )
                    }
                    Err(e) => format!("Failed to clean {}: {}", path, e),
                }
            }
            SuggestedAction::Info(_) => {
                action_label.clone()
            }
        };

        self.state.set_status(status.clone());

        // Close the result popup after executing
        self.state.command_result = None;
        self.state.command_result_scroll = 0;
        self.state.command_result_selected_action = 0;
    }

    /// Parse and execute a command palette command.
    fn execute_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        let cmd = parts[0].to_lowercase();
        let result = match cmd.as_str() {
            // System diagnostics — returns report with actions
            "why" | "slow" | "why-slow" | "contention" => {
                if let Some(system) = &self.state.system {
                    let report =
                        DiagnosticEngine::resource_contention(system, &self.state.processes);
                    CommandResult::from_report(&report)
                } else {
                    CommandResult::text_only("No system data available yet.".to_string())
                }
            }

            // Timeline / absence report
            "timeline" | "history" | "what-happened" | "away" => {
                let minutes = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
                if let Some(ref store) = self.event_store {
                    let report = DiagnosticEngine::timeline_report(store, minutes);
                    CommandResult::from_report(&report)
                } else {
                    CommandResult::text_only("Event store not available.".to_string())
                }
            }

            // Port investigation
            "port" => {
                if let Some(port) = parts.get(1).and_then(|s| s.parse::<u16>().ok()) {
                    if let Some(ref store) = self.event_store {
                        let report = DiagnosticEngine::port_diagnosis(store, port);
                        CommandResult::from_report(&report)
                    } else {
                        CommandResult::text_only("Event store not available.".to_string())
                    }
                } else {
                    CommandResult::text_only(
                        "Usage: port <number>\nExample: port 8080".to_string(),
                    )
                }
            }

            // Process investigation
            "pid" | "process" => {
                if let Some(pid) = parts.get(1).and_then(|s| s.parse::<u32>().ok()) {
                    let current = self.state.processes.iter().find(|p| p.pid == pid);
                    if let Some(ref store) = self.event_store {
                        let report = DiagnosticEngine::process_analysis(store, pid, current);
                        CommandResult::from_report(&report)
                    } else {
                        CommandResult::text_only("Event store not available.".to_string())
                    }
                } else {
                    CommandResult::text_only(
                        "Usage: pid <number>\nExample: pid 1234".to_string(),
                    )
                }
            }

            // Anomaly scan
            "anomaly" | "anomalies" | "scan" => {
                let minutes = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
                if let Some(ref store) = self.event_store {
                    let report = DiagnosticEngine::anomaly_scan(store, minutes);
                    CommandResult::from_report(&report)
                } else {
                    CommandResult::text_only("Event store not available.".to_string())
                }
            }

            // Disk analysis
            "disk" | "disks" | "storage" => {
                if let Some(system) = &self.state.system {
                    let report = DiagnosticEngine::disk_analysis(system);
                    CommandResult::from_report(&report)
                } else {
                    CommandResult::text_only("No system data available yet.".to_string())
                }
            }

            // Listeners
            "listeners" | "ports" | "listen" => {
                if let Some(ref store) = self.event_store {
                    match store.query_current_listeners() {
                        Ok(listeners) if listeners.is_empty() => {
                            CommandResult::text_only("No active listeners found.".to_string())
                        }
                        Ok(listeners) => {
                            let mut lines =
                                vec![format!("# Active Listeners ({} total)", listeners.len())];
                            for s in &listeners {
                                let who = match (&s.pid, &s.name) {
                                    (Some(pid), Some(name)) => {
                                        format!("{} (PID {})", name, pid)
                                    }
                                    (Some(pid), None) => format!("PID {}", pid),
                                    _ => "unknown".to_string(),
                                };
                                lines.push(format!(
                                    "  {} {}:{} <- {}",
                                    s.protocol, s.local_addr, s.local_port, who
                                ));
                            }
                            CommandResult::text_only(lines.join("\n"))
                        }
                        Err(e) => {
                            CommandResult::text_only(format!("Error querying listeners: {}", e))
                        }
                    }
                } else {
                    CommandResult::text_only("Event store not available.".to_string())
                }
            }

            // Event timeline — visual event log
            "events" | "log" | "event-log" => {
                let minutes = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
                if let Some(ref store) = self.event_store {
                    let since_ms =
                        crate::store::now_epoch_ms_pub() - (minutes as i64 * 60 * 1000);
                    match store.query_events_since(since_ms) {
                        Ok(events) if events.is_empty() => {
                            CommandResult::text_only(format!(
                                "# Event Timeline (last {} min)\n\nNo events recorded.",
                                minutes
                            ))
                        }
                        Ok(events) => {
                            let now = crate::store::now_epoch_ms_pub();
                            let mut lines = vec![format!(
                                "# Event Timeline (last {} min, {} events)",
                                minutes,
                                events.len()
                            )];
                            lines.push(String::new());

                            // Group by time buckets for readability
                            let mut last_bucket = String::new();
                            for e in events.iter().take(100) {
                                let age_ms = now - e.ts;
                                let bucket = if age_ms < 60_000 {
                                    "Just now".to_string()
                                } else if age_ms < 300_000 {
                                    format!("{}m ago", age_ms / 60_000)
                                } else if age_ms < 3_600_000 {
                                    format!("{}m ago", age_ms / 60_000)
                                } else {
                                    format!("{}h {}m ago", age_ms / 3_600_000, (age_ms % 3_600_000) / 60_000)
                                };

                                if bucket != last_bucket {
                                    if !last_bucket.is_empty() {
                                        lines.push(String::new());
                                    }
                                    lines.push(format!("  --- {} ---", bucket));
                                    last_bucket = bucket;
                                }

                                let icon = match e.kind.as_str() {
                                    "process_start" => "+",
                                    "process_exit" => "-",
                                    "port_bind" => ">",
                                    "port_release" => "<",
                                    "alert" => "!",
                                    "cpu_spike" => "^",
                                    "memory_spike" => "~",
                                    "oom_kill" => "X",
                                    _ => "?",
                                };

                                let pid_str = e
                                    .pid
                                    .map(|p| format!(" PID {}", p))
                                    .unwrap_or_default();
                                let name_str = e
                                    .name
                                    .as_deref()
                                    .map(|n| format!(" ({})", n))
                                    .unwrap_or_default();
                                let severity_str = e
                                    .severity
                                    .as_deref()
                                    .filter(|s| !s.is_empty())
                                    .map(|s| format!(" [{}]", s))
                                    .unwrap_or_default();

                                let kind_label = match e.kind.as_str() {
                                    "process_start" => "Started",
                                    "process_exit" => "Exited",
                                    "port_bind" => "Port bound",
                                    "port_release" => "Port released",
                                    "alert" => "Alert",
                                    "cpu_spike" => "CPU spike",
                                    "memory_spike" => "Memory spike",
                                    "oom_kill" => "OOM Kill",
                                    other => other,
                                };

                                lines.push(format!(
                                    "  {} {}{}{}{}",
                                    icon, kind_label, pid_str, name_str, severity_str
                                ));
                            }

                            if events.len() > 100 {
                                lines.push(String::new());
                                lines.push(format!(
                                    "  ... and {} more events (showing first 100)",
                                    events.len() - 100
                                ));
                            }

                            // Event count summary at the bottom
                            lines.push(String::new());
                            lines.push("# Summary".to_string());
                            let mut kind_counts: std::collections::HashMap<&str, usize> =
                                std::collections::HashMap::new();
                            for e in &events {
                                *kind_counts.entry(&e.kind).or_default() += 1;
                            }
                            let mut sorted: Vec<_> = kind_counts.into_iter().collect();
                            sorted.sort_by(|a, b| b.1.cmp(&a.1));
                            for (kind, count) in sorted {
                                lines.push(format!("  {}: {}", kind, count));
                            }

                            CommandResult::text_only(lines.join("\n"))
                        }
                        Err(e) => {
                            CommandResult::text_only(format!("Error querying events: {}", e))
                        }
                    }
                } else {
                    CommandResult::text_only("Event store not available.".to_string())
                }
            }

            // Configuration info
            "config" | "settings" | "cfg" => {
                let config_path = crate::constants::config_file_path();
                let themes_dir = crate::constants::custom_theme_dir();
                let db_path = crate::constants::data_dir().join("sentinel.db");

                let config_exists = config_path.exists();
                let db_exists = db_path.exists();
                let db_size = if db_exists {
                    std::fs::metadata(&db_path)
                        .map(|m| crate::models::format_bytes(m.len()))
                        .unwrap_or_else(|_| "?".to_string())
                } else {
                    "not created".to_string()
                };

                let mut lines = vec![
                    "# Sentinel Configuration".to_string(),
                    String::new(),
                    "# File Paths".to_string(),
                    format!(
                        "  Config:  {} {}",
                        config_path.display(),
                        if config_exists { "(loaded)" } else { "(not found - using defaults)" }
                    ),
                    format!("  Themes:  {}/", themes_dir.display()),
                    format!("  Data:    {} ({})", db_path.display(), db_size),
                    String::new(),
                    "# Current Settings".to_string(),
                    format!("  Theme:         {}", self.state.theme.name),
                    format!("  Language:      {}", self.state.current_lang),
                    format!("  CJK support:   {}", if self.state.cjk_supported { "yes" } else { "no (JA/ZH skipped)" }),
                    format!("  AI enabled:    {}", if self.has_key { "yes" } else { "no" }),
                ];

                if self.has_key {
                    lines.push(format!("  AI auth:       {}", self.state.ai_auth_method));
                }

                lines.push(format!("  Docker:        {}", if self.state.docker_available { "yes" } else { "no" }));
                lines.push(String::new());
                lines.push("# CLI Flags".to_string());
                lines.push("  Run 'sentinel --help' for all options".to_string());
                lines.push("  Key flags: --no-ai, --theme, --refresh-rate,".to_string());
                lines.push("             --prometheus, --lang, --no-auto-analysis".to_string());

                if !config_exists {
                    lines.push(String::new());
                    lines.push("# Create Config File".to_string());
                    lines.push(format!("  mkdir -p {}", config_path.parent().unwrap_or(std::path::Path::new("~")).display()));
                    lines.push(format!("  $EDITOR {}", config_path.display()));
                    lines.push(String::new());
                    lines.push("  Example config.toml:".to_string());
                    lines.push("  refresh_interval_ms = 1000".to_string());
                    lines.push("  theme = \"gruvbox\"".to_string());
                    lines.push("  lang = \"en\"".to_string());
                    lines.push("  max_alerts = 100".to_string());
                    lines.push("  auto_analysis_interval_secs = 300".to_string());
                }

                CommandResult::text_only(lines.join("\n"))
            }

            // Event store stats
            "stats" | "db" | "store" => {
                if let Some(ref store) = self.event_store {
                    match store.table_stats() {
                        Ok(stats) => {
                            let size = store.db_size_bytes();
                            let mut lines = vec![
                                "# Event Store Statistics".to_string(),
                                format!(
                                    "  Database size: {}",
                                    crate::models::format_bytes(size)
                                ),
                            ];
                            for (table, count) in &stats {
                                lines.push(format!("  {}: {} rows", table, count));
                            }
                            CommandResult::text_only(lines.join("\n"))
                        }
                        Err(e) => CommandResult::text_only(format!("Error: {}", e)),
                    }
                } else {
                    CommandResult::text_only("Event store not available.".to_string())
                }
            }

            // Thermal status
            "thermal" | "temps" | "temperature" => {
                if let Some(ref snap) = self.state.thermal {
                    let mut text = snap.to_text();
                    text.push_str("\n\n");
                    // Add config info
                    text.push_str(&format!("Warning threshold: {:.0}°C\n",
                        self.detector.config_thermal_warning()));
                    text.push_str(&format!("Critical threshold: {:.0}°C\n",
                        self.detector.config_thermal_critical()));
                    text.push_str(&format!("Emergency threshold: {:.0}°C\n",
                        self.detector.config_thermal_emergency()));
                    text.push_str(&format!("Auto-shutdown: {}\n",
                        if self.state.shutdown_manager.is_enabled() { "ENABLED" } else { "disabled" }));
                    if self.email_notifier.is_some() {
                        text.push_str("Email notifications: configured\n");
                    } else {
                        text.push_str("Email notifications: not configured (no .env credentials)\n");
                    }
                    CommandResult::text_only(text)
                } else {
                    CommandResult::text_only(
                        "# Thermal Monitor\n\n\
                         No thermal data available.\n\n\
                         LibreHardwareMonitor is not reachable or not running.\n\
                         Ensure LHM is running on Windows with Web Server enabled:\n\
                         Options → Web Server → Enable\n\n\
                         Expected URL: http://localhost:8085/data.json".to_string()
                    )
                }
            }

            // Email test
            "email-test" | "test-email" => {
                if let Some(ref mut notifier) = self.email_notifier {
                    let config = notifier.config().clone();
                    let recipient = config.recipient.clone();
                    let server = config.server.clone();
                    let port = config.port;
                    let mut temp_notifier = EmailNotifier::new(config);
                    // Fire the test email in background
                    tokio::spawn(async move {
                        match temp_notifier.send_test().await {
                            Ok(()) => eprintln!("Test email sent successfully"),
                            Err(e) => eprintln!("Test email failed: {}", e),
                        }
                    });
                    CommandResult::text_only(format!(
                        "# Email Test\n\n\
                         Sending test email...\n\n\
                         Server: {}:{}\n\
                         To: {}\n\n\
                         Check your inbox (and spam folder).",
                        server, port, recipient,
                    ))
                } else {
                    CommandResult::text_only(
                        "# Email Test\n\n\
                         Email notifications are not configured.\n\n\
                         Create ~/.config/sentinel/.env with:\n\
                         SENTINEL_SMTP_USER=your-email@gmail.com\n\
                         SENTINEL_SMTP_PASSWORD=xxxx xxxx xxxx xxxx\n\
                         SENTINEL_SMTP_RECIPIENT=destination@example.com\n\n\
                         For Gmail: use an App Password (not your main password).\n\
                         Enable 2FA first, then create an App Password at:\n\
                         https://myaccount.google.com/apppasswords".to_string()
                    )
                }
            }

            // Help
            "help" | "?" | "commands" => CommandResult::text_only(
                 "# Command Palette\n\n\
                 System:\n\
                 \x20 why / slow         - Resource contention analysis\n\
                 \x20 disk               - Disk usage analysis\n\
                 \x20 anomaly [minutes]  - Anomaly scan (default: 30 min)\n\
                 \x20 timeline [minutes] - What happened recently\n\n\
                 Thermal:\n\
                 \x20 thermal            - Current thermal snapshot (LHM)\n\
                 \x20 email-test         - Send a test notification email\n\n\
                 Network:\n\
                 \x20 port <number>      - Who's using this port?\n\
                 \x20 listeners          - All active port listeners\n\n\
                 Process:\n\
                 \x20 pid <number>       - Deep process analysis\n\n\
                 Events:\n\
                 \x20 events [minutes]   - Event timeline (default: 30 min)\n\n\
                 Meta:\n\
                 \x20 config             - Show configuration & paths\n\
                 \x20 stats              - Event store statistics\n\
                 \x20 help               - This help message\n\n\
                 Actions:\n\
                 \x20 When actions (kill, renice, clean) appear in results,\n\
                 \x20 use Tab to select and Enter to execute them."
                    .to_string(),
            ),

            _ => {
                // Natural language fallback: route to AI if available
                if self.has_key && self.claude_client.is_some() {
                    self.dispatch_command_ai(input);
                    CommandResult::text_only(format!(
                        "# AI Query: {}\n\nThinking...",
                        input
                    ))
                } else {
                    CommandResult::text_only(format!(
                        "Unknown command: '{}'\nType 'help' for available commands.\n\n\
                         Tip: With an AI key configured, unrecognized commands\n\
                         are automatically sent to the AI for analysis.",
                        input
                    ))
                }
            }
        };

        self.state.command_result = Some(result);
        self.state.command_result_scroll = 0;
        self.state.command_result_selected_action = 0;
        self.state.show_action_confirm = false;
    }

    // ── Contextual AI query ──────────────────────────────────────

    fn ask_ai_about_selected_process(&mut self) {
        if !self.has_key || self.state.ai_loading {
            return;
        }

        let proc_clone = {
            let filtered = self.state.filtered_processes();
            filtered
                .get(self.state.selected_process)
                .map(|p| (*p).clone())
        };
        if let Some(proc) = proc_clone {
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

            self.state.active_tab = Tab::AskAi;
            self.state.ai_conversation.add_user_message(&question);
            self.state.ai_loading = true;
            self.dispatch_ai_chat();
        }
    }

    // ── Tick-based logic ─────────────────────────────────────────

    fn tick_refresh(&mut self) {
        let should_refresh =
            self.state.tick_count == 0 || self.state.tick_count % REFRESH_THROTTLE_TICKS == 0;

        if should_refresh {
            let (system, processes) = self.collector.collect();
            let mut new_alerts = self.detector.analyze(&system, &processes);

            // Check thermal data for temperature alerts
            if let Some(ref thermal) = self.state.thermal {
                let thermal_alerts = self.detector.check_thermal(thermal);
                new_alerts.extend(thermal_alerts);
            }

            // Record to event store
            if let Some(ref mut store) = self.event_store {
                // System snapshot
                let _ = store.insert_system_snapshot(&system);

                // Process snapshots (top N)
                let _ = store.insert_process_snapshots(&processes);

                // Detect process start/exit events
                let _ = store.detect_process_lifecycle(&processes);

                // Record alerts as events
                for alert in &new_alerts {
                    let detail = serde_json::json!({
                        "category": alert.category.to_string(),
                        "message": alert.message,
                        "value": alert.value,
                        "threshold": alert.threshold,
                    })
                    .to_string();
                    let severity = alert.severity.to_string().to_lowercase();
                    let _ = store.insert_event(
                        crate::store::EventKind::Alert,
                        Some(alert.pid),
                        Some(&alert.process_name),
                        Some(&detail),
                        Some(&severity),
                    );
                }

                // Network socket scan (less frequent — every net_scan_interval ticks)
                if self.state.tick_count % self.net_scan_interval == 0 {
                    let _ = store.insert_network_sockets();
                    let _ = store.detect_port_changes();
                }
            }

            // Update event ticker for dashboard (last 5 events)
            if let Some(ref store) = self.event_store {
                let five_min_ago =
                    crate::store::now_epoch_ms_pub() - (5 * 60 * 1000);
                if let Ok(events) = store.query_events_since(five_min_ago) {
                    let now = crate::store::now_epoch_ms_pub();
                    self.state.recent_events = events
                        .iter()
                        .take(8)
                        .map(|e| {
                            let age_ms = now - e.ts;
                            let age = if age_ms < 60_000 {
                                "now".to_string()
                            } else {
                                format!("{}m", age_ms / 60_000)
                            };
                            let icon = match e.kind.as_str() {
                                "process_start" => "+",
                                "process_exit" => "-",
                                "port_bind" => ">",
                                "port_release" => "<",
                                "alert" => "!",
                                "cpu_spike" => "^",
                                "memory_spike" => "~",
                                "oom_kill" => "X",
                                _ => "?",
                            };
                            let name = e
                                .name
                                .as_deref()
                                .unwrap_or("unknown");
                            format!("[{}] {} {}", age, icon, name)
                        })
                        .collect();
                }
            }

            self.state.update(system, processes, new_alerts);

            // Update Prometheus metrics snapshot
            if let Some(ref metrics_handle) = self.shared_metrics {
                if let Ok(mut snap) = metrics_handle.lock() {
                    snap.system = self.state.system.clone();
                    snap.process_count = self.state.processes.len();
                    snap.alerts = self.state.alerts.clone();
                    snap.containers = self.state.containers.clone();
                }
            }
        } else {
            self.state.tick_count += 1;
        }
    }

    fn tick_auto_analysis(&mut self) {
        if !self.auto_analysis_enabled || !self.has_key || self.state.ai_insight_loading {
            return;
        }

        let should_analyze = match self.last_insight_time {
            None => self.state.tick_count >= STARTUP_SETTLE_TICKS,
            Some(t) => t.elapsed() >= self.insight_interval,
        };

        if should_analyze {
            self.state.ai_insight_loading = true;
            self.state.ai_insight = None;
            self.state.ai_insight_scroll = 0;
            self.last_insight_time = Some(std::time::Instant::now());
            self.dispatch_insight();
        }
    }

    /// Tick the thermal shutdown state machine and send email notifications.
    fn tick_shutdown(&mut self) {
        // Only tick on refresh cycles (not every UI poll)
        if self.state.tick_count % REFRESH_THROTTLE_TICKS != 0 {
            return;
        }

        let max_temp = self.state.thermal.as_ref()
            .map(|t| t.max_temp)
            .unwrap_or(0.0);

        let event = self.state.shutdown_manager.tick(max_temp);

        // Get hostname for emails
        let hostname = gethostname();

        match event {
            ShutdownEvent::None => {}
            ShutdownEvent::EmergencyStarted => {
                self.state.set_status(format!(
                    "THERMAL EMERGENCY: {:.1}°C — counting sustained seconds...",
                    max_temp
                ));
                self.send_thermal_email(NotifyEvent::ThermalCritical, max_temp, &hostname);
            }
            ShutdownEvent::Counting { elapsed_secs, required_secs } => {
                self.state.set_status(format!(
                    "THERMAL: {:.1}°C sustained {}/{}s",
                    max_temp, elapsed_secs, required_secs
                ));
            }
            ShutdownEvent::GracePeriodStarted => {
                self.state.set_status(
                    "SHUTDOWN GRACE PERIOD — Press Ctrl+X to ABORT".to_string()
                );
                self.send_thermal_email(NotifyEvent::ShutdownImminent, max_temp, &hostname);
            }
            ShutdownEvent::GracePeriodCountdown { remaining_secs } => {
                self.state.set_status(format!(
                    "SHUTDOWN IN {}s — Press Ctrl+X to ABORT",
                    remaining_secs
                ));
            }
            ShutdownEvent::ShutdownNow => {
                self.state.set_status("EXECUTING SHUTDOWN...".to_string());
                // Send final email before shutdown
                self.send_thermal_email(NotifyEvent::ThermalEmergency, max_temp, &hostname);
                // Execute shutdown
                if let Err(e) = crate::thermal::shutdown::execute_shutdown() {
                    self.state.set_status(format!("Shutdown failed: {}", e));
                }
            }
            ShutdownEvent::Recovered => {
                self.state.set_status(format!(
                    "Temperature recovered: {:.1}°C — normal operation",
                    max_temp
                ));
                self.send_thermal_email(NotifyEvent::Recovered, max_temp, &hostname);
            }
        }
    }

    /// Send a thermal email notification in the background.
    fn send_thermal_email(&mut self, event: NotifyEvent, temp: f32, hostname: &str) {
        if let Some(ref mut notifier) = self.email_notifier {
            let sensor = self.state.thermal.as_ref()
                .map(|t| {
                    if t.max_cpu_temp >= t.max_gpu_temp {
                        "CPU"
                    } else {
                        "GPU"
                    }
                })
                .unwrap_or("Unknown");

            // Check rate limit synchronously before spawning
            let body = notifications::thermal_alert_body(&event, temp, sensor, hostname);

            // We can't easily clone the notifier for async, so we do a synchronous
            // rate-limit check and only fire if allowed. The actual send is fire-and-forget.
            if notifier.can_send_check(&event) {
                notifier.mark_sent(&event);
                let smtp_config = notifier.config().clone();
                tokio::spawn(async move {
                    let mut temp_notifier = EmailNotifier::new(smtp_config);
                    let _ = temp_notifier.notify(event, &body).await;
                });
            }
        }
    }
}

/// Get the system hostname (best-effort).
fn gethostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "sentinel-host".to_string())
}
