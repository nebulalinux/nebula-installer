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

use crate::drivers::NvidiaVariant;
use crate::ui::colors::PURE_WHITE;

use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InstallSummary, NvidiaAction, NEBULA_ART};

// NVIDIA driver selector
pub fn run_nvidia_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    summary: &InstallSummary,
) -> Result<NvidiaAction> {
    let options = [
        ("Open kernel module (Turing+)", NvidiaVariant::Open),
        ("Proprietary driver", NvidiaVariant::Proprietary),
        ("Open-source nouveau", NvidiaVariant::Nouveau),
    ];
    let mut cursor: usize = 0;

    // Main loop for the selector screen
    loop {
        terminal.draw(|f| draw_nvidia_selector(f.size(), f, cursor, &options, summary))?;

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
                        if cursor + 1 < options.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        return Ok(NvidiaAction::Select(options[cursor].1));
                    }
                    KeyCode::Esc => return Ok(NvidiaAction::Back),
                    KeyCode::Char('s') | KeyCode::Char('S') => return Ok(NvidiaAction::Skip),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(NvidiaAction::Quit);
                    }
                    _ => {}
                }
            }
        }
    }
}

// NVIDIA driver selector UI
fn draw_nvidia_selector(
    area: Rect,
    f: &mut Frame<'_>,
    cursor: usize,
    options: &[(&str, NvidiaVariant)],
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
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(main_area);

    // Nebula ASCII art
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

    // NVIDIA step title
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Choose NVIDIA Driver",
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
            Span::raw(" to select."),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to go back, "),
            Span::styled("S", Style::default().fg(Color::Cyan)),
            Span::raw(" to skip."),
        ]),
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

    // Driver options list
    let list_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(6)])
        .split(layout[4]);
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, (label, _))| ListItem::new(Line::from(format!("{:>2}) {}", idx + 1, label))))
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
                        " NVIDIA options ",
                        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
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
    state.select(Some(cursor.min(options.len().saturating_sub(1))));
    f.render_stateful_widget(list, list_layout[0], &mut state);

    let info_lines = vec![
        Line::from(vec![
            Span::styled(
                "- ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Open module:",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Open-source kernel driver for modern GPUs (Turing and newer)"),
        ]),
        Line::from(vec![
            Span::styled(
                "- ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Proprietary:",
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Fully proprietary driver. Best compatibility and performance. Support for gaming, CUDA"),
        ]),
        Line::from(vec![
            Span::styled(
                "- ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Nouveau:",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Community developed open-source driver. Limited features"),
        ]),
    ];
    let info_block = Paragraph::new(info_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
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
    f.render_widget(info_block, list_layout[1]);

    // Footer text
    let footer = Paragraph::new(Line::from(Span::styled(
        "Choose the driver variant you prefer",
        Style::default().fg(Color::White),
    )));
    f.render_widget(footer, layout[5]);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}
