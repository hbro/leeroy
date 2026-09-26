use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
    },
};

use crate::{
    app::{App, ConnectionStatus, SettingsRow, SettingsState, StatusMessage, Tab, View},
    builds::{Build, BuildLoad, BuildRef, format_duration},
    config::{DEFAULT_REFRESH_SECS, HEADERS_DOC, SettingKey, header_env_var, redact_url},
    console::ConsoleLoad,
    event::{Binding, GLOBAL_BINDINGS, context_bindings},
    jobs::{JobStatus, JobsLoad},
    proxy::SystemProxy,
};

/// Draw the whole UI. Keep this deterministic (no clock, no randomness):
/// snapshot tests depend on it.
pub fn render(frame: &mut Frame, app: &App) {
    let [header, tabs, body, context_bar, global_bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_tabs(frame, tabs, app);
    match app.view {
        View::Jobs => render_jobs(frame, body, app),
        View::Build => render_build(frame, body, app),
        View::Console => render_console(frame, body, app),
        View::Settings => render_settings(frame, body, &app.settings),
    }
    render_context_bar(frame, context_bar, app);
    render_global_bar(frame, global_bar, app);

    if app.show_help {
        render_help(frame, body, app);
    }
    if app.confirm_quit {
        render_confirm_quit(frame, body);
    }
}

/// Header: app name + the Jenkins instance we're connected to.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" Leeroy ", Style::new().black().on_yellow().bold()),
        Span::raw(" "),
    ];
    // First, so it's never cut off on narrow terminals.
    if app
        .settings
        .connection_config()
        .is_some_and(|c| c.skip_tls_verify)
    {
        spans.push(Span::styled(
            "⚠ TLS NOT VERIFIED",
            Style::new().white().on_red().bold(),
        ));
        spans.push(Span::raw(" "));
    }
    let dim = Style::new().gray();
    spans.extend(match &app.connection {
        ConnectionStatus::NotConfigured => vec![
            Span::styled("○", Style::new().red()),
            Span::raw(" not connected"),
        ],
        ConnectionStatus::Connecting { url } => vec![
            Span::styled("◌", Style::new().yellow()),
            Span::raw(format!(" {url}")),
            Span::styled(" · connecting…", dim),
        ],
        ConnectionStatus::Connected { url, info } => {
            let mut spans = vec![
                Span::styled("●", Style::new().green()),
                Span::raw(format!(" {url}")),
            ];
            if let Some(version) = &info.version {
                spans.push(Span::styled(format!(" · Jenkins {version}"), dim));
            }
            spans.push(Span::styled(format!(" · {}", info.user), dim));
            spans
        }
        ConnectionStatus::Failed { url, error } => vec![
            Span::styled("✕", Style::new().red().bold()),
            Span::raw(format!(" {url}")),
            Span::styled(format!(" · {error}"), Style::new().red()),
        ],
    });
    // Refresh status on the far right; the connection info gets the rest.
    let status = refresh_status(app);
    let [left, right] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(status.width() as u16),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)).on_dark_gray(), left);
    frame.render_widget(Paragraph::new(status).on_dark_gray(), right);
}

fn view_block(title: &str) -> Block<'static> {
    Block::new()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
}

/// Tab bar: `F1 Jobs  F2 …`, the active one highlighted. Settings isn't a
/// tab but shows up as active while open, so it's clear where you are.
fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let active = Style::new().black().on_blue().bold();
    let inactive = Style::new().gray();
    let mut spans = vec![Span::raw(" ")];
    for tab in Tab::ALL {
        let style = if app.view.tab() == Some(tab) {
            active
        } else {
            inactive
        };
        spans.push(Span::styled(
            format!(" F{} {} ", tab.f_key(), tab.title()),
            style,
        ));
        spans.push(Span::raw(" "));
    }
    if app.view == View::Settings {
        spans.push(Span::styled(" Settings ", active));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_jobs(frame: &mut Frame, area: Rect, app: &App) {
    let hint = Style::new().dark_gray();
    let centered_message = |lines: Vec<Line<'static>>| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block("Jobs"))
    };
    let lines = match &app.connection {
        ConnectionStatus::NotConfigured => vec![
            Line::raw(""),
            Line::styled("No Jenkins instance configured", Style::new().italic()),
            Line::styled("Press s to open settings", hint),
        ],
        ConnectionStatus::Connecting { url } => vec![
            Line::raw(""),
            Line::styled(format!("Connecting to {url}…"), Style::new().italic()),
        ],
        ConnectionStatus::Failed { url, error } => vec![
            Line::raw(""),
            Line::styled(format!("Cannot reach {url}"), Style::new().italic()),
            Line::styled(error.clone(), Style::new().red()),
            Line::styled("Check the settings (s), or press r to retry", hint),
        ],
        ConnectionStatus::Connected { .. } => match &app.jobs.load {
            JobsLoad::NotLoaded | JobsLoad::Loading => vec![
                Line::raw(""),
                Line::styled("Loading jobs…", Style::new().italic()),
            ],
            JobsLoad::Failed(error) => vec![
                Line::raw(""),
                Line::styled("Could not load jobs", Style::new().italic()),
                Line::styled(error.clone(), Style::new().red()),
                Line::styled("Press r to retry", hint),
            ],
            JobsLoad::Loaded(_) => return render_job_list(frame, area, app),
        },
    };
    frame.render_widget(centered_message(lines), area);
}

fn render_job_list(frame: &mut Frame, area: Rect, app: &App) {
    let jobs = &app.jobs;
    let visible = jobs.visible();
    let mut title = if jobs.active_filter().trim().is_empty() {
        format!("Jobs ({})", jobs.total())
    } else {
        format!("Jobs ({}/{})", visible.len(), jobs.total())
    };
    if jobs.refreshing {
        title.push_str(" · refreshing…");
    }
    let block = view_block(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Filter line on top while typing or while a filter is applied.
    let show_filter = jobs.filter_input.is_some() || !jobs.filter.is_empty();
    let [filter_area, list_area] = Layout::vertical([
        Constraint::Length(u16::from(show_filter)),
        Constraint::Min(0),
    ])
    .areas(inner);
    if let Some(input) = &jobs.filter_input {
        let mut spans = vec![Span::styled(" / ", Style::new().yellow().bold())];
        let shown: Vec<char> = input.value().chars().collect();
        spans.extend(input_spans(&shown, input.cursor()));
        frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
    } else if show_filter {
        let line = Line::from(vec![
            Span::styled(" filter: ", Style::new().dark_gray()),
            Span::styled(jobs.filter.clone(), Style::new().yellow()),
        ]);
        frame.render_widget(Paragraph::new(line), filter_area);
    }

    if visible.is_empty() {
        let text = if jobs.total() == 0 {
            "This Jenkins instance has no jobs".to_owned()
        } else {
            format!("No jobs match \"{}\"", jobs.active_filter().trim())
        };
        let message = Paragraph::new(vec![
            Line::raw(""),
            Line::styled(text, Style::new().italic()),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(message, list_area);
        return;
    }

    let rows = visible.iter().enumerate().map(|(i, job)| {
        let (symbol, mut color) = status_symbol(job.status);
        // Text in the highlight colour would vanish on the selected row.
        if i == jobs.selected && color == SELECTED_BG {
            color = SELECTED_FG_ON_BG;
        }
        Row::new(vec![
            Cell::from(Span::styled(symbol, Style::new().fg(color))),
            Cell::from(Span::styled(job.status.label(), Style::new().fg(color))),
            Cell::from(if job.building {
                Span::styled("⟳", Style::new().yellow().bold())
            } else {
                Span::raw("")
            }),
            Cell::from(job.full_name.as_str()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(9),
            Constraint::Length(1),
            Constraint::Min(0),
        ],
    )
    .column_spacing(1)
    .row_highlight_style(Style::new().bg(SELECTED_BG).bold())
    .highlight_symbol("▶ ")
    .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default().with_selected(Some(jobs.selected));
    frame.render_stateful_widget(table, list_area, &mut state);
}

fn render_build(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = &app.build else {
        return;
    };
    let hint = Style::new().dark_gray();
    let message = |lines: Vec<Line<'static>>, title: &str| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(title))
    };
    let what = match view.target {
        BuildRef::Latest => "last build".to_owned(),
        BuildRef::Number(n) => format!("build #{n}"),
    };
    let build = match &view.load {
        BuildLoad::Loading => {
            let lines = vec![
                Line::raw(""),
                Line::styled(format!("Loading {what}…"), Style::new().italic()),
            ];
            return frame.render_widget(message(lines, &view.job), area);
        }
        BuildLoad::Failed(error) => {
            let lines = vec![
                Line::raw(""),
                Line::styled(format!("Could not load the {what}"), Style::new().italic()),
                Line::styled(error.clone(), Style::new().red()),
                Line::styled("Press r to retry, Esc to go back", hint),
            ];
            return frame.render_widget(message(lines, &view.job), area);
        }
        BuildLoad::Loaded(None) => {
            let lines = match view.target {
                BuildRef::Latest => vec![
                    Line::raw(""),
                    Line::styled("No builds yet", Style::new().italic()),
                    Line::styled("This job has never run", hint),
                ],
                BuildRef::Number(n) => vec![
                    Line::raw(""),
                    Line::styled(
                        format!("Build #{n} no longer exists"),
                        Style::new().italic(),
                    ),
                    Line::styled(
                        "It was probably deleted: ←/→ to move on, End for the latest",
                        hint,
                    ),
                ],
            };
            return frame.render_widget(message(lines, &view.job), area);
        }
        BuildLoad::Loaded(Some(build)) => build,
    };

    let mut title = format!("{} · {}", view.job, build.display_name);
    if let Some((position, total)) = view.position() {
        title.push_str(&format!(" · {position} of {total}"));
    }
    if view.refreshing {
        title.push_str(" · refreshing…");
    }
    let block = view_block(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = |text: &str| Span::styled(format!(" {text:<12}"), hint);
    let mut lines = Vec::new();

    lines.push(Line::from(vec![label("Result"), result_span(build)]));
    let age = app
        .wall_now
        .duration_since(build.started)
        .unwrap_or_default();
    lines.push(Line::from(vec![
        label("Started"),
        Span::raw(format!("{} ago", format_age(age))),
    ]));
    if build.building {
        let estimate = build
            .estimated
            .map(|e| format!(" of ~{}", format_duration(e)))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            label("Running for"),
            Span::raw(format!("{}{estimate}", format_duration(age))),
        ]));
        if let Some(estimated) = build.estimated {
            lines.push(Line::from(vec![label(""), progress_bar(age, estimated)]));
        }
    } else {
        lines.push(Line::from(vec![
            label("Duration"),
            Span::raw(format_duration(build.duration)),
        ]));
    }
    for (i, cause) in build.causes.iter().enumerate() {
        let name = if i == 0 { "Cause" } else { "" };
        lines.push(Line::from(vec![label(name), Span::raw(cause.clone())]));
    }
    for (i, (name, value)) in build.parameters.iter().enumerate() {
        let heading = if i == 0 { "Parameters" } else { "" };
        lines.push(Line::from(vec![
            label(heading),
            Span::styled(name.clone(), Style::new().bold()),
            Span::raw(format!(" = {value}")),
        ]));
    }
    if let Some(description) = &build.description {
        for (i, text) in description.lines().enumerate() {
            let heading = if i == 0 { "Description" } else { "" };
            lines.push(Line::from(vec![label(heading), Span::raw(text.to_owned())]));
        }
    }

    lines.push(Line::raw(""));
    if build.changes.is_empty() {
        lines.push(Line::styled(" No changes", hint));
    } else {
        lines.push(Line::styled(
            format!(" Changes ({})", build.changes.len()),
            Style::new().bold(),
        ));
        for change in &build.changes {
            let mut spans = vec![Span::raw("   ")];
            if let Some(commit) = &change.commit {
                spans.push(Span::styled(format!("{commit} "), Style::new().yellow()));
            }
            spans.push(Span::raw(change.message.clone()));
            if let Some(author) = &change.author {
                spans.push(Span::styled(format!(" — {author}"), hint));
            }
            lines.push(Line::from(spans));
        }
    }

    // Remember how far scrolling is useful, so the app can clamp it.
    let max_scroll = (lines.len() as u16).saturating_sub(inner.height);
    view.max_scroll.set(max_scroll);
    let scroll = view.scroll.min(max_scroll);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

fn render_console(frame: &mut Frame, area: Rect, app: &App) {
    let Some(console) = &app.console else {
        return;
    };
    let hint = Style::new().dark_gray();
    let mut title = format!("{} · #{} · console", console.job, console.number);
    match console.load {
        ConsoleLoad::Loaded if console.more && console.following => {
            title.push_str(" · ⟳ following")
        }
        ConsoleLoad::Loaded if console.more => title.push_str(" · paused (End to follow)"),
        ConsoleLoad::Loaded => title.push_str(" · complete"),
        _ => {}
    }
    if console.dropped > 0 {
        title.push_str(&format!(" · last {} lines", crate::console::MAX_LINES));
    }
    let block = view_block(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let message = |lines: Vec<Line<'static>>| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
    };
    match &console.load {
        ConsoleLoad::Loading => {
            let lines = vec![
                Line::raw(""),
                Line::styled("Loading console output…", Style::new().italic()),
            ];
            return frame.render_widget(message(lines), inner);
        }
        ConsoleLoad::Failed(error) => {
            let lines = vec![
                Line::raw(""),
                Line::styled("Could not load the console output", Style::new().italic()),
                Line::styled(error.clone(), Style::new().red()),
                Line::styled("Press r to retry, Esc to go back", hint),
            ];
            return frame.render_widget(message(lines), inner);
        }
        ConsoleLoad::Loaded if console.line_count() == 0 => {
            let lines = vec![
                Line::raw(""),
                Line::styled("No output yet", Style::new().italic()),
            ];
            return frame.render_widget(message(lines), inner);
        }
        ConsoleLoad::Loaded => {}
    }

    // Leave a column for the scrollbar.
    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    console.viewport.set(text_area.height);
    let top = console.visible_top();
    let visible: Vec<Line> = (top..top + usize::from(text_area.height))
        .filter_map(|i| console.line(i))
        .map(|line| Line::raw(line.chars().skip(console.left).collect::<String>()))
        .collect();
    frame.render_widget(Paragraph::new(visible), text_area);

    let mut scrollbar = ScrollbarState::new(console.max_top() + 1).position(top);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        bar_area,
        &mut scrollbar,
    );
}

fn result_span(build: &Build) -> Span<'static> {
    if build.building {
        return Span::styled("⟳ running", Style::new().yellow().bold());
    }
    match build.result {
        Some(status) => {
            let (symbol, color) = status_symbol(status);
            Span::styled(
                format!("{symbol} {}", status.label()),
                Style::new().fg(color),
            )
        }
        None => Span::styled("unknown", Style::new().dark_gray()),
    }
}

/// `████████░░░░░░░░ 50%`, or full and red once past the estimate.
fn progress_bar(elapsed: std::time::Duration, estimated: std::time::Duration) -> Span<'static> {
    const WIDTH: usize = 30;
    let ratio = elapsed.as_secs_f64() / estimated.as_secs_f64().max(1.0);
    let filled = ((ratio.min(1.0)) * WIDTH as f64).round() as usize;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled));
    if ratio > 1.0 {
        Span::styled(format!("{bar} longer than usual"), Style::new().red())
    } else {
        Span::styled(
            format!("{bar} {:>3.0}%", ratio * 100.0),
            Style::new().yellow(),
        )
    }
}

/// Background of the selected job row.
const SELECTED_BG: Color = Color::DarkGray;
/// Replaces status colours equal to [`SELECTED_BG`] on the selected row.
const SELECTED_FG_ON_BG: Color = Color::Gray;

/// Symbol + color per status; the status word is shown too (not color alone).
fn status_symbol(status: JobStatus) -> (&'static str, Color) {
    match status {
        JobStatus::Success => ("●", Color::Green),
        JobStatus::Unstable => ("●", Color::Yellow),
        JobStatus::Failed => ("●", Color::Red),
        JobStatus::Aborted => ("●", Color::Gray),
        JobStatus::NotBuilt => ("○", Color::DarkGray),
        JobStatus::Disabled => ("⊘", Color::DarkGray),
        JobStatus::Unknown => ("?", Color::DarkGray),
    }
}

const SECRET_MASK: &str = "••••••••";
const LABEL_WIDTH: usize = 16;

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
    let rows = s.rows();
    let selected_index = s.selected.min(rows.len() - 1);
    for (i, row) in rows.iter().enumerate() {
        if i > 0
            && matches!(row, SettingsRow::Header(_) | SettingsRow::AddHeader)
            && matches!(rows[i - 1], SettingsRow::Setting(_))
        {
            lines.push(Line::raw(""));
            lines.push(Line::styled("   Headers", Style::new().bold()));
        }
        let selected = i == selected_index;
        let marker = if selected { "▶" } else { " " };
        let label_style = if selected {
            Style::new().yellow().bold()
        } else {
            Style::new()
        };
        let label = match row {
            SettingsRow::Setting(key) => key.label().to_owned(),
            SettingsRow::Header(name) => name.clone(),
            SettingsRow::AddHeader => "+ add header".to_owned(),
        };
        let mut spans = vec![Span::styled(
            format!(" {marker} {label:<LABEL_WIDTH$}"),
            label_style,
        )];

        if let (Some(input), true) = (&s.editing, selected) {
            // Shown as typed: masking only applies when not editing.
            let shown: Vec<char> = input.value().chars().collect();
            spans.push(Span::raw("  "));
            spans.extend(input_spans(&shown, input.cursor()));
            lines.push(Line::from(spans));
            continue;
        }

        spans.push(Span::raw("  "));
        match row {
            SettingsRow::Setting(key) => {
                let key = *key;
                spans.push(match effective.get(key) {
                    _ if key.is_bool() => match effective.is_on(key) {
                        true if key == SettingKey::JenkinsSkipTlsVerify => {
                            Span::styled("on (insecure)", Style::new().red().bold())
                        }
                        true => Span::raw("on"),
                        false => Span::raw("off"),
                    },
                    None if key == SettingKey::RefreshInterval => {
                        Span::styled(format!("{} s (default)", DEFAULT_REFRESH_SECS), dim)
                    }
                    None if key == SettingKey::ProxyUrl => {
                        Span::styled(system_proxy_note(&s.system_proxy()), dim)
                    }
                    None => Span::styled("(not set)", dim),
                    Some(value) => Span::raw(key.display(value)),
                });
                if s.is_overridden(key) {
                    spans.push(Span::styled(format!("  (from ${})", key.env_var()), dim));
                }
            }
            SettingsRow::Header(name) => {
                // Header values are credentials more often than not: always
                // masked, fixed width so the length doesn't leak.
                spans.push(Span::raw(SECRET_MASK));
                if s.is_header_overridden(name) {
                    spans.push(Span::styled(
                        format!("  (from ${})", header_env_var(name)),
                        dim,
                    ));
                }
            }
            SettingsRow::AddHeader => {}
        }
        lines.push(Line::from(spans));
    }

    let block = view_block("Settings");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [top, _, docs] = Layout::vertical([
        Constraint::Length(lines.len() as u16),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(lines), top);

    // Status message + docs for the selected row, in their own inset area so
    // wrapped lines keep the indent.
    let [_, bottom] = Layout::horizontal([Constraint::Length(3), Constraint::Min(0)]).areas(docs);
    let mut bottom_lines = Vec::new();
    if let Some(message) = &s.message {
        bottom_lines.push(match message {
            StatusMessage::Info(text) => Line::styled(text.as_str(), Style::new().green()),
            StatusMessage::Error(text) => Line::styled(text.as_str(), Style::new().red()),
        });
        bottom_lines.push(Line::raw(""));
    }
    let doc = match &rows[selected_index] {
        SettingsRow::Setting(key) => key.doc(),
        SettingsRow::Header(_) | SettingsRow::AddHeader => HEADERS_DOC,
    };
    bottom_lines.extend(doc.lines().map(|line| Line::styled(line, dim)));
    frame.render_widget(
        Paragraph::new(bottom_lines).wrap(Wrap { trim: false }),
        bottom,
    );
}

/// An input's text with the cursor: highlighted char, or a block at the end.
fn input_spans(shown: &[char], cursor: usize) -> Vec<Span<'static>> {
    let (before, after) = shown.split_at(cursor.min(shown.len()));
    let text = Style::new().yellow().underlined();
    let mut spans = vec![Span::styled(before.iter().collect::<String>(), text)];
    match after.split_first() {
        Some((under, rest)) => {
            spans.push(Span::styled(
                under.to_string(),
                Style::new().black().on_yellow(),
            ));
            spans.push(Span::styled(rest.iter().collect::<String>(), text));
        }
        None => spans.push(Span::styled("█", Style::new().yellow())),
    }
    spans
}

fn system_proxy_note(system: &SystemProxy) -> String {
    match system {
        SystemProxy::Proxy { var, url } => format!("(not set; ${var}: {})", redact_url(url)),
        SystemProxy::Bypassed { var } => format!("(not set; ${var} bypasses the proxy)"),
        SystemProxy::None => "(not set; direct connection)".into(),
    }
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

/// Global bar: application-wide commands (dimmed while typing text, since
/// the keys then go to the input) and the refresh status on the right.
fn render_global_bar(frame: &mut Frame, area: Rect, app: &App) {
    let captured = app.context().global_keys_off();
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

/// `⟳ 4s`: how old the data is; the icon is green while auto-refresh is on,
/// gray while it's off. `⟳ …` while fetching, `✕` after a failed refresh.
/// Only shown when connected (there's nothing to refresh otherwise).
fn refresh_status(app: &App) -> Line<'static> {
    if !matches!(app.connection, ConnectionStatus::Connected { .. }) {
        return Line::default();
    }
    let icon = if app.auto_refresh {
        Style::new().green()
    } else {
        Style::new().gray()
    };
    // Leading space: a gap even when the connection text is cut off.
    let mut spans = vec![Span::raw(" "), Span::styled("⟳", icon)];
    // The data on screen: the open build, or the job list.
    let (busy, fetched_at, failed) = match (&app.view, &app.build) {
        (View::Console, _) if app.console.is_some() => {
            let console = app.console.as_ref().expect("checked");
            (
                console.in_flight,
                console.fetched_at,
                console.last_error.is_some() || matches!(console.load, ConsoleLoad::Failed(_)),
            )
        }
        (View::Build, Some(build)) => (
            build.fetch_in_flight(),
            build.fetched_at,
            matches!(build.load, BuildLoad::Failed(_)),
        ),
        _ => (
            app.jobs.fetch_in_flight(),
            app.jobs.fetched_at,
            matches!(app.jobs.load, JobsLoad::Failed(_)),
        ),
    };
    if busy {
        spans.push(Span::styled(" …", Style::new().yellow()));
    } else if let Some(at) = fetched_at {
        let age = format_age(app.now.saturating_duration_since(at));
        spans.push(Span::styled(format!(" {age}"), Style::new().gray()));
    }
    if failed && !busy {
        spans.push(Span::styled(" ✕", Style::new().red()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// `0s`, `12s`, `4m`, `2h`, `3d`.
fn format_age(age: std::time::Duration) -> String {
    match age.as_secs() {
        s @ 0..60 => format!("{s}s"),
        s @ 60..3600 => format!("{}m", s / 60),
        s @ 3600..86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn render_confirm_quit(frame: &mut Frame, area: Rect) {
    let key = Style::new().bold();
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("y", key),
            Span::raw(" / "),
            Span::styled("Enter", key),
            Span::raw(" quit     "),
            Span::styled("n", key),
            Span::raw(" / "),
            Span::styled("Esc", key),
            Span::raw(" stay"),
        ]),
    ];
    let popup = centered(area, 38, lines.len() as u16 + 2);
    let block = Block::bordered()
        .title(" Quit Leeroy? ")
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(Color::Red));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
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
    lines.push(Line::raw(""));
    lines.push(section("Tabs"));
    let tab_bindings: Vec<Binding> = Tab::ALL
        .iter()
        .map(|tab| Binding {
            key: TAB_KEYS[usize::from(tab.f_key()) - 1],
            desc: tab.title(),
        })
        .collect();
    lines.extend(rows(&tab_bindings));
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

/// Labels for F-keys, indexed by `Tab::f_key() - 1`.
const TAB_KEYS: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    area
}
