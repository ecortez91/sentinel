//! Persistent event store backed by SQLite.
//!
//! Records system snapshots, process snapshots, discrete events, and network
//! socket state. Provides time-range queries for diagnostics, anomaly detection,
//! and AI context enrichment.
//!
//! Design:
//! - WAL mode for concurrent reads during writes
//! - Batched inserts within transactions for throughput
//! - Automatic retention (purge data older than configured window)
//! - In-process only — no external DB server needed

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Result as SqlResult};

use crate::models::{ProcessInfo, SystemSnapshot};

// ── Constants ─────────────────────────────────────────────────────

/// Default data retention window (24 hours in seconds).
pub const DEFAULT_RETENTION_SECS: u64 = 24 * 3600;

/// How many top processes (by CPU) to snapshot each tick.
const TOP_CPU_SNAPSHOT_COUNT: usize = 50;

/// How many top processes (by memory) to snapshot each tick.
const TOP_MEM_SNAPSHOT_COUNT: usize = 30;

/// Cleanup runs every N inserts to avoid running every tick.
const CLEANUP_INTERVAL: u64 = 300;

// ── Event types ───────────────────────────────────────────────────

/// Discrete event kinds tracked by the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum EventKind {
    ProcessStart,
    ProcessExit,
    PortBind,
    PortRelease,
    Alert,
    CpuSpike,
    MemorySpike,
    OomKill,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventKind::ProcessStart => write!(f, "process_start"),
            EventKind::ProcessExit => write!(f, "process_exit"),
            EventKind::PortBind => write!(f, "port_bind"),
            EventKind::PortRelease => write!(f, "port_release"),
            EventKind::Alert => write!(f, "alert"),
            EventKind::CpuSpike => write!(f, "cpu_spike"),
            EventKind::MemorySpike => write!(f, "memory_spike"),
            EventKind::OomKill => write!(f, "oom_kill"),
        }
    }
}

#[allow(dead_code)]
impl EventKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "process_start" => Some(EventKind::ProcessStart),
            "process_exit" => Some(EventKind::ProcessExit),
            "port_bind" => Some(EventKind::PortBind),
            "port_release" => Some(EventKind::PortRelease),
            "alert" => Some(EventKind::Alert),
            "cpu_spike" => Some(EventKind::CpuSpike),
            "memory_spike" => Some(EventKind::MemorySpike),
            "oom_kill" => Some(EventKind::OomKill),
            _ => None,
        }
    }
}

// ── Query result types ────────────────────────────────────────────

/// A system snapshot row from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SystemSnapshotRow {
    pub ts: i64,
    pub cpu_global: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub load_1: f64,
    pub load_5: f64,
    pub load_15: f64,
    pub gpu_util: Option<u32>,
    pub gpu_mem_used: Option<u64>,
    pub gpu_temp: Option<u32>,
}

/// A process snapshot row from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessSnapshotRow {
    pub ts: i64,
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem_bytes: u64,
    pub disk_read: u64,
    pub disk_write: u64,
    pub status: String,
    pub user: String,
}

/// A discrete event row from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EventRow {
    pub ts: i64,
    pub kind: String,
    pub pid: Option<u32>,
    pub name: Option<String>,
    pub detail: Option<String>,
    pub severity: Option<String>,
}

/// A network socket row from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SocketRow {
    pub ts: i64,
    pub pid: Option<u32>,
    pub name: Option<String>,
    pub protocol: String,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: Option<String>,
    pub remote_port: Option<u16>,
    pub state: String,
}

// ── EventStore ────────────────────────────────────────────────────

/// Persistent event store backed by SQLite.
pub struct EventStore {
    conn: Connection,
    retention_secs: u64,
    insert_count: u64,
    /// PIDs seen in the previous tick (for start/exit detection).
    prev_pids: HashSet<u32>,
    /// PID -> name mapping from previous tick.
    prev_pid_names: std::collections::HashMap<u32, String>,
    /// Listening ports from previous tick: (protocol, local_port, pid).
    prev_listeners: HashSet<(String, u16, u32)>,
}

#[allow(dead_code)]
impl EventStore {
    /// Open (or create) the event store database.
    ///
    /// If `path` is `None`, uses an in-memory database (useful for tests).
    pub fn open(path: Option<&Path>) -> SqlResult<Self> {
        let conn = match path {
            Some(p) => {
                // Ensure parent directory exists
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                Connection::open(p)?
            }
            None => Connection::open_in_memory()?,
        };

        let mut store = Self {
            conn,
            retention_secs: DEFAULT_RETENTION_SECS,
            insert_count: 0,
            prev_pids: HashSet::new(),
            prev_pid_names: std::collections::HashMap::new(),
            prev_listeners: HashSet::new(),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// Default database path: `~/.local/share/sentinel/sentinel.db`
    pub fn default_path() -> PathBuf {
        crate::constants::home_dir()
            .join(".local")
            .join("share")
            .join("sentinel")
            .join("sentinel.db")
    }

    /// Set the data retention window (in seconds).
    pub fn set_retention(&mut self, secs: u64) {
        self.retention_secs = secs;
    }

    // ── Schema ────────────────────────────────────────────────────

    fn init_schema(&mut self) -> SqlResult<()> {
        // Enable WAL mode for better concurrent read performance
        self.conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        self.conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS system_snapshots (
                id          INTEGER PRIMARY KEY,
                ts          INTEGER NOT NULL,
                cpu_global  REAL,
                mem_used    INTEGER,
                mem_total   INTEGER,
                swap_used   INTEGER,
                swap_total  INTEGER,
                load_1      REAL,
                load_5      REAL,
                load_15     REAL,
                gpu_util    INTEGER,
                gpu_mem_used INTEGER,
                gpu_temp    INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_sys_ts ON system_snapshots(ts);

            CREATE TABLE IF NOT EXISTS process_snapshots (
                id          INTEGER PRIMARY KEY,
                ts          INTEGER NOT NULL,
                pid         INTEGER NOT NULL,
                name        TEXT NOT NULL,
                cpu         REAL,
                mem_bytes   INTEGER,
                disk_read   INTEGER,
                disk_write  INTEGER,
                status      TEXT,
                user        TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_proc_ts ON process_snapshots(ts);
            CREATE INDEX IF NOT EXISTS idx_proc_pid_ts ON process_snapshots(pid, ts);

            CREATE TABLE IF NOT EXISTS events (
                id          INTEGER PRIMARY KEY,
                ts          INTEGER NOT NULL,
                kind        TEXT NOT NULL,
                pid         INTEGER,
                name        TEXT,
                detail      TEXT,
                severity    TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
            CREATE INDEX IF NOT EXISTS idx_events_kind_ts ON events(kind, ts);

            CREATE TABLE IF NOT EXISTS network_sockets (
                id          INTEGER PRIMARY KEY,
                ts          INTEGER NOT NULL,
                pid         INTEGER,
                name        TEXT,
                protocol    TEXT NOT NULL,
                local_addr  TEXT NOT NULL,
                local_port  INTEGER NOT NULL,
                remote_addr TEXT,
                remote_port INTEGER,
                state       TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_net_ts ON network_sockets(ts);
            CREATE INDEX IF NOT EXISTS idx_net_port ON network_sockets(local_port, ts);",
        )?;

        Ok(())
    }

    // ── System snapshots ──────────────────────────────────────────

    /// Record a system-wide snapshot.
    pub fn insert_system_snapshot(&mut self, system: &SystemSnapshot) -> SqlResult<()> {
        let ts = now_epoch_ms();
        let (gpu_util, gpu_mem, gpu_temp) = match &system.gpu {
            Some(g) => (
                Some(g.utilization),
                Some(g.memory_used),
                Some(g.temperature),
            ),
            None => (None, None, None),
        };

        self.conn.execute(
            "INSERT INTO system_snapshots (ts, cpu_global, mem_used, mem_total, swap_used, swap_total, load_1, load_5, load_15, gpu_util, gpu_mem_used, gpu_temp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                ts,
                system.global_cpu_usage,
                system.used_memory,
                system.total_memory,
                system.used_swap,
                system.total_swap,
                system.load_avg_1,
                system.load_avg_5,
                system.load_avg_15,
                gpu_util,
                gpu_mem,
                gpu_temp,
            ],
        )?;

        self.maybe_cleanup();
        Ok(())
    }

    /// Query system snapshots within a time range.
    /// `since_ms` is a Unix epoch timestamp in milliseconds.
    pub fn query_system_history(&self, since_ms: i64) -> SqlResult<Vec<SystemSnapshotRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, cpu_global, mem_used, mem_total, swap_used, swap_total, load_1, load_5, load_15, gpu_util, gpu_mem_used, gpu_temp
             FROM system_snapshots WHERE ts >= ?1 ORDER BY ts ASC",
        )?;

        let rows = stmt.query_map(params![since_ms], |row| {
            Ok(SystemSnapshotRow {
                ts: row.get(0)?,
                cpu_global: row.get(1)?,
                mem_used: row.get(2)?,
                mem_total: row.get(3)?,
                swap_used: row.get(4)?,
                swap_total: row.get(5)?,
                load_1: row.get(6)?,
                load_5: row.get(7)?,
                load_15: row.get(8)?,
                gpu_util: row.get(9)?,
                gpu_mem_used: row.get(10)?,
                gpu_temp: row.get(11)?,
            })
        })?;

        rows.collect()
    }

    // ── Process snapshots ─────────────────────────────────────────

    /// Record snapshots for top processes (by CPU and memory).
    ///
    /// Only stores the top N processes to keep the database manageable,
    /// not all 500+ system processes.
    pub fn insert_process_snapshots(&mut self, processes: &[ProcessInfo]) -> SqlResult<()> {
        let ts = now_epoch_ms();

        // Collect top processes by CPU
        let mut by_cpu: Vec<&ProcessInfo> = processes.iter().collect();
        by_cpu.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        by_cpu.truncate(TOP_CPU_SNAPSHOT_COUNT);

        // Collect top processes by memory
        let mut by_mem: Vec<&ProcessInfo> = processes.iter().collect();
        by_mem.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
        by_mem.truncate(TOP_MEM_SNAPSHOT_COUNT);

        // Merge into deduplicated set
        let mut seen_pids = HashSet::new();
        let mut to_insert: Vec<&ProcessInfo> = Vec::new();
        for p in by_cpu.into_iter().chain(by_mem.into_iter()) {
            if seen_pids.insert(p.pid) {
                to_insert.push(p);
            }
        }

        // Batch insert in a transaction
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO process_snapshots (ts, pid, name, cpu, mem_bytes, disk_read, disk_write, status, user)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for p in &to_insert {
                stmt.execute(params![
                    ts,
                    p.pid,
                    p.name,
                    p.cpu_usage,
                    p.memory_bytes,
                    p.disk_read_bytes,
                    p.disk_write_bytes,
                    p.status.to_string(),
                    p.user,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Query the history of a specific process by PID.
    pub fn query_process_history(
        &self,
        pid: u32,
        since_ms: i64,
    ) -> SqlResult<Vec<ProcessSnapshotRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, pid, name, cpu, mem_bytes, disk_read, disk_write, status, user
             FROM process_snapshots WHERE pid = ?1 AND ts >= ?2 ORDER BY ts ASC",
        )?;

        let rows = stmt.query_map(params![pid, since_ms], |row| {
            Ok(ProcessSnapshotRow {
                ts: row.get(0)?,
                pid: row.get(1)?,
                name: row.get(2)?,
                cpu: row.get(3)?,
                mem_bytes: row.get(4)?,
                disk_read: row.get(5)?,
                disk_write: row.get(6)?,
                status: row.get(7)?,
                user: row.get(8)?,
            })
        })?;

        rows.collect()
    }

    /// Query top processes at a given point in time (closest snapshot).
    pub fn query_top_processes_at(
        &self,
        ts_ms: i64,
        limit: usize,
    ) -> SqlResult<Vec<ProcessSnapshotRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, pid, name, cpu, mem_bytes, disk_read, disk_write, status, user
             FROM process_snapshots
             WHERE ts = (SELECT ts FROM process_snapshots ORDER BY ABS(ts - ?1) LIMIT 1)
             ORDER BY cpu DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![ts_ms, limit as i64], |row| {
            Ok(ProcessSnapshotRow {
                ts: row.get(0)?,
                pid: row.get(1)?,
                name: row.get(2)?,
                cpu: row.get(3)?,
                mem_bytes: row.get(4)?,
                disk_read: row.get(5)?,
                disk_write: row.get(6)?,
                status: row.get(7)?,
                user: row.get(8)?,
            })
        })?;

        rows.collect()
    }

    // ── Events ────────────────────────────────────────────────────

    /// Record a discrete event.
    pub fn insert_event(
        &self,
        kind: EventKind,
        pid: Option<u32>,
        name: Option<&str>,
        detail: Option<&str>,
        severity: Option<&str>,
    ) -> SqlResult<()> {
        let ts = now_epoch_ms();
        self.conn.execute(
            "INSERT INTO events (ts, kind, pid, name, detail, severity) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ts, kind.to_string(), pid, name, detail, severity],
        )?;
        Ok(())
    }

    /// Query events since a given timestamp.
    pub fn query_events_since(&self, since_ms: i64) -> SqlResult<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, kind, pid, name, detail, severity FROM events WHERE ts >= ?1 ORDER BY ts DESC",
        )?;

        let rows = stmt.query_map(params![since_ms], |row| {
            Ok(EventRow {
                ts: row.get(0)?,
                kind: row.get(1)?,
                pid: row.get(2)?,
                name: row.get(3)?,
                detail: row.get(4)?,
                severity: row.get(5)?,
            })
        })?;

        rows.collect()
    }

    /// Query events of a specific kind since a given timestamp.
    pub fn query_events_by_kind(&self, kind: EventKind, since_ms: i64) -> SqlResult<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, kind, pid, name, detail, severity FROM events WHERE kind = ?1 AND ts >= ?2 ORDER BY ts DESC",
        )?;

        let rows = stmt.query_map(params![kind.to_string(), since_ms], |row| {
            Ok(EventRow {
                ts: row.get(0)?,
                kind: row.get(1)?,
                pid: row.get(2)?,
                name: row.get(3)?,
                detail: row.get(4)?,
                severity: row.get(5)?,
            })
        })?;

        rows.collect()
    }

    // ── Network sockets ───────────────────────────────────────────

    /// Record current network socket state from /proc/net/tcp and /proc/net/tcp6.
    pub fn insert_network_sockets(&mut self) -> SqlResult<()> {
        let ts = now_epoch_ms();
        let sockets = read_proc_net_tcp();

        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO network_sockets (ts, pid, name, protocol, local_addr, local_port, remote_addr, remote_port, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for s in &sockets {
                stmt.execute(params![
                    ts,
                    s.pid,
                    s.name,
                    s.protocol,
                    s.local_addr,
                    s.local_port,
                    s.remote_addr,
                    s.remote_port,
                    s.state,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Query which process is/was listening on a specific port.
    pub fn query_port_history(&self, port: u16, since_ms: i64) -> SqlResult<Vec<SocketRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, pid, name, protocol, local_addr, local_port, remote_addr, remote_port, state
             FROM network_sockets WHERE local_port = ?1 AND ts >= ?2 ORDER BY ts DESC",
        )?;

        let rows = stmt.query_map(params![port, since_ms], |row| {
            Ok(SocketRow {
                ts: row.get(0)?,
                pid: row.get(1)?,
                name: row.get(2)?,
                protocol: row.get(3)?,
                local_addr: row.get(4)?,
                local_port: row.get(5)?,
                remote_addr: row.get(6)?,
                remote_port: row.get(7)?,
                state: row.get(8)?,
            })
        })?;

        rows.collect()
    }

    /// Query current listeners (most recent snapshot, LISTEN state only).
    pub fn query_current_listeners(&self) -> SqlResult<Vec<SocketRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, pid, name, protocol, local_addr, local_port, remote_addr, remote_port, state
             FROM network_sockets
             WHERE state = 'LISTEN' AND ts = (SELECT MAX(ts) FROM network_sockets)
             ORDER BY local_port ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SocketRow {
                ts: row.get(0)?,
                pid: row.get(1)?,
                name: row.get(2)?,
                protocol: row.get(3)?,
                local_addr: row.get(4)?,
                local_port: row.get(5)?,
                remote_addr: row.get(6)?,
                remote_port: row.get(7)?,
                state: row.get(8)?,
            })
        })?;

        rows.collect()
    }

    // ── Process lifecycle detection ───────────────────────────────

    /// Diff current PIDs against previous tick to detect process start/exit events.
    ///
    /// Call this once per tick after collecting processes. It will:
    /// 1. Insert `ProcessStart` events for new PIDs
    /// 2. Insert `ProcessExit` events for disappeared PIDs
    /// 3. Update internal state for the next tick
    pub fn detect_process_lifecycle(&mut self, processes: &[ProcessInfo]) -> SqlResult<()> {
        let current_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
        let current_names: std::collections::HashMap<u32, String> =
            processes.iter().map(|p| (p.pid, p.name.clone())).collect();

        // Skip on first tick (no previous data to diff)
        if !self.prev_pids.is_empty() {
            // New PIDs = started processes
            for &pid in current_pids.difference(&self.prev_pids) {
                let name = current_names.get(&pid).map(|s| s.as_str());
                self.insert_event(EventKind::ProcessStart, Some(pid), name, None, Some("info"))?;
            }

            // Disappeared PIDs = exited processes
            for &pid in self.prev_pids.difference(&current_pids) {
                let name = self.prev_pid_names.get(&pid).map(|s| s.as_str());
                self.insert_event(EventKind::ProcessExit, Some(pid), name, None, Some("info"))?;
            }
        }

        self.prev_pids = current_pids;
        self.prev_pid_names = current_names;
        Ok(())
    }

    /// Detect port bind/release events by diffing current listeners.
    ///
    /// Should be called after `insert_network_sockets()`.
    pub fn detect_port_changes(&mut self) -> SqlResult<()> {
        let current_listeners: HashSet<(String, u16, u32)> = self
            .query_current_listeners()?
            .into_iter()
            .filter_map(|s| s.pid.map(|pid| (s.protocol.clone(), s.local_port, pid)))
            .collect();

        if !self.prev_listeners.is_empty() {
            // New listeners
            for (proto, port, pid) in current_listeners.difference(&self.prev_listeners) {
                let detail = format!("{}:{}", proto, port);
                self.insert_event(
                    EventKind::PortBind,
                    Some(*pid),
                    None,
                    Some(&detail),
                    Some("info"),
                )?;
            }

            // Released listeners
            for (proto, port, pid) in self.prev_listeners.difference(&current_listeners) {
                let detail = format!("{}:{}", proto, port);
                self.insert_event(
                    EventKind::PortRelease,
                    Some(*pid),
                    None,
                    Some(&detail),
                    Some("info"),
                )?;
            }
        }

        self.prev_listeners = current_listeners;
        Ok(())
    }

    // ── Aggregate queries (for AI context) ────────────────────────

    /// Get a textual summary of recent events for AI context.
    pub fn recent_events_summary(&self, limit: usize) -> SqlResult<String> {
        let since = now_epoch_ms() - (self.retention_secs as i64 * 1000);
        let events = self.query_events_since(since)?;
        let mut lines = Vec::new();
        for e in events.iter().take(limit) {
            let age = format_age_ms(now_epoch_ms() - e.ts);
            let pid_str = e.pid.map(|p| format!(" PID {}", p)).unwrap_or_default();
            let name_str = e
                .name
                .as_deref()
                .map(|n| format!(" ({})", n))
                .unwrap_or_default();
            lines.push(format!("[{}] {}{}{}", age, e.kind, pid_str, name_str));
        }
        Ok(lines.join("\n"))
    }

    /// Get event counts by kind in the last N minutes.
    pub fn event_counts(&self, minutes: u64) -> SqlResult<std::collections::HashMap<String, u64>> {
        let since = now_epoch_ms() - (minutes as i64 * 60 * 1000);
        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM events WHERE ts >= ?1 GROUP BY kind")?;

        let mut counts = std::collections::HashMap::new();
        let rows = stmt.query_map(params![since], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;

        for row in rows {
            let (kind, count) = row?;
            counts.insert(kind, count);
        }
        Ok(counts)
    }

    // ── Retention / cleanup ───────────────────────────────────────

    /// Purge data older than the retention window.
    pub fn cleanup(&self) -> SqlResult<()> {
        let cutoff = now_epoch_ms() - (self.retention_secs as i64 * 1000);
        self.conn.execute(
            "DELETE FROM system_snapshots WHERE ts < ?1",
            params![cutoff],
        )?;
        self.conn.execute(
            "DELETE FROM process_snapshots WHERE ts < ?1",
            params![cutoff],
        )?;
        self.conn
            .execute("DELETE FROM events WHERE ts < ?1", params![cutoff])?;
        self.conn
            .execute("DELETE FROM network_sockets WHERE ts < ?1", params![cutoff])?;
        Ok(())
    }

    /// Run cleanup periodically (every CLEANUP_INTERVAL inserts).
    fn maybe_cleanup(&mut self) {
        self.insert_count += 1;
        if self.insert_count % CLEANUP_INTERVAL == 0 {
            let _ = self.cleanup();
        }
    }

    /// Get database file size in bytes (0 for in-memory).
    pub fn db_size_bytes(&self) -> u64 {
        let path: String = self
            .conn
            .query_row("PRAGMA database_list", [], |row| row.get(2))
            .unwrap_or_default();
        if path.is_empty() {
            return 0;
        }
        std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
    }

    /// Get row counts for each table (for diagnostics display).
    pub fn table_stats(&self) -> SqlResult<Vec<(String, u64)>> {
        let tables = [
            "system_snapshots",
            "process_snapshots",
            "events",
            "network_sockets",
        ];
        let mut stats = Vec::new();
        for table in &tables {
            let count: u64 =
                self.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| {
                        row.get(0)
                    })?;
            stats.push((table.to_string(), count));
        }
        Ok(stats)
    }
}

// ── /proc/net/tcp parser ──────────────────────────────────────────

/// A parsed network socket from /proc/net/tcp.
#[derive(Debug)]
struct ParsedSocket {
    pid: Option<u32>,
    name: Option<String>,
    protocol: String,
    local_addr: String,
    local_port: u16,
    remote_addr: Option<String>,
    remote_port: Option<u16>,
    state: String,
}

/// Read and parse /proc/net/tcp and /proc/net/tcp6.
///
/// Maps inodes to PIDs by scanning /proc/<pid>/fd/ symlinks.
fn read_proc_net_tcp() -> Vec<ParsedSocket> {
    let inode_to_pid = build_inode_pid_map();
    let mut sockets = Vec::new();

    for (path, proto) in &[("/proc/net/tcp", "tcp"), ("/proc/net/tcp6", "tcp6")] {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines().skip(1) {
                if let Some(s) = parse_proc_net_line(line, proto, &inode_to_pid) {
                    sockets.push(s);
                }
            }
        }
    }

    sockets
}

/// Parse a single line from /proc/net/tcp.
///
/// Format: `sl local_address rem_address st tx_queue:rx_queue tr:tm->when retrnsmt uid timeout inode`
fn parse_proc_net_line(
    line: &str,
    protocol: &str,
    inode_to_pid: &std::collections::HashMap<u64, (u32, String)>,
) -> Option<ParsedSocket> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 10 {
        return None;
    }

    let (local_addr, local_port) = parse_hex_addr(fields[1])?;
    let (remote_addr, remote_port) = parse_hex_addr(fields[2])?;
    let state_hex = u8::from_str_radix(fields[3], 16).ok()?;
    let state = tcp_state_name(state_hex);
    let inode: u64 = fields[9].parse().ok()?;

    let (pid, name) = if inode > 0 {
        inode_to_pid
            .get(&inode)
            .map(|(p, n)| (Some(*p), Some(n.clone())))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };

    Some(ParsedSocket {
        pid,
        name,
        protocol: protocol.to_string(),
        local_addr,
        local_port,
        remote_addr: if remote_port == 0 && remote_addr == "0.0.0.0" {
            None
        } else {
            Some(remote_addr)
        },
        remote_port: if remote_port == 0 {
            None
        } else {
            Some(remote_port)
        },
        state,
    })
}

/// Parse hex-encoded address:port from /proc/net/tcp (e.g. "0100007F:1F90").
fn parse_hex_addr(s: &str) -> Option<(String, u16)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let port = u16::from_str_radix(parts[1], 16).ok()?;

    let addr_hex = parts[0];
    let addr = if addr_hex.len() == 8 {
        // IPv4: stored as little-endian 32-bit
        let n = u32::from_str_radix(addr_hex, 16).ok()?;
        format!(
            "{}.{}.{}.{}",
            n & 0xFF,
            (n >> 8) & 0xFF,
            (n >> 16) & 0xFF,
            (n >> 24) & 0xFF,
        )
    } else if addr_hex.len() == 32 {
        // IPv6: stored as four little-endian 32-bit words
        // For display, just use :: notation for common cases
        if addr_hex == "00000000000000000000000000000000" {
            "::".to_string()
        } else if addr_hex.starts_with("0000000000000000FFFF0000") {
            // IPv4-mapped IPv6
            let v4_hex = &addr_hex[24..32];
            let n = u32::from_str_radix(v4_hex, 16).ok()?;
            format!(
                "::ffff:{}.{}.{}.{}",
                n & 0xFF,
                (n >> 8) & 0xFF,
                (n >> 16) & 0xFF,
                (n >> 24) & 0xFF,
            )
        } else {
            // Full IPv6 — just show hex
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                &addr_hex[0..4],
                &addr_hex[4..8],
                &addr_hex[8..12],
                &addr_hex[12..16],
                &addr_hex[16..20],
                &addr_hex[20..24],
                &addr_hex[24..28],
                &addr_hex[28..32]
            )
        }
    } else {
        addr_hex.to_string()
    };

    Some((addr, port))
}

/// Map TCP state number to name.
fn tcp_state_name(state: u8) -> String {
    match state {
        0x01 => "ESTABLISHED",
        0x02 => "SYN_SENT",
        0x03 => "SYN_RECV",
        0x04 => "FIN_WAIT1",
        0x05 => "FIN_WAIT2",
        0x06 => "TIME_WAIT",
        0x07 => "CLOSE",
        0x08 => "CLOSE_WAIT",
        0x09 => "LAST_ACK",
        0x0A => "LISTEN",
        0x0B => "CLOSING",
        _ => "UNKNOWN",
    }
    .to_string()
}

/// Build a mapping from socket inode → (PID, process name).
///
/// Scans /proc/<pid>/fd/ for each process to find socket symlinks.
fn build_inode_pid_map() -> std::collections::HashMap<u64, (u32, String)> {
    let mut map = std::collections::HashMap::new();

    let proc_dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return map,
    };

    for entry in proc_dir.flatten() {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        let pid: u32 = match fname_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Read process name from /proc/<pid>/comm
        let comm = std::fs::read_to_string(format!("/proc/{}/comm", pid))
            .unwrap_or_default()
            .trim()
            .to_string();

        // Scan file descriptors for socket inodes
        let fd_dir = format!("/proc/{}/fd", pid);
        if let Ok(fds) = std::fs::read_dir(&fd_dir) {
            for fd_entry in fds.flatten() {
                if let Ok(link) = std::fs::read_link(fd_entry.path()) {
                    let link_str = link.to_string_lossy();
                    // Socket symlinks look like "socket:[12345]"
                    if let Some(inode_str) = link_str
                        .strip_prefix("socket:[")
                        .and_then(|s| s.strip_suffix(']'))
                    {
                        if let Ok(inode) = inode_str.parse::<u64>() {
                            map.insert(inode, (pid, comm.clone()));
                        }
                    }
                }
            }
        }
    }

    map
}

// ── Helpers ───────────────────────────────────────────────────────

/// Current time as Unix epoch milliseconds (public for diagnostics).
#[allow(dead_code)]
pub fn now_epoch_ms_pub() -> i64 {
    now_epoch_ms()
}

/// Current time as Unix epoch milliseconds.
fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Format a duration in milliseconds as a human-readable age string.
#[allow(dead_code)]
fn format_age_ms(ms: i64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProcessInfo, ProcessStatus};

    fn make_process(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: name.to_string(),
            cpu_usage: cpu,
            memory_bytes: mem,
            memory_percent: 0.0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: ProcessStatus::Running,
            user: "test".to_string(),
            start_time: 0,
            parent_pid: None,
            thread_count: None,
        }
    }

    fn make_system() -> SystemSnapshot {
        crate::models::SystemSnapshot {
            total_memory: 16 * 1024 * 1024 * 1024,
            used_memory: 8 * 1024 * 1024 * 1024,
            total_swap: 4 * 1024 * 1024 * 1024,
            used_swap: 1024 * 1024 * 1024,
            cpu_count: 8,
            cpu_usages: vec![50.0; 8],
            global_cpu_usage: 50.0,
            uptime: 3600,
            hostname: "test".to_string(),
            os_name: "Linux".to_string(),
            load_avg_1: 2.0,
            load_avg_5: 1.5,
            load_avg_15: 1.0,
            total_processes: 200,
            networks: vec![],
            disks: vec![],
            cpu_temp: None,
            gpu: None,
            battery: None,
        }
    }

    // ── Schema ────────────────────────────────────────────────────

    #[test]
    fn open_in_memory() {
        let store = EventStore::open(None);
        assert!(store.is_ok());
    }

    #[test]
    fn table_stats_empty() {
        let store = EventStore::open(None).unwrap();
        let stats = store.table_stats().unwrap();
        assert_eq!(stats.len(), 4);
        for (_, count) in &stats {
            assert_eq!(*count, 0);
        }
    }

    // ── System snapshots ──────────────────────────────────────────

    #[test]
    fn insert_and_query_system_snapshot() {
        let mut store = EventStore::open(None).unwrap();
        let sys = make_system();
        store.insert_system_snapshot(&sys).unwrap();

        let rows = store.query_system_history(0).unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].cpu_global - 50.0).abs() < 0.1);
        assert_eq!(rows[0].mem_total, 16 * 1024 * 1024 * 1024);
    }

    #[test]
    fn system_snapshot_time_filter() {
        let mut store = EventStore::open(None).unwrap();
        let sys = make_system();
        store.insert_system_snapshot(&sys).unwrap();

        // Query with future timestamp should return nothing
        let far_future = now_epoch_ms() + 100_000;
        let rows = store.query_system_history(far_future).unwrap();
        assert_eq!(rows.len(), 0);
    }

    // ── Process snapshots ─────────────────────────────────────────

    #[test]
    fn insert_and_query_process_snapshots() {
        let mut store = EventStore::open(None).unwrap();
        let procs = vec![
            make_process(1, "high_cpu", 90.0, 1024),
            make_process(2, "low_cpu", 1.0, 2048),
        ];
        store.insert_process_snapshots(&procs).unwrap();

        let rows = store.query_process_history(1, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "high_cpu");
    }

    #[test]
    fn process_snapshots_deduplication() {
        let mut store = EventStore::open(None).unwrap();
        // A process that's both top CPU and top memory should only appear once
        let procs = vec![make_process(1, "hog", 99.0, 999999999)];
        store.insert_process_snapshots(&procs).unwrap();

        let stats = store.table_stats().unwrap();
        let proc_count = stats
            .iter()
            .find(|(t, _)| t == "process_snapshots")
            .unwrap()
            .1;
        assert_eq!(proc_count, 1); // Not 2
    }

    // ── Events ────────────────────────────────────────────────────

    #[test]
    fn insert_and_query_events() {
        let store = EventStore::open(None).unwrap();
        store
            .insert_event(
                EventKind::ProcessStart,
                Some(42),
                Some("nginx"),
                None,
                Some("info"),
            )
            .unwrap();
        store
            .insert_event(
                EventKind::Alert,
                None,
                None,
                Some("CPU spike"),
                Some("warn"),
            )
            .unwrap();

        let events = store.query_events_since(0).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn query_events_by_kind() {
        let store = EventStore::open(None).unwrap();
        store
            .insert_event(EventKind::ProcessStart, Some(1), Some("a"), None, None)
            .unwrap();
        store
            .insert_event(EventKind::ProcessExit, Some(2), Some("b"), None, None)
            .unwrap();
        store
            .insert_event(EventKind::ProcessStart, Some(3), Some("c"), None, None)
            .unwrap();

        let starts = store
            .query_events_by_kind(EventKind::ProcessStart, 0)
            .unwrap();
        assert_eq!(starts.len(), 2);

        let exits = store
            .query_events_by_kind(EventKind::ProcessExit, 0)
            .unwrap();
        assert_eq!(exits.len(), 1);
    }

    // ── Process lifecycle detection ───────────────────────────────

    #[test]
    fn detect_process_start_exit() {
        let mut store = EventStore::open(None).unwrap();

        // First tick: establish baseline
        let procs1 = vec![
            make_process(1, "init", 0.0, 0),
            make_process(2, "bash", 0.0, 0),
        ];
        store.detect_process_lifecycle(&procs1).unwrap();

        // No events on first tick (no previous state)
        let events = store.query_events_since(0).unwrap();
        assert_eq!(events.len(), 0);

        // Second tick: PID 2 gone, PID 3 appeared
        let procs2 = vec![
            make_process(1, "init", 0.0, 0),
            make_process(3, "vim", 0.0, 0),
        ];
        store.detect_process_lifecycle(&procs2).unwrap();

        let events = store.query_events_since(0).unwrap();
        assert_eq!(events.len(), 2);

        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains(&"process_start"));
        assert!(kinds.contains(&"process_exit"));
    }

    // ── Retention / cleanup ───────────────────────────────────────

    #[test]
    fn cleanup_removes_old_data() {
        let mut store = EventStore::open(None).unwrap();
        store.set_retention(0); // Expire everything immediately

        let sys = make_system();
        store.insert_system_snapshot(&sys).unwrap();
        store
            .insert_event(EventKind::Alert, None, None, None, None)
            .unwrap();

        // Wait a tiny bit so the data is "old"
        std::thread::sleep(std::time::Duration::from_millis(10));
        store.cleanup().unwrap();

        let rows = store.query_system_history(0).unwrap();
        assert_eq!(rows.len(), 0);

        let events = store.query_events_since(0).unwrap();
        assert_eq!(events.len(), 0);
    }

    // ── Event counts ──────────────────────────────────────────────

    #[test]
    fn event_counts_by_kind() {
        let store = EventStore::open(None).unwrap();
        store
            .insert_event(EventKind::ProcessStart, Some(1), None, None, None)
            .unwrap();
        store
            .insert_event(EventKind::ProcessStart, Some(2), None, None, None)
            .unwrap();
        store
            .insert_event(EventKind::ProcessExit, Some(3), None, None, None)
            .unwrap();

        let counts = store.event_counts(60).unwrap();
        assert_eq!(counts.get("process_start"), Some(&2));
        assert_eq!(counts.get("process_exit"), Some(&1));
    }

    // ── EventKind ─────────────────────────────────────────────────

    #[test]
    fn event_kind_roundtrip() {
        let kinds = [
            EventKind::ProcessStart,
            EventKind::ProcessExit,
            EventKind::PortBind,
            EventKind::PortRelease,
            EventKind::Alert,
            EventKind::CpuSpike,
            EventKind::MemorySpike,
            EventKind::OomKill,
        ];
        for kind in &kinds {
            let s = kind.to_string();
            let parsed = EventKind::from_str(&s);
            assert_eq!(parsed, Some(*kind), "Roundtrip failed for {:?}", kind);
        }
    }

    #[test]
    fn event_kind_from_str_unknown() {
        assert_eq!(EventKind::from_str("bogus"), None);
    }

    // ── recent_events_summary ─────────────────────────────────────

    #[test]
    fn recent_events_summary_format() {
        let store = EventStore::open(None).unwrap();
        store
            .insert_event(
                EventKind::ProcessStart,
                Some(42),
                Some("nginx"),
                None,
                Some("info"),
            )
            .unwrap();

        let summary = store.recent_events_summary(10).unwrap();
        assert!(summary.contains("process_start"));
        assert!(summary.contains("PID 42"));
        assert!(summary.contains("nginx"));
    }

    // ── Hex address parser ────────────────────────────────────────

    #[test]
    fn parse_hex_addr_ipv4() {
        // 127.0.0.1:8080 = 0100007F:1F90
        let (addr, port) = parse_hex_addr("0100007F:1F90").unwrap();
        assert_eq!(addr, "127.0.0.1");
        assert_eq!(port, 8080);
    }

    #[test]
    fn parse_hex_addr_ipv4_any() {
        // 0.0.0.0:80 = 00000000:0050
        let (addr, port) = parse_hex_addr("00000000:0050").unwrap();
        assert_eq!(addr, "0.0.0.0");
        assert_eq!(port, 80);
    }

    // ── TCP state names ───────────────────────────────────────────

    #[test]
    fn tcp_state_names() {
        assert_eq!(tcp_state_name(0x0A), "LISTEN");
        assert_eq!(tcp_state_name(0x01), "ESTABLISHED");
        assert_eq!(tcp_state_name(0x06), "TIME_WAIT");
        assert_eq!(tcp_state_name(0xFF), "UNKNOWN");
    }

    // ── format_age_ms ─────────────────────────────────────────────

    #[test]
    fn format_age_seconds() {
        assert_eq!(format_age_ms(5000), "5s ago");
        assert_eq!(format_age_ms(59000), "59s ago");
    }

    #[test]
    fn format_age_minutes() {
        assert_eq!(format_age_ms(60000), "1m ago");
        assert_eq!(format_age_ms(3599000), "59m ago");
    }

    #[test]
    fn format_age_hours() {
        assert_eq!(format_age_ms(3600000), "1h ago");
        assert_eq!(format_age_ms(7200000), "2h ago");
    }
}
