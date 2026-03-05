//! Security dashboard state types.
//!
//! All data models for the Security tab: listeners, connections,
//! events, threat summary, integrity, and scoring.

use chrono::{DateTime, Local};
use std::fmt;

use crate::models::AlertSeverity;

// ── Panel focus ──────────────────────────────────────────────────

/// Which panel is currently focused for keyboard interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityPanel {
    Listeners,
    Connections,
    Timeline,
    ThreatSummary,
    Integrity,
}

impl SecurityPanel {
    /// Cycle to the next panel.
    pub fn next(self) -> Self {
        match self {
            Self::Listeners => Self::Connections,
            Self::Connections => Self::Timeline,
            Self::Timeline => Self::ThreatSummary,
            Self::ThreatSummary => Self::Integrity,
            Self::Integrity => Self::Listeners,
        }
    }

    /// Cycle to the previous panel.
    pub fn prev(self) -> Self {
        match self {
            Self::Listeners => Self::Integrity,
            Self::Connections => Self::Listeners,
            Self::Timeline => Self::Connections,
            Self::ThreatSummary => Self::Timeline,
            Self::Integrity => Self::ThreatSummary,
        }
    }

    /// Display label for the panel.
    pub fn label(&self) -> &str {
        match self {
            Self::Listeners => "Active Listeners",
            Self::Connections => "Connections",
            Self::Timeline => "Security Events",
            Self::ThreatSummary => "Threat Summary",
            Self::Integrity => "System Integrity",
        }
    }
}

// ── Port risk classification ─────────────────────────────────────

/// Risk level for a listening port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortRisk {
    /// Standard port with known process.
    Known,
    /// Unexpected service or unusual port.
    Suspicious,
    /// Listening but PID=0 or process unknown.
    Unowned,
}

impl fmt::Display for PortRisk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortRisk::Known => write!(f, "OK"),
            PortRisk::Suspicious => write!(f, "SUSPECT"),
            PortRisk::Unowned => write!(f, "UNOWNED"),
        }
    }
}

// ── Listener info ────────────────────────────────────────────────

/// A TCP/UDP listener detected on the system.
#[derive(Debug, Clone)]
pub struct ListenerInfo {
    pub port: u16,
    pub protocol: String,
    pub pid: Option<u32>,
    pub process_name: String,
    pub bind_addr: String,
    pub risk: PortRisk,
}

// ── Connection info ──────────────────────────────────────────────

/// An established TCP connection.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub pid: Option<u32>,
    pub process_name: String,
    pub state: String,
}

// ── Security event ───────────────────────────────────────────────

/// Category of security event for the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEventKind {
    /// Security threat or suspicious process alert.
    Threat,
    /// Port opened or closed.
    PortChange,
    /// New or exited process.
    ProcessChange,
    /// Authentication event (from auth.log).
    AuthEvent,
    /// Security score change.
    ScoreChange,
}

impl fmt::Display for SecurityEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Threat => write!(f, "THREAT"),
            Self::PortChange => write!(f, "PORT"),
            Self::ProcessChange => write!(f, "PROC"),
            Self::AuthEvent => write!(f, "AUTH"),
            Self::ScoreChange => write!(f, "SCORE"),
        }
    }
}

/// A single event in the security timeline.
#[derive(Debug, Clone)]
pub struct SecurityEvent {
    pub timestamp: DateTime<Local>,
    pub kind: SecurityEventKind,
    pub severity: AlertSeverity,
    pub message: String,
    pub pid: Option<u32>,
}

impl SecurityEvent {
    /// Icon character for the event kind.
    pub fn icon(&self) -> &str {
        match self.kind {
            SecurityEventKind::Threat => "!",
            SecurityEventKind::PortChange => ">",
            SecurityEventKind::ProcessChange => "+",
            SecurityEventKind::AuthEvent => "@",
            SecurityEventKind::ScoreChange => "#",
        }
    }

    /// Human-readable age string.
    pub fn age_display(&self) -> String {
        let elapsed = Local::now()
            .signed_duration_since(self.timestamp)
            .num_seconds();
        if elapsed < 60 {
            format!("{}s", elapsed)
        } else if elapsed < 3600 {
            format!("{}m", elapsed / 60)
        } else if elapsed < 86400 {
            format!("{}h", elapsed / 3600)
        } else {
            format!("{}d", elapsed / 86400)
        }
    }
}

// ── Score label ──────────────────────────────────────────────────

/// Compute the score label from a numeric score.
pub fn score_label(score: u8) -> &'static str {
    if score >= 80 {
        "GOOD"
    } else if score >= 60 {
        "FAIR"
    } else if score >= 40 {
        "POOR"
    } else {
        "CRITICAL"
    }
}

// ── Main security state ─────────────────────────────────────────

/// Full security dashboard state, refreshed periodically.
#[derive(Debug, Clone)]
pub struct SecurityState {
    // ── Data panels ──
    /// Active TCP/UDP listeners.
    pub listeners: Vec<ListenerInfo>,
    /// Established connections.
    pub connections: Vec<ConnectionInfo>,
    /// Security event timeline (newest first).
    pub events: Vec<SecurityEvent>,

    // ── Threat summary ──
    pub active_threats: usize,
    pub suspicious_count: usize,
    pub risky_ports: Vec<u16>,
    pub unowned_listeners: usize,

    // ── System integrity ──
    pub logged_in_users: Vec<String>,
    pub auth_event_count_24h: usize,
    pub modified_packages: Vec<String>,
    pub auth_log_readable: bool,

    // ── Score ──
    pub score: u8,
    pub prev_score: u8,

    // ── UI state ──
    pub focused_panel: SecurityPanel,
    pub listener_scroll: usize,
    pub connection_scroll: usize,
    pub event_scroll: usize,
    /// Selected item index within the focused panel (for Enter).
    pub selected_index: usize,
    /// Whether the detail popup is visible.
    pub detail_popup: bool,

    // ── Refresh tracking ──
    pub last_refresh: Option<std::time::Instant>,
    /// Slow-refresh counter for expensive operations (dpkg --verify).
    pub slow_refresh_count: u64,
}

impl Default for SecurityState {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
            connections: Vec::new(),
            events: Vec::new(),
            active_threats: 0,
            suspicious_count: 0,
            risky_ports: Vec::new(),
            unowned_listeners: 0,
            logged_in_users: Vec::new(),
            auth_event_count_24h: 0,
            modified_packages: Vec::new(),
            auth_log_readable: false,
            score: 100,
            prev_score: 100,
            focused_panel: SecurityPanel::Listeners,
            listener_scroll: 0,
            connection_scroll: 0,
            event_scroll: 0,
            selected_index: 0,
            detail_popup: false,
            last_refresh: None,
            slow_refresh_count: 0,
        }
    }
}

impl SecurityState {
    /// Reset scroll/selection when panel focus changes.
    pub fn focus_panel(&mut self, panel: SecurityPanel) {
        self.focused_panel = panel;
        self.selected_index = 0;
    }

    /// Number of items in the currently focused panel.
    pub fn focused_item_count(&self) -> usize {
        match self.focused_panel {
            SecurityPanel::Listeners => self.listeners.len(),
            SecurityPanel::Connections => self.connections.len(),
            SecurityPanel::Timeline => self.events.len(),
            SecurityPanel::ThreatSummary => 0,
            SecurityPanel::Integrity => 0,
        }
    }

    /// Scroll offset for the currently focused panel.
    pub fn focused_scroll(&self) -> usize {
        match self.focused_panel {
            SecurityPanel::Listeners => self.listener_scroll,
            SecurityPanel::Connections => self.connection_scroll,
            SecurityPanel::Timeline => self.event_scroll,
            _ => 0,
        }
    }

    /// Set scroll offset for the currently focused panel.
    pub fn set_focused_scroll(&mut self, offset: usize) {
        match self.focused_panel {
            SecurityPanel::Listeners => self.listener_scroll = offset,
            SecurityPanel::Connections => self.connection_scroll = offset,
            SecurityPanel::Timeline => self.event_scroll = offset,
            _ => {}
        }
    }

    /// Score display label.
    pub fn score_label(&self) -> &'static str {
        score_label(self.score)
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_next_cycles_all() {
        let mut p = SecurityPanel::Listeners;
        let start = p;
        let mut visited = vec![p];
        for _ in 0..5 {
            p = p.next();
            visited.push(p);
        }
        // Should return to start after 5 nexts
        assert_eq!(visited.last(), Some(&start));
        // All 5 panels visited
        assert_eq!(visited.len(), 6);
    }

    #[test]
    fn panel_prev_is_inverse_of_next() {
        let p = SecurityPanel::Connections;
        assert_eq!(p.next().prev(), p);
        let p2 = SecurityPanel::Integrity;
        assert_eq!(p2.prev().next(), p2);
    }

    #[test]
    fn score_label_thresholds() {
        assert_eq!(score_label(100), "GOOD");
        assert_eq!(score_label(80), "GOOD");
        assert_eq!(score_label(79), "FAIR");
        assert_eq!(score_label(60), "FAIR");
        assert_eq!(score_label(59), "POOR");
        assert_eq!(score_label(40), "POOR");
        assert_eq!(score_label(39), "CRITICAL");
        assert_eq!(score_label(0), "CRITICAL");
    }

    #[test]
    fn default_state_is_clean() {
        let state = SecurityState::default();
        assert_eq!(state.score, 100);
        assert_eq!(state.focused_panel, SecurityPanel::Listeners);
        assert!(state.listeners.is_empty());
        assert!(state.events.is_empty());
        assert!(!state.detail_popup);
    }

    #[test]
    fn focus_panel_resets_selection() {
        let mut state = SecurityState::default();
        state.selected_index = 5;
        state.focus_panel(SecurityPanel::Timeline);
        assert_eq!(state.focused_panel, SecurityPanel::Timeline);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn focused_item_count_per_panel() {
        let mut state = SecurityState::default();
        state.listeners = vec![ListenerInfo {
            port: 8080,
            protocol: "tcp".into(),
            pid: Some(123),
            process_name: "nginx".into(),
            bind_addr: "0.0.0.0".into(),
            risk: PortRisk::Known,
        }];
        state.connections = vec![ConnectionInfo {
            local_addr: "127.0.0.1".into(),
            local_port: 5432,
            remote_addr: "127.0.0.1".into(),
            remote_port: 49001,
            pid: Some(456),
            process_name: "psql".into(),
            state: "ESTABLISHED".into(),
        }];

        state.focused_panel = SecurityPanel::Listeners;
        assert_eq!(state.focused_item_count(), 1);

        state.focused_panel = SecurityPanel::Connections;
        assert_eq!(state.focused_item_count(), 1);

        state.focused_panel = SecurityPanel::ThreatSummary;
        assert_eq!(state.focused_item_count(), 0);
    }

    #[test]
    fn security_event_age_display() {
        let event = SecurityEvent {
            timestamp: Local::now(),
            kind: SecurityEventKind::Threat,
            severity: AlertSeverity::Warning,
            message: "test".into(),
            pid: None,
        };
        let age = event.age_display();
        assert!(
            age.ends_with('s'),
            "Fresh event should show seconds, got: {}",
            age
        );
    }

    #[test]
    fn port_risk_ordering() {
        assert!(PortRisk::Known < PortRisk::Suspicious);
        assert!(PortRisk::Suspicious < PortRisk::Unowned);
    }
}
