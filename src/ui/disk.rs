/////////
/// Disk selection
////////
use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::disks::DiskInfo;

use super::colors::PURE_WHITE;
use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InstallSummary, SelectionAction, NEBULA_ART};

// Disk selector
pub fn run_disk_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    disks: &[DiskInfo],
    initial: usize,
    summary: &InstallSummary,
) -> Result<SelectionAction<usize>> {
    if disks.is_empty() {
        // If there are no disks, there's nothing to do
        return Ok(SelectionAction::Quit);
    }
    let mut cursor = initial.min(disks.len() - 1);

    // Main loop for the disk selection screen
    loop {
        terminal.draw(|f| draw_disk_selector(f.size(), f, disks, cursor, summary))?;

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
                        if cursor + 1 < disks.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::Enter => return Ok(SelectionAction::Submit(cursor)),
                    KeyCode::Esc => return Ok(SelectionAction::Back),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(SelectionAction::Quit)
                    }
                    _ => {}
                }
            }
        }
    }
}

// Disk selector UI
fn draw_disk_selector(
    area: Rect,
    f: &mut Frame<'_>,
    disks: &[DiskInfo],
    cursor: usize,
    summary: &InstallSummary,
) {
    let (main_area, summary_area) = split_main_and_summary(area);
    // Layout of the main area
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(NEBULA_ART.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Min(7),
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

    // Select disk step title
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Select disk",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" -/"),
    ]);
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    // Controls box
    let help = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("󰁞/󰁆", Style::default().fg(Color::Cyan)),
            Span::raw(" to move, "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" to select, "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to go back."),
        ]),
        Line::from(vec![Span::styled(
            "Warning: selecting the wrong disk will erase its data",
            Style::default().fg(Color::White),
        )]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Black))
            .padding(Padding::new(1, 0, 1, 0))
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

    // List of available disks
    let items: Vec<ListItem> = disks
        .iter()
        .enumerate()
        .map(|(idx, disk)| {
            let line = Line::from(vec![
                Span::raw(format!("{:>2}) ", idx + 1)),
                Span::styled("󰋊  ", Style::default().fg(Color::Blue)),
                Span::raw(disk.label()),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(
                        " Disks ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    if !disks.is_empty() {
        state.select(Some(cursor));
    }
    f.render_stateful_widget(list, layout[4], &mut state);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}
