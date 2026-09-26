use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::{
    app::{App, ConnectionStatus, SettingsState, StatusMessage, View},
    config::SettingKey,
    event::{Binding, GLOBAL_BINDINGS, context_bindings},
};

/// Draw the whole UI. Keep this deterministic (no clock, no randomness):
/// snapshot tests depend on it.
pub fn render(frame: &mut Frame, app: &App) {
    let [header, body, context_bar, global_bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    match app.view {
        View::Jobs => render_jobs(frame, body, app),
        View::Settings => render_settings(frame, body, &app.settings),
    }
    render_context_bar(frame, context_bar, app);
    render_global_bar(frame, global_bar, app);

    if app.show_help {
        render_help(frame, body, app);
    }
}

/// Header: app name + the Jenkins instance we're connected to.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" Leeroy ", Style::new().black().on_yellow().bold()),
        Span::raw(" "),
    ];
    spans.extend(match &app.connection {
        ConnectionStatus::NotConfigured => [
            Span::styled("○", Style::new().red()),
            Span::raw(" not connected"),
        ],
        ConnectionStatus::Connected { url } => [
            Span::styled("●", Style::new().green()),
            Span::raw(format!(" {url}")),
        ],
    });
    frame.render_widget(Paragraph::new(Line::from(spans)).on_dark_gray(), area);
}

fn view_block(title: &str) -> Block<'_> {
    Block::new()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
}

fn render_jobs(frame: &mut Frame, area: Rect, app: &App) {
    let lines = match app.settings.effective().get(SettingKey::JenkinsUrl) {
        None => vec![
            Line::raw(""),
            Line::styled("No Jenkins instance configured", Style::new().italic()),
            Line::styled("Press s to open settings", Style::new().dark_gray()),
        ],
        Some(url) => vec![
            Line::raw(""),
            Line::styled(format!("Jenkins instance: {url}"), Style::new().italic()),
            Line::styled(
                "Connecting is not implemented yet",
                Style::new().dark_gray(),
            ),
        ],
    };
    let placeholder = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(view_block("Jobs"));
    frame.render_widget(placeholder, area);
}

const SECRET_MASK: &str = "••••••••";
const LABEL_WIDTH: usize = 14;

fn render_settings(frame: &mut Frame, area: Rect, s: &SettingsState) {
    let dim = Style::new().dark_gray();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("   {:<LABEL_WIDTH$}", "Config file"), dim),
            Span::raw(format!("  {}", s.path.display())),
        ]),
        Line::raw(""),
    ];

    let effective = s.effective();
    for (i, key) in SettingKey::ALL.into_iter().enumerate() {
        let selected = i == s.selected;
        let marker = if selected { "▶" } else { " " };
        let label_style = if selected {
            Style::new().yellow().bold()
        } else {
            Style::new()
        };
        let mut row = vec![Span::styled(
            format!(" {marker} {:<LABEL_WIDTH$}", key.label()),
            label_style,
        )];

        match (&s.editing, selected) {
            (Some(buf), true) => {
                // While typing a secret, show one bullet per char so progress is visible.
                let shown = if key.is_secret() {
                    "•".repeat(buf.chars().count())
                } else {
                    buf.clone()
                };
                row.push(Span::raw("  "));
                row.push(Span::styled(shown, Style::new().yellow().underlined()));
                row.push(Span::styled("█", Style::new().yellow()));
            }
            _ => {
                row.push(Span::raw("  "));
                row.push(match effective.get(key) {
                    None => Span::styled("(not set)", dim),
                    // Fixed-width mask: doesn't leak the secret's length.
                    Some(_) if key.is_secret() => Span::raw(SECRET_MASK),
                    Some(value) => Span::raw(value.to_owned()),
                });
            }
        }
        if s.is_overridden(key) {
            row.push(Span::styled(format!("  (from ${})", key.env_var()), dim));
        }
        lines.push(Line::from(row));
    }

    if let Some(message) = &s.message {
        lines.push(Line::raw(""));
        lines.push(match message {
            StatusMessage::Info(text) => Line::styled(format!("   {text}"), Style::new().green()),
            StatusMessage::Error(text) => Line::styled(format!("   {text}"), Style::new().red()),
        });
    }

    frame.render_widget(Paragraph::new(lines).block(view_block("Settings")), area);
}

/// Keybindings as `key desc` pairs, keys highlighted with `key_style`.
fn binding_spans(bindings: &[Binding], key_style: Style) -> Vec<Span<'static>> {
    bindings
        .iter()
        .flat_map(|b| {
            [
                Span::styled(format!(" {} ", b.key), key_style),
                Span::raw(format!(" {}  ", b.desc)),
            ]
        })
        .collect()
}

/// Context bar: what has focus + the keys that only work there.
fn render_context_bar(frame: &mut Frame, area: Rect, app: &App) {
    let context = app.context();
    let mut spans = vec![
        Span::styled(
            format!(" {} ", context.title()),
            Style::new().black().on_magenta().bold(),
        ),
        Span::raw(" "),
    ];
    spans.extend(binding_spans(
        context_bindings(context),
        Style::new().black().on_blue(),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Global bar: application-wide commands. Dimmed while typing text, since
/// the keys then go to the input instead.
fn render_global_bar(frame: &mut Frame, area: Rect, app: &App) {
    let captured = app.context().captures_input();
    let key_style = if captured {
        Style::new().black().on_gray()
    } else {
        Style::new().black().on_yellow()
    };
    let mut commands = Line::from(binding_spans(GLOBAL_BINDINGS, key_style));
    if captured {
        commands = commands.gray().italic();
    }
    frame.render_widget(Paragraph::new(commands).on_dark_gray(), area);
}

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let key_style = Style::new().add_modifier(Modifier::BOLD);
    let section = |title: &str| Line::styled(format!(" {title}"), Style::new().yellow().bold());
    let rows = |bindings: &[Binding]| -> Vec<Line<'static>> {
        bindings
            .iter()
            .map(|b| {
                Line::from(vec![
                    Span::styled(format!("{:>8}", b.key), key_style),
                    Span::raw(format!("  {}", b.desc)),
                ])
            })
            .collect()
    };

    // Describe the view underneath the popup, not the popup itself.
    let view_context = app.view.context();
    let mut lines = vec![section("Global")];
    lines.extend(rows(GLOBAL_BINDINGS));
    let view_bindings = context_bindings(view_context);
    if !view_bindings.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section(view_context.title()));
        lines.extend(rows(view_bindings));
    }

    let popup = centered(area, 40, lines.len() as u16 + 2);
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
