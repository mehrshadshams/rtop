use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Clear, Gauge, Paragraph, Row, Sparkline, Table, Wrap},
};

use crate::app::{App, format_bytes, format_uptime, percentage};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;

pub fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
}

fn usage_color(ratio: f64) -> Color {
    if ratio >= 0.9 {
        Color::Red
    } else if ratio >= 0.65 {
        Color::Yellow
    } else {
        Color::Green
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 60 || area.height < 20 {
        frame.render_widget(
            Paragraph::new("rtop: resize terminal to at least 60 columns x 20 rows.\nq / Ctrl+C quits; ? opens help.")
                .block(panel(" rtop ")).wrap(Wrap { trim: true }),
            area,
        );
    } else {
        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);
        let state = if app.paused { "PAUSED" } else { "LIVE" };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(
                        " rtop ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        state,
                        Style::default().fg(if app.paused {
                            Color::Yellow
                        } else {
                            Color::Green
                        }),
                    ),
                    Span::raw(format!(
                        "  |  {}  |  {}",
                        safe_text(&app.host),
                        safe_text(&app.os)
                    )),
                ]),
                Line::from(format!(
                    " Uptime {}  |  {} processes  |  {} logical CPUs",
                    format_uptime(app.uptime),
                    app.processes.len(),
                    app.cores.len()
                )),
            ]),
            sections[0],
        );
        let meters = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[1]);
        draw_cpu(frame, app, meters[0]);
        let memory = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(meters[1]);
        draw_meter(frame, memory[0], " RAM ", app.used_memory, app.total_memory);
        draw_meter(frame, memory[1], " Swap ", app.used_swap, app.total_swap);
        frame.render_widget(
            Paragraph::new(format!(
                " RX {}/s  TX {}/s",
                format_bytes(app.received_per_second),
                format_bytes(app.sent_per_second)
            ))
            .style(Style::default().fg(ACCENT)),
            memory[2],
        );
        let history: Vec<u64> = app
            .history
            .iter()
            .skip(
                app.history
                    .len()
                    .saturating_sub(sections[2].width.saturating_sub(2) as usize),
            )
            .copied()
            .collect();
        frame.render_widget(
            Sparkline::default()
                .block(panel(" CPU history (0-100%) "))
                .data(&history)
                .max(100)
                .style(Style::default().fg(ACCENT)),
            sections[2],
        );
        draw_processes(frame, app, sections[3]);
        draw_footer(frame, app, sections[4]);
    }
    if app.help {
        let text = vec![
            Line::from("Up/Down, j/k      Move selection"),
            Line::from("PgUp/PgDn        Move 10 rows; Home/End jump"),
            Line::from("c / m / p / n    Sort CPU / memory / PID / name"),
            Line::from("Same sort key/r  Reverse sort direction"),
            Line::from("/ or F3          Search name, command or PID"),
            Line::from("Enter / Esc      Keep search / clear search"),
            Line::from("Space            Pause/resume metric sampling"),
            Line::from("x or F9          Confirm process termination"),
            Line::from("y / n or Esc     Confirm / cancel termination"),
            Line::from("? or F1          Help; Esc closes"),
            Line::from("q / F10 / Ctrl+C Quit"),
            Line::from(""),
            Line::from("CPU% is normalized to the whole machine."),
            Line::from("Metrics may be limited by OS permissions."),
            Line::from("Windows: force termination; Unix: SIGTERM."),
        ];
        popup(frame, " Keyboard help ", text, 66, 19, Color::Cyan);
    }
    if let Some(target) = &app.pending_kill {
        popup(
            frame,
            " Confirm termination ",
            vec![
                Line::from(format!("PID {}: {}", target.pid, safe_text(&target.name))),
                Line::from(""),
                Line::from(if cfg!(windows) {
                    "Force terminate? Unsaved work may be lost."
                } else {
                    "Send SIGTERM? Unsaved work may be lost."
                }),
                Line::from(""),
                Line::styled(
                    "y = confirm    n / Esc = cancel",
                    Style::default().fg(Color::Yellow),
                ),
            ],
            64,
            9,
            Color::Red,
        );
    }
}

fn draw_cpu(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel(format!(" CPU {:.1}% ", app.cpu));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    frame.render_widget(
        Gauge::default()
            .ratio((app.cpu as f64 / 100.0).clamp(0.0, 1.0))
            .gauge_style(Style::default().fg(usage_color(app.cpu as f64 / 100.0)))
            .label(format!("{:.1}%", app.cpu)),
        chunks[0],
    );
    let per_line = (inner.width / 12).max(1) as usize;
    let lines: Vec<Line> = app
        .cores
        .chunks(per_line)
        .take(chunks[1].height as usize)
        .enumerate()
        .map(|(row, cores)| {
            Line::from(
                cores
                    .iter()
                    .enumerate()
                    .map(|(column, usage)| {
                        Span::styled(
                            format!("{:>3}:{:>5.1}% ", row * per_line + column, usage),
                            Style::default().fg(usage_color(*usage as f64 / 100.0)),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), chunks[1]);
}

fn draw_meter(frame: &mut Frame, area: Rect, title: &'static str, used: u64, total: u64) {
    let label = if total == 0 {
        "Not available".into()
    } else {
        format!("{} / {}", format_bytes(used), format_bytes(total))
    };
    let ratio = percentage(used, total);
    frame.render_widget(
        Gauge::default()
            .block(panel(title))
            .ratio(ratio)
            .label(label)
            .gauge_style(Style::default().fg(usage_color(ratio))),
        area,
    );
}

fn draw_processes(frame: &mut Frame, app: &mut App, area: Rect) {
    let wide = area.width >= 100;
    let header = if wide {
        vec!["PID", "CPU%", "MEM%", "MEMORY", "STATE", "NAME / COMMAND"]
    } else {
        vec!["PID", "CPU%", "MEMORY", "NAME"]
    };
    let rows: Vec<Row> = app
        .visible
        .iter()
        .map(|i| {
            let p = &app.processes[*i];
            let mut cells = vec![
                Cell::from(p.target.pid.to_string()),
                Cell::from(format!("{:.1}", p.cpu))
                    .style(Style::default().fg(usage_color(p.cpu as f64 / 100.0))),
            ];
            if wide {
                cells.push(Cell::from(format!(
                    "{:.1}",
                    percentage(p.memory, app.total_memory) * 100.0
                )));
            }
            cells.push(Cell::from(format_bytes(p.memory)));
            if wide {
                cells.push(Cell::from(safe_text(&p.status)));
            }
            cells.push(Cell::from(safe_text(if wide && !p.command.is_empty() {
                &p.command
            } else {
                &p.target.name
            })));
            Row::new(cells)
        })
        .collect();
    let widths = if wide {
        vec![
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(11),
            Constraint::Length(10),
            Constraint::Min(10),
        ]
    } else {
        vec![
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(11),
            Constraint::Min(10),
        ]
    };
    let direction = if app.descending {
        "descending"
    } else {
        "ascending"
    };
    let title = format!(
        " Processes {}/{} | {} {} ",
        app.visible.len(),
        app.processes.len(),
        app.sort.label(),
        direction
    );
    let block = panel(title);
    let inner = block.inner(area);
    frame.render_stateful_widget(
        Table::new(rows, widths)
            .block(block)
            .header(
                Row::new(header).style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            )
            .row_highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> "),
        area,
        &mut app.table,
    );
    if app.visible.is_empty() && inner.height > 1 {
        frame.render_widget(
            Paragraph::new("No matching processes").style(Style::default().fg(Color::Yellow)),
            Rect {
                y: inner.y + 1,
                height: inner.height - 1,
                ..inner
            },
        );
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Some((message, error)) = &app.message {
        Line::styled(
            safe_text(message),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        )
    } else if app.searching || !app.filter.is_empty() {
        Line::styled(
            format!(
                "/{}{}",
                safe_text(&app.filter),
                if app.searching { "_" } else { "" }
            ),
            Style::default().fg(ACCENT),
        )
    } else {
        Line::from(vec![
            Span::styled("? ", Style::default().fg(ACCENT)),
            Span::raw("Help  "),
            Span::styled("/ ", Style::default().fg(ACCENT)),
            Span::raw("Search  "),
            Span::styled("c/m/p/n ", Style::default().fg(ACCENT)),
            Span::raw("Sort  "),
            Span::styled("Space ", Style::default().fg(ACCENT)),
            Span::raw("Pause  "),
            Span::styled("x ", Style::default().fg(Color::Yellow)),
            Span::raw("Terminate  "),
            Span::styled("q ", Style::default().fg(ACCENT)),
            Span::raw("Quit"),
        ])
    };
    frame.render_widget(Paragraph::new(line).block(panel(" Controls ")), area);
}

fn popup(
    frame: &mut Frame,
    title: &'static str,
    text: Vec<Line<'static>>,
    width: u16,
    height: u16,
    color: Color,
) {
    let area = frame.area();
    let width = width.min(area.width);
    let height = height.min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .block(panel(title).border_style(Style::default().fg(color)))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn sanitizes_control_characters() {
        assert_eq!(safe_text("a\x1b[31m\nb\tc"), "a [31m b c");
    }

    #[test]
    fn shows_termination_errors_with_active_filter() {
        let mut app = App::new();
        app.filter = "rust".into();
        app.message = Some(("Access denied".into(), true));
        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("Access denied"));
    }

    #[test]
    fn narrow_history_shows_newest_samples() {
        let mut app = App::new();
        app.history = std::iter::repeat_n(0, 119).chain([100]).collect();
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let history_line: String = (0..60).map(|x| buffer[(x, 10)].symbol()).collect();
        assert!(history_line.contains('\u{2588}'));
    }

    #[test]
    fn pause_state_is_visible_with_long_metadata() {
        let mut app = App::new();
        app.host = "host".repeat(30);
        app.paused = true;
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let header: String = (0..60).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(header.contains("PAUSED"));
    }

    #[test]
    fn renders_dashboard_and_small_terminals() {
        let mut app = App::new();
        for (width, height) in [(120, 32), (60, 20), (60, 18), (20, 5), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let content: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(content.contains("rtop") || width == 1);
            if width == 120 {
                assert!(content.contains("CPU history"));
                assert!(content.contains("No matching processes"));
            }
            app.help = true;
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            app.help = false;
        }
    }
}
