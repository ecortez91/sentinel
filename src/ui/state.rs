use std::collections::VecDeque;

use crate::ai::Conversation;
use crate::models::{Alert, ProcessInfo, SystemSnapshot};
use crate::monitor::ContainerInfo;

use super::theme::Theme;

/// Common Unix signals for the signal picker popup.
pub const SIGNAL_LIST: &[(i32, &str, &str)] = &[
    (1, "SIGHUP", "Hangup / reload config"),
    (2, "SIGINT", "Interrupt (Ctrl+C)"),
    (3, "SIGQUIT", "Quit with core dump"),
    (9, "SIGKILL", "Force kill (unblockable)"),
    (10, "SIGUSR1", "User-defined signal 1"),
    (12, "SIGUSR2", "User-defined signal 2"),
    (15, "SIGTERM", "Graceful termination"),
    (17, "SIGCHLD", "Child process stopped"),
    (18, "SIGCONT", "Continue if stopped"),
    (19, "SIGSTOP", "Stop (unblockable)"),
    (20, "SIGTSTP", "Terminal stop (Ctrl+Z)"),
    (28, "SIGWINCH", "Window size change"),
];

/// Time window for history charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryWindow {
    OneMin,
    FiveMin,
    FifteenMin,
    OneHour,
}

impl HistoryWindow {
    pub fn label(&self) -> String {
        match self {
            HistoryWindow::OneMin => t!("history.1min").to_string(),
            HistoryWindow::FiveMin => t!("history.5min").to_string(),
            HistoryWindow::FifteenMin => t!("history.15min").to_string(),
            HistoryWindow::OneHour => t!("history.1hr").to_string(),
        }
    }

    /// Number of data points to show (at 1 sample/sec).
    pub fn points(&self) -> usize {
        match self {
            HistoryWindow::OneMin => 60,
            HistoryWindow::FiveMin => 300,
            HistoryWindow::FifteenMin => 900,
            HistoryWindow::OneHour => 3600,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            HistoryWindow::OneMin => HistoryWindow::FiveMin,
            HistoryWindow::FiveMin => HistoryWindow::FifteenMin,
            HistoryWindow::FifteenMin => HistoryWindow::OneHour,
            HistoryWindow::OneHour => HistoryWindow::OneMin,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            HistoryWindow::OneMin => HistoryWindow::OneHour,
            HistoryWindow::FiveMin => HistoryWindow::OneMin,
            HistoryWindow::FifteenMin => HistoryWindow::FiveMin,
            HistoryWindow::OneHour => HistoryWindow::FifteenMin,
        }
    }
}

/// Which dashboard section is focused/expanded (if any).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FocusedWidget {
    SystemGauges,
    CpuCores,
    Sparklines,
    Gpu,
    Network,
    Disk,
    AiInsight,
    TopProcesses,
    Alerts,
}

/// Extended process details fetched from /proc on demand.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessDetail {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub memory_percent: f32,
    pub status: String,
    pub user: String,
    pub parent_pid: Option<u32>,
    pub thread_count: Option<u32>,
    pub start_time: u64,
    pub open_fds: usize,
    pub fd_sample: Vec<String>, // First N file descriptors
    pub environ: Vec<String>,   // Environment variables
}

/// Which tab is currently active in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Processes,
    Alerts,
    AskAi,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[Tab::Dashboard, Tab::Processes, Tab::Alerts, Tab::AskAi]
    }

    pub fn label(&self) -> String {
        match self {
            Tab::Dashboard => t!("tab.dashboard").to_string(),
            Tab::Processes => t!("tab.processes").to_string(),
            Tab::Alerts => t!("tab.alerts").to_string(),
            Tab::AskAi => t!("tab.ask_ai").to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn index(&self) -> usize {
        match self {
            Tab::Dashboard => 0,
            Tab::Processes => 1,
            Tab::Alerts => 2,
            Tab::AskAi => 3,
        }
    }
}

/// How to sort the process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    Name,
    Cpu,
    Memory,
    DiskIo,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// Central application state - the single source of truth.
#[allow(dead_code)]
pub struct AppState {
    pub active_tab: Tab,
    pub system: Option<SystemSnapshot>,
    pub processes: Vec<ProcessInfo>,
    pub alerts: Vec<Alert>,
    pub sort_column: SortColumn,
    pub sort_direction: SortDirection,
    pub process_scroll: usize,
    pub alert_scroll: usize,
    pub selected_process: usize,
    pub filter_text: String,
    pub show_help: bool,
    pub tick_count: u64,
    pub max_alerts: usize,

    // ── Status message (shown in status bar) ───────────────────
    pub status_message: Option<(String, std::time::Instant)>,

    // ── Process detail popup ────────────────────────────────────
    pub show_process_detail: bool,
    pub process_detail: Option<ProcessDetail>,
    pub detail_scroll: usize,

    // ── Process tree view ────────────────────────────────────────
    pub tree_view: bool,

    // ── History ring buffers for sparklines (3600 points = 1 hr at 1s tick) ──
    pub cpu_history: VecDeque<u64>,
    pub mem_history: VecDeque<u64>,

    // ── AI Chat State ──────────────────────────────────────────
    pub ai_input: String,
    pub ai_conversation: Conversation,
    pub ai_loading: bool,
    pub ai_scroll: usize,
    pub ai_has_key: bool,
    pub ai_cursor_pos: usize,
    pub ai_auth_method: String,

    // ── AI Auto-Analysis (Dashboard insight card) ────────────
    pub ai_insight: Option<String>,
    pub ai_insight_loading: bool,
    pub ai_insight_updated: Option<std::time::Instant>,
    pub ai_insight_scroll: usize,
    pub ai_insight_expanded: bool,

    // ── Theme ────────────────────────────────────────────────
    pub theme: Theme,

    // ── Signal picker popup ──────────────────────────────────
    pub show_signal_picker: bool,
    pub signal_picker_selected: usize,
    pub signal_picker_pid: Option<u32>,
    pub signal_picker_name: String,

    // ── Renice dialog ────────────────────────────────────────
    pub show_renice_dialog: bool,
    pub renice_value: i32,
    pub renice_pid: Option<u32>,
    pub renice_name: String,

    // ── Zoomable history ─────────────────────────────────────
    pub history_window: HistoryWindow,

    // ── Widget focus/expand ──────────────────────────────────
    pub focused_widget: Option<FocusedWidget>,

    // ── Docker containers ────────────────────────────────────
    pub docker_available: bool,
    pub containers: Vec<ContainerInfo>,
    pub container_scroll: usize,

    // ── Language ────────────────────────────────────────────
    pub current_lang: String,
}

impl AppState {
    pub fn new(max_alerts: usize, has_api_key: bool, theme: Theme) -> Self {
        Self {
            active_tab: Tab::Dashboard,
            system: None,
            processes: Vec::new(),
            alerts: Vec::new(),
            sort_column: SortColumn::Cpu,
            sort_direction: SortDirection::Desc,
            process_scroll: 0,
            alert_scroll: 0,
            selected_process: 0,
            filter_text: String::new(),
            show_help: false,
            tick_count: 0,
            max_alerts,
            status_message: None,
            show_process_detail: false,
            process_detail: None,
            detail_scroll: 0,
            tree_view: false,
            cpu_history: VecDeque::with_capacity(3600),
            mem_history: VecDeque::with_capacity(3600),
            // AI
            ai_input: String::new(),
            ai_conversation: Conversation::new(50),
            ai_loading: false,
            ai_scroll: 0,
            ai_has_key: has_api_key,
            ai_cursor_pos: 0,
            ai_auth_method: String::new(),
            ai_insight: None,
            ai_insight_loading: false,
            ai_insight_updated: None,
            ai_insight_scroll: 0,
            ai_insight_expanded: false,
            // Theme
            theme,
            // Signal picker
            show_signal_picker: false,
            signal_picker_selected: 6, // Default to SIGTERM (index 6)
            signal_picker_pid: None,
            signal_picker_name: String::new(),
            // Renice dialog
            show_renice_dialog: false,
            renice_value: 0,
            renice_pid: None,
            renice_name: String::new(),
            // History
            history_window: HistoryWindow::FiveMin,
            // Focus
            focused_widget: None,
            // Docker
            docker_available: false,
            containers: Vec::new(),
            container_scroll: 0,
            // Language
            current_lang: rust_i18n::locale().to_string(),
        }
    }

    /// Cycle to the next built-in theme.
    pub fn cycle_theme(&mut self) {
        self.theme = self.theme.next_builtin();
    }

    /// Available UI languages.
    const LANGUAGES: &[&str] = &["en", "ja", "es", "de", "zh"];

    /// Cycle to the next UI language.
    pub fn cycle_lang(&mut self) {
        let current_idx = Self::LANGUAGES
            .iter()
            .position(|&l| l == self.current_lang)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % Self::LANGUAGES.len();
        let next_lang = Self::LANGUAGES[next_idx];
        rust_i18n::set_locale(next_lang);
        self.current_lang = next_lang.to_string();
    }

    /// Open the signal picker for the currently selected process.
    pub fn open_signal_picker(&mut self) {
        let info: Option<(u32, String)> = {
            let filtered = self.filtered_processes();
            filtered
                .get(self.selected_process)
                .map(|p| (p.pid, p.name.clone()))
        };
        if let Some((pid, name)) = info {
            self.signal_picker_pid = Some(pid);
            self.signal_picker_name = name;
            self.signal_picker_selected = 6; // SIGTERM
            self.show_signal_picker = true;
        }
    }

    /// Close the signal picker.
    pub fn close_signal_picker(&mut self) {
        self.show_signal_picker = false;
        self.signal_picker_pid = None;
    }

    /// Open the renice dialog for the currently selected process.
    pub fn open_renice_dialog(&mut self) {
        let info: Option<(u32, String)> = {
            let filtered = self.filtered_processes();
            filtered
                .get(self.selected_process)
                .map(|p| (p.pid, p.name.clone()))
        };
        if let Some((pid, name)) = info {
            self.renice_pid = Some(pid);
            self.renice_name = name;
            self.renice_value = 0;
            self.show_renice_dialog = true;
        }
    }

    /// Close the renice dialog.
    pub fn close_renice_dialog(&mut self) {
        self.show_renice_dialog = false;
        self.renice_pid = None;
    }

    /// Toggle focus/expand on a dashboard widget (or unfocus).
    pub fn toggle_focus(&mut self) {
        if self.focused_widget.is_some() {
            self.focused_widget = None;
        } else if self.active_tab == Tab::Dashboard {
            // Cycle through focusable widgets based on what's visible
            self.focused_widget = Some(FocusedWidget::TopProcesses);
        }
    }

    /// Cycle the focused widget forward.
    pub fn cycle_focus_forward(&mut self) {
        self.focused_widget = Some(match self.focused_widget {
            Some(FocusedWidget::SystemGauges) => FocusedWidget::CpuCores,
            Some(FocusedWidget::CpuCores) => FocusedWidget::Sparklines,
            Some(FocusedWidget::Sparklines) => FocusedWidget::Network,
            Some(FocusedWidget::Gpu) => FocusedWidget::Network,
            Some(FocusedWidget::Network) => FocusedWidget::Disk,
            Some(FocusedWidget::Disk) => FocusedWidget::TopProcesses,
            Some(FocusedWidget::AiInsight) => FocusedWidget::TopProcesses,
            Some(FocusedWidget::TopProcesses) => FocusedWidget::Alerts,
            Some(FocusedWidget::Alerts) => FocusedWidget::SystemGauges,
            None => FocusedWidget::TopProcesses,
        });
    }

    pub fn update(
        &mut self,
        system: SystemSnapshot,
        mut processes: Vec<ProcessInfo>,
        new_alerts: Vec<Alert>,
    ) {
        // Push history for sparklines (keep up to 3600 samples = 1hr)
        if self.cpu_history.len() >= 3600 {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(system.global_cpu_usage as u64);

        if self.mem_history.len() >= 3600 {
            self.mem_history.pop_front();
        }
        self.mem_history.push_back(system.memory_percent() as u64);

        self.system = Some(system);

        // Sort processes
        self.sort_processes(&mut processes);
        self.processes = processes;

        // Append new alerts, trim old ones
        for alert in new_alerts {
            self.alerts.insert(0, alert);
        }
        self.alerts.truncate(self.max_alerts);

        self.tick_count += 1;
    }

    fn sort_processes(&self, procs: &mut Vec<ProcessInfo>) {
        let dir = self.sort_direction;
        match self.sort_column {
            SortColumn::Pid => procs.sort_by(|a, b| {
                let cmp = a.pid.cmp(&b.pid);
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
            SortColumn::Name => procs.sort_by(|a, b| {
                let cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
            SortColumn::Cpu => procs.sort_by(|a, b| {
                let cmp = a
                    .cpu_usage
                    .partial_cmp(&b.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal);
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
            SortColumn::Memory => procs.sort_by(|a, b| {
                let cmp = a.memory_bytes.cmp(&b.memory_bytes);
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
            SortColumn::DiskIo => procs.sort_by(|a, b| {
                let total_a = a.disk_read_bytes + a.disk_write_bytes;
                let total_b = b.disk_read_bytes + b.disk_write_bytes;
                let cmp = total_a.cmp(&total_b);
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
            SortColumn::Status => procs.sort_by(|a, b| {
                let cmp = a.status.to_string().cmp(&b.status.to_string());
                if dir == SortDirection::Desc {
                    cmp.reverse()
                } else {
                    cmp
                }
            }),
        }
    }

    pub fn next_tab(&mut self) {
        self.active_tab = match self.active_tab {
            Tab::Dashboard => Tab::Processes,
            Tab::Processes => Tab::Alerts,
            Tab::Alerts => Tab::AskAi,
            Tab::AskAi => Tab::Dashboard,
        };
    }

    pub fn prev_tab(&mut self) {
        self.active_tab = match self.active_tab {
            Tab::Dashboard => Tab::AskAi,
            Tab::Processes => Tab::Dashboard,
            Tab::Alerts => Tab::Processes,
            Tab::AskAi => Tab::Alerts,
        };
    }

    pub fn cycle_sort(&mut self) {
        self.sort_column = match self.sort_column {
            SortColumn::Pid => SortColumn::Name,
            SortColumn::Name => SortColumn::Cpu,
            SortColumn::Cpu => SortColumn::Memory,
            SortColumn::Memory => SortColumn::DiskIo,
            SortColumn::DiskIo => SortColumn::Status,
            SortColumn::Status => SortColumn::Pid,
        };
    }

    pub fn toggle_sort_direction(&mut self) {
        self.sort_direction = match self.sort_direction {
            SortDirection::Asc => SortDirection::Desc,
            SortDirection::Desc => SortDirection::Asc,
        };
    }

    pub fn scroll_up(&mut self) {
        match self.active_tab {
            Tab::Dashboard => {
                if self.ai_insight_scroll > 0 {
                    self.ai_insight_scroll -= 1;
                }
            }
            Tab::Processes => {
                if self.selected_process > 0 {
                    self.selected_process -= 1;
                }
            }
            Tab::Alerts => {
                if self.alert_scroll > 0 {
                    self.alert_scroll -= 1;
                }
            }
            Tab::AskAi => {
                if self.ai_scroll > 0 {
                    self.ai_scroll -= 1;
                }
            }
        }
    }

    pub fn scroll_down(&mut self) {
        match self.active_tab {
            Tab::Dashboard => {
                self.ai_insight_scroll += 1;
            }
            Tab::Processes => {
                let filtered = self.filtered_processes();
                if self.selected_process < filtered.len().saturating_sub(1) {
                    self.selected_process += 1;
                }
            }
            Tab::Alerts => {
                if self.alert_scroll < self.alerts.len().saturating_sub(1) {
                    self.alert_scroll += 1;
                }
            }
            Tab::AskAi => {
                self.ai_scroll += 1;
            }
        }
    }

    pub fn page_up(&mut self) {
        match self.active_tab {
            Tab::Processes => {
                self.selected_process = self.selected_process.saturating_sub(20);
            }
            Tab::Alerts => {
                self.alert_scroll = self.alert_scroll.saturating_sub(20);
            }
            Tab::AskAi => {
                self.ai_scroll = self.ai_scroll.saturating_sub(20);
            }
            _ => {}
        }
    }

    pub fn page_down(&mut self) {
        match self.active_tab {
            Tab::Processes => {
                let max = self.filtered_processes().len().saturating_sub(1);
                self.selected_process = (self.selected_process + 20).min(max);
            }
            Tab::Alerts => {
                let max = self.alerts.len().saturating_sub(1);
                self.alert_scroll = (self.alert_scroll + 20).min(max);
            }
            Tab::AskAi => {
                self.ai_scroll += 20;
            }
            _ => {}
        }
    }

    pub fn filtered_processes(&self) -> Vec<&ProcessInfo> {
        if self.filter_text.is_empty() {
            self.processes.iter().collect()
        } else {
            let filter = self.filter_text.to_lowercase();
            self.processes
                .iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&filter)
                        || p.cmd.to_lowercase().contains(&filter)
                        || p.pid.to_string().contains(&filter)
                })
                .collect()
        }
    }

    /// Populate the process detail popup from a selected process.
    /// Reads /proc/<pid>/fd and /proc/<pid>/environ for extra info.
    pub fn open_process_detail(&mut self, proc_info: &ProcessInfo) {
        let pid = proc_info.pid;

        // Read open file descriptors from /proc/<pid>/fd
        let fd_path = format!("/proc/{}/fd", pid);
        let (open_fds, fd_sample) = match std::fs::read_dir(&fd_path) {
            Ok(entries) => {
                let mut fds: Vec<String> = Vec::new();
                let mut count = 0usize;
                for entry in entries.flatten() {
                    count += 1;
                    if fds.len() < 20 {
                        // Resolve symlink to see what the fd points to
                        if let Ok(target) = std::fs::read_link(entry.path()) {
                            fds.push(format!(
                                "fd/{} -> {}",
                                entry.file_name().to_string_lossy(),
                                target.display()
                            ));
                        } else {
                            fds.push(format!("fd/{}", entry.file_name().to_string_lossy()));
                        }
                    }
                }
                (count, fds)
            }
            Err(_) => (0, vec!["(permission denied)".to_string()]),
        };

        // Read environment variables from /proc/<pid>/environ
        let env_path = format!("/proc/{}/environ", pid);
        let environ = match std::fs::read(&env_path) {
            Ok(data) => {
                // environ is NUL-separated
                let raw = String::from_utf8_lossy(&data);
                let mut vars: Vec<String> = raw
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                vars.sort();
                vars.truncate(50); // Limit to 50 env vars
                vars
            }
            Err(_) => vec!["(permission denied)".to_string()],
        };

        self.process_detail = Some(ProcessDetail {
            pid,
            name: proc_info.name.clone(),
            cmd: proc_info.cmd.clone(),
            cpu_usage: proc_info.cpu_usage,
            memory_bytes: proc_info.memory_bytes,
            memory_percent: proc_info.memory_percent,
            status: proc_info.status.to_string(),
            user: proc_info.user.clone(),
            parent_pid: proc_info.parent_pid,
            thread_count: proc_info.thread_count,
            start_time: proc_info.start_time,
            open_fds,
            fd_sample,
            environ,
        });
        self.show_process_detail = true;
        self.detail_scroll = 0;
    }

    pub fn close_process_detail(&mut self) {
        self.show_process_detail = false;
        self.process_detail = None;
        self.detail_scroll = 0;
    }

    /// Build a flattened process tree for display.
    /// Returns Vec<(indent_prefix, &ProcessInfo)> in tree-walk order.
    pub fn tree_processes(&self) -> Vec<(String, &ProcessInfo)> {
        use std::collections::HashMap;

        let processes = if self.filter_text.is_empty() {
            self.processes.iter().collect::<Vec<_>>()
        } else {
            self.filtered_processes().into_iter().collect()
        };

        // Build children map: parent_pid -> [children]
        let mut children_map: HashMap<u32, Vec<&ProcessInfo>> = HashMap::new();
        let pid_set: std::collections::HashSet<u32> = processes.iter().map(|p| p.pid).collect();

        for p in &processes {
            let parent = p.parent_pid.unwrap_or(0);
            children_map.entry(parent).or_default().push(p);
        }

        // Sort children within each parent by CPU desc
        for children in children_map.values_mut() {
            children.sort_by(|a, b| {
                b.cpu_usage
                    .partial_cmp(&a.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Find root processes: those whose parent is not in our pid set
        let mut roots: Vec<&ProcessInfo> = processes
            .iter()
            .filter(|p| {
                p.parent_pid
                    .map(|pp| !pid_set.contains(&pp))
                    .unwrap_or(true)
            })
            .copied()
            .collect();
        roots.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut result: Vec<(String, &ProcessInfo)> = Vec::with_capacity(processes.len());

        fn walk<'a>(
            pid: u32,
            prefix: &str,
            _is_last: bool,
            children_map: &HashMap<u32, Vec<&'a ProcessInfo>>,
            result: &mut Vec<(String, &'a ProcessInfo)>,
            depth: usize,
        ) {
            if depth > 20 {
                return; // Guard against cycles
            }
            if let Some(children) = children_map.get(&pid) {
                let count = children.len();
                for (i, child) in children.iter().enumerate() {
                    let is_last_child = i == count - 1;
                    let connector = if depth == 0 {
                        String::new()
                    } else if is_last_child {
                        format!("{}└── ", prefix)
                    } else {
                        format!("{}├── ", prefix)
                    };

                    result.push((connector, child));

                    let new_prefix = if depth == 0 {
                        String::new()
                    } else if is_last_child {
                        format!("{}    ", prefix)
                    } else {
                        format!("{}│   ", prefix)
                    };

                    walk(
                        child.pid,
                        &new_prefix,
                        is_last_child,
                        children_map,
                        result,
                        depth + 1,
                    );
                }
            }
        }

        // Walk from each root
        for (i, root) in roots.iter().enumerate() {
            result.push((String::new(), root));
            let prefix = String::new();
            walk(
                root.pid,
                &prefix,
                i == roots.len() - 1,
                &children_map,
                &mut result,
                1,
            );
        }

        result
    }

    pub fn danger_alert_count(&self) -> usize {
        self.alerts
            .iter()
            .filter(|a| {
                a.severity == crate::models::AlertSeverity::Danger
                    || a.severity == crate::models::AlertSeverity::Critical
            })
            .count()
    }

    // ── AI helpers ─────────────────────────────────────────────

    /// Count total rendered lines in the conversation for scrolling.
    #[allow(dead_code)]
    pub fn ai_total_lines(&self, width: usize) -> usize {
        let wrap_width = if width > 6 { width - 6 } else { width };
        let mut total = 0;
        for msg in &self.ai_conversation.messages {
            // Header line (role)
            total += 1;
            // Content lines (wrapped)
            let lines = textwrap::wrap(&msg.content, wrap_width);
            total += lines.len().max(1);
            // Spacing
            total += 1;
        }
        total
    }

    /// Auto-scroll to bottom of AI chat when new content arrives.
    #[allow(dead_code)]
    pub fn ai_scroll_to_bottom(&mut self, visible_height: usize, content_width: usize) {
        let total = self.ai_total_lines(content_width);
        if total > visible_height {
            self.ai_scroll = total - visible_height;
        } else {
            self.ai_scroll = 0;
        }
    }

    pub fn ai_input_char(&mut self, c: char) {
        self.ai_input.insert(self.ai_cursor_pos, c);
        self.ai_cursor_pos += c.len_utf8();
    }

    pub fn ai_input_backspace(&mut self) {
        if self.ai_cursor_pos > 0 {
            // Find the previous char boundary
            let prev = self.ai_input[..self.ai_cursor_pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.ai_input.remove(prev);
            self.ai_cursor_pos = prev;
        }
    }

    pub fn ai_cursor_left(&mut self) {
        if self.ai_cursor_pos > 0 {
            self.ai_cursor_pos = self.ai_input[..self.ai_cursor_pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn ai_cursor_right(&mut self) {
        if self.ai_cursor_pos < self.ai_input.len() {
            self.ai_cursor_pos = self.ai_input[self.ai_cursor_pos..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.ai_cursor_pos + i)
                .unwrap_or(self.ai_input.len());
        }
    }

    pub fn ai_submit(&mut self) -> Option<String> {
        let text = self.ai_input.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.ai_input.clear();
        self.ai_cursor_pos = 0;
        self.ai_conversation.add_user_message(&text);
        Some(text)
    }
}
