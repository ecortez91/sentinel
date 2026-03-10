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

// ── Auth log parsing (single-pass scan) ──────────────────────────

/// Result of a single-pass scan of /var/log/auth.log.
///
/// Consolidates what was previously two separate scans (count_auth_events_24h
/// + collect_auth_events) into one file read, plus SSH brute-force extraction.
pub struct AuthLogScanResult {
    /// Whether the file was readable at all.
    pub readable: bool,
    /// Count of meaningful auth events (sessions, failures, sudo).
    pub event_count: usize,
    /// Individual auth events for the timeline (capped at MAX_AUTH_EVENTS).
    pub events: Vec<SecurityEvent>,
    /// IPs with >= SSH_BRUTE_FORCE_THRESHOLD failed attempts.
    pub brute_force_entries: Vec<SshBruteForceEntry>,
}

/// Perform a single-pass scan of /var/log/auth.log.
///
/// Extracts:
/// - Auth event count (replaces `count_auth_events_24h`)
/// - Timeline events (replaces `collect_auth_events`)
/// - SSH brute-force entries (new: feature #11)
///
/// All from one file read instead of the previous two.
pub fn scan_auth_log() -> AuthLogScanResult {
    use std::collections::HashMap;

    let path = std::path::Path::new("/var/log/auth.log");
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return AuthLogScanResult {
                readable: false,
                event_count: 0,
                events: Vec::new(),
                brute_force_entries: Vec::new(),
            }
        }
    };

    let reader = std::io::BufReader::new(file);
    let mut event_count = 0usize;
    let mut events = Vec::new();
    // Track failed SSH attempts per source IP: ip -> (count, usernames)
    let mut failed_ssh: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for line in reader.lines().take(10_000).flatten() {
        // Count meaningful auth events
        let is_auth_event = line.contains("session opened")
            || line.contains("session closed")
            || line.contains("authentication failure")
            || line.contains("Failed password")
            || line.contains("Accepted password")
            || line.contains("sudo:")
            || line.contains("su:");

        if is_auth_event {
            event_count += 1;
        }

        // Build timeline events
        let timeline_entry = if line.contains("Failed password") {
            // Also track for brute-force detection
            if let Some((ip, user)) = extract_failed_ssh_info(&line) {
                let entry = failed_ssh.entry(ip).or_insert_with(|| (0, Vec::new()));
                entry.0 += 1;
                if !entry.1.contains(&user) {
                    entry.1.push(user);
                }
            }
            let msg = extract_auth_detail(&line, "Failed password");
            Some((AlertSeverity::Warning, format!("Failed login: {}", msg)))
        } else if line.contains("authentication failure") {
            let msg = extract_auth_detail(&line, "authentication failure");
            Some((AlertSeverity::Warning, format!("Auth failure: {}", msg)))
        } else if line.contains("Accepted password") || line.contains("Accepted publickey") {
            let msg = extract_auth_detail(&line, "Accepted");
            Some((AlertSeverity::Info, format!("Login accepted: {}", msg)))
        } else if line.contains("sudo:") {
            let msg = extract_sudo_detail(&line);
            Some((AlertSeverity::Info, format!("sudo: {}", msg)))
        } else {
            None
        };

        if let Some((severity, message)) = timeline_entry {
            events.push(SecurityEvent {
                timestamp: Local::now(), // approximate — syslog dates not parsed
                kind: SecurityEventKind::AuthEvent,
                severity,
                message,
                pid: None,
            });
        }
    }

    // Keep only the most recent timeline entries
    if events.len() > MAX_AUTH_EVENTS {
        events.drain(..events.len() - MAX_AUTH_EVENTS);
    }

    // Build brute-force entries from IPs exceeding the threshold
    let brute_force_entries: Vec<SshBruteForceEntry> = failed_ssh
        .into_iter()
        .filter(|(_, (count, _))| *count >= SSH_BRUTE_FORCE_THRESHOLD)
        .map(|(ip, (count, users))| SshBruteForceEntry {
            source_ip: ip,
            attempt_count: count,
            last_seen: Local::now(), // approximate
            target_users: users,
        })
        .collect();

    AuthLogScanResult {
        readable: true,
        event_count,
        events,
        brute_force_entries,
    }
}

/// Extract source IP and target username from a "Failed password" auth.log line.
///
/// Typical format: `... Failed password for [invalid user] <user> from <ip> port ...`
fn extract_failed_ssh_info(line: &str) -> Option<(String, String)> {
    // Find the IP: look for "from <ip> port"
    let from_pos = line.find("from ")?;
    let after_from = &line[from_pos + 5..];
    let ip = after_from.split_whitespace().next()?;

    // Find the username: "for <user> from" or "for invalid user <user> from"
    let user = if let Some(for_pos) = line.find("for invalid user ") {
        let after = &line[for_pos + 17..];
        after.split_whitespace().next().unwrap_or("?")
    } else if let Some(for_pos) = line.find("for ") {
        let after = &line[for_pos + 4..];
        after.split_whitespace().next().unwrap_or("?")
    } else {
        "?"
    };

    Some((ip.to_string(), user.to_string()))
}

/// Extract meaningful detail from a "Failed password" or "Accepted" auth.log line.
fn extract_auth_detail(line: &str, keyword: &str) -> String {
    if let Some(pos) = line.find(keyword) {
        line[pos..].chars().take(80).collect()
    } else {
        line.chars().take(80).collect()
    }
}

/// Extract meaningful detail from a sudo line.
fn extract_sudo_detail(line: &str) -> String {
    if let Some(pos) = line.find("sudo:") {
        line[pos + 5..].trim().chars().take(80).collect()
    } else {
        line.chars().take(80).collect()
    }
}

// ── Cron entry collection (#12) ──────────────────────────────────

/// Collect cron entries from the system.
///
/// Reads from:
/// - User crontabs via `crontab -l` (current user only, best-effort)
/// - System cron directories: `/etc/crontab`, `/etc/cron.d/`
///
/// Returns up to `MAX_CRON_ENTRIES` entries.
pub fn collect_cron_entries() -> Vec<CronEntry> {
    let mut entries = Vec::new();

    // User crontab (current user)
    if let Ok(output) = std::process::Command::new("crontab").arg("-l").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
            for line in text.lines() {
                let trimmed = line.trim();
                // Skip comments and empty lines
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((schedule, command)) = parse_cron_line(trimmed) {
                    entries.push(CronEntry {
                        user: user.clone(),
                        schedule,
                        command,
                        source: "user crontab".to_string(),
                    });
                }
                if entries.len() >= MAX_CRON_ENTRIES {
                    return entries;
                }
            }
        }
    }

    // System crontab
    if let Ok(content) = std::fs::read_to_string("/etc/crontab") {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // System crontab lines have 6 fields before command (5 schedule + user)
            if let Some((schedule, command)) = parse_system_cron_line(trimmed) {
                entries.push(CronEntry {
                    user: "system".to_string(),
                    schedule,
                    command,
                    source: "/etc/crontab".to_string(),
                });
            }
            if entries.len() >= MAX_CRON_ENTRIES {
                return entries;
            }
        }
    }

    // /etc/cron.d/ directory
    if let Ok(dir) = std::fs::read_dir("/etc/cron.d") {
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((schedule, command)) = parse_system_cron_line(trimmed) {
                        entries.push(CronEntry {
                            user: "system".to_string(),
                            schedule,
                            command,
                            source: path.display().to_string(),
                        });
                    }
                    if entries.len() >= MAX_CRON_ENTRIES {
                        return entries;
                    }
                }
            }
        }
    }

    entries
}

/// Parse a user crontab line into (schedule, command).
///
/// Format: `min hour dom mon dow command...`
/// Or shorthand: `@reboot command...`, `@daily command...`
/// Returns None for lines that don't look like cron entries.
fn parse_cron_line(line: &str) -> Option<(String, String)> {
    let first = line.split_whitespace().next()?;

    if !first
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_digit() || c == '*' || c == '@')
    {
        return None;
    }

    // Handle @reboot, @daily, @hourly, etc.
    if first.starts_with('@') {
        let command = line[first.len()..].trim().to_string();
        if command.is_empty() {
            return None;
        }
        return Some((first.to_string(), command));
    }

    // Standard 5-field schedule + command
    let parts: Vec<&str> = line.splitn(6, char::is_whitespace).collect();
    if parts.len() < 6 {
        return None;
    }
    let schedule = parts[..5].join(" ");
    let command = parts[5].to_string();
    Some((schedule, command))
}

/// Parse a system crontab line (has user field after schedule).
///
/// Format: `min hour dom mon dow user command...`
fn parse_system_cron_line(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.splitn(7, char::is_whitespace).collect();
    if parts.len() < 7 {
        return None;
    }
    let first = parts[0];
    if !first
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_digit() || c == '*' || c == '@')
    {
        return None;
    }
    let schedule = parts[..5].join(" ");
    // parts[5] is the user, parts[6] is the command
    let command = parts[6].to_string();
    Some((schedule, command))
}

// ── Systemd timer collection (#12) ───────────────────────────────

/// Collect active systemd timers.
///
/// Runs `systemctl list-timers --all --no-pager --plain` and parses the output.
/// Returns up to `MAX_SYSTEMD_TIMERS` entries.
pub fn collect_systemd_timers() -> Vec<SystemdTimer> {
    let output = match std::process::Command::new("systemctl")
        .args(["list-timers", "--all", "--no-pager", "--plain"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut timers = Vec::new();

    // Output format (plain): NEXT LEFT LAST PASSED UNIT ACTIVATES
    // Skip header line
    for line in text.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("NEXT") || trimmed.contains("timers listed") {
            continue;
        }

        // The plain format has variable whitespace; the UNIT is the 5th-to-last field
        // and ACTIVATES is the last field. Parse from the right side.
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        // In plain mode: the last two fields are always UNIT and ACTIVATES
        let activates = parts[parts.len() - 1].to_string();
        let unit = parts[parts.len() - 2].to_string();

        // NEXT is the first few fields (date+time), or "n/a"
        let next_trigger = if parts[0] == "n/a" {
            None
        } else {
            // Join the first few fields as the next trigger time
            // Typically: "Wed 2026-03-11 06:00:00 UTC"
            let next_parts: Vec<&str> = parts.iter().take(parts.len() - 2).copied().collect();
            if next_parts.is_empty() {
                None
            } else {
                Some(next_parts.join(" "))
            }
        };

        timers.push(SystemdTimer {
            unit,
            activates,
            active: true, // listed timers are active
            next_trigger,
        });

        if timers.len() >= MAX_SYSTEMD_TIMERS {
            break;
        }
    }

    timers
}

// ── Suspicious outbound analysis (#13) ───────────────────────────

/// Analyze established connections for suspicious outbound traffic.
///
/// Flags connections to remote ports not in `STANDARD_OUTBOUND_PORTS`.
/// Excludes loopback connections (127.0.0.0/8, ::1).
/// Returns up to `MAX_SUSPICIOUS_OUTBOUND` entries.
pub fn analyze_suspicious_outbound(connections: &[ConnectionInfo]) -> Vec<SuspiciousOutbound> {
    connections
        .iter()
        .filter(|c| {
            // Skip loopback
            if c.remote_addr.starts_with("127.")
                || c.remote_addr == "::1"
                || c.remote_addr == "0.0.0.0"
            {
                return false;
            }
            // Flag if remote port is not standard
            !STANDARD_OUTBOUND_PORTS.contains(&c.remote_port)
        })
        .take(MAX_SUSPICIOUS_OUTBOUND)
        .map(|c| SuspiciousOutbound {
            process_name: c.process_name.clone(),
            pid: c.pid,
            remote_addr: c.remote_addr.clone(),
            remote_port: c.remote_port,
            local_port: c.local_port,
        })
        .collect()
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
/// - SSH brute-force IPs: -15 each (#11)
/// - Suspicious outbound: -5 each, capped at -20 (#13)
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

    // SSH brute-force penalty (#11)
    score -= (state.ssh_brute_force.len() as i32) * (SCORE_PENALTY_SSH_BRUTE_FORCE as i32);

    // Suspicious outbound penalty (#13), capped
    let outbound_penalty =
        (state.suspicious_outbound.len() as i32) * (SCORE_PENALTY_SUSPICIOUS_OUTBOUND as i32);
    score -= outbound_penalty.min(SCORE_SUSPICIOUS_OUTBOUND_CAP as i32);

    score.clamp(0, 100) as u8
}

// ── Full refresh ─────────────────────────────────────────────────

/// Perform a full security data refresh.
///
/// `slow_ops` controls whether expensive operations (dpkg --verify,
/// cron/systemd enumeration) are run.
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

    // Single-pass auth.log scan (replaces separate count + collect calls)
    let auth_scan = scan_auth_log();
    state.auth_event_count_24h = auth_scan.event_count;
    state.auth_log_readable = auth_scan.readable;
    state.ssh_brute_force = auth_scan.brute_force_entries;

    // Merge auth timeline events
    state.events.extend(auth_scan.events);
    state.events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    state.events.truncate(MAX_SECURITY_EVENTS);

    state.logged_in_users = collect_logged_in_users();

    // Suspicious outbound analysis (#13) — derived from connections
    state.suspicious_outbound = analyze_suspicious_outbound(&state.connections);

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

    // Slow operations (only on slow_ops cycles)
    if slow_ops {
        state.modified_packages = collect_modified_packages();
        state.cron_entries = collect_cron_entries();
        state.systemd_timers = collect_systemd_timers();
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
    fn auth_log_scan_best_effort() {
        let result = scan_auth_log();
        // Either readable or not — no panic
        if result.readable {
            assert!(result.event_count < 100_000);
        } else {
            assert_eq!(result.event_count, 0);
        }
    }

    #[test]
    fn auth_log_scan_events_bounded() {
        // Best-effort: may be empty on systems without auth.log
        let result = scan_auth_log();
        assert!(result.events.len() <= MAX_AUTH_EVENTS);
        for ev in &result.events {
            assert_eq!(ev.kind, SecurityEventKind::AuthEvent);
        }
    }

    #[test]
    fn auth_log_scan_brute_force_threshold() {
        let result = scan_auth_log();
        // All brute-force entries must meet the threshold
        for entry in &result.brute_force_entries {
            assert!(
                entry.attempt_count >= SSH_BRUTE_FORCE_THRESHOLD,
                "Brute-force entry {} has {} attempts, below threshold {}",
                entry.source_ip,
                entry.attempt_count,
                SSH_BRUTE_FORCE_THRESHOLD,
            );
        }
    }

    #[test]
    fn extract_auth_detail_truncates() {
        let line =
            "Jan  1 00:00:00 host sshd[123]: Failed password for root from 10.0.0.1 port 22 ssh2";
        let detail = extract_auth_detail(line, "Failed password");
        assert!(detail.starts_with("Failed password"));
        assert!(detail.len() <= 80);
    }

    #[test]
    fn extract_sudo_detail_extracts_command() {
        let line = "Jan  1 00:00:00 host sudo: user : TTY=pts/0 ; PWD=/home ; COMMAND=/usr/bin/apt";
        let detail = extract_sudo_detail(line);
        assert!(detail.contains("user"));
    }

    // ── SSH brute-force extraction tests ─────────────────────────

    #[test]
    fn extract_failed_ssh_info_standard_line() {
        let line = "Jan  1 00:00:00 host sshd[123]: Failed password for root from 192.168.1.100 port 54321 ssh2";
        let result = extract_failed_ssh_info(line);
        assert_eq!(result, Some(("192.168.1.100".into(), "root".into())));
    }

    #[test]
    fn extract_failed_ssh_info_invalid_user() {
        let line = "Jan  1 00:00:00 host sshd[123]: Failed password for invalid user admin from 10.0.0.5 port 12345 ssh2";
        let result = extract_failed_ssh_info(line);
        assert_eq!(result, Some(("10.0.0.5".into(), "admin".into())));
    }

    #[test]
    fn extract_failed_ssh_info_no_from_keyword() {
        let line = "Jan  1 00:00:00 host sshd[123]: Some other message";
        let result = extract_failed_ssh_info(line);
        assert_eq!(result, None);
    }

    // ── Cron parsing tests ───────────────────────────────────────

    #[test]
    fn parse_cron_line_standard() {
        let result = parse_cron_line("*/5 * * * * /usr/bin/backup.sh");
        assert!(result.is_some());
        let (schedule, command) = result.unwrap();
        assert_eq!(schedule, "*/5 * * * *");
        assert_eq!(command, "/usr/bin/backup.sh");
    }

    #[test]
    fn parse_cron_line_at_reboot() {
        let result = parse_cron_line("@reboot /usr/local/bin/startup.sh");
        assert!(result.is_some());
        let (schedule, command) = result.unwrap();
        assert_eq!(schedule, "@reboot");
        assert!(command.contains("/usr/local/bin/startup.sh"));
    }

    #[test]
    fn parse_cron_line_rejects_non_cron() {
        // Lines starting with letters (not digits or *) are not cron
        assert!(parse_cron_line("SHELL=/bin/bash").is_none());
        assert!(parse_cron_line("PATH=/usr/bin:/bin").is_none());
    }

    #[test]
    fn parse_cron_line_too_few_fields() {
        assert!(parse_cron_line("* * *").is_none());
    }

    #[test]
    fn parse_system_cron_line_standard() {
        let result = parse_system_cron_line("0 3 * * * root /usr/bin/certbot renew");
        assert!(result.is_some());
        let (schedule, command) = result.unwrap();
        assert_eq!(schedule, "0 3 * * *");
        assert_eq!(command, "/usr/bin/certbot renew");
    }

    // ── Suspicious outbound tests ────────────────────────────────

    #[test]
    fn analyze_suspicious_outbound_flags_nonstandard_port() {
        let connections = vec![ConnectionInfo {
            local_addr: "192.168.1.10".into(),
            local_port: 49001,
            remote_addr: "203.0.113.50".into(),
            remote_port: 4444, // not in STANDARD_OUTBOUND_PORTS
            pid: Some(789),
            process_name: "suspicious".into(),
            state: "ESTABLISHED".into(),
        }];
        let result = analyze_suspicious_outbound(&connections);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].remote_port, 4444);
        assert_eq!(result[0].process_name, "suspicious");
    }

    #[test]
    fn analyze_suspicious_outbound_allows_standard_port() {
        let connections = vec![ConnectionInfo {
            local_addr: "192.168.1.10".into(),
            local_port: 49002,
            remote_addr: "8.8.8.8".into(),
            remote_port: 443, // standard HTTPS
            pid: Some(100),
            process_name: "curl".into(),
            state: "ESTABLISHED".into(),
        }];
        let result = analyze_suspicious_outbound(&connections);
        assert!(result.is_empty());
    }

    #[test]
    fn analyze_suspicious_outbound_skips_loopback() {
        let connections = vec![ConnectionInfo {
            local_addr: "127.0.0.1".into(),
            local_port: 49003,
            remote_addr: "127.0.0.1".into(),
            remote_port: 9999, // non-standard but loopback
            pid: Some(200),
            process_name: "local-svc".into(),
            state: "ESTABLISHED".into(),
        }];
        let result = analyze_suspicious_outbound(&connections);
        assert!(result.is_empty());
    }

    #[test]
    fn analyze_suspicious_outbound_capped() {
        let connections: Vec<ConnectionInfo> = (0..100)
            .map(|i| ConnectionInfo {
                local_addr: "10.0.0.1".into(),
                local_port: 40000 + i,
                remote_addr: format!("203.0.113.{}", i % 256),
                remote_port: 5555, // non-standard
                pid: Some(300),
                process_name: "bulk".into(),
                state: "ESTABLISHED".into(),
            })
            .collect();
        let result = analyze_suspicious_outbound(&connections);
        assert_eq!(result.len(), MAX_SUSPICIOUS_OUTBOUND);
    }

    // ── Score computation with new penalties ──────────────────────

    #[test]
    fn score_deducts_for_ssh_brute_force() {
        let mut state = clean_state();
        state.ssh_brute_force = vec![SshBruteForceEntry {
            source_ip: "10.0.0.1".into(),
            attempt_count: 10,
            last_seen: Local::now(),
            target_users: vec!["root".into()],
        }];
        // 100 - 15 = 85
        assert_eq!(compute_security_score(&state), 85);
    }

    #[test]
    fn score_deducts_for_suspicious_outbound_capped() {
        let mut state = clean_state();
        // 10 suspicious outbound * 5 = 50, but capped at 20
        state.suspicious_outbound = (0..10)
            .map(|i| SuspiciousOutbound {
                process_name: "test".into(),
                pid: Some(i as u32),
                remote_addr: "1.2.3.4".into(),
                remote_port: 4444,
                local_port: 40000 + i,
            })
            .collect();
        // 100 - 20 (cap) = 80
        assert_eq!(compute_security_score(&state), 80);
    }

    #[test]
    fn score_combined_new_penalties() {
        let mut state = clean_state();
        state.ssh_brute_force = vec![SshBruteForceEntry {
            source_ip: "10.0.0.1".into(),
            attempt_count: 20,
            last_seen: Local::now(),
            target_users: vec!["root".into(), "admin".into()],
        }];
        state.suspicious_outbound = vec![SuspiciousOutbound {
            process_name: "nc".into(),
            pid: Some(999),
            remote_addr: "evil.example.com".into(),
            remote_port: 1337,
            local_port: 55555,
        }];
        // 100 - 15 (ssh) - 5 (1 outbound) = 80
        assert_eq!(compute_security_score(&state), 80);
    }

    // ── Collector best-effort tests ──────────────────────────────

    #[test]
    fn collect_cron_entries_returns_bounded_vec() {
        let entries = collect_cron_entries();
        assert!(entries.len() <= MAX_CRON_ENTRIES);
    }

    #[test]
    fn collect_systemd_timers_returns_bounded_vec() {
        let timers = collect_systemd_timers();
        assert!(timers.len() <= MAX_SYSTEMD_TIMERS);
    }
}
