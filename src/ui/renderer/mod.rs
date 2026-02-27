//! Renderer module: split into focused submodules.
//!
//! - `header`: Logo, tab strip, system summary
//! - `status_bar`: Bottom status bar with keybinds and health
//! - `dashboard`: Dashboard tab with all widgets
//! - `processes`: Processes tab (flat + tree view)
//! - `alerts`: Alerts tab
//! - `ai_chat`: Ask AI tab (chat history + input)
//! - `overlays`: Popup overlays (process detail, help, signal picker, renice)
//! - `helpers`: Shared rendering utilities

mod ai_chat;
mod alerts;
mod dashboard;
mod header;
pub mod helpers;
mod overlays;
mod processes;
mod status_bar;
mod thermal;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use super::state::{AppState, Tab};
use crate::plugins::registry::PluginRegistry;

/// Render with plugin support.
pub fn render_with_plugins(frame: &mut Frame, state: &AppState, plugins: Option<&PluginRegistry>) {
    let size = frame.area();

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header bar
            Constraint::Min(10),   // Content area
            Constraint::Length(1), // Status bar
        ])
        .split(size);

    header::render_header_with_plugins(frame, main_chunks[0], state, plugins);
    status_bar::render_status_bar_with_plugins(frame, main_chunks[2], state, plugins);

    match state.active_tab {
        Tab::Dashboard => dashboard::render_dashboard(frame, main_chunks[1], state),
        Tab::Processes => processes::render_processes(frame, main_chunks[1], state),
        Tab::Alerts => alerts::render_alerts(frame, main_chunks[1], state),
        Tab::AskAi => ai_chat::render_ask_ai(frame, main_chunks[1], state),
        Tab::Thermal => thermal::render_thermal_tab(frame, main_chunks[1], state),
        Tab::Security => crate::security::render_security(frame, main_chunks[1], state),
        Tab::Plugin(i) => {
            if let Some(registry) = plugins {
                if let Some(plugin) = registry.get(i) {
                    plugin.render(frame, main_chunks[1], &state.theme);
                }
            }
        }
    }

    // Render plugin overlays (detail popups, etc.) for the active plugin tab
    if let Tab::Plugin(i) = state.active_tab {
        if let Some(registry) = plugins {
            if let Some(plugin) = registry.get(i) {
                plugin.render_overlay(frame, size, &state.theme);
            }
        }
    }

    if state.show_process_detail {
        overlays::render_process_detail(frame, size, state);
    }

    if state.show_signal_picker {
        overlays::render_signal_picker(frame, size, state);
    }

    if state.show_renice_dialog {
        overlays::render_renice_dialog(frame, size, state);
    }

    if state.show_help {
        overlays::render_help_overlay_with_plugins(frame, size, state, plugins);
    }

    if state.command_result.is_some() {
        overlays::render_command_result(frame, size, state);
    }

    if state.show_command_palette {
        overlays::render_command_palette(frame, size, state);
    }

    // Shutdown overlay renders on top of everything when active
    if state.shutdown_manager.state.is_active() {
        overlays::render_shutdown_overlay(frame, size, state);
    }
}
