/////////
/// Applications to install
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

use crate::selection::{
    browser_choices, compositor_choices, editor_choices, terminal_choices, AppSelectionFlags,
};
use crate::ui::colors::PURE_WHITE;

use super::common::{aligned_summary_area, draw_install_summary, split_main_and_summary};
use super::{InstallSummary, SelectionAction, NEBULA_ART};

// Currently focused application columns
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppSelectionFocus {
    Compositors,
    Browsers,
    Editors,
    Terminals,
}

fn normalize_flags(flags: &mut Vec<bool>, len: usize) {
    flags.truncate(len);
    if flags.len() < len {
        flags.extend(std::iter::repeat(false).take(len - flags.len()));
    }
}

// Application selector UI
fn draw_application_selector(
    area: Rect,
    f: &mut Frame<'_>,
    focus: AppSelectionFocus,
    compositor_cursor: usize,
    browser_cursor: usize,
    editor_cursor: usize,
    terminal_cursor: usize,
    flags: &AppSelectionFlags,
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
            "Select packages",
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
            Span::raw(" move, "),
            Span::styled("󰁎/󰁕", Style::default().fg(Color::Cyan)),
            Span::raw(" switch column, "),
            Span::styled("Space", Style::default().fg(Color::Cyan)),
            Span::raw(" toggle."),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" confirm, "),
            Span::styled("B", Style::default().fg(Color::Cyan)),
            Span::raw(" back."),
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

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(5)])
        .split(layout[4]);

    // Multiple columns of application lists
    let columns_area = main_layout[0];
    let gap = 1u16;
    let available = columns_area.width.saturating_sub(gap * 2);
    let base = available / 3;
    let extra = available % 3;
    let mut widths = [base; 3];
    if extra > 0 {
        widths[0] += 1;
    }
    if extra > 1 {
        widths[1] += 1;
    }
    widths[2] = available.saturating_sub(widths[0] + widths[1]);
    let left_area = Rect {
        x: columns_area.x,
        y: columns_area.y,
        width: widths[0],
        height: columns_area.height,
    };
    let editor_area = Rect {
        x: columns_area.x + widths[0] + gap,
        y: columns_area.y,
        width: widths[1],
        height: columns_area.height,
    };
    let terminal_area = Rect {
        x: columns_area.x + widths[0] + gap + widths[1] + gap,
        y: columns_area.y,
        width: widths[2],
        height: columns_area.height,
    };

    let compositor_height = (compositor_choices().len() as u16) + 4;
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(compositor_height), Constraint::Min(4)])
        .split(left_area);
    let compositor_area = left_layout[0];
    let browser_area = left_layout[1];

    // --- Render Compositor List ---
    let compositor_items: Vec<ListItem> = compositor_choices()
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let is_selected = flags.compositors.get(idx).copied().unwrap_or(false);
            if is_selected {
                ListItem::new(Line::from(vec![
                    Span::styled("[󰸞]", Style::default().fg(Color::LightGreen)), // Checkbox checked
                    Span::raw(" "),
                    Span::styled(choice.label.as_str(), Style::default().fg(Color::Blue)),
                ]))
            } else {
                ListItem::new(Line::from(format!("[ ] {}", choice.label))) // Checkbox unchecked
            }
        })
        .collect();
    let compositor_active = focus == AppSelectionFocus::Compositors;
    let compositor_title_style = if compositor_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD)
    };
    let compositor_list = List::new(compositor_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(" Wayland compositor ", compositor_title_style),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut compositor_state = ListState::default();
    let compositor_len = compositor_choices().len();
    if compositor_active && compositor_len > 0 {
        compositor_state.select(Some(compositor_cursor.min(compositor_len - 1)));
    }
    f.render_stateful_widget(compositor_list, compositor_area, &mut compositor_state);

    // --- Render Browser List ---
    let browser_items: Vec<ListItem> = browser_choices()
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let is_selected = flags.browsers.get(idx).copied().unwrap_or(false);
            if is_selected {
                ListItem::new(Line::from(vec![
                    Span::styled("[󰸞]", Style::default().fg(Color::LightGreen)),
                    Span::raw(" "),
                    Span::styled(choice.label.as_str(), Style::default().fg(Color::Blue)),
                ]))
            } else {
                ListItem::new(Line::from(format!("[ ] {}", choice.label)))
            }
        })
        .collect();
    let browser_active = focus == AppSelectionFocus::Browsers;
    let browser_title_style = if browser_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD)
    };
    let browser_list = List::new(browser_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(" Web Browser ", browser_title_style),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut browser_state = ListState::default();
    if browser_active && !browser_choices().is_empty() {
        browser_state.select(Some(browser_cursor.min(browser_choices().len() - 1)));
    }
    f.render_stateful_widget(browser_list, browser_area, &mut browser_state);

    // --- Render Editor List ---
    let editor_items: Vec<ListItem> = editor_choices()
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let is_selected = flags.editors.get(idx).copied().unwrap_or(false);
            if is_selected {
                ListItem::new(Line::from(vec![
                    Span::styled("[󰸞]", Style::default().fg(Color::LightGreen)),
                    Span::raw(" "),
                    Span::styled(choice.label.as_str(), Style::default().fg(Color::Blue)),
                ]))
            } else {
                ListItem::new(Line::from(format!("[ ] {}", choice.label)))
            }
        })
        .collect();
    let editor_active = focus == AppSelectionFocus::Editors;
    let editor_title_style = if editor_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD)
    };
    let editor_list = List::new(editor_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(" Code Editor ", editor_title_style),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut editor_state = ListState::default();
    if editor_active && !editor_choices().is_empty() {
        editor_state.select(Some(editor_cursor.min(editor_choices().len() - 1)));
    }
    f.render_stateful_widget(editor_list, editor_area, &mut editor_state);

    // --- Render Terminal List ---
    let terminal_items: Vec<ListItem> = terminal_choices()
        .iter()
        .enumerate()
        .map(|(idx, choice)| {
            let is_selected = flags.terminals.get(idx).copied().unwrap_or(false);
            if is_selected {
                ListItem::new(Line::from(vec![
                    Span::styled("[󰸞]", Style::default().fg(Color::LightGreen)),
                    Span::raw(" "),
                    Span::styled(choice.label.as_str(), Style::default().fg(Color::Blue)),
                ]))
            } else {
                ListItem::new(Line::from(format!("[ ] {}", choice.label)))
            }
        })
        .collect();
    let terminal_active = focus == AppSelectionFocus::Terminals;
    let terminal_title_style = if terminal_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(PURE_WHITE).add_modifier(Modifier::BOLD)
    };

    let terminal_list = List::new(terminal_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black))
                .padding(Padding::new(1, 0, 1, 0))
                .title(Line::from(vec![
                    Span::styled("[", Style::default().fg(Color::Black)),
                    Span::styled(" Terminal ", terminal_title_style),
                    Span::styled("]", Style::default().fg(Color::Black)),
                ])),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let mut terminal_state = ListState::default();
    if terminal_active && !terminal_choices().is_empty() {
        terminal_state.select(Some(terminal_cursor.min(terminal_choices().len() - 1)));
    }
    f.render_stateful_widget(terminal_list, terminal_area, &mut terminal_state);

    // --- Render Confirmation Box ---
    let total_selected = flags
        .compositors
        .iter()
        .chain(flags.browsers.iter())
        .chain(flags.editors.iter())
        .chain(flags.terminals.iter())
        .filter(|flag| **flag)
        .count();
    let confirm_title_style = Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD);
    let confirm_text_style = Style::default().fg(Color::White);
    let confirm_lines = vec![
        Line::from(Span::styled("Press Enter to continue", confirm_text_style)),
        Line::from(Span::styled(
            format!("Selected: {total_selected} apps"),
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
    f.render_widget(confirm_block, main_layout[1]);

    let footer = Paragraph::new(Line::from(Span::styled(
        "Selections apply to this run only",
        Style::default().fg(Color::White),
    )));
    f.render_widget(footer, layout[5]);

    // Installation summary on the right side
    let summary_area = aligned_summary_area(summary_area, main_area, layout[3]);
    draw_install_summary(summary_area, f, summary);
}

// Application selector
pub fn run_application_selector(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial: &AppSelectionFlags,
    summary: &InstallSummary,
) -> Result<SelectionAction<AppSelectionFlags>> {
    let mut flags = initial.clone();
    flags.enforce_defaults();
    // Ensure flag vectors are the correct length
    normalize_flags(&mut flags.compositors, compositor_choices().len());
    normalize_flags(&mut flags.browsers, browser_choices().len());
    normalize_flags(&mut flags.editors, editor_choices().len());
    normalize_flags(&mut flags.terminals, terminal_choices().len());

    // State for the focused column and the cursor position in each column
    let mut focus = AppSelectionFocus::Browsers;
    let mut compositor_cursor = flags.compositors.iter().position(|flag| *flag).unwrap_or(0);
    let mut browser_cursor = flags.browsers.iter().position(|flag| *flag).unwrap_or(0);
    let mut editor_cursor = flags.editors.iter().position(|flag| *flag).unwrap_or(0);
    let mut terminal_cursor = flags.terminals.iter().position(|flag| *flag).unwrap_or(0);

    // Main loop for the application selection screen
    loop {
        terminal.draw(|f| {
            draw_application_selector(
                f.size(),
                f,
                focus,
                compositor_cursor,
                browser_cursor,
                editor_cursor,
                terminal_cursor,
                &flags,
                summary,
            )
        })?;

        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    // --- Focus and Navigation ---
                    KeyCode::Left => {
                        focus = match focus {
                            AppSelectionFocus::Compositors => AppSelectionFocus::Compositors,
                            AppSelectionFocus::Browsers => AppSelectionFocus::Browsers,
                            AppSelectionFocus::Editors => AppSelectionFocus::Browsers,
                            AppSelectionFocus::Terminals => AppSelectionFocus::Editors,
                        };
                    }
                    KeyCode::Right => {
                        focus = match focus {
                            AppSelectionFocus::Compositors => AppSelectionFocus::Editors,
                            AppSelectionFocus::Browsers => AppSelectionFocus::Editors,
                            AppSelectionFocus::Editors => AppSelectionFocus::Terminals,
                            AppSelectionFocus::Terminals => AppSelectionFocus::Terminals,
                        };
                    }
                    KeyCode::Up => match focus {
                        AppSelectionFocus::Compositors => {
                            if compositor_cursor > 0 {
                                compositor_cursor -= 1;
                            }
                        }
                        AppSelectionFocus::Browsers => {
                            if browser_cursor > 0 {
                                browser_cursor -= 1;
                            } else if !compositor_choices().is_empty() {
                                focus = AppSelectionFocus::Compositors;
                            }
                        }
                        AppSelectionFocus::Editors => {
                            if editor_cursor > 0 {
                                editor_cursor -= 1;
                            }
                        }
                        AppSelectionFocus::Terminals => {
                            if terminal_cursor > 0 {
                                terminal_cursor -= 1;
                            }
                        }
                    },
                    KeyCode::Down => match focus {
                        AppSelectionFocus::Compositors => {
                            if compositor_cursor + 1 < compositor_choices().len() {
                                compositor_cursor += 1;
                            } else if !browser_choices().is_empty() {
                                focus = AppSelectionFocus::Browsers;
                            }
                        }
                        AppSelectionFocus::Browsers => {
                            if browser_cursor + 1 < browser_choices().len() {
                                browser_cursor += 1;
                            }
                        }
                        AppSelectionFocus::Editors => {
                            if editor_cursor + 1 < editor_choices().len() {
                                editor_cursor += 1;
                            }
                        }
                        AppSelectionFocus::Terminals => {
                            if terminal_cursor + 1 < terminal_choices().len() {
                                terminal_cursor += 1;
                            }
                        }
                    },
                    // --- Selection and Actions ---
                    KeyCode::Char(' ') => match focus {
                        AppSelectionFocus::Compositors => {
                            if compositor_cursor < flags.compositors.len() {
                                flags.compositors.iter_mut().for_each(|flag| *flag = false);
                                flags.compositors[compositor_cursor] = true;
                            }
                        }
                        AppSelectionFocus::Browsers => {
                            if let Some(flag) = flags.browsers.get_mut(browser_cursor) {
                                *flag = !*flag;
                            }
                        }
                        AppSelectionFocus::Editors => {
                            if let Some(flag) = flags.editors.get_mut(editor_cursor) {
                                *flag = !*flag;
                            }
                        }
                        AppSelectionFocus::Terminals => {
                            if let Some(flag) = flags.terminals.get_mut(terminal_cursor) {
                                *flag = !*flag;
                            }
                        }
                    },
                    KeyCode::Enter => {
                        flags.enforce_defaults();
                        return Ok(SelectionAction::Submit(flags));
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Esc => {
                        return Ok(SelectionAction::Back);
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        return Ok(SelectionAction::Quit);
                    }
                    _ => {}
                }
            }
        }
    }
}
