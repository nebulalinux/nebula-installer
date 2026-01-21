/////////
/// Network // Wi-Fi
////////
use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::colors::PURE_WHITE;
use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InstallSummary, NetworkAction, NEBULA_ART};

// Runs the "Network Required" screen, waiting for the user to retry or quit
pub fn run_network_required(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    summary: &InstallSummary,
) -> Result<NetworkAction> {
    // Main loop for the screen
    loop {
        terminal.draw(|f| draw_network_required(f.size(), f, summary))?;

        // User input
        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('r') | KeyCode::Char('R') => return Ok(NetworkAction::Retry),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(NetworkAction::Quit)
                    }
                    _ => {}
                }
            }
        }
    }
}

// "Network Required" UI
fn draw_network_required(area: Rect, f: &mut Frame<'_>, summary: &InstallSummary) {
    let (main_area, summary_area) = split_main_and_summary(area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(NEBULA_ART.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
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

    // Network required step title
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Network required",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" -/"),
    ]);
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    // Info box explaining the issue
    let info = Paragraph::new(vec![
        Line::from("A Wi-Fi device was not detected"),
        Line::from("Connect ethernet and press R to retry"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Black))
            .title(Line::from(vec![
                Span::styled("[", Style::default().fg(Color::Black)),
                Span::styled(
                    " Info ",
                    Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                ),
                Span::styled("]", Style::default().fg(Color::Black)),
            ])),
    )
    .wrap(Wrap { trim: false });
    f.render_widget(info, layout[3]);

    // Controls box
    let controls = Paragraph::new(vec![Line::from(vec![
        Span::styled("R", Style::default().fg(Color::Cyan)),
        Span::raw(" to retry, "),
        Span::styled("Ctrl+Q", Style::default().fg(Color::Cyan)),
        Span::raw(" to quit."),
    ])])
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
    f.render_widget(controls, layout[4]);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}
