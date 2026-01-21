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

use crate::ui::colors::PURE_WHITE;

use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{ConfirmAction, InstallSummary, NEBULA_ART};

// Waiting for the user to select "Yes" or "No".
pub fn run_confirm_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    warning_lines: &[Line<'_>],
    info_lines: &[Line<'_>],
    summary: &InstallSummary,
) -> Result<ConfirmAction> {
    let options = ["Yes", "No"];
    let mut cursor = 0usize;

    // Main loop for the confirmation screen
    loop {
        // Draw the UI.
        terminal.draw(|f| {
            draw_confirm_selector(
                f.size(),
                f,
                title,
                warning_lines,
                info_lines,
                cursor,
                &options,
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
                        if cursor + 1 < options.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::Enter => {
                        return Ok(if cursor == 0 {
                            ConfirmAction::Yes
                        } else {
                            ConfirmAction::No
                        })
                    }
                    KeyCode::Char('1') => return Ok(ConfirmAction::Yes),
                    KeyCode::Char('2') => return Ok(ConfirmAction::No),
                    KeyCode::Esc => return Ok(ConfirmAction::Back),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(ConfirmAction::Quit)
                    }
                    _ => {}
                }
            }
        }
    }
}

// Confirmation screen UI
fn draw_confirm_selector(
    area: Rect,
    f: &mut Frame<'_>,
    title: &str,
    warning_lines: &[Line<'_>],
    info_lines: &[Line<'_>],
    cursor: usize,
    options: &[&str],
    summary: &InstallSummary,
) {
    let (main_area, summary_area) = split_main_and_summary(area);
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

    // Confirms step titles
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            title,
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
            Span::styled("1/2", Style::default().fg(Color::Cyan)),
            Span::raw(" quick select"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to go back"),
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

    // Layout for the main content area
    let has_warning = !warning_lines.is_empty();
    let has_info = !info_lines.is_empty();
    let main_constraints = match (has_warning, has_info) {
        (true, true) => vec![
            Constraint::Percentage(45),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
        ],
        (true, false) | (false, true) => {
            vec![Constraint::Percentage(60), Constraint::Percentage(40)]
        }
        (false, false) => vec![Constraint::Percentage(100)],
    };
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(layout[4]);

    let mut section_idx = 0usize;

    // Optional warning box
    if has_warning {
        let warning = Paragraph::new(warning_lines.to_vec())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Black))
                    .padding(Padding::new(1, 0, 1, 0))
                    .title(Line::from(vec![
                        Span::styled("[", Style::default().fg(Color::Black)),
                        Span::styled(
                            " Warning ",
                            Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("]", Style::default().fg(Color::Black)),
                    ])),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(warning, main_layout[section_idx]);
        section_idx += 1;
    }

    // Optional info box
    if has_info {
        let info = Paragraph::new(info_lines.to_vec())
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
        f.render_widget(info, main_layout[section_idx]);
        section_idx += 1;
    }

    // "Yes"/"No" selection list
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, label)| {
            let line = Line::from(vec![
                Span::raw(format!("{:>2}) ", idx + 1)),
                Span::raw(*label),
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
                        " Confirm ",
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
    state.select(Some(cursor.min(options.len() - 1)));
    f.render_stateful_widget(list, main_layout[section_idx], &mut state);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}
