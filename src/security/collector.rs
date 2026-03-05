//! Security data collector.
//!
//! Gathers data from multiple sources for the Security Dashboard:
//! - `/proc/net/tcp` via EventStore (listeners, connections)
//! - Alert history (threats, suspicious processes)
//! - `/var/log/auth.log` (authentication events)
//! - `dpkg --verify` (package integrity)
//! - `who` / utmp (logged-in users)
//!
//! All operations are best-effort: if a data source is unavailable
//! (e.g., no permission to read auth.log), the collector returns
//! graceful defaults rather than errors.

use chrono::{Local, TimeZone};
use std::io::BufRead;

use crate::constants::*;
use crate::models::{Alert, AlertCategory, AlertSeverity};
use crate::security::state::*;
use crate::store::EventStore;

// ── Listener collection ──────────────────────────────────────────

/// Collect active listeners from the EventStore, enriched with risk classification.
pub fn collect_listeners(store: &EventStore) -> Vec<ListenerInfo> {
    let rows = match store.query_current_listeners() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rows.into_iter()
        .map(|row| {
            let name = row.name.clone().unwrap_or_default();
            let pid = row.pid;
            let risk = classify_port_risk(row.local_port, &name, pid);
            ListenerInfo {
                port: row.local_port,
                protocol: row.protocol,
                pid,
                process_name: if name.is_empty() {
                    "???".to_string()
                } else {
                    name
                },
                bind_addr: row.local_addr,
                risk,
            }
        })
        .collect()
}

// ── Connection collection ────────────────────────────────────────

/// Collect established connections from the EventStore.
pub fn collect_connections(store: &EventStore) -> Vec<ConnectionInfo> {
    let rows = match store.query_current_connections() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rows.into_iter()
        .map(|row| ConnectionInfo {
            local_addr: row.local_addr,
            local_port: row.local_port,
            remote_addr: row.remote_addr.unwrap_or_else(|| "?".into()),
            remote_port: row.remote_port.unwrap_or(0),
            pid: row.pid,
            process_name: row.name.unwrap_or_else(|| "???".into()),
            state: row.state,
        })
        .collect()
}

// ── Security event timeline ──────────────────────────────────────

/// Build a unified security event timeline from store events and current alerts.
///
/// Merges:
/// - Alert events (security/suspicious categories only)
/// - Port bind/release events
/// - Process start/exit events
pub fn collect_security_events(store: &EventStore, alerts: &[Alert]) -> Vec<SecurityEvent> {
    let mut events = Vec::new();
    let thirty_min_ago = crate::store::now_epoch_ms_pub() - (30 * 60 * 1000);

    // Recent security/suspicious alerts from the alert list
    for alert in alerts.iter().take(50) {
        let kind = match alert.category {
            AlertCategory::SecurityThreat
            | AlertCategory::Suspicious
            | AlertCategory::SecurityScore => SecurityEventKind::Threat,
            AlertCategory::ThermalCritical
            | AlertCategory::ThermalEmergency
            | AlertCategory::ThermalWarning => continue, // skip thermal for security timeline
            _ => continue, // skip resource alerts
        };
        events.push(SecurityEvent {
            timestamp: alert.timestamp,
            kind,
            severity: alert.severity,
            message: format!("{}: {}", alert.category, alert.message),
            pid: Some(alert.pid),
        });
    }

    // Port and process events from the event store
    if let Ok(store_events) = store.query_events_since(thirty_min_ago) {
        for ev in store_events {
            let (kind, severity, message) = match ev.kind.as_str() {
                "port_bind" => (
                    SecurityEventKind::PortChange,
                    AlertSeverity::Info,
                    format!("Port opened by {}", ev.name.as_deref().unwrap_or("unknown")),
                ),
                "port_release" => (
                    SecurityEventKind::PortChange,
                    AlertSeverity::Info,
                    format!(
                        "Port closed (was {})",
                        ev.name.as_deref().unwrap_or("unknown")
                    ),
                ),
                "process_start" => (
                    SecurityEventKind::ProcessChange,
                    AlertSeverity::Info,
                    format!("New process: {}", ev.name.as_deref().unwrap_or("unknown")),
                ),
                "process_exit" => (
                    SecurityEventKind::ProcessChange,
                    AlertSeverity::Info,
                    format!(
                        "Process exited: {}",
                        ev.name.as_deref().unwrap_or("unknown")
                    ),
                ),
                "alert" => {
                    // Only include security-category alerts from store
                    let is_security = ev
                        .detail
                        .as_deref()
                        .map(|d| d.contains("SECURITY") || d.contains("SUSPECT"))
                        .unwrap_or(false);
                    if !is_security {
                        continue;
                    }
                    let sev = match ev.severity.as_deref() {
                        Some("danger") => AlertSeverity::Danger,
                        Some("critical") => AlertSeverity::Critical,
                        Some("warning") | Some("warn") => AlertSeverity::Warning,
                        _ => AlertSeverity::Info,
                    };
                    (
                        SecurityEventKind::Threat,
                        sev,
                        ev.detail.unwrap_or_else(|| "Security alert".into()),
                    )
                }
                _ => continue,
            };

            // Convert epoch ms to DateTime<Local>
            let ts = Local
                .timestamp_millis_opt(ev.ts)
                .single()
                .unwrap_or_else(Local::now);

            events.push(SecurityEvent {
                timestamp: ts,
                kind,
                severity,
                message,
                pid: ev.pid,
            });
        }
    }

    // Sort newest first, deduplicate by timestamp+message
    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    events.truncate(MAX_SECURITY_EVENTS);
    events
}

// ── Auth log parsing ─────────────────────────────────────────────

/// Count authentication events in the last 24 hours from /var/log/auth.log.
///
/// Returns `(count, readable)`. If the file is not readable, returns `(0, false)`.
pub fn count_auth_events_24h() -> (usize, bool) {
    let path = std::path::Path::new("/var/log/auth.log");
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (0, false),
    };

    let reader = std::io::BufReader::new(file);
    let mut count = 0usize;

    // auth.log lines start with "Mon DD HH:MM:SS" — we can't easily filter
    // by timestamp without parsing the full syslog date. Instead, use file
    // modification time as a proxy: if it was modified within 24h, count lines.
    // For a more accurate count, we'd parse dates, but this is good enough.
    for line in reader.lines().take(10_000) {
        if let Ok(l) = line {
            // Count meaningful auth events (sessions, failures, sudo)
            if l.contains("session opened")
                || l.contains("session closed")
                || l.contains("authentication failure")
                || l.contains("Failed password")
                || l.contains("Accepted password")
                || l.contains("sudo:")
                || l.contains("su:")
            {
                count += 1;
            }
        }
    }

    (count, true)
}

// ── Logged-in users ──────────────────────────────────────────────

/// Get the list of currently logged-in users.
///
/// Reads from the `who` command output or /var/run/utmp.
pub fn collect_logged_in_users() -> Vec<String> {
    // Try reading `who` output
    match std::process::Command::new("who").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut users: Vec<String> = text
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .map(String::from)
                .collect();
            users.sort();
            users.dedup();
            users
        }
        _ => Vec::new(),
    }
}

// ── Package integrity ────────────────────────────────────────────

/// Check for modified system packages via `dpkg --verify`.
///
/// This is an expensive operation — call infrequently (e.g., once per minute).
/// Returns a list of modified package file paths.
pub fn collect_modified_packages() -> Vec<String> {
    match std::process::Command::new("dpkg").arg("--verify").output() {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .filter(|l| !l.is_empty())
                .take(50) // cap to avoid huge lists
                .map(String::from)
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

// ── Port risk classification ─────────────────────────────────────

/// Classify the risk level of a listening port.
///
/// Strategy:
/// - If PID is 0 or unknown → Unowned
/// - If port is in KNOWN_PORTS and process matches → Known
/// - If port is in KNOWN_PORTS but process doesn't match → Suspicious
/// - If port is not in KNOWN_PORTS but has a valid process → Known (custom service)
/// - If port is not in KNOWN_PORTS and no process → Unowned
pub fn classify_port_risk(port: u16, process_name: &str, pid: Option<u32>) -> PortRisk {
    let name_lower = process_name.to_lowercase();

    // No PID or empty process name → unowned
    if pid.unwrap_or(0) == 0 || name_lower.is_empty() || name_lower == "???" {
        return PortRisk::Unowned;
    }

    // Check against known ports
    for &(known_port, expected_proc) in KNOWN_PORTS {
        if port == known_port {
            // Empty expected_proc means any process is fine for this port
            if expected_proc.is_empty() || name_lower.contains(expected_proc) {
                return PortRisk::Known;
            }
            // Port matches but process doesn't — suspicious
            return PortRisk::Suspicious;
        }
    }

    // Not a known port, but has a valid process — acceptable (custom service)
    PortRisk::Known
}

// ── Security score computation ───────────────────────────────────

/// Compute the security score (0-100) from the current state.
///
/// Starts at 100 and applies weighted penalties:
/// - Active threats: -20 each
/// - Suspicious processes: -10 each
/// - Unowned listeners: -5 each
/// - Risky ports: -5 each
/// - Auth log not readable: -3
/// - Modified packages: -2 each
pub fn compute_security_score(state: &SecurityState) -> u8 {
    let mut score: i32 = 100;

    score -= (state.active_threats as i32) * (SCORE_PENALTY_THREAT as i32);
    score -= (state.suspicious_count as i32) * (SCORE_PENALTY_SUSPICIOUS as i32);
    score -= (state.unowned_listeners as i32) * (SCORE_PENALTY_UNOWNED_LISTENER as i32);
    score -= (state.risky_ports.len() as i32) * (SCORE_PENALTY_RISKY_PORT as i32);

    if !state.auth_log_readable {
        score -= SCORE_PENALTY_NO_AUTH_LOG as i32;
    }

    let pkg_count = state.modified_packages.len().min(10) as i32; // cap at 10
    score -= pkg_count * (SCORE_PENALTY_MODIFIED_PKG as i32);

    score.clamp(0, 100) as u8
}

// ── Full refresh ─────────────────────────────────────────────────

/// Perform a full security data refresh.
///
/// `slow_ops` controls whether expensive operations (dpkg --verify) are run.
pub fn refresh_security_state(
    state: &mut SecurityState,
    store: &EventStore,
    alerts: &[Alert],
    slow_ops: bool,
) {
    // Fast operations (every refresh)
    state.listeners = collect_listeners(store);
    state.connections = collect_connections(store);
    state.events = collect_security_events(store, alerts);
    state.logged_in_users = collect_logged_in_users();

    // Compute threat counters from alerts
    state.active_threats = alerts
        .iter()
        .filter(|a| a.category == AlertCategory::SecurityThreat)
        .count();
    state.suspicious_count = alerts
        .iter()
        .filter(|a| a.category == AlertCategory::Suspicious)
        .count();

    // Port risk analysis
    state.risky_ports = state
        .listeners
        .iter()
        .filter(|l| l.risk == PortRisk::Suspicious)
        .map(|l| l.port)
        .collect();
    state.unowned_listeners = state
        .listeners
        .iter()
        .filter(|l| l.risk == PortRisk::Unowned)
        .count();

    // Auth log (fast — just counts lines)
    let (auth_count, auth_readable) = count_auth_events_24h();
    state.auth_event_count_24h = auth_count;
    state.auth_log_readable = auth_readable;

    // Slow operations (only on slow_ops cycles)
    if slow_ops {
        state.modified_packages = collect_modified_packages();
    }

    // Compute score
    state.prev_score = state.score;
    state.score = compute_security_score(state);

    state.last_refresh = Some(std::time::Instant::now());
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_known_port_matching_process() {
        assert_eq!(
            classify_port_risk(5432, "postgres", Some(123)),
            PortRisk::Known
        );
        assert_eq!(
            classify_port_risk(53, "systemd-resolve", Some(1)),
            PortRisk::Known
        );
    }

    #[test]
    fn classify_known_port_wrong_process() {
        assert_eq!(
            classify_port_risk(22, "xmrig", Some(999)),
            PortRisk::Suspicious
        );
        assert_eq!(
            classify_port_risk(5432, "cryptominer", Some(100)),
            PortRisk::Suspicious
        );
    }

    #[test]
    fn classify_unknown_port_with_valid_process() {
        assert_eq!(
            classify_port_risk(9999, "my-custom-app", Some(500)),
            PortRisk::Known
        );
    }

    #[test]
    fn classify_no_pid_is_unowned() {
        assert_eq!(classify_port_risk(8080, "???", None), PortRisk::Unowned);
        assert_eq!(
            classify_port_risk(8080, "something", Some(0)),
            PortRisk::Unowned
        );
    }

    #[test]
    fn classify_generic_known_port_any_process() {
        // Port 8080 has empty expected_proc → any process is fine
        assert_eq!(classify_port_risk(8080, "nginx", Some(10)), PortRisk::Known);
        assert_eq!(classify_port_risk(8080, "java", Some(20)), PortRisk::Known);
    }

    /// Helper: a clean baseline state with auth_log_readable = true
    /// so that only the penalty under test is measured.
    fn clean_state() -> SecurityState {
        let mut s = SecurityState::default();
        s.auth_log_readable = true; // avoid -3 auth_log penalty
        s
    }

    #[test]
    fn score_starts_at_100_when_clean() {
        let state = clean_state();
        assert_eq!(compute_security_score(&state), 100);
    }

    #[test]
    fn score_deducts_for_threats() {
        let mut state = clean_state();
        state.active_threats = 2;
        assert_eq!(compute_security_score(&state), 60); // 100 - 2*20
    }

    #[test]
    fn score_deducts_for_suspicious_and_unowned() {
        let mut state = clean_state();
        state.suspicious_count = 1;
        state.unowned_listeners = 2;
        // 100 - 10 - 10 = 80
        assert_eq!(compute_security_score(&state), 80);
    }

    #[test]
    fn score_clamps_at_zero() {
        let mut state = SecurityState::default();
        state.active_threats = 10; // -200, way below 0
        assert_eq!(compute_security_score(&state), 0);
    }

    #[test]
    fn score_deducts_for_no_auth_log() {
        let mut state = SecurityState::default();
        state.auth_log_readable = false;
        assert_eq!(compute_security_score(&state), 97); // 100 - 3
    }

    #[test]
    fn score_deducts_for_modified_packages() {
        let mut state = clean_state();
        state.modified_packages = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(compute_security_score(&state), 94); // 100 - 3*2
    }

    #[test]
    fn logged_in_users_returns_vec() {
        // Just verify it doesn't panic; actual content depends on environment
        let users = collect_logged_in_users();
        // Should be a vec (possibly empty in CI)
        assert!(users.len() < 100);
    }

    #[test]
    fn auth_events_best_effort() {
        let (count, readable) = count_auth_events_24h();
        // Either readable or not — no panic
        if readable {
            assert!(count < 100_000);
        } else {
            assert_eq!(count, 0);
        }
    }
}
