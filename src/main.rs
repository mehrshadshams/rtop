mod app;
mod ui;

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use app::{App, format_bytes, format_uptime};
use crossterm::event::{self, Event, KeyEventKind};

const HELP: &str = "\
rtop - rich-text terminal system monitor

Usage: rtop [--once] [--interval MILLISECONDS]

  --once       Print one live snapshot without a terminal UI
  --interval   Refresh interval, 250..60000 ms (default: 1000)
  -h, --help   Show this help

Keys: arrows/j/k navigate; c/m/p/n sort CPU/memory/PID/name; r reverse;
      / search; Space pause; x/F9 terminate (y confirms); ? help; q quit.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    once: bool,
    interval: Duration,
}

fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        once: false,
        interval: Duration::from_secs(1),
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--once" => options.once = true,
            "--interval" => {
                let ms = args
                    .next()
                    .ok_or("--interval requires a value in milliseconds")?
                    .parse::<u64>()
                    .map_err(|_| "--interval must be an integer")?;
                if !(250..=60_000).contains(&ms) {
                    return Err("--interval must be between 250 and 60000 ms".into());
                }
                options.interval = Duration::from_millis(ms);
            }
            _ => return Err(format!("Unknown argument: {arg}. Use --help for usage.")),
        }
    }
    Ok(Some(options))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rtop: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(options) = parse_options(std::env::args().skip(1))? else {
        println!("{HELP}");
        return Ok(());
    };
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return Err("System monitoring is not supported on this platform".into());
    }
    if !options.once && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
        return Err("Interactive mode requires a terminal. Use --once for a text snapshot.".into());
    }
    let mut app = App::new();
    // CPU usage is a delta: seed counters, then wait for a meaningful first sample.
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(250)));
    app.refresh();
    if options.once {
        print_snapshot(&app)?;
        return Ok(());
    }
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            ratatui::restore();
            return Err(error.into());
        }
    };
    let result = event_loop(&mut terminal, &mut app, options.interval);
    let restore = ratatui::try_restore();
    result?;
    restore?;
    Ok(())
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    interval: Duration,
) -> io::Result<()> {
    let mut last_refresh = Instant::now();
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        let timeout = interval
            .saturating_sub(last_refresh.elapsed())
            .min(Duration::from_millis(100));
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if app.handle_key(key) {
                        return Ok(());
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if last_refresh.elapsed() >= interval {
            if !app.paused {
                app.refresh();
            }
            last_refresh = Instant::now();
        }
    }
}

fn print_snapshot(app: &App) -> io::Result<()> {
    use io::Write;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(
        out,
        "rtop | {} | {}",
        ui::safe_text(&app.host),
        ui::safe_text(&app.os)
    )?;
    writeln!(
        out,
        "CPU {:.1}% ({} cores) | RAM {} / {} | Swap {} / {}",
        app.cpu,
        app.cores.len(),
        format_bytes(app.used_memory),
        format_bytes(app.total_memory),
        format_bytes(app.used_swap),
        format_bytes(app.total_swap),
    )?;
    writeln!(
        out,
        "Uptime {} | Processes {} | Network RX {}/s TX {}/s",
        format_uptime(app.uptime),
        app.processes.len(),
        format_bytes(app.received_per_second),
        format_bytes(app.sent_per_second),
    )?;
    writeln!(out, "{:>8} {:>7} {:>12}  NAME", "PID", "CPU%", "MEMORY")?;
    for i in app.visible.iter().take(20) {
        let p = &app.processes[*i];
        writeln!(
            out,
            "{:>8} {:>7.1} {:>12}  {}",
            p.target.pid,
            p.cpu,
            format_bytes(p.memory),
            ui::safe_text(&p.target.name),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_refresh_interval() {
        for value in ["0", "249", "60001", "oops"] {
            assert!(parse_options(["--interval".into(), value.into()]).is_err());
        }
        assert!(parse_options(["--interval".into()]).is_err());
        assert!(parse_options(["--unknown".into()]).is_err());
        assert_eq!(
            parse_options(["--once".into(), "--interval".into(), "250".into()]),
            Ok(Some(Options {
                once: true,
                interval: Duration::from_millis(250)
            }))
        );
    }
}
