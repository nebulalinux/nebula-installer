use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::colors::PURE_WHITE;

use super::keybinds::{draw_keybinds, keybinds_height};
use super::InstallSummary;

#[derive(Clone, Copy, Debug)]
enum SummaryStatus {
    Pending,
    Current,
    Done,
}

fn summary_status_style(status: SummaryStatus) -> Style {
    match status {
        SummaryStatus::Pending => Style::default().fg(Color::White),
        SummaryStatus::Current => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        SummaryStatus::Done => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    }
}

// Builds the lines of text to be displayed in the installation summary panel
fn summary_lines(summary: &InstallSummary) -> Vec<Line<'_>> {
    let mut entries = Vec::with_capacity(9);
    entries.push(("Network", " ", summary.network.as_deref()));
    if summary.include_drivers {
        entries.push(("Drivers", " ", summary.drivers.as_deref()));
    }
    entries.extend([
        ("Disk", " ", summary.disk.as_deref()),
        ("Keymap", " ", summary.keymap.as_deref()),
        ("Timezone", " ", summary.timezone.as_deref()),
        ("Hostname", " ", summary.hostname.as_deref()),
        ("Username", " ", summary.username.as_deref()),
        ("Encryption", " ", summary.encryption.as_deref()),
        ("Zram swap", " ", summary.zram_swap.as_deref()),
    ]);
    let mut lines = Vec::with_capacity(entries.len());

    for (idx, (label, icon, value)) in entries.iter().enumerate() {
        // Determine the status of the current entry
        let status = if summary.current_index >= entries.len() {
            SummaryStatus::Done
        } else if idx < summary.current_index {
            SummaryStatus::Done
        } else if idx == summary.current_index {
            SummaryStatus::Current
        } else {
            SummaryStatus::Pending
        };

        let (prefix, show_value) = match status {
            SummaryStatus::Done => ("[OK]", true),
            SummaryStatus::Current | SummaryStatus::Pending => ("[..]", false),
        };

        let mut spans = Vec::new();
        match status {
            // Completed steps show "[OK]" and their selected value
            SummaryStatus::Done => {
                spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    icon.to_string(),
                    Style::default().fg(Color::Blue),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("{label}:"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                if show_value {
                    if let Some(value) = value {
                        spans.push(Span::styled(
                            format!(" {value}"),
                            Style::default().fg(Color::Blue),
                        ));
                    }
                }
            }
            // Current and pending steps are styled
            SummaryStatus::Current | SummaryStatus::Pending => {
                let style = summary_status_style(status);
                spans.push(Span::styled(prefix, style));
                spans.push(Span::styled(" ", style));
                spans.push(Span::styled(format!("{icon} {label}:"), style));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

// Split an area into a main content area and a summary sidebar
pub(crate) fn split_main_and_summary(area: Rect) -> (Rect, Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(74), Constraint::Percentage(26)])
        .split(area);
    (layout[0], layout[1])
}

// Vertically align the summary panel with a widget in the main area
pub(crate) fn aligned_summary_area(summary_area: Rect, main_area: Rect, anchor: Rect) -> Rect {
    let offset = anchor.y.saturating_sub(main_area.y);
    Rect {
        x: summary_area.x,
        y: summary_area.y.saturating_add(offset),
        width: summary_area.width,
        height: summary_area.height.saturating_sub(offset),
    }
}

// Renders the installation summary widget in a given area
pub(crate) fn draw_install_summary(area: Rect, f: &mut Frame<'_>, summary: &InstallSummary) {
    let lines = summary_lines(summary);
    let summary_height = (lines.len() as u16).saturating_add(3); // Add 2 for borders + 1 for top padding
    let summary_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Length(summary_height),
            Constraint::Length(keybinds_height()),
            Constraint::Min(0),
        ])
        .split(area);
    let block = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(
                        " Summary ",
                        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(block, summary_layout[0]);
    draw_keybinds(summary_layout[1], f);
}

// Filtering function for searchable lists
pub(crate) fn filter_items(items: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let needle = query.to_ascii_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(idx, zone)| {
            if zone.to_ascii_lowercase().contains(&needle) {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}
