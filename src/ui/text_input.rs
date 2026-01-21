use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::colors::PURE_WHITE;
use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InputAction, InstallSummary, NEBULA_ART};

// Text input screen
pub fn run_text_input(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    controls: &[Line<'_>],
    info: &[Line<'_>],
    input_title: &str,
    initial: Option<&str>,
    mask: bool, // Whether to mask the input (for passwords)
    summary: &InstallSummary,
) -> Result<InputAction> {
    let mut input = initial.unwrap_or("").to_string();
    let mut cursor_visible = true;
    let mut last_toggle = Instant::now();

    // Main loop for the text input screen
    loop {
        // Toggle cursor visibility to create a blinking effect
        if last_toggle.elapsed() > Duration::from_millis(500) {
            cursor_visible = !cursor_visible;
            last_toggle = Instant::now();
        }

        // Draw the UI
        terminal.draw(|f| {
            draw_text_input(
                f.size(),
                f,
                title,
                controls,
                info,
                input_title,
                &input,
                mask,
                cursor_visible,
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
                    KeyCode::Enter => return Ok(InputAction::Submit(input.clone())),
                    KeyCode::Esc => return Ok(InputAction::Back),
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(InputAction::Quit)
                    }
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        input.clear();
                    }
                    KeyCode::Char(ch) if ch.is_ascii() && !ch.is_ascii_control() => {
                        input.push(ch);
                    }
                    _ => {}
                }
            }
        }
    }
}

// Text input UI
fn draw_text_input(
    area: Rect,
    f: &mut Frame<'_>,
    title: &str,
    controls: &[Line<'_>],
    info: &[Line<'_>],
    input_title: &str,
    input: &str,
    mask: bool,
    cursor_visible: bool,
    summary: &InstallSummary,
) {
    let (main_area, summary_area) = split_main_and_summary(area);
    let has_info = !info.is_empty();
    let use_padding = matches!(
        title,
        "Hostname"
            | "User account"
            | "User password"
            | "Confirm password"
            | "Disk encryption passphrase"
            | "Confirm passphrase"
    );
    let controls_height = if use_padding { 5 } else { 4 };
    let input_height = 3;
    let info_min_height = if use_padding { 4 } else { 3 };
    let mut layout_constraints = vec![
        Constraint::Length(NEBULA_ART.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(controls_height),
        Constraint::Length(input_height),
    ];
    if has_info {
        layout_constraints.push(Constraint::Min(info_min_height));
    }
    layout_constraints.push(Constraint::Length(1));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(layout_constraints)
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

    let title = if matches!(
        title,
        "Hostname"
            | "User account"
            | "User password"
            | "Confirm password"
            | "Disk encryption passphrase"
            | "Confirm passphrase"
            | "Wi-Fi password"
    ) {
        Line::from(vec![
            Span::raw("/- "),
            Span::styled(
                title,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" -/"),
        ])
    } else {
        Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        )])
    };
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    let mut help_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Black))
        .title(Line::from(vec![
            Span::styled("[", Style::default().fg(Color::Black)),
            Span::styled(
                " Controls ",
                Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
            ),
            Span::styled("]", Style::default().fg(Color::Black)),
        ]));
    if use_padding {
        help_block = help_block.padding(Padding::new(1, 0, 1, 0));
    }
    let help_block = Paragraph::new(controls.to_vec())
        .block(help_block)
        .wrap(Wrap { trim: false });
    f.render_widget(help_block, layout[3]);

    // Show the input string, masked if necessary with a blinking cursor
    let mut shown = if mask {
        "*".repeat(input.len())
    } else {
        input.to_string()
    };
    if cursor_visible {
        shown.push('|');
    }
    let input_title_line = if matches!(
        input_title,
        "Hostname"
            | "Username"
            | "Password"
            | "Encryption passphras"
            | "Re-enter password"
            | "Re-enter encryption passphras"
            | "Wi-Fi password"
    ) {
        Line::from(vec![
            Span::styled("[", Style::default().fg(Color::Black)),
            Span::styled(
                format!(" {} ", input_title),
                Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
            ),
            Span::styled("]", Style::default().fg(Color::Black)),
        ])
    } else {
        Line::from(Span::raw(input_title))
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Black))
        .title(input_title_line);
    let input_block = Paragraph::new(Line::from(Span::styled(
        shown,
        Style::default().fg(Color::Yellow),
    )))
    .block(input_block);
    f.render_widget(input_block, layout[4]);

    // Optionally, draw an info box
    let status_idx = if has_info {
        let mut info_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Black))
            .title(Line::from(vec![
                Span::styled("[", Style::default().fg(Color::Black)),
                Span::styled(
                    " Info ",
                    Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                ),
                Span::styled("]", Style::default().fg(Color::Black)),
            ]));
        if use_padding {
            info_block = info_block.padding(Padding::new(1, 0, 1, 0));
        }
        let info_block = Paragraph::new(info.to_vec())
            .block(info_block)
            .wrap(Wrap { trim: false });
        f.render_widget(info_block, layout[5]);
        6
    } else {
        5
    };

    let status = Paragraph::new(Line::from(Span::styled(
        "Press Enter to submit.",
        Style::default().fg(Color::White),
    )));
    f.render_widget(status, layout[status_idx]);

    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}

// Non-interactive version of the text input UI
pub fn render_text_input(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    title: &str,
    controls: &[Line<'_>],
    info: &[Line<'_>],
    input_title: &str,
    input: &str,
    mask: bool,
    summary: &InstallSummary,
) -> Result<()> {
    terminal.draw(|f| {
        draw_text_input(
            f.size(),
            f,
            title,
            controls,
            info,
            input_title,
            input,
            mask,
            false, // Cursor is not visible in the non-interactive version
            summary,
        )
    })?;
    Ok(())
}
