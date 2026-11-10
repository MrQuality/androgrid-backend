//! Terminal Observability User Interface (TUI)
//!
//! Provides an interactive, cross-platform real-time terminal monitoring interface
//! for operator inspection of worker node telemetry, task queues, and speculative preemption.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;

use crate::observability::snapshot::{
    ClusterMetrics, ClusterSnapshot, TaskSnapshot, WorkerNodeSnapshot,
};
use crate::{TaskInfo, TaskState, WorkerNodeInfo};

/// Actions produced by operator keyboard events in the TUI event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// Request clean termination of the TUI process.
    Quit,
    /// Request cancellation of a specific task.
    CancelTask(String),
    /// Request generation of an ephemeral device pairing token.
    GeneratePairingToken,
    /// Request an immediate telemetry refresh.
    Refresh,
}

/// RAII Drop guard ensuring terminal state restoration upon exit or panic.
///
/// Enforces Windows VT100 and Unix terminal escape cleanup.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enables raw mode and enters alternate screen buffer.
    pub fn init() -> Result<Self, std::io::Error> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

/// State machine backing the terminal observability application.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[derive(Debug, Clone)]
pub struct TuiApp {
    /// Remote control plane endpoint URL
    pub endpoint: String,
    /// List of connected worker nodes
    pub nodes: Vec<WorkerNodeSnapshot>,
    /// List of cluster compute tasks
    pub tasks: Vec<TaskSnapshot>,
    /// Aggregate cluster metrics
    pub metrics: ClusterMetrics,
    /// Recent event activity log
    pub event_log: Vec<String>,
    /// Status notification displayed in footer
    pub status_message: Option<String>,
    /// Whether the user requested application exit
    pub should_quit: bool,
    /// Whether the task cancellation prompt dialog is active
    cancel_modal_open: bool,
    /// Text input buffer for task cancellation dialog
    pub cancel_input: String,
    /// Whether the device pairing info dialog is active
    pair_modal_open: bool,
    /// Text buffer holding generated pairing information
    pub pair_info: String,
}

impl TuiApp {
    /// Constructs a new TuiApp instance connected to the specified endpoint.
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            nodes: Vec::new(),
            tasks: Vec::new(),
            metrics: ClusterMetrics::default(),
            event_log: Vec::new(),
            status_message: None,
            should_quit: false,
            cancel_modal_open: false,
            cancel_input: String::new(),
            pair_modal_open: false,
            pair_info: String::new(),
        }
    }

    /// Updates internal state from a comprehensive `ClusterSnapshot`.
    pub fn update_from_snapshot(&mut self, snapshot: ClusterSnapshot) {
        self.nodes = snapshot.nodes;
        self.tasks = snapshot.tasks;
        self.metrics = snapshot.metrics;
    }

    /// Updates internal state from gRPC `WorkerNodeInfo` and `TaskInfo` records.
    pub fn update_from_nodes_and_tasks(
        &mut self,
        nodes: Vec<WorkerNodeInfo>,
        tasks: Vec<TaskInfo>,
    ) {
        let mut wifi_nodes = 0;
        let mut cellular_nodes = 0;

        self.nodes = nodes
            .into_iter()
            .map(|node| {
                let caps = node.capabilities.unwrap_or_default();
                let is_wifi = caps.current_transport == 1;
                if is_wifi {
                    wifi_nodes += 1;
                } else {
                    cellular_nodes += 1;
                }

                let thermal_badge = match caps.thermal_status {
                    0 => "NOMINAL",
                    1 => "LIGHT",
                    2 => "MODERATE",
                    3 => "SEVERE",
                    _ => "CRITICAL",
                }
                .to_string();

                WorkerNodeSnapshot {
                    node_id: node.node_id,
                    is_rooted: caps.is_rooted,
                    transport: if is_wifi {
                        "Wi-Fi LAN".to_string()
                    } else {
                        "Cellular WAN".to_string()
                    },
                    battery_level_pct: 100, // Nominal placeholder if not reported
                    is_charging: false,
                    thermal_status: caps.thermal_status,
                    thermal_badge,
                    cpu_cores: caps.cpu_cores,
                    ram_mb: caps.total_ram_bytes / (1024 * 1024),
                    os_version: caps.os_version,
                    abi: caps.abi,
                    client_version: node.client_version,
                    last_seen_epoch_ms: node.last_heartbeat_epoch_seconds * 1000,
                }
            })
            .collect();

        let mut pending_tasks = 0;
        let mut leased_tasks = 0;
        let mut completed_tasks = 0;
        let mut failed_tasks = 0;
        let mut preempted_tasks = 0;
        let mut timed_out_tasks = 0;

        self.tasks = tasks
            .into_iter()
            .map(|t| {
                let state_enum = TaskState::try_from(t.state).unwrap_or(TaskState::Unspecified);
                let state_str = match state_enum {
                    TaskState::Pending => {
                        pending_tasks += 1;
                        "Pending"
                    }
                    TaskState::Leased => {
                        leased_tasks += 1;
                        "Leased"
                    }
                    TaskState::Running => {
                        leased_tasks += 1;
                        "Running"
                    }
                    TaskState::Completed => {
                        completed_tasks += 1;
                        "Completed"
                    }
                    TaskState::Failed => {
                        failed_tasks += 1;
                        "Failed"
                    }
                    TaskState::Preempted => {
                        preempted_tasks += 1;
                        "Preempted"
                    }
                    TaskState::TimedOut => {
                        timed_out_tasks += 1;
                        "TimedOut"
                    }
                    _ => "Unknown",
                }
                .to_string();

                let engine_str = match t.engine {
                    1 => "WasmWasi",
                    2 => "RootShell",
                    3 => "ToyboxUser",
                    _ => "Unknown",
                }
                .to_string();

                TaskSnapshot {
                    task_id: t.task_id,
                    state: state_str,
                    engine: engine_str,
                    run_attempt: t.run_attempt,
                    assigned_node_id: if t.assigned_node_id.is_empty() {
                        None
                    } else {
                        Some(t.assigned_node_id)
                    },
                    speculative_node_id: if t.speculative_node_id.is_empty() {
                        None
                    } else {
                        Some(t.speculative_node_id)
                    },
                    timeout_seconds: t.timeout_seconds,
                    requires_unmetered: t.requires_unmetered,
                    exit_code: Some(t.exit_code),
                    execution_duration_ms: Some(t.execution_duration_ms),
                    error_message: if t.error_message.is_empty() {
                        None
                    } else {
                        Some(t.error_message)
                    },
                }
            })
            .collect();

        self.metrics = ClusterMetrics {
            total_nodes: self.nodes.len(),
            wifi_nodes,
            cellular_nodes,
            total_tasks: self.tasks.len(),
            pending_tasks,
            leased_tasks,
            completed_tasks,
            failed_tasks,
            preempted_tasks,
            timed_out_tasks,
        };
    }

    /// Returns whether the cancel modal prompt is currently open.
    pub fn is_cancel_modal_open(&self) -> bool {
        self.cancel_modal_open
    }

    /// Returns whether the pair modal prompt is currently open.
    pub fn is_pair_modal_open(&self) -> bool {
        self.pair_modal_open
    }

    /// Handles keyboard input events, updating UI state or producing actions.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<TuiAction> {
        // Modal: Task Cancellation input
        if self.cancel_modal_open {
            match key.code {
                KeyCode::Esc => {
                    self.cancel_modal_open = false;
                    self.cancel_input.clear();
                    None
                }
                KeyCode::Enter => {
                    let task_id = self.cancel_input.trim().to_string();
                    self.cancel_modal_open = false;
                    self.cancel_input.clear();
                    if !task_id.is_empty() {
                        Some(TuiAction::CancelTask(task_id))
                    } else {
                        None
                    }
                }
                KeyCode::Backspace => {
                    self.cancel_input.pop();
                    None
                }
                KeyCode::Char(c) => {
                    self.cancel_input.push(c);
                    None
                }
                _ => None,
            }
        } else if self.pair_modal_open {
            // Modal: Device Pairing Info (any key closes)
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('p') => {
                    self.pair_modal_open = false;
                    self.pair_info.clear();
                    None
                }
                _ => None,
            }
        } else {
            // Main screen hotkeys
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.should_quit = true;
                    Some(TuiAction::Quit)
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.cancel_modal_open = true;
                    self.cancel_input.clear();
                    None
                }
                KeyCode::Char('p') | KeyCode::Char('P') => Some(TuiAction::GeneratePairingToken),
                KeyCode::Char('r') | KeyCode::Char('R') => Some(TuiAction::Refresh),
                _ => None,
            }
        }
    }

    /// Opens the pairing dialog with the provided token and payload summary.
    pub fn show_pairing_info(&mut self, info: String) {
        self.pair_modal_open = true;
        self.pair_info = info;
    }

    /// Sets an ephemeral status notification in the footer.
    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
    }

    /// Renders the complete TUI layout to the provided frame.
    pub fn render(&self, frame: &mut Frame) {
        let size = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header & Metrics
                Constraint::Min(8),    // Worker Nodes Table
                Constraint::Min(8),    // Tasks Table
                Constraint::Length(2), // Footer Keybindings & Status
            ])
            .split(size);

        self.render_header(frame, chunks[0]);
        self.render_nodes(frame, chunks[1]);
        self.render_tasks(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);

        if self.cancel_modal_open {
            self.render_cancel_modal(frame, size);
        } else if self.pair_modal_open {
            self.render_pair_modal(frame, size);
        }
    }

    /// Renders the top header bar with cluster metrics.
    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let header_text = vec![Line::from(vec![
            Span::styled(
                " AndroGrid Cluster Top Monitor ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | Endpoint: "),
            Span::styled(&self.endpoint, Style::default().fg(Color::Yellow)),
            Span::raw(" | Nodes: "),
            Span::styled(
                format!(
                    "{} (Wi-Fi: {}, Cell: {})",
                    self.metrics.total_nodes, self.metrics.wifi_nodes, self.metrics.cellular_nodes
                ),
                Style::default().fg(Color::Green),
            ),
            Span::raw(" | Tasks: "),
            Span::styled(
                format!(
                    "{} (Running: {}, Done: {}, Preempted: {})",
                    self.metrics.total_tasks,
                    self.metrics.leased_tasks,
                    self.metrics.completed_tasks,
                    self.metrics.preempted_tasks
                ),
                Style::default().fg(Color::Magenta),
            ),
        ])];

        let header = Paragraph::new(header_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        );
        frame.render_widget(header, area);
    }

    /// Renders the Worker Nodes inventory table.
    fn render_nodes(&self, frame: &mut Frame, area: Rect) {
        let header_cells = [
            "Node ID",
            "Transport",
            "Battery",
            "Thermal",
            "Root",
            "Cores",
            "RAM (MB)",
            "OS / ABI",
        ]
        .into_iter()
        .map(|h| {
            Cell::from(h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
        let header = Row::new(header_cells).height(1);

        let rows: Vec<Row> = self
            .nodes
            .iter()
            .map(|node| {
                let transport_color = if node.transport.contains("Wi-Fi") {
                    Color::Cyan
                } else {
                    Color::Yellow
                };

                let thermal_color = match node.thermal_status {
                    0 => Color::Green,
                    1 => Color::Yellow,
                    2 => Color::Rgb(255, 165, 0),
                    _ => Color::Red,
                };

                let root_str = if node.is_rooted { "YES" } else { "NO" };

                let cells = vec![
                    Cell::from(node.node_id.clone()).style(Style::default().fg(Color::White)),
                    Cell::from(node.transport.clone()).style(Style::default().fg(transport_color)),
                    Cell::from(format!("{}%", node.battery_level_pct))
                        .style(Style::default().fg(Color::Green)),
                    Cell::from(node.thermal_badge.clone())
                        .style(Style::default().fg(thermal_color)),
                    Cell::from(root_str),
                    Cell::from(node.cpu_cores.to_string()),
                    Cell::from(node.ram_mb.to_string()),
                    Cell::from(format!("Android {} / {}", node.os_version, node.abi)),
                ];
                Row::new(cells)
            })
            .collect();

        let widths = [
            Constraint::Percentage(20),
            Constraint::Percentage(14),
            Constraint::Percentage(10),
            Constraint::Percentage(12),
            Constraint::Percentage(8),
            Constraint::Percentage(8),
            Constraint::Percentage(10),
            Constraint::Percentage(18),
        ];

        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Active Worker Nodes ({}) ", self.nodes.len()))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(table, area);
    }

    /// Renders the Compute Tasks pipeline table.
    fn render_tasks(&self, frame: &mut Frame, area: Rect) {
        let header_cells = [
            "Task ID",
            "State",
            "Engine",
            "Attempt",
            "Assigned Worker",
            "Duration",
            "Exit Code",
        ]
        .into_iter()
        .map(|h| {
            Cell::from(h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
        let header = Row::new(header_cells).height(1);

        let rows: Vec<Row> = self
            .tasks
            .iter()
            .map(|task| {
                let state_color = match task.state.as_str() {
                    "Pending" => Color::DarkGray,
                    "Leased" | "Running" => Color::Yellow,
                    "Completed" => Color::Green,
                    "Failed" => Color::Red,
                    "Preempted" => Color::Magenta,
                    _ => Color::White,
                };

                let dur_str = task
                    .execution_duration_ms
                    .map(|d| format!("{d} ms"))
                    .unwrap_or_else(|| "-".to_string());
                let exit_str = task
                    .exit_code
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "-".to_string());

                let worker_str = task.assigned_node_id.as_deref().unwrap_or("unassigned");

                let cells = vec![
                    Cell::from(task.task_id.clone()).style(Style::default().fg(Color::White)),
                    Cell::from(task.state.clone()).style(
                        Style::default()
                            .fg(state_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(task.engine.clone()),
                    Cell::from(format!("#{}", task.run_attempt)),
                    Cell::from(worker_str),
                    Cell::from(dur_str),
                    Cell::from(exit_str),
                ];
                Row::new(cells)
            })
            .collect();

        let widths = [
            Constraint::Percentage(25),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(10),
            Constraint::Percentage(17),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ];

        let table = Table::new(rows, widths).header(header).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Task Execution Pipeline ({}) ", self.tasks.len()))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(table, area);
    }

    /// Renders the footer with keybinding hints and status notification.
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let status = self.status_message.as_deref().unwrap_or("Ready");
        let line = Line::from(vec![
            Span::styled(
                " [q] Quit ",
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(" "),
            Span::styled(
                " [c] Cancel Task ",
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ),
            Span::raw(" "),
            Span::styled(
                " [p] Pair Device ",
                Style::default().fg(Color::Black).bg(Color::Green),
            ),
            Span::raw(" "),
            Span::styled(
                " [r] Refresh ",
                Style::default().fg(Color::Black).bg(Color::Magenta),
            ),
            Span::raw(" | Status: "),
            Span::styled(status, Style::default().fg(Color::White)),
        ]);

        let p = Paragraph::new(line);
        frame.render_widget(p, area);
    }

    /// Renders the task cancellation input dialog popup.
    fn render_cancel_modal(&self, frame: &mut Frame, area: Rect) {
        let popup_area = centered_rect(60, 20, area);
        frame.render_widget(Clear, popup_area);

        let text = vec![
            Line::from("Enter Task ID to cancel:"),
            Line::from(""),
            Line::from(vec![
                Span::raw(" > "),
                Span::styled(
                    &self.cancel_input,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("_", Style::default().fg(Color::Cyan)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("[Enter] Submit  ", Style::default().fg(Color::Green)),
                Span::styled("[Esc] Dismiss", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let popup = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Cancel Task Prompt ")
                    .border_style(Style::default().fg(Color::Red)),
            )
            .alignment(Alignment::Left);

        frame.render_widget(popup, popup_area);
    }

    /// Renders the device pairing token information dialog popup.
    fn render_pair_modal(&self, frame: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 30, area);
        frame.render_widget(Clear, popup_area);

        let text = vec![
            Line::from(Span::styled(
                "Ephemeral Device Pairing Token Generated",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::raw(&self.pair_info)),
            Line::from(""),
            Line::from(Span::styled(
                "Press [Esc] or [Enter] to dismiss",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let popup = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Mobile Node Onboarding ")
                    .border_style(Style::default().fg(Color::Green)),
            )
            .alignment(Alignment::Center);

        frame.render_widget(popup, popup_area);
    }
}

/// Helper calculating a centered Rect within bounds.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
