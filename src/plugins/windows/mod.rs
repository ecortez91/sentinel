//! Windows host monitoring plugin (#1, #3, #4).
//!
//! Polls a sentinel-agent HTTP endpoint for system snapshots and renders
//! them in a dedicated TUI tab. Uses the same channel-based async pattern
//! as the Market plugin to keep the event loop responsive.

pub mod models;
pub mod renderer;
pub mod state;

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{layout::Rect, Frame};
use tokio::sync::mpsc;

use crate::constants::{ENV_AGENT_URL, PAGE_SIZE};
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
            ("PgUp/Dn", "Page"),
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
