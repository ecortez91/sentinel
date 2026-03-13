//! Windows plugin UI state management (#1).

use std::time::Instant;

use super::models::WindowsHostSnapshot;

/// Windows host monitoring plugin state.
pub struct WindowsState {
    /// Latest snapshot from the agent.
    pub snapshot: Option<WindowsHostSnapshot>,
    /// Whether data is currently loading.
    pub loading: bool,
    /// Connection error message.
    pub error: Option<String>,
    /// When the last successful update occurred.
    pub last_updated: Option<Instant>,
    /// Selected process index in the process list.
    pub selected_process: usize,
    /// Scroll offset for the process list.
    pub scroll_offset: usize,
    /// Whether agent is reachable.
    pub agent_connected: bool,
}

impl WindowsState {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            loading: true,
            error: None,
            last_updated: None,
            selected_process: 0,
            scroll_offset: 0,
            agent_connected: false,
        }
    }

    /// Move process selection up.
    pub fn move_selection_up(&mut self) {
        if self.selected_process > 0 {
            self.selected_process -= 1;
            if self.selected_process < self.scroll_offset {
                self.scroll_offset = self.selected_process;
            }
        }
    }

    /// Move process selection down.
    pub fn move_selection_down(&mut self) {
        let max = self
            .snapshot
            .as_ref()
            .map(|s| s.top_processes.len().saturating_sub(1))
            .unwrap_or(0);
        if self.selected_process < max {
            self.selected_process += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_defaults() {
        let state = WindowsState::new();
        assert!(state.snapshot.is_none());
        assert!(state.loading);
        assert!(state.error.is_none());
        assert!(!state.agent_connected);
        assert_eq!(state.selected_process, 0);
    }

    #[test]
    fn selection_does_not_underflow() {
        let mut state = WindowsState::new();
        state.selected_process = 0;
        state.move_selection_up();
        assert_eq!(state.selected_process, 0);
    }
}
