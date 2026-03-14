//! Windows plugin UI state management (#1).

use std::time::Instant;

use super::models::WindowsHostSnapshot;

/// Which column the Windows process list is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsSortField {
    Cpu,
    Memory,
    Pid,
    Name,
}

impl WindowsSortField {
    /// Rotate to the next sort field.
    pub fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::Cpu,
        }
    }

    /// Column header label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU %",
            Self::Memory => "Memory",
            Self::Pid => "PID",
            Self::Name => "Name",
        }
    }
}

/// Which panel is focused/expanded in the Windows tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPanel {
    SystemOverview,
    ProcessList,
    Security,
    Connections,
    Disks,
    Network,
    StartupPrograms,
    AiAnalysis,
}

impl WindowsPanel {
    /// Cycle to the next panel.
    pub fn next(self) -> Self {
        match self {
            Self::SystemOverview => Self::ProcessList,
            Self::ProcessList => Self::Security,
            Self::Security => Self::Connections,
            Self::Connections => Self::Disks,
            Self::Disks => Self::Network,
            Self::Network => Self::StartupPrograms,
            Self::StartupPrograms => Self::AiAnalysis,
            Self::AiAnalysis => Self::SystemOverview,
        }
    }
}

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
    /// Current sort field for the process list.
    pub sort_field: WindowsSortField,
    /// Whether sort order is ascending (false = descending).
    pub sort_ascending: bool,
    /// Which panel is focused/expanded (None = normal layout).
    pub focused_panel: Option<WindowsPanel>,
    /// AI security analysis result.
    pub ai_analysis: Option<String>,
    /// Whether AI analysis is currently streaming.
    pub ai_loading: bool,
    /// AI analysis scroll offset.
    pub ai_scroll: usize,
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
            sort_field: WindowsSortField::Cpu,
            sort_ascending: false,
            focused_panel: None,
            ai_analysis: None,
            ai_loading: false,
            ai_scroll: 0,
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

    /// Cycle to the next sort field and reset selection.
    pub fn cycle_sort(&mut self) {
        self.sort_field = self.sort_field.next();
        self.selected_process = 0;
        self.scroll_offset = 0;
    }

    /// Toggle sort direction (ascending ↔ descending).
    pub fn toggle_sort_direction(&mut self) {
        self.sort_ascending = !self.sort_ascending;
        self.selected_process = 0;
        self.scroll_offset = 0;
    }

    /// Toggle focus mode (enter or exit expanded panel view).
    pub fn toggle_panel_focus(&mut self) {
        if self.focused_panel.is_some() {
            self.focused_panel = None;
        } else {
            self.focused_panel = Some(WindowsPanel::ProcessList);
        }
    }

    /// Cycle to the next panel in focus mode.
    pub fn cycle_panel_forward(&mut self) {
        if let Some(panel) = self.focused_panel {
            self.focused_panel = Some(panel.next());
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
        assert_eq!(state.sort_field, WindowsSortField::Cpu);
        assert!(!state.sort_ascending);
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn selection_does_not_underflow() {
        let mut state = WindowsState::new();
        state.selected_process = 0;
        state.move_selection_up();
        assert_eq!(state.selected_process, 0);
    }

    #[test]
    fn cycle_sort_rotates_all_fields() {
        let mut state = WindowsState::new();
        assert_eq!(state.sort_field, WindowsSortField::Cpu);
        state.cycle_sort();
        assert_eq!(state.sort_field, WindowsSortField::Memory);
        state.cycle_sort();
        assert_eq!(state.sort_field, WindowsSortField::Pid);
        state.cycle_sort();
        assert_eq!(state.sort_field, WindowsSortField::Name);
        state.cycle_sort();
        assert_eq!(state.sort_field, WindowsSortField::Cpu);
    }

    #[test]
    fn cycle_sort_resets_selection() {
        let mut state = WindowsState::new();
        state.selected_process = 5;
        state.scroll_offset = 3;
        state.cycle_sort();
        assert_eq!(state.selected_process, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn toggle_sort_direction() {
        let mut state = WindowsState::new();
        assert!(!state.sort_ascending);
        state.toggle_sort_direction();
        assert!(state.sort_ascending);
        state.toggle_sort_direction();
        assert!(!state.sort_ascending);
    }

    #[test]
    fn toggle_sort_direction_resets_selection() {
        let mut state = WindowsState::new();
        state.selected_process = 3;
        state.scroll_offset = 2;
        state.toggle_sort_direction();
        assert_eq!(state.selected_process, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn panel_focus_toggle() {
        let mut state = WindowsState::new();
        assert!(state.focused_panel.is_none());
        state.toggle_panel_focus();
        assert_eq!(state.focused_panel, Some(WindowsPanel::ProcessList));
        state.toggle_panel_focus();
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn panel_cycle_forward_wraps() {
        let mut state = WindowsState::new();
        state.focused_panel = Some(WindowsPanel::SystemOverview);
        state.cycle_panel_forward();
        assert_eq!(state.focused_panel, Some(WindowsPanel::ProcessList));
        // Cycle all the way through
        state.cycle_panel_forward(); // Security
        state.cycle_panel_forward(); // Connections
        state.cycle_panel_forward(); // Disks
        state.cycle_panel_forward(); // Network
        state.cycle_panel_forward(); // StartupPrograms
        state.cycle_panel_forward(); // AiAnalysis
        state.cycle_panel_forward(); // wraps → SystemOverview
        assert_eq!(state.focused_panel, Some(WindowsPanel::SystemOverview));
    }

    #[test]
    fn panel_cycle_noop_when_not_focused() {
        let mut state = WindowsState::new();
        assert!(state.focused_panel.is_none());
        state.cycle_panel_forward();
        assert!(state.focused_panel.is_none());
    }

    #[test]
    fn sort_field_labels() {
        assert_eq!(WindowsSortField::Cpu.label(), "CPU %");
        assert_eq!(WindowsSortField::Memory.label(), "Memory");
        assert_eq!(WindowsSortField::Pid.label(), "PID");
        assert_eq!(WindowsSortField::Name.label(), "Name");
    }
}
