use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::colors::PURE_WHITE;
use super::{ReviewAction, ReviewItem, NEBULA_ART};

// Review screen, waiting for the user to confirm, go back, or quit
pub fn run_review(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    system_items: &[ReviewItem],
    package_items: &[ReviewItem],
    selected_packages: usize,
) -> Result<ReviewAction> {
    // Main loop for the review screen
    loop {
        terminal
            .draw(|f| draw_review(f.size(), f, system_items, package_items, selected_packages))?;

        // User input
        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Enter => return Ok(ReviewAction::Confirm),
                    KeyCode::Esc => return Ok(ReviewAction::Back),
                    KeyCode::Char('s') | KeyCode::Char('S') => return Ok(ReviewAction::Edit),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(ReviewAction::Quit)
                    }
                    _ => {}
                }
            }
        }
    }
}

// Review screen UI
fn draw_review(
    area: Rect,
    f: &mut Frame<'_>,
    system_items: &[ReviewItem],
    package_items: &[ReviewItem],
    selected_packages: usize,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(NEBULA_ART.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Min(6),
            Constraint::Length(5),
        ])
        .split(area);

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

    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Review installation",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" -/"),
    ]);
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    // Controls box
    let help = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" to confirm, "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to go back, "),
            Span::styled("S", Style::default().fg(Color::Cyan)),
            Span::raw(" to start over."),
        ]),
        Line::from(vec![
            Span::styled("SuperKey", Style::default().fg(Color::Cyan)),
            Span::raw(" + "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" opens a terminal, "),
            Span::styled("SuperKey", Style::default().fg(Color::Cyan)),
            Span::raw(" + "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" close terminal window"),
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

    let grid_area = layout[4];
    let gap = 1u16;
    let available = grid_area.width.saturating_sub(gap);
    let left_width = available / 2;
    let right_width = available.saturating_sub(left_width);
    let left_area = Rect {
        x: grid_area.x,
        y: grid_area.y,
        width: left_width,
        height: grid_area.height,
    };
    let right_area = Rect {
        x: grid_area.x + left_width + gap,
        y: grid_area.y,
        width: right_width,
        height: grid_area.height,
    };

    let system_block = Paragraph::new(review_lines(system_items))
        .block(review_block("System"))
        .wrap(Wrap { trim: false });
    f.render_widget(system_block, left_area);

    let packages_block = Paragraph::new(review_lines(package_items))
        .block(review_block("Packages"))
        .wrap(Wrap { trim: false });
    f.render_widget(packages_block, right_area);

    let confirm_title_style = Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD);
    let confirm_text_style = Style::default().fg(Color::White);
    let confirm_lines = vec![
        Line::from(Span::styled(
            "Press Enter to start installation process",
            confirm_text_style,
        )),
        Line::from(Span::styled(
            format!("Selected: {selected_packages} apps."),
            confirm_text_style,
        )),
    ];
    let confirm_block = Paragraph::new(confirm_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Black))
            .padding(Padding::new(1, 0, 1, 0))
            .title(Line::from(vec![
                Span::styled("[", Style::default().fg(Color::Black)),
                Span::styled(" Confirm ", confirm_title_style),
                Span::styled("]", Style::default().fg(Color::Black)),
            ])),
    );
    f.render_widget(confirm_block, layout[5]);
}

// End review boxes
fn review_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Black))
        .padding(Padding::new(1, 0, 1, 0))
        .title(Line::from(vec![
            Span::styled("[ ", Style::default().fg(Color::Black)),
            Span::styled(
                title,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ]", Style::default().fg(Color::Black)),
        ]))
}

fn review_lines(items: &[ReviewItem]) -> Vec<Line<'_>> {
    items
        .iter()
        .map(|item| {
            let icon = review_icon(&item.label);
            Line::from(vec![
                // Span::styled(
                //     " ",
                //     Style::default()
                //         .fg(Color::Black)
                //         .add_modifier(Modifier::BOLD),
                // ),
                Span::raw(" "),
                Span::styled(icon.to_string(), Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(
                    format!("{}:", item.label),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}", item.value), Style::default().fg(Color::Blue)),
            ])
        })
        .collect()
}

fn review_icon(label: &str) -> &'static str {
    match label {
        "Network" => " ",
        "Disk" => " ",
        "Filesystem" => " ",
        "GPU" => " ",
        "Swap" => " ",
        "Hostname" => " ",
        "Username" => " ",
        "Keyboard" => " ",
        "Timezone" => " ",
        "Compositor" => " ",
        "Browsers" => " ",
        "Editors" => " ",
        "Terminals" => " ",
        _ => " ",
    }
}
