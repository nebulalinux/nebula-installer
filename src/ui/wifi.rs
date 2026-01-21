use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::network::WifiNetwork;

use super::colors::PURE_WHITE;
use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InstallSummary, WifiAction, NEBULA_ART};

// Wi-Fi selector
pub fn run_wifi_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    networks: &[WifiNetwork],
    status: Option<&str>,
    wifi_connected: bool,
    internet_ready: bool,
    summary: &InstallSummary,
) -> Result<WifiAction> {
    let mut cursor = 0usize;
    let last_refresh = Instant::now();
    // Main loop for the Wi-Fi selection screen
    loop {
        // Draw the UI
        terminal.draw(|f| {
            draw_wifi_selector(
                f.size(),
                f,
                cursor,
                networks,
                status,
                wifi_connected,
                internet_ready,
                false,
                None,
                summary,
            )
        })?;

        // User input
        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Up => {
                        if cursor > 0 {
                            cursor -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if cursor + 1 < networks.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if !networks.is_empty() {
                            return Ok(WifiAction::Submit(cursor));
                        }
                    }
                    KeyCode::Char('1') => {
                        if internet_ready {
                            return Ok(WifiAction::Continue);
                        }
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => return Ok(WifiAction::Rescan),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(WifiAction::Quit)
                    }
                    _ => {}
                }
            }
        } else if wifi_connected
            && !internet_ready
            && last_refresh.elapsed() >= Duration::from_secs(1)
        {
            // Refresh if connected to Wi-Fi but no internet, to check for IP assignment
            return Ok(WifiAction::Refresh);
        }
    }
}

// Wi-Fi selector UI
fn draw_wifi_selector(
    area: Rect,
    f: &mut Frame<'_>,
    cursor: usize,
    networks: &[WifiNetwork],
    status: Option<&str>,
    wifi_connected: bool,
    internet_ready: bool,
    searching: bool,
    connecting_spinner: Option<&str>,
    summary: &InstallSummary,
) {
    let (main_area, summary_area) = split_main_and_summary(area);
    // Layout of the main area
    let mut constraints = vec![
        Constraint::Length(NEBULA_ART.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Min(6),
        Constraint::Length(3),
    ];
    let next_step_idx = if internet_ready {
        constraints.push(Constraint::Length(3));
        Some(6)
    } else {
        None
    };
    let status_line_idx = if internet_ready { 7 } else { 6 };
    constraints.push(Constraint::Length(1));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(constraints)
        .split(main_area);

    // Draw the Nebula ASCII art
    let art_lines: Vec<Line> = NEBULA_ART
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                *line,
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    let art = Paragraph::new(art_lines).block(Block::default());
    f.render_widget(art, layout[0]);

    // Select Wi-Fi network step title
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Select Wi-Fi network",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" -/"),
    ]);
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    // Controls box
    let mut help_lines = vec![Line::from(vec![
        Span::styled("󰁞/󰁆", Style::default().fg(Color::Cyan)),
        Span::raw(" to move, "),
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(" to connect"),
    ])];
    let mut rescan_line = vec![
        Span::styled("R", Style::default().fg(Color::Cyan)),
        Span::raw(" to rescan"),
    ];
    if internet_ready {
        rescan_line.push(Span::raw(", "));
        rescan_line.push(Span::styled("1", Style::default().fg(Color::Cyan)));
        rescan_line.push(Span::raw(" to continue"));
    }
    help_lines.push(Line::from(rescan_line));
    if let Some(status) = status {
        help_lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Red)),
            Span::raw(status),
        ]));
    }
    let help = Paragraph::new(help_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(
                        " Controls ",
                        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(help, layout[3]);

    // List of Wi-Fi networks
    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Black))
        .title(Span::styled(
            "Wi-Fi networks",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    if searching {
        let searching_line = Line::from(Span::styled(
            "Searching...",
            Style::default().fg(Color::White),
        ));
        let searching_block = Paragraph::new(searching_line)
            .block(list_block)
            .wrap(Wrap { trim: false });
        f.render_widget(searching_block, layout[4]);
    } else {
        let items: Vec<ListItem> = networks
            .iter()
            .enumerate()
            .map(|(idx, network)| {
                let in_use = if network.in_use { "*" } else { " " };
                let signal = format!("{:>3}%", network.signal);
                let security = if network.is_open() {
                    "open".to_string()
                } else if network.security.is_empty() {
                    "secured".to_string()
                } else {
                    network.security.clone()
                };
                let line = Line::from(vec![
                    Span::raw(format!("{:>2}) ", idx + 1)),
                    Span::raw(in_use),
                    Span::raw(" "),
                    Span::styled("󰤨 ", Style::default().fg(Color::LightBlue)),
                    Span::styled(&network.ssid, Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(signal, Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(security, Style::default().fg(Color::White)),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).block(list_block).highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let mut state = ListState::default();
        if !networks.is_empty() {
            state.select(Some(cursor.min(networks.len() - 1)));
        }
        f.render_stateful_widget(list, layout[4], &mut state);
    }

    // Connection status box
    let mut status_lines = Vec::new();
    if let Some(spinner) = connecting_spinner {
        status_lines.push(Line::from(Span::styled(
            format!("Connecting... {}", spinner),
            Style::default().fg(Color::Green),
        )));
    } else if wifi_connected {
        status_lines.push(Line::from(Span::styled(
            "Successfully connected to Wi-Fi.",
            Style::default().fg(Color::Green),
        )));
    } else {
        status_lines.push(Line::from(Span::styled(
            "Not connected to any Wi-Fi network.",
            Style::default().fg(Color::Red),
        )));
    }

    let status_block = Paragraph::new(status_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(
                        " Status ",
                        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(status_block, layout[5]);

    // If internet is ready, show a "Next step" box
    if let Some(next_step_idx) = next_step_idx {
        let mut next_step_lines = Vec::new();
        if internet_ready {
            next_step_lines.push(Line::from(vec![
                Span::raw("Internet is ready. Press "),
                Span::styled("[1]", Style::default().fg(Color::Cyan)),
                Span::raw(" to continue to disk selection."),
            ]));
        } else {
            next_step_lines.push(Line::from(""));
        }
        let next_step_block = Paragraph::new(next_step_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Black))
                    .title(Line::from(vec![
                        Span::styled("[", Style::default().fg(Color::Black)),
                        Span::styled(
                            " Next Step ",
                            Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("]", Style::default().fg(Color::Black)),
                    ])),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(next_step_block, layout[next_step_idx]);
    }

    // Status line at the bottom
    let status_line = Paragraph::new(Line::from(Span::styled(
        "Enter to connect, R to rescan.",
        Style::default().fg(Color::White),
    )));
    f.render_widget(status_line, layout[status_line_idx]);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}

// "Searching for networks..."
pub fn render_wifi_searching(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    status: Option<&str>,
    wifi_connected: bool,
    internet_ready: bool,
    summary: &InstallSummary,
) -> Result<()> {
    terminal.draw(|f| {
        draw_wifi_selector(
            f.size(),
            f,
            0,
            &[],
            status,
            wifi_connected,
            internet_ready,
            true,
            None,
            summary,
        )
    })?;
    Ok(())
}

pub fn render_wifi_connecting(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cursor: usize,
    networks: &[WifiNetwork],
    status: Option<&str>,
    wifi_connected: bool,
    internet_ready: bool,
    summary: &InstallSummary,
    spinner: &str,
) -> Result<()> {
    terminal.draw(|f| {
        draw_wifi_selector(
            f.size(),
            f,
            cursor,
            networks,
            status,
            wifi_connected,
            internet_ready,
            false,
            Some(spinner),
            summary,
        )
    })?;
    Ok(())
}
