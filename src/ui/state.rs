use std::cmp::Ordering;
use std::collections::VecDeque;

use crate::ai::Conversation;
use crate::constants::*;
use crate::diagnostics::SuggestedAction;
use crate::models::{Alert, ProcessInfo, SystemSnapshot};
use crate::monitor::ContainerInfo;

use super::theme::Theme;

/// Result from a command palette execution — holds rendered text + any
/// executable actions extracted from the diagnostic report.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// The rendered text output (for display).
    pub text: String,
    /// Extracted actions from the diagnostic findings, with labels.
    pub actions: Vec<(String, SuggestedAction)>,
}

impl CommandResult {
    /// Create a plain text result with no actions.
    pub fn text_only(text: String) -> Self {
        Self {
            text,
            actions: Vec::new(),
        }
    }

    /// Create from a DiagnosticReport, extracting actions.
    pub fn from_report(report: &crate::diagnostics::DiagnosticReport) -> Self {
        let text = report.to_text();
        let actions: Vec<(String, SuggestedAction)> = report
            .findings
            .iter()
            .filter_map(|f| {
                f.action.as_ref().map(|a| {
                    let label = match a {
                        SuggestedAction::KillProcess { pid, name, signal } => {
                            format!("Kill PID {} ({}) with {}", pid, name, signal)
                        }
                        SuggestedAction::ReniceProcess { pid, name, nice } => {
                            format!("Set nice {} for PID {} ({})", nice, pid, name)
                        }
                        SuggestedAction::FreePort { port, pid, name } => {
                            format!("Kill PID {} ({}) to free port {}", pid, name, port)
                        }
                        SuggestedAction::CleanDirectory { path, size_bytes } => {
                            format!(
                                "Clean {} ({:.1} GB)",
                                path,
                                *size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                            )
                        }
                        SuggestedAction::Info(msg) => msg.clone(),
                    };
                    (label, a.clone())
                })
            })
            .collect();
        Self { text, actions }
    }

    /// Whether this result has executable (non-Info) actions.
    pub fn has_executable_actions(&self) -> bool {
        self.actions
            .iter()
            .any(|(_, a)| !matches!(a, SuggestedAction::Info(_)))
    }
}

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
#[allow(dead_code)] // Gpu and AiInsight are valid focus targets, handled in match arms
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
pub struct AppState {
    pub active_tab: Tab,
    pub system: Option<SystemSnapshot>,
    pub processes: Vec<ProcessInfo>,
    pub alerts: Vec<Alert>,
    pub sort_column: SortColumn,
    pub sort_direction: SortDirection,

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

    // ── Command palette ───────────────────────────────────
    pub show_command_palette: bool,
    pub command_input: String,
    pub command_cursor_pos: usize,
    /// Result from the last command execution.
    pub command_result: Option<CommandResult>,
    pub command_result_scroll: usize,
    /// Currently selected action index within command_result.actions.
    pub command_result_selected_action: usize,
    /// Whether we're showing a confirmation dialog for the selected action.
    pub show_action_confirm: bool,
}

/// Apply sort direction to an ordering.
fn apply_direction(cmp: Ordering, dir: SortDirection) -> Ordering {
    if dir == SortDirection::Desc {
        cmp.reverse()
    } else {
        cmp
    }
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
            cpu_history: VecDeque::with_capacity(HISTORY_CAPACITY),
            mem_history: VecDeque::with_capacity(HISTORY_CAPACITY),
            // AI
            ai_input: String::new(),
            ai_conversation: Conversation::new(MAX_CONVERSATION_HISTORY),
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
            signal_picker_selected: DEFAULT_SIGNAL_INDEX, // SIGTERM
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
            // Command palette
            show_command_palette: false,
            command_input: String::new(),
            command_cursor_pos: 0,
            command_result: None,
            command_result_scroll: 0,
            command_result_selected_action: 0,
            show_action_confirm: false,
        }
    }

    /// Cycle to the next built-in theme.
    pub fn cycle_theme(&mut self) {
        self.theme = self.theme.next_builtin();
    }

    /// Cycle to the next UI language.
    pub fn cycle_lang(&mut self) {
        let current_idx = LANGUAGES
            .iter()
            .position(|&l| l == self.current_lang)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % LANGUAGES.len();
        let next_lang = LANGUAGES[next_idx];
        rust_i18n::set_locale(next_lang);
        self.current_lang = next_lang.to_string();
    }

    /// Set a status bar message with automatic timestamp.
    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, std::time::Instant::now()));
    }

    /// Get the PID and name of the currently selected process.
    pub fn selected_process_info(&self) -> Option<(u32, String)> {
        let filtered = self.filtered_processes();
        filtered
            .get(self.selected_process)
            .map(|p| (p.pid, p.name.clone()))
    }

    /// Open the signal picker for the currently selected process.
    pub fn open_signal_picker(&mut self) {
        if let Some((pid, name)) = self.selected_process_info() {
            self.signal_picker_pid = Some(pid);
            self.signal_picker_name = name;
            self.signal_picker_selected = DEFAULT_SIGNAL_INDEX;
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
        if let Some((pid, name)) = self.selected_process_info() {
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
        // Push history for sparklines (keep up to HISTORY_CAPACITY samples = 1hr)
        if self.cpu_history.len() >= HISTORY_CAPACITY {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(system.global_cpu_usage as u64);

        if self.mem_history.len() >= HISTORY_CAPACITY {
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
            SortColumn::Pid => procs.sort_by(|a, b| apply_direction(a.pid.cmp(&b.pid), dir)),
            SortColumn::Name => procs.sort_by(|a, b| {
                apply_direction(a.name.to_lowercase().cmp(&b.name.to_lowercase()), dir)
            }),
            SortColumn::Cpu => procs.sort_by(|a, b| {
                apply_direction(
                    a.cpu_usage
                        .partial_cmp(&b.cpu_usage)
                        .unwrap_or(Ordering::Equal),
                    dir,
                )
            }),
            SortColumn::Memory => {
                procs.sort_by(|a, b| apply_direction(a.memory_bytes.cmp(&b.memory_bytes), dir))
            }
            SortColumn::DiskIo => procs.sort_by(|a, b| {
                let total_a = a.disk_read_bytes + a.disk_write_bytes;
                let total_b = b.disk_read_bytes + b.disk_write_bytes;
                apply_direction(total_a.cmp(&total_b), dir)
            }),
            SortColumn::Status => procs.sort_by(|a, b| {
                apply_direction(a.status.to_string().cmp(&b.status.to_string()), dir)
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
                self.selected_process = self.selected_process.saturating_sub(PAGE_SIZE);
            }
            Tab::Alerts => {
                self.alert_scroll = self.alert_scroll.saturating_sub(PAGE_SIZE);
            }
            Tab::AskAi => {
                self.ai_scroll = self.ai_scroll.saturating_sub(PAGE_SIZE);
            }
            _ => {}
        }
    }

    pub fn page_down(&mut self) {
        match self.active_tab {
            Tab::Processes => {
                let max = self.filtered_processes().len().saturating_sub(1);
                self.selected_process = (self.selected_process + PAGE_SIZE).min(max);
            }
            Tab::Alerts => {
                let max = self.alerts.len().saturating_sub(1);
                self.alert_scroll = (self.alert_scroll + PAGE_SIZE).min(max);
            }
            Tab::AskAi => {
                self.ai_scroll += PAGE_SIZE;
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
                    if fds.len() < MAX_FD_SAMPLE {
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
                vars.truncate(MAX_ENV_VARS);
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
                    .unwrap_or(Ordering::Equal)
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
                .unwrap_or(Ordering::Equal)
        });

        let mut result: Vec<(String, &ProcessInfo)> = Vec::with_capacity(processes.len());

        fn walk<'a>(
            pid: u32,
            prefix: &str,
            children_map: &HashMap<u32, Vec<&'a ProcessInfo>>,
            result: &mut Vec<(String, &'a ProcessInfo)>,
            depth: usize,
        ) {
            if depth > MAX_TREE_DEPTH {
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

                    walk(child.pid, &new_prefix, children_map, result, depth + 1);
                }
            }
        }

        // Walk from each root
        for root in &roots {
            result.push((String::new(), root));
            let prefix = String::new();
            walk(root.pid, &prefix, &children_map, &mut result, 1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertCategory, AlertSeverity, ProcessInfo, ProcessStatus};

    fn make_state() -> AppState {
        rust_i18n::set_locale("en");
        AppState::new(100, false, Theme::default_dark())
    }

    fn make_process(pid: u32, name: &str, cpu: f32, mem_bytes: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: format!("/usr/bin/{}", name),
            cpu_usage: cpu,
            memory_bytes: mem_bytes,
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

    fn make_process_with_parent(
        pid: u32,
        name: &str,
        cpu: f32,
        parent: Option<u32>,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cmd: format!("/usr/bin/{}", name),
            cpu_usage: cpu,
            memory_bytes: 1024,
            memory_percent: 0.0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: ProcessStatus::Running,
            user: "test".to_string(),
            start_time: 0,
            parent_pid: parent,
            thread_count: None,
        }
    }

    // ── HistoryWindow ─────────────────────────────────────────────

    #[test]
    fn history_window_points() {
        assert_eq!(HistoryWindow::OneMin.points(), 60);
        assert_eq!(HistoryWindow::FiveMin.points(), 300);
        assert_eq!(HistoryWindow::FifteenMin.points(), 900);
        assert_eq!(HistoryWindow::OneHour.points(), 3600);
    }

    #[test]
    fn history_window_next_cycles() {
        let w = HistoryWindow::OneMin;
        assert_eq!(w.next(), HistoryWindow::FiveMin);
        assert_eq!(w.next().next(), HistoryWindow::FifteenMin);
        assert_eq!(w.next().next().next(), HistoryWindow::OneHour);
        assert_eq!(w.next().next().next().next(), HistoryWindow::OneMin);
    }

    #[test]
    fn history_window_prev_cycles() {
        let w = HistoryWindow::OneMin;
        assert_eq!(w.prev(), HistoryWindow::OneHour);
        assert_eq!(w.prev().prev(), HistoryWindow::FifteenMin);
    }

    #[test]
    fn history_window_next_prev_inverse() {
        for w in [
            HistoryWindow::OneMin,
            HistoryWindow::FiveMin,
            HistoryWindow::FifteenMin,
            HistoryWindow::OneHour,
        ] {
            assert_eq!(w.next().prev(), w);
            assert_eq!(w.prev().next(), w);
        }
    }

    // ── Tab ───────────────────────────────────────────────────────

    #[test]
    fn tab_all_has_four() {
        assert_eq!(Tab::all().len(), 4);
    }

    #[test]
    fn tab_index() {
        assert_eq!(Tab::Dashboard.index(), 0);
        assert_eq!(Tab::Processes.index(), 1);
        assert_eq!(Tab::Alerts.index(), 2);
        assert_eq!(Tab::AskAi.index(), 3);
    }

    // ── Tab navigation ────────────────────────────────────────────

    #[test]
    fn next_tab_cycles() {
        let mut s = make_state();
        assert_eq!(s.active_tab, Tab::Dashboard);
        s.next_tab();
        assert_eq!(s.active_tab, Tab::Processes);
        s.next_tab();
        assert_eq!(s.active_tab, Tab::Alerts);
        s.next_tab();
        assert_eq!(s.active_tab, Tab::AskAi);
        s.next_tab();
        assert_eq!(s.active_tab, Tab::Dashboard);
    }

    #[test]
    fn prev_tab_cycles() {
        let mut s = make_state();
        s.prev_tab();
        assert_eq!(s.active_tab, Tab::AskAi);
        s.prev_tab();
        assert_eq!(s.active_tab, Tab::Alerts);
    }

    // ── Sort cycling ──────────────────────────────────────────────

    #[test]
    fn cycle_sort_goes_through_all() {
        let mut s = make_state();
        assert_eq!(s.sort_column, SortColumn::Cpu); // default
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::Memory);
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::DiskIo);
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::Status);
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::Pid);
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::Name);
        s.cycle_sort();
        assert_eq!(s.sort_column, SortColumn::Cpu);
    }

    #[test]
    fn toggle_sort_direction() {
        let mut s = make_state();
        assert_eq!(s.sort_direction, SortDirection::Desc);
        s.toggle_sort_direction();
        assert_eq!(s.sort_direction, SortDirection::Asc);
        s.toggle_sort_direction();
        assert_eq!(s.sort_direction, SortDirection::Desc);
    }

    // ── Filtering ─────────────────────────────────────────────────

    #[test]
    fn filtered_processes_no_filter() {
        let mut s = make_state();
        s.processes = vec![
            make_process(1, "firefox", 10.0, 1024),
            make_process(2, "chrome", 20.0, 2048),
        ];
        assert_eq!(s.filtered_processes().len(), 2);
    }

    #[test]
    fn filtered_processes_by_name() {
        let mut s = make_state();
        s.processes = vec![
            make_process(1, "firefox", 10.0, 1024),
            make_process(2, "chrome", 20.0, 2048),
            make_process(3, "firefox-esr", 5.0, 512),
        ];
        s.filter_text = "fire".to_string();
        let filtered = s.filtered_processes();
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|p| p.name.contains("fire")));
    }

    #[test]
    fn filtered_processes_by_pid() {
        let mut s = make_state();
        s.processes = vec![
            make_process(1234, "firefox", 10.0, 1024),
            make_process(5678, "chrome", 20.0, 2048),
        ];
        s.filter_text = "1234".to_string();
        let filtered = s.filtered_processes();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 1234);
    }

    #[test]
    fn filtered_processes_case_insensitive() {
        let mut s = make_state();
        s.processes = vec![make_process(1, "Firefox", 10.0, 1024)];
        s.filter_text = "firefox".to_string();
        assert_eq!(s.filtered_processes().len(), 1);
    }

    #[test]
    fn filtered_processes_by_cmd() {
        let mut s = make_state();
        s.processes = vec![make_process(1, "python3", 10.0, 1024)];
        // cmd is "/usr/bin/python3"
        s.filter_text = "/usr/bin".to_string();
        assert_eq!(s.filtered_processes().len(), 1);
    }

    // ── Sorting ───────────────────────────────────────────────────

    #[test]
    fn sort_by_cpu_desc() {
        let mut s = make_state();
        s.sort_column = SortColumn::Cpu;
        s.sort_direction = SortDirection::Desc;
        let mut procs = vec![
            make_process(1, "low", 5.0, 0),
            make_process(2, "high", 90.0, 0),
            make_process(3, "mid", 50.0, 0),
        ];
        s.sort_processes(&mut procs);
        assert_eq!(procs[0].name, "high");
        assert_eq!(procs[1].name, "mid");
        assert_eq!(procs[2].name, "low");
    }

    #[test]
    fn sort_by_cpu_asc() {
        let mut s = make_state();
        s.sort_column = SortColumn::Cpu;
        s.sort_direction = SortDirection::Asc;
        let mut procs = vec![
            make_process(1, "low", 5.0, 0),
            make_process(2, "high", 90.0, 0),
        ];
        s.sort_processes(&mut procs);
        assert_eq!(procs[0].name, "low");
        assert_eq!(procs[1].name, "high");
    }

    #[test]
    fn sort_by_memory() {
        let mut s = make_state();
        s.sort_column = SortColumn::Memory;
        s.sort_direction = SortDirection::Desc;
        let mut procs = vec![
            make_process(1, "small", 0.0, 100),
            make_process(2, "big", 0.0, 999999),
        ];
        s.sort_processes(&mut procs);
        assert_eq!(procs[0].name, "big");
    }

    #[test]
    fn sort_by_pid() {
        let mut s = make_state();
        s.sort_column = SortColumn::Pid;
        s.sort_direction = SortDirection::Asc;
        let mut procs = vec![
            make_process(100, "b", 0.0, 0),
            make_process(1, "a", 0.0, 0),
            make_process(50, "c", 0.0, 0),
        ];
        s.sort_processes(&mut procs);
        assert_eq!(procs[0].pid, 1);
        assert_eq!(procs[1].pid, 50);
        assert_eq!(procs[2].pid, 100);
    }

    #[test]
    fn sort_by_name() {
        let mut s = make_state();
        s.sort_column = SortColumn::Name;
        s.sort_direction = SortDirection::Asc;
        let mut procs = vec![
            make_process(1, "Zebra", 0.0, 0),
            make_process(2, "alpha", 0.0, 0),
            make_process(3, "Beta", 0.0, 0),
        ];
        s.sort_processes(&mut procs);
        // Case-insensitive: alpha, Beta, Zebra
        assert_eq!(procs[0].name, "alpha");
        assert_eq!(procs[1].name, "Beta");
        assert_eq!(procs[2].name, "Zebra");
    }

    // ── Scroll ────────────────────────────────────────────────────

    #[test]
    fn scroll_up_at_zero_stays() {
        let mut s = make_state();
        s.active_tab = Tab::Processes;
        s.selected_process = 0;
        s.scroll_up();
        assert_eq!(s.selected_process, 0);
    }

    #[test]
    fn scroll_down_increases() {
        let mut s = make_state();
        s.active_tab = Tab::Processes;
        s.processes = vec![
            make_process(1, "a", 0.0, 0),
            make_process(2, "b", 0.0, 0),
            make_process(3, "c", 0.0, 0),
        ];
        s.scroll_down();
        assert_eq!(s.selected_process, 1);
        s.scroll_down();
        assert_eq!(s.selected_process, 2);
        // At the end, stays
        s.scroll_down();
        assert_eq!(s.selected_process, 2);
    }

    #[test]
    fn scroll_alerts_tab() {
        let mut s = make_state();
        s.active_tab = Tab::Alerts;
        s.alerts = vec![
            Alert::new(
                AlertSeverity::Info,
                AlertCategory::HighCpu,
                "a",
                1,
                "msg".into(),
                0.0,
                0.0,
            ),
            Alert::new(
                AlertSeverity::Info,
                AlertCategory::HighCpu,
                "b",
                2,
                "msg".into(),
                0.0,
                0.0,
            ),
        ];
        s.scroll_down();
        assert_eq!(s.alert_scroll, 1);
        s.scroll_down();
        assert_eq!(s.alert_scroll, 1); // clamped
        s.scroll_up();
        assert_eq!(s.alert_scroll, 0);
    }

    // ── danger_alert_count ────────────────────────────────────────

    #[test]
    fn danger_alert_count_mixed() {
        let mut s = make_state();
        s.alerts = vec![
            Alert::new(
                AlertSeverity::Info,
                AlertCategory::HighCpu,
                "a",
                1,
                "".into(),
                0.0,
                0.0,
            ),
            Alert::new(
                AlertSeverity::Warning,
                AlertCategory::HighCpu,
                "b",
                2,
                "".into(),
                0.0,
                0.0,
            ),
            Alert::new(
                AlertSeverity::Critical,
                AlertCategory::HighCpu,
                "c",
                3,
                "".into(),
                0.0,
                0.0,
            ),
            Alert::new(
                AlertSeverity::Danger,
                AlertCategory::HighCpu,
                "d",
                4,
                "".into(),
                0.0,
                0.0,
            ),
        ];
        assert_eq!(s.danger_alert_count(), 2); // Critical + Danger
    }

    #[test]
    fn danger_alert_count_none() {
        let s = make_state();
        assert_eq!(s.danger_alert_count(), 0);
    }

    // ── AI input ──────────────────────────────────────────────────

    #[test]
    fn ai_input_char_and_backspace() {
        let mut s = make_state();
        s.ai_input_char('h');
        s.ai_input_char('i');
        assert_eq!(s.ai_input, "hi");
        assert_eq!(s.ai_cursor_pos, 2);
        s.ai_input_backspace();
        assert_eq!(s.ai_input, "h");
        assert_eq!(s.ai_cursor_pos, 1);
    }

    #[test]
    fn ai_input_backspace_at_start() {
        let mut s = make_state();
        s.ai_input_backspace(); // should be safe no-op
        assert_eq!(s.ai_input, "");
        assert_eq!(s.ai_cursor_pos, 0);
    }

    #[test]
    fn ai_cursor_movement() {
        let mut s = make_state();
        s.ai_input_char('a');
        s.ai_input_char('b');
        s.ai_input_char('c');
        assert_eq!(s.ai_cursor_pos, 3);

        s.ai_cursor_left();
        assert_eq!(s.ai_cursor_pos, 2);
        s.ai_cursor_left();
        assert_eq!(s.ai_cursor_pos, 1);
        s.ai_cursor_left();
        assert_eq!(s.ai_cursor_pos, 0);
        s.ai_cursor_left(); // stays at 0
        assert_eq!(s.ai_cursor_pos, 0);

        s.ai_cursor_right();
        assert_eq!(s.ai_cursor_pos, 1);
    }

    #[test]
    fn ai_cursor_right_at_end() {
        let mut s = make_state();
        s.ai_input_char('x');
        s.ai_cursor_right(); // already at end
        assert_eq!(s.ai_cursor_pos, 1);
    }

    #[test]
    fn ai_submit_returns_text_and_clears() {
        let mut s = make_state();
        s.ai_input_char('t');
        s.ai_input_char('e');
        s.ai_input_char('s');
        s.ai_input_char('t');
        let result = s.ai_submit();
        assert_eq!(result, Some("test".to_string()));
        assert_eq!(s.ai_input, "");
        assert_eq!(s.ai_cursor_pos, 0);
        // Should have added to conversation
        assert_eq!(s.ai_conversation.messages.len(), 1);
    }

    #[test]
    fn ai_submit_empty_returns_none() {
        let mut s = make_state();
        assert_eq!(s.ai_submit(), None);
    }

    #[test]
    fn ai_submit_whitespace_only_returns_none() {
        let mut s = make_state();
        s.ai_input = "   ".to_string();
        s.ai_cursor_pos = 3;
        assert_eq!(s.ai_submit(), None);
    }

    // ── Signal picker ─────────────────────────────────────────────

    #[test]
    fn signal_picker_open_close() {
        let mut s = make_state();
        s.processes = vec![make_process(42, "vim", 1.0, 100)];
        s.selected_process = 0;
        s.open_signal_picker();
        assert!(s.show_signal_picker);
        assert_eq!(s.signal_picker_pid, Some(42));
        assert_eq!(s.signal_picker_name, "vim");

        s.close_signal_picker();
        assert!(!s.show_signal_picker);
        assert_eq!(s.signal_picker_pid, None);
    }

    #[test]
    fn signal_picker_no_process() {
        let mut s = make_state();
        // No processes loaded
        s.open_signal_picker();
        assert!(!s.show_signal_picker);
    }

    // ── Renice dialog ─────────────────────────────────────────────

    #[test]
    fn renice_dialog_open_close() {
        let mut s = make_state();
        s.processes = vec![make_process(99, "htop", 2.0, 200)];
        s.selected_process = 0;
        s.open_renice_dialog();
        assert!(s.show_renice_dialog);
        assert_eq!(s.renice_pid, Some(99));
        assert_eq!(s.renice_value, 0);

        s.close_renice_dialog();
        assert!(!s.show_renice_dialog);
        assert_eq!(s.renice_pid, None);
    }

    // ── Focus cycling ─────────────────────────────────────────────

    #[test]
    fn toggle_focus_on_dashboard() {
        let mut s = make_state();
        s.active_tab = Tab::Dashboard;
        s.toggle_focus();
        assert_eq!(s.focused_widget, Some(FocusedWidget::TopProcesses));
        s.toggle_focus();
        assert_eq!(s.focused_widget, None);
    }

    #[test]
    fn cycle_focus_forward() {
        let mut s = make_state();
        s.focused_widget = Some(FocusedWidget::SystemGauges);
        s.cycle_focus_forward();
        assert_eq!(s.focused_widget, Some(FocusedWidget::CpuCores));
        s.cycle_focus_forward();
        assert_eq!(s.focused_widget, Some(FocusedWidget::Sparklines));
        s.cycle_focus_forward();
        assert_eq!(s.focused_widget, Some(FocusedWidget::Network));
    }

    // ── Tree view ─────────────────────────────────────────────────

    #[test]
    fn tree_processes_flat() {
        let mut s = make_state();
        s.processes = vec![
            make_process(1, "init", 0.0, 0),
            make_process(2, "bash", 0.0, 0),
        ];
        let tree = s.tree_processes();
        assert_eq!(tree.len(), 2);
        // Both are roots (no parent in set)
        assert!(tree[0].0.is_empty());
        assert!(tree[1].0.is_empty());
    }

    #[test]
    fn tree_processes_parent_child() {
        let mut s = make_state();
        s.processes = vec![
            make_process_with_parent(1, "init", 10.0, None),
            make_process_with_parent(2, "bash", 5.0, Some(1)),
            make_process_with_parent(3, "vim", 2.0, Some(2)),
        ];
        let tree = s.tree_processes();
        assert_eq!(tree.len(), 3);
        // First is root
        assert!(tree[0].0.is_empty());
        assert_eq!(tree[0].1.pid, 1);
        // Second is child of root (has tree connector)
        assert!(tree[1].0.contains("└") || tree[1].0.contains("├"));
        assert_eq!(tree[1].1.pid, 2);
    }

    // ── set_status ────────────────────────────────────────────────

    #[test]
    fn set_status_stores_message() {
        let mut s = make_state();
        assert!(s.status_message.is_none());
        s.set_status("test message".to_string());
        assert!(s.status_message.is_some());
        let (msg, _) = s.status_message.as_ref().unwrap();
        assert_eq!(msg, "test message");
    }

    // ── selected_process_info ─────────────────────────────────────

    #[test]
    fn selected_process_info_returns_correct() {
        let mut s = make_state();
        s.processes = vec![
            make_process(10, "first", 0.0, 0),
            make_process(20, "second", 0.0, 0),
        ];
        s.selected_process = 1;
        let info = s.selected_process_info();
        assert_eq!(info, Some((20, "second".to_string())));
    }

    #[test]
    fn selected_process_info_empty() {
        let s = make_state();
        assert_eq!(s.selected_process_info(), None);
    }

    // ── apply_direction ───────────────────────────────────────────

    #[test]
    fn apply_direction_asc() {
        use std::cmp::Ordering;
        assert_eq!(
            apply_direction(Ordering::Less, SortDirection::Asc),
            Ordering::Less
        );
        assert_eq!(
            apply_direction(Ordering::Greater, SortDirection::Asc),
            Ordering::Greater
        );
    }

    #[test]
    fn apply_direction_desc() {
        use std::cmp::Ordering;
        assert_eq!(
            apply_direction(Ordering::Less, SortDirection::Desc),
            Ordering::Greater
        );
        assert_eq!(
            apply_direction(Ordering::Greater, SortDirection::Desc),
            Ordering::Less
        );
    }

    // ── cycle_theme ───────────────────────────────────────────────

    #[test]
    fn cycle_theme_changes() {
        let mut s = make_state();
        let initial = s.theme.name.clone();
        s.cycle_theme();
        assert_ne!(s.theme.name, initial);
    }

    // ── CommandResult ──────────────────────────────────────────────

    #[test]
    fn command_result_text_only_has_no_actions() {
        let cr = CommandResult::text_only("hello".to_string());
        assert_eq!(cr.text, "hello");
        assert!(cr.actions.is_empty());
        assert!(!cr.has_executable_actions());
    }

    #[test]
    fn command_result_from_report_extracts_actions() {
        use crate::diagnostics::*;
        let mut report = DiagnosticReport::new("Test Report");
        report.findings.push(Finding {
            severity: FindingSeverity::Warning,
            title: "High CPU".to_string(),
            detail: "PID 42".to_string(),
            action: Some(SuggestedAction::KillProcess {
                pid: 42,
                name: "hog".to_string(),
                signal: "SIGTERM",
            }),
        });
        report.findings.push(Finding {
            severity: FindingSeverity::Info,
            title: "Info".to_string(),
            detail: "".to_string(),
            action: Some(SuggestedAction::Info("just info".to_string())),
        });
        report.findings.push(Finding {
            severity: FindingSeverity::Info,
            title: "No action".to_string(),
            detail: "".to_string(),
            action: None,
        });

        let cr = CommandResult::from_report(&report);
        assert_eq!(cr.actions.len(), 2); // KillProcess + Info
        assert!(cr.has_executable_actions()); // KillProcess is executable
        assert!(cr.actions[0].0.contains("Kill PID 42"));
        assert!(cr.actions[1].0.contains("just info"));
    }

    #[test]
    fn command_result_has_executable_actions_info_only() {
        let cr = CommandResult {
            text: "test".to_string(),
            actions: vec![("info".to_string(), SuggestedAction::Info("x".to_string()))],
        };
        assert!(!cr.has_executable_actions());
    }

    #[test]
    fn command_result_from_report_renice_action() {
        use crate::diagnostics::*;
        let mut report = DiagnosticReport::new("Test");
        report.findings.push(Finding {
            severity: FindingSeverity::Warning,
            title: "High CPU".to_string(),
            detail: "".to_string(),
            action: Some(SuggestedAction::ReniceProcess {
                pid: 99,
                name: "compile".to_string(),
                nice: 10,
            }),
        });

        let cr = CommandResult::from_report(&report);
        assert!(cr.has_executable_actions());
        assert!(cr.actions[0].0.contains("nice 10"));
        assert!(cr.actions[0].0.contains("PID 99"));
    }

    #[test]
    fn command_result_from_report_free_port_action() {
        use crate::diagnostics::*;
        let mut report = DiagnosticReport::new("Test");
        report.findings.push(Finding {
            severity: FindingSeverity::Info,
            title: "Port bound".to_string(),
            detail: "".to_string(),
            action: Some(SuggestedAction::FreePort {
                port: 8080,
                pid: 55,
                name: "node".to_string(),
            }),
        });

        let cr = CommandResult::from_report(&report);
        assert!(cr.has_executable_actions());
        assert!(cr.actions[0].0.contains("port 8080"));
    }

    #[test]
    fn command_result_from_report_clean_dir_action() {
        use crate::diagnostics::*;
        let mut report = DiagnosticReport::new("Test");
        report.findings.push(Finding {
            severity: FindingSeverity::Info,
            title: "Cache".to_string(),
            detail: "".to_string(),
            action: Some(SuggestedAction::CleanDirectory {
                path: "/tmp/cache".to_string(),
                size_bytes: 2_000_000_000,
            }),
        });

        let cr = CommandResult::from_report(&report);
        assert!(cr.has_executable_actions());
        assert!(cr.actions[0].0.contains("/tmp/cache"));
        assert!(cr.actions[0].0.contains("GB"));
    }
}
