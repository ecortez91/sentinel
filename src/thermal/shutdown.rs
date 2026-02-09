//! Auto-shutdown state machine for thermal emergencies.
//!
//! State flow: Normal → Counting(start) → GracePeriod(start) → Shutdown
//! Cancels and returns to Normal if temperature drops below critical.
//! OFF by default — double-gated: config + .env flag.

use std::time::Instant;

/// Auto-shutdown state machine states.
#[derive(Debug, Clone)]
pub enum ShutdownState {
    /// Normal operation — no thermal emergency.
    Normal,
    /// Temperature exceeded emergency threshold; counting sustained seconds.
    Counting { since: Instant, required_secs: u64 },
    /// Sustained emergency confirmed; grace period before shutdown.
    GracePeriod { since: Instant, grace_secs: u64 },
    /// Shutdown command issued.
    Shutdown,
}

impl Default for ShutdownState {
    fn default() -> Self {
        Self::Normal
    }
}

impl ShutdownState {
    /// Human-readable label for the current state.
    pub fn label(&self) -> &str {
        match self {
            ShutdownState::Normal => "Normal",
            ShutdownState::Counting { .. } => "Thermal Warning - Counting",
            ShutdownState::GracePeriod { .. } => "SHUTDOWN IMMINENT",
            ShutdownState::Shutdown => "SHUTTING DOWN",
        }
    }

    /// Whether the state is anything other than Normal.
    pub fn is_active(&self) -> bool {
        !matches!(self, ShutdownState::Normal)
    }

    /// Seconds remaining before next escalation, if applicable.
    pub fn seconds_remaining(&self) -> Option<u64> {
        match self {
            ShutdownState::Counting {
                since,
                required_secs,
            } => {
                let elapsed = since.elapsed().as_secs();
                Some(required_secs.saturating_sub(elapsed))
            }
            ShutdownState::GracePeriod { since, grace_secs } => {
                let elapsed = since.elapsed().as_secs();
                Some(grace_secs.saturating_sub(elapsed))
            }
            _ => None,
        }
    }
}

/// Manages the auto-shutdown state machine.
pub struct ShutdownManager {
    /// Current state.
    pub state: ShutdownState,
    /// Whether auto-shutdown is enabled (config + .env double-gate).
    enabled: bool,
    /// Emergency temperature threshold.
    emergency_threshold: f32,
    /// Critical temperature threshold (below this = recovery).
    critical_threshold: f32,
    /// Required sustained seconds at emergency before escalation.
    sustained_secs: u64,
    /// Grace period seconds before actual shutdown.
    grace_secs: u64,
    /// Schedule start hour (0-23).
    schedule_start: u8,
    /// Schedule end hour (0-24).
    schedule_end: u8,
}

impl ShutdownManager {
    /// Create a new shutdown manager from config + .env settings.
    pub fn new(
        config_enabled: bool,
        emergency_threshold: f32,
        critical_threshold: f32,
        sustained_secs: u64,
        grace_secs: u64,
        schedule_start: u8,
        schedule_end: u8,
    ) -> Self {
        // Double-gate: config must enable it AND .env must have the flag
        let env_enabled = std::env::var("SENTINEL_AUTO_SHUTDOWN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        Self {
            state: ShutdownState::Normal,
            enabled: config_enabled && env_enabled,
            emergency_threshold,
            critical_threshold,
            sustained_secs,
            grace_secs,
            schedule_start,
            schedule_end,
        }
    }

    /// Whether the shutdown manager is actively enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if we're within the active schedule window.
    fn in_schedule(&self) -> bool {
        let hour = chrono::Local::now().hour() as u8;
        if self.schedule_start <= self.schedule_end {
            hour >= self.schedule_start && hour < self.schedule_end
        } else {
            // Wraps midnight, e.g. 22..6
            hour >= self.schedule_start || hour < self.schedule_end
        }
    }

    /// Tick the state machine with the current max temperature.
    /// Returns a `ShutdownEvent` describing what happened this tick.
    pub fn tick(&mut self, max_temp: f32) -> ShutdownEvent {
        if !self.enabled || !self.in_schedule() {
            // If disabled or outside schedule, reset to normal
            if self.state.is_active() {
                self.state = ShutdownState::Normal;
                return ShutdownEvent::Recovered;
            }
            return ShutdownEvent::None;
        }

        match &self.state {
            ShutdownState::Normal => {
                if max_temp >= self.emergency_threshold {
                    self.state = ShutdownState::Counting {
                        since: Instant::now(),
                        required_secs: self.sustained_secs,
                    };
                    ShutdownEvent::EmergencyStarted
                } else {
                    ShutdownEvent::None
                }
            }
            ShutdownState::Counting {
                since,
                required_secs,
            } => {
                if max_temp < self.critical_threshold {
                    self.state = ShutdownState::Normal;
                    return ShutdownEvent::Recovered;
                }
                let elapsed = since.elapsed().as_secs();
                if elapsed >= *required_secs {
                    self.state = ShutdownState::GracePeriod {
                        since: Instant::now(),
                        grace_secs: self.grace_secs,
                    };
                    ShutdownEvent::GracePeriodStarted
                } else {
                    ShutdownEvent::Counting {
                        elapsed_secs: elapsed,
                        required_secs: *required_secs,
                    }
                }
            }
            ShutdownState::GracePeriod { since, grace_secs } => {
                if max_temp < self.critical_threshold {
                    self.state = ShutdownState::Normal;
                    return ShutdownEvent::Recovered;
                }
                let elapsed = since.elapsed().as_secs();
                if elapsed >= *grace_secs {
                    self.state = ShutdownState::Shutdown;
                    ShutdownEvent::ShutdownNow
                } else {
                    ShutdownEvent::GracePeriodCountdown {
                        remaining_secs: grace_secs - elapsed,
                    }
                }
            }
            ShutdownState::Shutdown => ShutdownEvent::ShutdownNow,
        }
    }

    /// Force abort — reset to Normal from any state.
    pub fn abort(&mut self) -> bool {
        if self.state.is_active() {
            self.state = ShutdownState::Normal;
            true
        } else {
            false
        }
    }
}

/// Events emitted by the shutdown state machine per tick.
#[derive(Debug, Clone, PartialEq)]
pub enum ShutdownEvent {
    /// Nothing noteworthy.
    None,
    /// Temperature crossed emergency threshold — counting started.
    EmergencyStarted,
    /// Still counting sustained seconds.
    Counting {
        elapsed_secs: u64,
        required_secs: u64,
    },
    /// Sustained emergency confirmed — grace period started.
    GracePeriodStarted,
    /// Grace period countdown.
    GracePeriodCountdown { remaining_secs: u64 },
    /// Execute shutdown NOW.
    ShutdownNow,
    /// Temperature dropped below critical — recovered.
    Recovered,
}

/// Execute the actual system shutdown (WSL: powershell Stop-Computer).
pub fn execute_shutdown() -> std::io::Result<()> {
    std::process::Command::new("powershell.exe")
        .args(["-Command", "Stop-Computer -Force"])
        .spawn()?;
    Ok(())
}

use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager(enabled: bool) -> ShutdownManager {
        ShutdownManager {
            state: ShutdownState::Normal,
            enabled,
            emergency_threshold: 100.0,
            critical_threshold: 95.0,
            sustained_secs: 2,
            grace_secs: 2,
            schedule_start: 0,
            schedule_end: 24, // Always in schedule for testing
        }
    }

    #[test]
    fn disabled_manager_does_nothing() {
        let mut mgr = make_manager(false);
        assert_eq!(mgr.tick(110.0), ShutdownEvent::None);
        assert!(!mgr.state.is_active());
    }

    #[test]
    fn normal_stays_normal_below_emergency() {
        let mut mgr = make_manager(true);
        assert_eq!(mgr.tick(90.0), ShutdownEvent::None);
        assert!(!mgr.state.is_active());
    }

    #[test]
    fn normal_to_counting_at_emergency() {
        let mut mgr = make_manager(true);
        assert_eq!(mgr.tick(100.0), ShutdownEvent::EmergencyStarted);
        assert!(mgr.state.is_active());
    }

    #[test]
    fn counting_recovers_below_critical() {
        let mut mgr = make_manager(true);
        mgr.tick(100.0); // Start counting
        assert_eq!(mgr.tick(90.0), ShutdownEvent::Recovered); // Below critical
        assert!(!mgr.state.is_active());
    }

    #[test]
    fn abort_resets_state() {
        let mut mgr = make_manager(true);
        mgr.tick(100.0); // Start counting
        assert!(mgr.abort());
        assert!(!mgr.state.is_active());
    }

    #[test]
    fn abort_on_normal_returns_false() {
        let mut mgr = make_manager(true);
        assert!(!mgr.abort());
    }

    #[test]
    fn state_labels() {
        assert_eq!(ShutdownState::Normal.label(), "Normal");
        assert_eq!(
            ShutdownState::Counting {
                since: Instant::now(),
                required_secs: 30,
            }
            .label(),
            "Thermal Warning - Counting"
        );
        assert_eq!(
            ShutdownState::GracePeriod {
                since: Instant::now(),
                grace_secs: 30,
            }
            .label(),
            "SHUTDOWN IMMINENT"
        );
        assert_eq!(ShutdownState::Shutdown.label(), "SHUTTING DOWN");
    }

    #[test]
    fn seconds_remaining_counting() {
        let state = ShutdownState::Counting {
            since: Instant::now(),
            required_secs: 30,
        };
        let remaining = state.seconds_remaining().unwrap();
        assert!(remaining <= 30);
    }

    #[test]
    fn seconds_remaining_normal_is_none() {
        assert!(ShutdownState::Normal.seconds_remaining().is_none());
    }
}
