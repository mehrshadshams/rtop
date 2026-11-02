use std::{collections::VecDeque, time::Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use sysinfo::{Networks, Pid, ProcessesToUpdate, Signal, System};

pub const HISTORY_LENGTH: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: Pid,
    pub start_time: u64,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ProcessRow {
    pub target: Target,
    pub command: String,
    pub status: String,
    pub cpu: f32,
    pub memory: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Memory,
    Pid,
    Name,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "MEM",
            Self::Pid => "PID",
            Self::Name => "NAME",
        }
    }
}

pub struct App {
    system: System,
    networks: Networks,
    last_sample: Instant,
    pub host: String,
    pub os: String,
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub used_memory: u64,
    pub total_memory: u64,
    pub used_swap: u64,
    pub total_swap: u64,
    pub uptime: u64,
    pub received_per_second: u64,
    pub sent_per_second: u64,
    pub history: VecDeque<u64>,
    pub processes: Vec<ProcessRow>,
    pub visible: Vec<usize>,
    pub table: TableState,
    pub sort: Sort,
    pub descending: bool,
    pub filter: String,
    pub searching: bool,
    pub paused: bool,
    pub help: bool,
    pub pending_kill: Option<Target>,
    pub message: Option<(String, bool)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            last_sample: Instant::now(),
            host: System::host_name().unwrap_or_else(|| "Unknown host".into()),
            os: System::long_os_version().unwrap_or_else(|| "Unknown OS".into()),
            cpu: 0.0,
            cores: Vec::new(),
            used_memory: 0,
            total_memory: 0,
            used_swap: 0,
            total_swap: 0,
            uptime: 0,
            received_per_second: 0,
            sent_per_second: 0,
            history: VecDeque::new(),
            processes: Vec::new(),
            visible: Vec::new(),
            table: TableState::default(),
            sort: Sort::Cpu,
            descending: true,
            filter: String::new(),
            searching: false,
            paused: false,
            help: false,
            pending_kill: None,
            message: None,
        }
    }

    pub fn refresh(&mut self) {
        let selected = self.selected().map(|p| p.target.clone());
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.networks.refresh(true);
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sample).as_secs_f64();
        self.last_sample = now;
        self.cpu = self.system.global_cpu_usage();
        self.cores = self.system.cpus().iter().map(|c| c.cpu_usage()).collect();
        self.used_memory = self.system.used_memory();
        self.total_memory = self.system.total_memory();
        self.used_swap = self.system.used_swap();
        self.total_swap = self.system.total_swap();
        self.uptime = System::uptime();
        self.received_per_second =
            rate(self.networks.values().map(|n| n.received()).sum(), elapsed);
        self.sent_per_second = rate(
            self.networks.values().map(|n| n.transmitted()).sum(),
            elapsed,
        );
        if self.history.len() == HISTORY_LENGTH {
            self.history.pop_front();
        }
        self.history.push_back(self.cpu.clamp(0.0, 100.0) as u64);
        let core_count = self.cores.len().max(1) as f32;
        self.processes = self
            .system
            .processes()
            .values()
            .map(|p| ProcessRow {
                target: Target {
                    pid: p.pid(),
                    start_time: p.start_time(),
                    name: p.name().to_string_lossy().into_owned(),
                },
                command: p
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                status: p.status().to_string(),
                cpu: p.cpu_usage() / core_count,
                memory: p.memory(),
            })
            .collect();
        self.rebuild(selected);
    }

    pub fn selected(&self) -> Option<&ProcessRow> {
        self.table
            .selected()
            .and_then(|i| self.visible.get(i))
            .and_then(|i| self.processes.get(*i))
    }

    pub fn rebuild(&mut self, selected: Option<Target>) {
        let query = self.filter.to_lowercase();
        self.visible = self
            .processes
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.target.name.to_lowercase().contains(&query)
                    || p.command.to_lowercase().contains(&query)
                    || p.target.pid.to_string().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        self.visible.sort_by(|a, b| {
            let a = &self.processes[*a];
            let b = &self.processes[*b];
            let order = match self.sort {
                Sort::Cpu => a.cpu.total_cmp(&b.cpu),
                Sort::Memory => a.memory.cmp(&b.memory),
                Sort::Pid => a.target.pid.cmp(&b.target.pid),
                Sort::Name => a
                    .target
                    .name
                    .to_lowercase()
                    .cmp(&b.target.name.to_lowercase()),
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| a.target.pid.cmp(&b.target.pid))
        });
        let index = selected
            .and_then(|target| {
                self.visible
                    .iter()
                    .position(|i| self.processes[*i].target == target)
            })
            .or_else(|| (!self.visible.is_empty()).then_some(0));
        self.table.select(index);
    }

    fn set_sort(&mut self, sort: Sort) {
        let selected = self.selected().map(|p| p.target.clone());
        if self.sort == sort {
            self.descending = !self.descending;
        } else {
            self.sort = sort;
            self.descending = matches!(sort, Sort::Cpu | Sort::Memory);
        }
        self.rebuild(selected);
    }

    fn navigate(&mut self, delta: isize) {
        if !self.visible.is_empty() {
            let index = self.table.selected().unwrap_or(0);
            let next = index
                .saturating_add_signed(delta)
                .min(self.visible.len() - 1);
            self.table.select(Some(next));
        }
    }

    // Modal input is handled first so search text cannot trigger destructive actions.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if self.pending_kill.is_some() {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    if let Some(target) = self.pending_kill.take() {
                        self.message = Some(match terminate(&target) {
                            Ok(()) => (
                                format!("Termination requested for PID {}", target.pid),
                                false,
                            ),
                            Err(error) => (error, true),
                        });
                        if !self.paused {
                            self.refresh();
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => self.pending_kill = None,
                _ => {}
            }
            return false;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::F(1)
            ) {
                self.help = false;
            }
            return false;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter.push(c);
                }
                _ => {}
            }
            self.rebuild(None);
            return false;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::F(10) => return true,
            KeyCode::Down | KeyCode::Char('j') => self.navigate(1),
            KeyCode::Up | KeyCode::Char('k') => self.navigate(-1),
            KeyCode::PageDown => self.navigate(10),
            KeyCode::PageUp => self.navigate(-10),
            KeyCode::Home => self.navigate(isize::MIN),
            KeyCode::End => self.navigate(isize::MAX),
            KeyCode::Char('c') => self.set_sort(Sort::Cpu),
            KeyCode::Char('m') => self.set_sort(Sort::Memory),
            KeyCode::Char('p') => self.set_sort(Sort::Pid),
            KeyCode::Char('n') => self.set_sort(Sort::Name),
            KeyCode::Char('r') => {
                self.descending = !self.descending;
                self.rebuild(self.selected().map(|p| p.target.clone()));
            }
            KeyCode::Char('/') | KeyCode::F(3) => {
                self.searching = true;
                self.message = None;
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.message = None;
                self.rebuild(self.selected().map(|p| p.target.clone()));
            }
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::F(9) | KeyCode::Char('x') => {
                if let Some(p) = self.selected() {
                    if p.target.pid.as_u32() == std::process::id() || p.target.pid.as_u32() == 0 {
                        self.message = Some(("Cannot terminate rtop itself or PID 0".into(), true));
                    } else {
                        self.pending_kill = Some(p.target.clone());
                    }
                } else {
                    self.message = Some(("No process selected".into(), true));
                }
            }
            _ => {}
        }
        false
    }
}

pub fn terminate(target: &Target) -> Result<(), String> {
    if target.pid.as_u32() == 0 || target.pid.as_u32() == std::process::id() {
        return Err("Cannot terminate rtop itself or PID 0".into());
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[target.pid]), true);
    let process = system
        .process(target.pid)
        .ok_or_else(|| format!("PID {} has exited or cannot be inspected", target.pid))?;
    if process.start_time() != target.start_time || process.name().to_string_lossy() != target.name
    {
        return Err(format!("PID {} changed; termination cancelled", target.pid));
    }
    #[cfg(windows)]
    let signal = Signal::Kill;
    #[cfg(not(windows))]
    let signal = Signal::Term;
    match process.kill_with(signal) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "Could not terminate PID {}: access denied or process exited",
            target.pid
        )),
        None => Err("Termination is not supported on this platform".into()),
    }
}

pub fn percentage(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    }
}

fn rate(bytes: u64, elapsed: f64) -> u64 {
    if elapsed > 0.0 {
        (bytes as f64 / elapsed) as u64
    } else {
        0
    }
}

pub fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

pub fn format_uptime(seconds: u64) -> String {
    format!(
        "{}d {:02}:{:02}:{:02}",
        seconds / 86_400,
        seconds / 3_600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn process(pid: u32, name: &str, cpu: f32, memory: u64) -> ProcessRow {
        ProcessRow {
            target: Target {
                pid: Pid::from_u32(pid),
                start_time: 100,
                name: name.into(),
            },
            command: format!("{name} --example"),
            status: "Run".into(),
            cpu,
            memory,
        }
    }

    fn fixture() -> App {
        let mut app = App::new();
        app.processes = vec![
            process(10, "Alpha", 12.0, 200),
            process(20, "beta", 40.0, 100),
            process(30, "gamma", 40.0, 300),
        ];
        app.rebuild(None);
        app
    }

    #[test]
    fn formats_memory_in_binary_units() {
        assert_eq!(format_bytes(1_572_864), "1.5 MiB");
    }

    #[test]
    fn formats_boundaries_and_clamps_gauges() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_uptime(90_061), "1d 01:01:01");
        assert_eq!(percentage(100, 0), 0.0);
        assert_eq!(percentage(200, 100), 1.0);
        assert_eq!(rate(1000, 2.0), 500);
    }

    #[test]
    fn sorts_deterministically_and_preserves_selection() {
        let mut app = fixture();
        assert_eq!(app.visible, vec![1, 2, 0]);
        let selected = app.selected().unwrap().target.clone();
        app.handle_key(key(KeyCode::Char('m')));
        assert_eq!(app.visible, vec![2, 0, 1]);
        assert_eq!(app.selected().unwrap().target, selected);
        app.handle_key(key(KeyCode::Char('m')));
        assert_eq!(app.visible, vec![1, 0, 2]);
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.visible, vec![0, 1, 2]);
        app.handle_key(key(KeyCode::Char('p')));
        assert_eq!(app.visible, vec![0, 1, 2]);
        app.handle_key(key(KeyCode::Char('r')));
        assert_eq!(app.visible, vec![2, 1, 0]);
    }

    #[test]
    fn preserves_identity_when_snapshot_reorders_and_handles_process_exit() {
        let mut app = fixture();
        let selected = app.selected().unwrap().target.clone();
        app.processes.reverse();
        app.rebuild(Some(selected.clone()));
        assert_eq!(app.selected().unwrap().target, selected);
        app.processes.retain(|p| p.target != selected);
        app.rebuild(Some(selected));
        assert!(app.selected().is_some());
        let stale = app.selected().unwrap().target.clone();
        app.processes
            .iter_mut()
            .for_each(|p| p.target.start_time += 1);
        app.rebuild(Some(stale.clone()));
        assert_ne!(app.selected().unwrap().target, stale);
    }

    #[test]
    fn filters_name_command_and_pid_without_case_sensitivity() {
        let mut app = fixture();
        for query in ["ALPHA", "--EXAMPLE", "30", "no-such-process"] {
            app.filter = query.into();
            app.rebuild(None);
            let expected = match query {
                "ALPHA" => 1,
                "--EXAMPLE" => 3,
                "30" => 1,
                _ => 0,
            };
            assert_eq!(app.visible.len(), expected);
        }
        assert!(app.selected().is_none());
        app.handle_key(key(KeyCode::End));
        assert!(app.selected().is_none());
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.visible.len(), 3);
    }

    #[test]
    fn navigation_clamps_and_pause_is_reversible() {
        let mut app = fixture();
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.table.selected(), Some(2));
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.table.selected(), Some(2));
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.table.selected(), Some(0));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.table.selected(), Some(0));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.table.selected(), Some(1));
        app.handle_key(key(KeyCode::Char(' ')));
        assert!(app.paused);
        app.handle_key(key(KeyCode::Char(' ')));
        assert!(!app.paused);
    }

    #[test]
    fn search_and_help_are_modal() {
        let mut app = fixture();
        app.handle_key(key(KeyCode::Char('/')));
        for c in ['x', 'q', 'c'] {
            assert!(!app.handle_key(key(KeyCode::Char(c))));
        }
        assert_eq!(app.filter, "xqc");
        assert!(app.pending_kill.is_none());
        assert_eq!(app.sort, Sort::Cpu);
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.filter, "xq");
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.searching);
        assert_eq!(app.filter, "xq");
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help);
        app.handle_key(key(KeyCode::Char('x')));
        assert!(app.pending_kill.is_none());
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.help);
    }

    #[test]
    fn termination_requires_confirmation_and_freezes_target() {
        let mut app = fixture();
        app.handle_key(key(KeyCode::Char('x')));
        let pending = app.pending_kill.clone().unwrap();
        app.processes.reverse();
        app.rebuild(None);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.pending_kill, Some(pending));
        app.handle_key(key(KeyCode::Char('n')));
        assert!(app.pending_kill.is_none());
        app.handle_key(key(KeyCode::F(9)));
        app.handle_key(key(KeyCode::Esc));
        assert!(app.pending_kill.is_none());
    }

    #[test]
    fn self_and_pid_zero_are_protected() {
        let mut app = fixture();
        for pid in [0, std::process::id()] {
            app.processes = vec![process(pid, "protected", 0.0, 0)];
            app.rebuild(None);
            app.handle_key(key(KeyCode::Char('x')));
            assert!(app.pending_kill.is_none());
            assert!(app.message.as_ref().unwrap().1);
            assert!(terminate(&app.processes[0].target).is_err());
        }
    }

    #[test]
    fn ctrl_c_quits_even_in_modal_input() {
        let mut app = fixture();
        app.searching = true;
        assert!(app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        app.searching = false;
        app.help = true;
        assert!(app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        app.help = false;
        app.handle_key(key(KeyCode::Char('x')));
        assert!(app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn refresh_collects_live_metrics_and_bounds_history() {
        let mut app = App::new();
        std::thread::sleep(std::time::Duration::from_millis(250));
        app.history = std::iter::repeat_n(10, HISTORY_LENGTH).collect();
        app.refresh();
        assert!(app.total_memory > 0);
        assert!(!app.cores.is_empty());
        assert!(
            app.processes
                .iter()
                .any(|p| p.target.pid.as_u32() == std::process::id())
        );
        assert_eq!(app.history.len(), HISTORY_LENGTH);
        assert!((0.0..=100.0).contains(&app.cpu));
    }

    #[test]
    fn termination_rechecks_identity_and_terminates_only_own_child() {
        use std::process::{Child, Command, Stdio};
        use std::time::Duration;

        struct ChildGuard(Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        let mut child = ChildGuard(
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pid = Pid::from_u32(child.0.id());
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let process = system.process(pid).unwrap();
        let target = Target {
            pid,
            start_time: process.start_time(),
            name: process.name().to_string_lossy().into_owned(),
        };
        let mut stale = target.clone();
        stale.start_time += 1;
        assert!(terminate(&stale).unwrap_err().contains("changed"));
        assert!(child.0.try_wait().unwrap().is_none());
        terminate(&target).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.0.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "Child did not exit after termination"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(terminate(&target).is_err());
    }
}
