/////////
/// Keymap selection
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

use crate::ui::colors::PURE_WHITE;

use super::common::{
    aligned_summary_area, draw_install_summary, filter_items, split_main_and_summary,
};
use super::{InstallSummary, SelectionAction, NEBULA_ART};

// Keymap selector
pub fn run_keymap_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    keymaps: &[String],
    initial: usize,
    summary: &InstallSummary,
) -> Result<SelectionAction<usize>> {
    if keymaps.is_empty() {
        return Ok(SelectionAction::Quit);
    }

    // State for the search/filter
    let mut query = String::new();
    let mut filtered = filter_items(keymaps, &query);
    let mut cursor = filtered.iter().position(|idx| *idx == initial).unwrap_or(0);

    // Main loop for the keymap selection screen
    loop {
        terminal.draw(|f| {
            draw_keymap_selector(f.size(), f, cursor, keymaps, &filtered, &query, summary)
        })?;

        // User input
        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    // Navigation controls
                    KeyCode::Up => {
                        if cursor > 0 {
                            cursor -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if cursor + 1 < filtered.len() {
                            cursor += 1;
                        }
                    }
                    KeyCode::PageUp => {
                        cursor = cursor.saturating_sub(15);
                    }
                    KeyCode::PageDown => {
                        if !filtered.is_empty() {
                            cursor = (cursor + 15).min(filtered.len() - 1);
                        }
                    }
                    KeyCode::Home => cursor = 0,
                    KeyCode::End => {
                        if !filtered.is_empty() {
                            cursor = filtered.len() - 1;
                        }
                    }
                    // Action controls
                    KeyCode::Enter => {
                        if let Some(idx) = filtered.get(cursor) {
                            // Return the index from the *original* unfiltered list
                            return Ok(SelectionAction::Submit(*idx));
                        }
                    }
                    KeyCode::Esc => return Ok(SelectionAction::Back),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(SelectionAction::Quit)
                    }
                    // Search/filter controls
                    KeyCode::Backspace => {
                        query.pop();
                        filtered = filter_items(keymaps, &query);
                        cursor = 0;
                    }
                    KeyCode::Char('/') => {
                        query.clear();
                        filtered = filter_items(keymaps, &query);
                        cursor = 0;
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.clear();
                        filtered = filter_items(keymaps, &query);
                        cursor = 0;
                    }
                    KeyCode::Char(ch) if ch.is_ascii() && !ch.is_ascii_control() => {
                        query.push(ch);
                        filtered = filter_items(keymaps, &query);
                        cursor = 0;
                    }
                    _ => {}
                }
            }
        }
    }
}

// Main keymap selector UI
fn draw_keymap_selector(
    area: Rect,
    f: &mut Frame<'_>,
    cursor: usize,
    keymaps: &[String],
    filtered: &[usize],
    query: &str,
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
            Constraint::Min(6),
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

    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Select keyboard layout",
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
            Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
            Span::raw(" to scroll, "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" to select"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
            Span::raw(" or "),
            Span::styled("/", Style::default().fg(Color::Cyan)),
            Span::raw(" clear search, "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" go back"),
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

    // Scrolling logic for the list
    let list_height = layout[4].height.saturating_sub(2) as usize;
    let window = list_height.max(1);
    let max_start = filtered.len().saturating_sub(window);
    let start = cursor.saturating_sub(window / 2).min(max_start);
    let end = (start + window).min(filtered.len());
    let visible = &filtered[start..end];

    // Create the list items from the visible part of the filtered list
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(idx, keymap_idx)| {
            let keymap = keymaps.get(*keymap_idx).map(|s| s.as_str()).unwrap_or("");
            let line = Line::from(vec![
                Span::raw(format!("{:>4}) ", start + idx + 1)),
                Span::raw(keymap),
            ]);
            ListItem::new(line)
        })
        .collect();

    // List of keymaps
    let title = format!("Keymaps ({} / {} total)", filtered.len(), keymaps.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(cursor.saturating_sub(start)));
    }
    f.render_stateful_widget(list, layout[4], &mut state);

    // Current search query at the bottom
    let query_line = format!("Search: {}", query);
    let query_widget = Paragraph::new(Line::from(Span::styled(
        query_line,
        Style::default().fg(Color::White),
    )));
    f.render_widget(query_widget, layout[5]);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}
