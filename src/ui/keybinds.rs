use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::colors::PURE_WHITE;

const KEYBINDS: [&str; 2] = [
    "SuperKey + Enter opens a terminal",
    "SuperKey + Q close terminal window",
];
const KEYBINDS_KEYS: [&str; 3] = ["SuperKey", "Enter", "Q"];

fn styled_keybind_line(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    for part in line.split_inclusive(' ') {
        let (token, trailing_space) = match part.strip_suffix(' ') {
            Some(token) => (token, " "),
            None => (part, ""),
        };
        if KEYBINDS_KEYS.iter().any(|key| key == &token) {
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(Color::Cyan),
            ));
        } else {
            spans.push(Span::raw(token.to_string()));
        }

        if !trailing_space.is_empty() {
            spans.push(Span::raw(trailing_space));
        }
    }

    spans
}

fn keybinds_lines() -> Vec<Line<'static>> {
    KEYBINDS
        .iter()
        .map(|line| Line::from(styled_keybind_line(line)))
        .collect()
}

pub(crate) fn keybinds_height() -> u16 {
    (KEYBINDS.len() as u16).saturating_add(3)
}

pub(crate) fn draw_keybinds(area: Rect, f: &mut Frame<'_>) {
    let keybinds_block = Paragraph::new(keybinds_lines())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(
                        " Keybinds ",
                        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(keybinds_block, area);
}
