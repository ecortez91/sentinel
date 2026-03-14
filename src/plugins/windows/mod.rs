//! Windows host monitoring plugin (#1, #3, #4).
//!
//! Polls a sentinel-agent HTTP endpoint for system snapshots and renders
//! them in a dedicated TUI tab. Uses the same channel-based async pattern
//! as the Market plugin to keep the event loop responsive.

pub mod models;
pub mod renderer;
pub mod state;

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, Frame};
use tokio::sync::mpsc;

use crate::constants::{ALERT_COOLDOWN_SECS, ENV_AGENT_URL, PAGE_SIZE, WINDOWS_UPDATE_STALE_DAYS};
use crate::models::{Alert, AlertCategory, AlertSeverity};
use crate::plugins::{Plugin, PluginAction};
use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

use models::WindowsHostSnapshot;
use state::WindowsState;

/// Result from a background agent poll.
enum AgentPollResult {
    /// Successfully received a snapshot.
    Data(WindowsHostSnapshot),
    /// Agent unreachable or returned an error.
    Error(String),
}

/// Windows host monitoring plugin.
pub struct WindowsPlugin {
    state: WindowsState,
    /// Receiver for background polling results.
    poll_rx: mpsc::UnboundedReceiver<AgentPollResult>,
    /// Sender cloned into the background polling task.
    poll_tx: mpsc::UnboundedSender<AgentPollResult>,
    /// Whether the background poller has been spawned.
    poller_spawned: bool,
    /// Agent snapshot endpoint URL.
    agent_url: String,
    /// Polling interval in seconds.
    poll_interval_secs: u64,
    /// Whether the plugin is enabled.
    enabled: bool,
    /// Alert cooldowns keyed by (pseudo_pid, category) to prevent duplicates.
    alert_cooldowns: HashMap<(u32, AlertCategory), Instant>,
}

impl WindowsPlugin {
    /// Create a new Windows host monitoring plugin.
    pub fn new(enabled: bool, agent_url: String, poll_interval_secs: u64) -> Self {
        let (poll_tx, poll_rx) = mpsc::unbounded_channel();

        // Allow ENV_AGENT_URL to override the config URL
        let agent_url = std::env::var(ENV_AGENT_URL).unwrap_or(agent_url);

        Self {
            state: WindowsState::new(),
            poll_rx,
            poll_tx,
            poller_spawned: false,
            agent_url,
            poll_interval_secs,
            enabled,
            alert_cooldowns: HashMap::new(),
        }
    }

    /// Resolve the agent URL, auto-detecting WSL host IP if needed.
    fn resolve_agent_url(&self) -> String {
        // If the URL already points to a non-localhost address, use it as-is
        if !self.agent_url.contains("localhost") && !self.agent_url.contains("127.0.0.1") {
            return self.agent_url.clone();
        }

        // On WSL2, localhost doesn't reach the Windows host — detect the host IP
        if crate::utils::is_wsl() {
            if let Some(host_ip) = crate::thermal::detect_wsl_host_ip() {
                return self
                    .agent_url
                    .replace("localhost", &host_ip)
                    .replace("127.0.0.1", &host_ip);
            }
        }

        self.agent_url.clone()
    }

    /// Build AI context string from the current snapshot for security analysis.
    fn build_ai_context(&self) -> Option<String> {
        let snap = self.state.snapshot.as_ref()?;
        let mut ctx = String::with_capacity(2048);

        ctx.push_str("Analyze this Windows host security posture:\n\n");
        ctx.push_str("=== SYSTEM ===\n");
        ctx.push_str(&format!("Hostname: {}\n", snap.hostname));
        ctx.push_str(&format!("OS: {}\n", snap.os_version));
        ctx.push_str(&format!("Uptime: {}h\n", snap.uptime_secs / 3600));
        ctx.push_str(&format!(
            "CPU: {:.1}% | RAM: {:.0}% ({} cores)\n",
            snap.cpu_usage_pct,
            snap.memory_usage_pct(),
            snap.cpu_cores,
        ));

        // Security status
        if let Some(ref sec) = snap.security {
            ctx.push_str("\n=== SECURITY STATUS ===\n");
            for p in &sec.firewall_profiles {
                ctx.push_str(&format!(
                    "Firewall {}: {}\n",
                    p.name,
                    if p.enabled { "ON" } else { "OFF" }
                ));
            }
            if let Some(d) = sec.defender_enabled {
                ctx.push_str(&format!("Defender: {}\n", if d { "ON" } else { "OFF" }));
            }
            if let Some(rt) = sec.defender_realtime {
                ctx.push_str(&format!("Real-time protection: {}\n", if rt { "ON" } else { "OFF" }));
            }
            if let Some(days) = sec.last_update_days {
                ctx.push_str(&format!("Last Windows Update: {} days ago\n", days));
            }
        }

        // Connections summary
        if !snap.tcp_connections.is_empty() {
            let established = snap.tcp_connections.iter().filter(|c| c.state == "ESTABLISHED").count();
            let suspicious = snap.tcp_connections.iter().filter(|c| {
                c.state == "ESTABLISHED" && !crate::constants::STANDARD_PORTS.contains(&c.remote_port)
            }).count();
            ctx.push_str(&format!(
                "\n=== CONNECTIONS ({} total, {} established, {} suspicious) ===\n",
                snap.tcp_connections.len(), established, suspicious
            ));
            // Include top 10 connections (prioritize suspicious)
            let mut conns: Vec<_> = snap.tcp_connections.iter().collect();
            conns.sort_by_key(|c| if c.state == "ESTABLISHED" && !crate::constants::STANDARD_PORTS.contains(&c.remote_port) { 0 } else { 1 });
            for c in conns.iter().take(10) {
                ctx.push_str(&format!(
                    "  {}:{} -> {}:{} [{}] {}\n",
                    c.local_addr, c.local_port, c.remote_addr, c.remote_port, c.state, c.process_name
                ));
            }
        }

        // Listening ports
        if !snap.listening_ports.is_empty() {
            ctx.push_str(&format!("\n=== LISTENING PORTS ({}) ===\n", snap.listening_ports.len()));
            for p in snap.listening_ports.iter().take(15) {
                ctx.push_str(&format!("  {} {} {} (PID:{})\n", p.port, p.protocol, p.process_name, p.pid));
            }
        }

        // Startup programs
        if !snap.startup_programs.is_empty() {
            ctx.push_str(&format!("\n=== STARTUP PROGRAMS ({}) ===\n", snap.startup_programs.len()));
            for s in snap.startup_programs.iter().take(10) {
                ctx.push_str(&format!("  {} -> {} [{}]\n", s.name, s.command, s.location));
            }
        }

        // Users
        if !snap.logged_in_users.is_empty() {
            ctx.push_str(&format!("\n=== LOGGED-IN USERS ({}) ===\n", snap.logged_in_users.len()));
            for u in &snap.logged_in_users {
                ctx.push_str(&format!("  {} ({}, {})\n", u.username, u.session_type, u.state));
            }
        }

        ctx.push_str("\nProvide a security assessment covering:\n");
        ctx.push_str("1. Overall security posture (good/moderate/poor)\n");
        ctx.push_str("2. Immediate risks or concerns\n");
        ctx.push_str("3. Suspicious connections or processes\n");
        ctx.push_str("4. Specific recommendations\n");
        ctx.push_str("Keep it concise and actionable (under 300 words).\n");

        Some(ctx)
    }

    /// Spawn the background polling task. Called once on first tick.
    fn spawn_poller(&mut self) {
        if self.poller_spawned {
            return;
        }
        if !self.enabled {
            // Show "Agent not connected" with setup instructions
            // instead of "Connecting..." forever.
            self.state.loading = false;
            return;
        }
        self.poller_spawned = true;

        let tx = self.poll_tx.clone();
        let url = self.resolve_agent_url();
        let interval = self.poll_interval_secs;

        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default();

            loop {
                match client.get(&url).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            match resp.json::<WindowsHostSnapshot>().await {
                                Ok(snapshot) => {
                                    if tx.send(AgentPollResult::Data(snapshot)).is_err() {
                                        return; // channel closed
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(AgentPollResult::Error(format!(
                                        "JSON parse error: {}",
                                        e
                                    )));
                                }
                            }
                        } else {
                            let _ = tx.send(AgentPollResult::Error(format!(
                                "HTTP {}",
                                resp.status()
                            )));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(AgentPollResult::Error(format!(
                            "Connection failed: {}",
                            e
                        )));
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        });
    }
}

impl Plugin for WindowsPlugin {
    fn id(&self) -> &str {
        "windows"
    }

    fn tab_label(&self) -> &str {
        "Windows"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn tick(&mut self) {
        // Spawn poller on first tick
        self.spawn_poller();

        // Drain the poll channel
        while let Ok(result) = self.poll_rx.try_recv() {
            match result {
                AgentPollResult::Data(snapshot) => {
                    self.state.snapshot = Some(snapshot);
                    self.state.loading = false;
                    self.state.error = None;
                    self.state.last_updated = Some(Instant::now());
                    self.state.agent_connected = true;
                }
                AgentPollResult::Error(e) => {
                    self.state.loading = false;
                    self.state.agent_connected = false;
                    self.state.error = Some(e);
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PluginAction {
        match key.code {
            // AI analysis
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if !self.state.ai_loading {
                    if let Some(ctx) = self.build_ai_context() {
                        self.state.ai_loading = true;
                        self.state.ai_analysis = None;
                        self.state.ai_scroll = 0;
                        return PluginAction::RequestAiAnalysis(ctx);
                    } else {
                        return PluginAction::SetStatus(
                            "No snapshot available for AI analysis".to_string(),
                        );
                    }
                }
                PluginAction::Consumed
            }
            // Sort controls
            KeyCode::Char('s') => {
                self.state.cycle_sort();
                PluginAction::Consumed
            }
            KeyCode::Char('S') => {
                self.state.toggle_sort_direction();
                PluginAction::Consumed
            }
            // Focus/expand controls
            KeyCode::Char('f') => {
                self.state.toggle_panel_focus();
                PluginAction::Consumed
            }
            KeyCode::Char('F') if self.state.focused_panel.is_some() => {
                self.state.cycle_panel_forward();
                PluginAction::Consumed
            }
            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.move_selection_up();
                PluginAction::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.move_selection_down();
                PluginAction::Consumed
            }
            KeyCode::PageUp => {
                for _ in 0..PAGE_SIZE {
                    self.state.move_selection_up();
                }
                PluginAction::Consumed
            }
            KeyCode::PageDown => {
                for _ in 0..PAGE_SIZE {
                    self.state.move_selection_down();
                }
                PluginAction::Consumed
            }
            KeyCode::Home => {
                self.state.selected_process = 0;
                self.state.scroll_offset = 0;
                PluginAction::Consumed
            }
            KeyCode::End => {
                if let Some(ref snap) = self.state.snapshot {
                    self.state.selected_process =
                        snap.top_processes.len().saturating_sub(1);
                }
                PluginAction::Consumed
            }
            _ => PluginAction::Ignored,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme, glyphs: &Glyphs) {
        renderer::render_windows(frame, area, &self.state, theme, glyphs);
    }

    fn status_bar_hints(&self) -> Vec<(&str, &str)> {
        vec![
            ("j/k", "Navigate"),
            ("s", "Sort"),
            ("S", "Direction"),
            ("f", "Focus"),
            ("a", "AI Analysis"),
        ]
    }

    fn help_entries(&self) -> Vec<(&str, &str)> {
        vec![
            ("j / Up", "Move selection up"),
            ("k / Down", "Move selection down"),
            ("s", "Cycle sort field (CPU/RAM/PID/Name)"),
            ("S", "Toggle sort direction (asc/desc)"),
            ("f", "Focus/expand current panel"),
            ("F", "Cycle focused panel"),
            ("a", "AI security analysis (Haiku)"),
            ("PgUp", "Page up in process list"),
            ("PgDn", "Page down in process list"),
            ("Home", "Jump to first process"),
            ("End", "Jump to last process"),
        ]
    }

    fn commands(&self) -> Vec<(&str, &str)> {
        vec![("windows", "Show Windows host status")]
    }

    fn execute_command(&mut self, cmd: &str, _args: &str) -> Option<String> {
        match cmd {
            "windows" => {
                if let Some(ref snap) = self.state.snapshot {
                    let mut lines = vec![format!(
                        "# {} ({})",
                        snap.hostname, snap.os_version
                    )];
                    lines.push(format!(
                        "CPU: {:.1}% | RAM: {:.0}% | Cores: {}",
                        snap.cpu_usage_pct,
                        snap.memory_usage_pct(),
                        snap.cpu_cores,
                    ));
                    lines.push(format!("Processes: {}", snap.top_processes.len()));
                    lines.push(format!("Disks: {}", snap.disks.len()));
                    if let Some(ref gpu) = snap.gpu {
                        lines.push(format!(
                            "GPU: {} ({:.0}%, {:.0}C)",
                            gpu.name, gpu.usage_pct, gpu.temp_celsius
                        ));
                    }
                    Some(lines.join("\n"))
                } else if self.state.agent_connected {
                    Some("Windows agent connected, waiting for data...".to_string())
                } else {
                    Some(format!(
                        "Windows agent not connected. URL: {}",
                        self.agent_url
                    ))
                }
            }
            _ => None,
        }
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

    fn security_alerts(&mut self) -> Vec<Alert> {
        let snap = match &self.state.snapshot {
            Some(s) => s,
            None => return Vec::new(),
        };
        let sec = match &snap.security {
            Some(s) => s,
            None => return Vec::new(),
        };

        let mut raw_alerts = Vec::new();
        let hostname = &snap.hostname;

        // Firewall profile OFF alerts
        for profile in &sec.firewall_profiles {
            if !profile.enabled {
                // Use a pseudo-PID based on profile name for cooldown dedup
                let pseudo_pid = profile.name.bytes().fold(10000u32, |acc, b| acc.wrapping_add(b as u32));
                raw_alerts.push(Alert::new(
                    AlertSeverity::Warning,
                    AlertCategory::WindowsFirewall,
                    hostname,
                    pseudo_pid,
                    format!(
                        "Windows Firewall is OFF on {} profile ({})",
                        profile.name, hostname
                    ),
                    0.0,
                    1.0,
                ));
            }
        }

        // Defender disabled
        if let Some(false) = sec.defender_enabled {
            raw_alerts.push(Alert::new(
                AlertSeverity::Danger,
                AlertCategory::WindowsDefender,
                hostname,
                20000,
                format!("Windows Defender is disabled ({})", hostname),
                0.0,
                1.0,
            ));
        }

        // Defender real-time protection OFF
        if let Some(false) = sec.defender_realtime {
            // Only alert if Defender itself is enabled — if Defender is off,
            // the above alert already covers it.
            if sec.defender_enabled != Some(false) {
                raw_alerts.push(Alert::new(
                    AlertSeverity::Warning,
                    AlertCategory::WindowsDefender,
                    hostname,
                    20001,
                    format!(
                        "Windows Defender real-time protection is OFF ({})",
                        hostname
                    ),
                    0.0,
                    1.0,
                ));
            }
        }

        // Stale Windows updates
        if let Some(days) = sec.last_update_days {
            if days > WINDOWS_UPDATE_STALE_DAYS {
                raw_alerts.push(Alert::new(
                    AlertSeverity::Warning,
                    AlertCategory::WindowsUpdates,
                    hostname,
                    30000,
                    format!(
                        "Windows not updated in {} days ({})",
                        days, hostname
                    ),
                    days as f64,
                    WINDOWS_UPDATE_STALE_DAYS as f64,
                ));
            }
        }

        // Apply cooldown dedup (same pattern as AlertDetector)
        let now = Instant::now();
        let cooldown = std::time::Duration::from_secs(ALERT_COOLDOWN_SECS);
        raw_alerts
            .into_iter()
            .filter(|alert| {
                let key = (alert.pid, alert.category);
                match self.alert_cooldowns.get(&key) {
                    Some(last_fired) if now.duration_since(*last_fired) < cooldown => false,
                    _ => {
                        self.alert_cooldowns.insert(key, now);
                        true
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::state::WindowsSortField;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_plugin() -> WindowsPlugin {
        WindowsPlugin::new(true, "http://localhost:8085/api/snapshot".into(), 5)
    }

    /// Helper: run a closure inside a tokio runtime so `tokio::spawn` works.
    fn with_runtime<F: FnOnce()>(f: F) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { f() });
    }

    fn make_snapshot() -> WindowsHostSnapshot {
        models::make_test_snapshot()
    }

    #[test]
    fn plugin_id_and_label() {
        let plugin = make_plugin();
        assert_eq!(plugin.id(), "windows");
        assert_eq!(plugin.tab_label(), "Windows");
    }

    #[test]
    fn is_enabled_respects_config() {
        let enabled = WindowsPlugin::new(true, String::new(), 5);
        assert!(enabled.is_enabled());

        let disabled = WindowsPlugin::new(false, String::new(), 5);
        assert!(!disabled.is_enabled());
    }

    #[test]
    fn key_navigation_up_down() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());

        // Start at 0, down twice
        let action = plugin.handle_key(make_key(KeyCode::Down));
        assert!(matches!(action, PluginAction::Consumed));
        assert_eq!(plugin.state.selected_process, 1);

        let action = plugin.handle_key(make_key(KeyCode::Down));
        assert!(matches!(action, PluginAction::Consumed));
        assert_eq!(plugin.state.selected_process, 2);

        // Up
        let action = plugin.handle_key(make_key(KeyCode::Up));
        assert!(matches!(action, PluginAction::Consumed));
        assert_eq!(plugin.state.selected_process, 1);
    }

    #[test]
    fn key_up_does_not_underflow() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());
        plugin.state.selected_process = 0;

        plugin.handle_key(make_key(KeyCode::Up));
        assert_eq!(plugin.state.selected_process, 0);
    }

    #[test]
    fn key_down_does_not_overflow() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());
        plugin.state.selected_process = 2; // last index

        plugin.handle_key(make_key(KeyCode::Down));
        assert_eq!(plugin.state.selected_process, 2);
    }

    #[test]
    fn home_end_keys() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());
        plugin.state.selected_process = 1;

        plugin.handle_key(make_key(KeyCode::End));
        assert_eq!(plugin.state.selected_process, 2);

        plugin.handle_key(make_key(KeyCode::Home));
        assert_eq!(plugin.state.selected_process, 0);
    }

    #[test]
    fn unhandled_key_returns_ignored() {
        let mut plugin = make_plugin();
        let action = plugin.handle_key(make_key(KeyCode::Char('z')));
        assert!(matches!(action, PluginAction::Ignored));
    }

    #[test]
    fn execute_command_no_snapshot() {
        let mut plugin = make_plugin();
        let result = plugin.execute_command("windows", "");
        assert!(result.is_some());
        assert!(result.unwrap().contains("not connected"));
    }

    #[test]
    fn execute_command_with_snapshot() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());
        plugin.state.agent_connected = true;

        let result = plugin.execute_command("windows", "");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("DESKTOP-TEST"));
        assert!(text.contains("CPU:"));
    }

    #[test]
    fn execute_command_unknown() {
        let mut plugin = make_plugin();
        let result = plugin.execute_command("unknown", "");
        assert!(result.is_none());
    }

    #[test]
    fn status_bar_hints_not_empty() {
        let plugin = make_plugin();
        assert!(!plugin.status_bar_hints().is_empty());
    }

    #[test]
    fn help_entries_not_empty() {
        let plugin = make_plugin();
        assert!(!plugin.help_entries().is_empty());
    }

    #[test]
    fn tick_drains_data_from_channel() {
        with_runtime(|| {
            let mut plugin = make_plugin();

            // Manually send a snapshot through the channel
            let snapshot = make_snapshot();
            plugin
                .poll_tx
                .send(AgentPollResult::Data(snapshot))
                .unwrap();

            plugin.tick();

            assert!(plugin.state.snapshot.is_some());
            assert!(plugin.state.agent_connected);
            assert!(!plugin.state.loading);
            assert!(plugin.state.error.is_none());
        });
    }

    #[test]
    fn tick_drains_error_from_channel() {
        with_runtime(|| {
            let mut plugin = make_plugin();

            plugin
                .poll_tx
                .send(AgentPollResult::Error("Connection refused".into()))
                .unwrap();

            plugin.tick();

            assert!(!plugin.state.agent_connected);
            assert!(plugin.state.error.is_some());
            assert!(plugin.state.error.as_ref().unwrap().contains("refused"));
        });
    }

    #[test]
    fn sort_key_cycles_field() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());
        assert_eq!(plugin.state.sort_field, WindowsSortField::Cpu);

        let action = plugin.handle_key(make_key(KeyCode::Char('s')));
        assert!(matches!(action, PluginAction::Consumed));
        assert_eq!(plugin.state.sort_field, WindowsSortField::Memory);

        plugin.handle_key(make_key(KeyCode::Char('s')));
        assert_eq!(plugin.state.sort_field, WindowsSortField::Pid);
    }

    #[test]
    fn sort_direction_key_toggles() {
        let mut plugin = make_plugin();
        assert!(!plugin.state.sort_ascending);

        let action = plugin.handle_key(make_key(KeyCode::Char('S')));
        assert!(matches!(action, PluginAction::Consumed));
        assert!(plugin.state.sort_ascending);
    }

    #[test]
    fn focus_key_toggles_panel() {
        let mut plugin = make_plugin();
        assert!(plugin.state.focused_panel.is_none());

        let action = plugin.handle_key(make_key(KeyCode::Char('f')));
        assert!(matches!(action, PluginAction::Consumed));
        assert!(plugin.state.focused_panel.is_some());

        plugin.handle_key(make_key(KeyCode::Char('f')));
        assert!(plugin.state.focused_panel.is_none());
    }

    #[test]
    fn ai_key_emits_request_with_snapshot() {
        let mut plugin = make_plugin();
        plugin.state.snapshot = Some(make_snapshot());

        let action = plugin.handle_key(make_key(KeyCode::Char('a')));
        assert!(matches!(action, PluginAction::RequestAiAnalysis(_)));
        assert!(plugin.state.ai_loading);
        assert!(plugin.state.ai_analysis.is_none());
    }

    #[test]
    fn ai_key_sets_status_without_snapshot() {
        let mut plugin = make_plugin();
        // No snapshot set
        let action = plugin.handle_key(make_key(KeyCode::Char('a')));
        assert!(matches!(action, PluginAction::SetStatus(_)));
        assert!(!plugin.state.ai_loading);
    }

    #[test]
    fn ai_receive_chunk_and_done() {
        let mut plugin = make_plugin();
        plugin.state.ai_loading = true;

        plugin.receive_ai_chunk("Hello ");
        assert_eq!(plugin.state.ai_analysis, Some("Hello ".to_string()));

        plugin.receive_ai_chunk("World");
        assert_eq!(plugin.state.ai_analysis, Some("Hello World".to_string()));

        plugin.ai_analysis_done();
        assert!(!plugin.state.ai_loading);
    }

    #[test]
    fn ai_error_sets_message() {
        let mut plugin = make_plugin();
        plugin.state.ai_loading = true;

        plugin.ai_analysis_error("API key invalid");
        assert!(!plugin.state.ai_loading);
        assert!(plugin.state.ai_analysis.as_ref().unwrap().contains("Error:"));
    }

    #[test]
    fn security_alerts_firewall_off() {
        let mut plugin = make_plugin();
        let mut snap = make_snapshot();
        snap.security = Some(models::WindowsSecurityStatus {
            firewall_profiles: vec![
                models::WindowsFirewallProfile { name: "Domain".into(), enabled: true },
                models::WindowsFirewallProfile { name: "Public".into(), enabled: false },
            ],
            defender_enabled: Some(true),
            defender_realtime: Some(true),
            last_update_days: Some(5),
        });
        plugin.state.snapshot = Some(snap);

        let alerts = plugin.security_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::WindowsFirewall);
        assert!(alerts[0].message.contains("Public"));
    }

    #[test]
    fn security_alerts_defender_off() {
        let mut plugin = make_plugin();
        let mut snap = make_snapshot();
        snap.security = Some(models::WindowsSecurityStatus {
            firewall_profiles: vec![],
            defender_enabled: Some(false),
            defender_realtime: Some(false),
            last_update_days: None,
        });
        plugin.state.snapshot = Some(snap);

        let alerts = plugin.security_alerts();
        // Should fire Defender disabled (but NOT realtime, since Defender is off)
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::WindowsDefender);
        assert_eq!(alerts[0].severity, AlertSeverity::Danger);
    }

    #[test]
    fn security_alerts_stale_updates() {
        let mut plugin = make_plugin();
        let mut snap = make_snapshot();
        snap.security = Some(models::WindowsSecurityStatus {
            firewall_profiles: vec![],
            defender_enabled: Some(true),
            defender_realtime: Some(true),
            last_update_days: Some(45),
        });
        plugin.state.snapshot = Some(snap);

        let alerts = plugin.security_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, AlertCategory::WindowsUpdates);
        assert!(alerts[0].message.contains("45 days"));
    }

    #[test]
    fn security_alerts_all_healthy_no_alerts() {
        let mut plugin = make_plugin();
        let mut snap = make_snapshot();
        snap.security = Some(models::WindowsSecurityStatus {
            firewall_profiles: vec![
                models::WindowsFirewallProfile { name: "Domain".into(), enabled: true },
                models::WindowsFirewallProfile { name: "Private".into(), enabled: true },
                models::WindowsFirewallProfile { name: "Public".into(), enabled: true },
            ],
            defender_enabled: Some(true),
            defender_realtime: Some(true),
            last_update_days: Some(5),
        });
        plugin.state.snapshot = Some(snap);

        let alerts = plugin.security_alerts();
        assert!(alerts.is_empty());
    }

    #[test]
    fn security_alerts_cooldown_dedup() {
        let mut plugin = make_plugin();
        let mut snap = make_snapshot();
        snap.security = Some(models::WindowsSecurityStatus {
            firewall_profiles: vec![
                models::WindowsFirewallProfile { name: "Public".into(), enabled: false },
            ],
            defender_enabled: Some(true),
            defender_realtime: Some(true),
            last_update_days: None,
        });
        plugin.state.snapshot = Some(snap);

        // First call: should fire
        let alerts1 = plugin.security_alerts();
        assert_eq!(alerts1.len(), 1);

        // Second call within cooldown: should be suppressed
        let alerts2 = plugin.security_alerts();
        assert!(alerts2.is_empty());
    }

    #[test]
    fn focus_cycle_key_only_when_focused() {
        let mut plugin = make_plugin();
        // Not focused — 'F' should be Ignored (falls to _ arm)
        let action = plugin.handle_key(make_key(KeyCode::Char('F')));
        assert!(matches!(action, PluginAction::Ignored));

        // Enter focus mode
        plugin.handle_key(make_key(KeyCode::Char('f')));
        assert!(plugin.state.focused_panel.is_some());

        // Now 'F' should cycle
        let action = plugin.handle_key(make_key(KeyCode::Char('F')));
        assert!(matches!(action, PluginAction::Consumed));
    }
}
