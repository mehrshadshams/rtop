# rtop

A cross-platform, htop-inspired Rust system monitor with a rich-text terminal
interface powered by Ratatui, Crossterm, and sysinfo. No server or elevated
permissions are required to launch it.

## Run

Install a current stable Rust toolchain, then run in a real terminal (Windows
Terminal, PowerShell, or a Linux/macOS terminal):

```sh
cargo run --release
cargo run --release -- --interval 500
cargo run --release -- --once
```

`--interval` accepts 250-60000 milliseconds (default: 1000). `--once` prints a
live snapshot and the top 20 processes by CPU usage, suitable for redirected
output. Both modes wait briefly to collect a meaningful initial CPU sample.
Use `--help` for command-line help.

## Dashboard

- Overall CPU gauge, per-core utilization, and the last 120 CPU samples.
- RAM and swap gauges, host/OS information, uptime, and aggregate network rates.
- Scrollable process table with CPU, memory, PID, and name. Wider terminals
  additionally show memory percentage, state, and command line.
- Case-insensitive search by process name, command line, or PID.
- Stable process selection across refreshes and sorting.
- Pause/resume sampling and confirmed process termination.

The minimum dashboard size is 60 columns by 20 rows. Smaller windows display
a resize hint; quitting still works. Core readings that do not fit the CPU
panel are clipped; the aggregate gauge includes all cores.

## Keys

| Key | Action |
| --- | --- |
| Up/Down, j/k | Move selection |
| PageUp/PageDown | Move 10 rows |
| Home/End | First/last process |
| c / m / p / n | Sort by CPU / memory / PID / name |
| Same sort key, r | Reverse sort direction |
| /, F3 | Search |
| Enter | Keep search and leave search mode |
| Esc | Clear search/status, or cancel a dialog |
| Space | Pause/resume sampling |
| x, F9 | Open termination confirmation |
| y / n | Confirm/cancel termination |
| ?, F1 | Help |
| q, F10, Ctrl+C | Quit (Ctrl+C works in dialogs/search too) |

## Metric and safety notes

Process CPU percentages are normalized to total machine capacity (0-100%),
rather than reporting up to 100% per logical core. Memory uses binary units.
Network rates aggregate all interfaces, potentially including loopback and
virtual adapters, and use the actual elapsed sample time.

Process information can be unavailable or incomplete due to OS permissions.
Unavailable command lines fall back to names; missing host/OS metadata is
explicitly labeled unknown. Swap with zero capacity is shown as unavailable.

Termination requires an explicit `y` confirmation. rtop rechecks PID, process
start time, and name immediately before signaling to reduce PID-reuse risk.
It refuses to terminate itself or PID 0. On Unix it requests graceful exit
with SIGTERM; on Windows it force-terminates the process. Unsaved work can be
lost. A successful request is not a guarantee that the process has exited;
Unix processes may ignore SIGTERM. Failures appear in the status bar.
There remains a small OS-level race between identity verification and signaling.

Terminal state is restored on normal exit, I/O errors, and Rust panics.
Forced external termination cannot guarantee terminal restoration.

## Development

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```