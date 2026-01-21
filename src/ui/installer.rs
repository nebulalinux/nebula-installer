/////////
/// Installation progress screen
////////
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
use ratatui::Frame;

use crate::model::{App, Step, StepStatus};
use crate::ui::colors::PURE_WHITE;

use super::{NEBULA_ART, SPINNER};

// Installation progress UI
pub fn draw_ui(area: Rect, f: &mut Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(NEBULA_ART.len() as u16), // ASCII art
            Constraint::Length(1),                       // Spacer
            Constraint::Length(1),                       // Title
            Constraint::Length(4),                       // Progress bar
            Constraint::Length(app.steps.len() as u16 + 2), // Installation steps
            Constraint::Min(6),                          // Logs
            Constraint::Length(1),                       // Final status
        ])
        .split(area);

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

    // Installer step title
    let title = Line::from(vec![
        Span::raw("/- "),
        Span::styled(
            "Installer",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" -/"),
    ]);
    let title_block = Paragraph::new(title).block(Block::default());
    f.render_widget(title_block, layout[1]);

    // Overall progress bar
    let progress = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Span::styled(
                    "Progress",
                    Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                )),
        )
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(app.progress);
    f.render_widget(progress, layout[3]);

    // List of installation steps
    let step_lines: Vec<Line> = app
        .steps
        .iter()
        .map(|step| render_step(step, app.spinner_idx))
        .collect();
    let steps = Paragraph::new(step_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Span::styled(
                    "Steps",
                    Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(steps, layout[4]);

    // Log output panel
    let log_lines: Vec<Line> = app
        .logs
        .iter()
        .map(|line| Line::from(Span::raw(line.clone())))
        .collect();
    let log_height = layout[5].height.saturating_sub(2) as usize;
    let scroll_offset = log_lines.len().saturating_sub(log_height);
    let scroll_offset = scroll_offset.min(u16::MAX as usize) as u16;
    let logs = Paragraph::new(log_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .title(Span::styled(
                    "Logs",
                    Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0));
    f.render_widget(logs, layout[5]);

    // Final status message at the bottom when the installation is done
    let status_line = if app.done {
        if app.err.is_some() {
            Line::from(Span::styled(
                "Installation failed.",
                Style::default().fg(Color::LightRed),
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    "Installation complete.",
                    Style::default().fg(Color::LightGreen),
                ),
                Span::raw(" "),
                Span::styled(
                    "Press B to reboot.",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        }
    } else {
        Line::from("")
    };
    let status_line = Paragraph::new(status_line);
    f.render_widget(status_line, layout[6]);
}

fn render_step(step: &Step, spinner_idx: usize) -> Line<'static> {
    let icon = match step.status {
        StepStatus::Pending => "[ ]",
        StepStatus::Running => "[..]",
        StepStatus::Done => "[OK]",
        StepStatus::Skipped => "[SKIP]",
        StepStatus::Failed => "[x]",
    };

    // Style the line based on the status
    let mut spans = vec![Span::styled(
        format!("{} {}", icon, step.name),
        style_for_status(step.status),
    )];

    // Add a spinner if the step is currently running
    if step.status == StepStatus::Running {
        spans.push(Span::raw(format!(" {}", SPINNER[spinner_idx])));
    }

    // Add an error message if the step failed
    if let Some(err) = &step.err {
        spans.push(Span::styled(
            format!(" ({})", err),
            Style::default().fg(Color::Red),
        ));
    }

    Line::from(spans)
}

// Returns a style (color) for a given step status
fn style_for_status(status: StepStatus) -> Style {
    match status {
        StepStatus::Pending => Style::default().fg(Color::White),
        StepStatus::Running => Style::default().fg(Color::Yellow),
        StepStatus::Done => Style::default().fg(Color::Green),
        StepStatus::Skipped => Style::default().fg(Color::Yellow),
        StepStatus::Failed => Style::default().fg(Color::Red),
    }
}
