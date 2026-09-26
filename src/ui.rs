use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::App;

const KEYBINDINGS: &[(&str, &str)] = &[("q / Esc", "quit"), ("?", "toggle help")];

/// Draw the whole UI. Keep this deterministic (no clock, no randomness):
/// snapshot tests depend on it.
pub fn render(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header);
    render_body(frame, body);
    render_footer(frame, footer);

    if app.show_help {
        render_help(frame, body);
    }
}

fn render_header(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(" leeroy ", Style::new().black().on_yellow().bold()),
        Span::raw(" Jenkins TUI"),
    ]);
    frame.render_widget(Paragraph::new(title).on_dark_gray(), area);
}

fn render_body(frame: &mut Frame, area: Rect) {
    let block = Block::new()
        .title(" Jobs ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue));
    let placeholder = Paragraph::new(vec![
        Line::raw(""),
        Line::styled("No Jenkins instance configured", Style::new().italic()),
    ])
    .alignment(Alignment::Center)
    .block(block);
    frame.render_widget(placeholder, area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let mut spans = Vec::new();
    for (key, desc) in KEYBINDINGS {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().black().on_blue(),
        ));
        spans.push(Span::raw(format!(" {desc}  ")));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 40, KEYBINDINGS.len() as u16 + 2);
    let lines: Vec<Line> = KEYBINDINGS
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("{key:>10}"),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {desc}")),
            ])
        })
        .collect();
    let block = Block::bordered()
        .title(" Help ")
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(Color::Yellow));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    area
}
