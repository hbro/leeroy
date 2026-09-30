use std::time::SystemTime;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
    },
};

use crate::{
    app::{App, ConnectionStatus, NOTICE_FOR, SettingsRow, StatusMessage, Tab, View},
    builds::{Build, BuildLoad, BuildRef, Stage, StageStatus, format_duration},
    config::{DEFAULT_REFRESH_SECS, HEADERS_DOC, SettingKey, header_env_var, redact_url},
    console::ConsoleLoad,
    event::{Binding, GLOBAL_BINDINGS, context_bindings},
    graph::{self, Cell as GraphCell},
    history::HistoryLoad,
    instance::InfoLoad,
    jobs::{JobStatus, JobsLoad},
    pipelines::{PipelineLoad, RUN_BUILDS, RunStatus, TreeItem, run_span, run_tree},
    proxy::SystemProxy,
    theme::{Appearance, Theme},
};

/// Draw the whole UI. Keep this deterministic (no clock, no randomness):
/// snapshot tests depend on it.
pub fn render(frame: &mut Frame, app: &App) {
    let [header, tabs, body, context_bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_tabs(frame, tabs, app);
    match app.view {
        View::Jobs => render_jobs(frame, body, app),
        View::Builds => render_history(frame, body, app),
        View::Pipelines => render_pipelines(frame, body, app),
        View::Runs => render_runs(frame, body, app),
        View::Run => render_run(frame, body, app),
        View::Build => render_build(frame, body, app),
        View::Console => render_console(frame, body, app),
        View::Settings => render_settings(frame, body, app),
    }
    render_context_bar(frame, context_bar, app);

    if app.show_help {
        render_help(frame, body, app);
    }
    if app.show_info {
        render_info(frame, body, app);
    }
    if app.promote.is_some() {
        render_promote(frame, body, app);
    }
    if app.confirm_start.is_some() {
        render_confirm_start(frame, body, app);
    }
    if app.confirm_quit {
        render_confirm_quit(frame, body, app.theme());
    }
}

/// Header: app name + the Jenkins instance we're connected to.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let mut spans = vec![
        Span::styled(" Leeroy ", Style::new().bold()),
        Span::raw(" "),
    ];
    // First, so it's never cut off on narrow terminals.
    if app
        .settings
        .connection_config()
        .is_some_and(|c| c.skip_tls_verify)
    {
        spans.push(Span::styled("⚠ TLS NOT VERIFIED", t.danger.bold()));
        spans.push(Span::raw(" "));
    }
    // Secondary text on the bar, not on the terminal background.
    let dim = Style::new().fg(t.bar_dim);
    spans.extend(match &app.connection {
        ConnectionStatus::NotConfigured => vec![
            Span::styled("○", Style::new().fg(t.error)),
            Span::raw(" not connected"),
        ],
        ConnectionStatus::Connecting { url } => vec![
            Span::styled("◌", dim),
            Span::raw(format!(" {url}")),
            Span::styled(" · connecting…", dim),
        ],
        ConnectionStatus::Connected { url, info } => {
            let mut spans = vec![
                Span::styled("●", Style::new().fg(t.success)),
                Span::raw(format!(" {url}")),
            ];
            if let Some(version) = &info.version {
                spans.push(Span::styled(format!(" · Jenkins {version}"), dim));
            }
            spans.push(Span::styled(format!(" · {}", info.user), dim));
            spans
        }
        ConnectionStatus::Failed { url, error } => vec![
            Span::styled("✕", Style::new().fg(t.error).bold()),
            Span::raw(format!(" {url}")),
            Span::styled(format!(" · {error}"), Style::new().fg(t.error)),
        ],
    });
    // Right end: the help hint, then the refresh status; the connection
    // info gets the rest.
    let mut right = vec![Span::styled(" h/? ", t.bar_key), Span::raw(" help ")];
    right.extend(refresh_status(app).spans);
    let status = Line::from(right);
    let [left, right] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(status.width() as u16),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(t.bar), left);
    frame.render_widget(Paragraph::new(status).style(t.bar), right);
}

/// Frame of the main content: a line above (with the title) and below, no
/// side borders.
fn view_block(t: &Theme, title: &str) -> Block<'static> {
    Block::new()
        .title(format!("{title} "))
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::new().fg(t.accent))
}

/// Tab bar: `1 Jobs  2 …` from the left, `0 Settings` at the far right; the
/// active tab highlighted.
fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let active = t.on_accent.bold();
    let inactive = Style::new().fg(t.tab_inactive);
    let label = |tab: Tab| {
        let style = if app.tab() == Some(tab) {
            active
        } else {
            inactive
        };
        Span::styled(format!(" {} {} ", tab.key(), tab.title()), style)
    };
    // Content tabs from the left; Settings at the far right.
    let mut left = Vec::new();
    for tab in Tab::ALL.into_iter().filter(|t| *t != Tab::Settings) {
        left.push(label(tab));
        left.push(Span::raw(" "));
    }
    let right = Line::from(label(Tab::Settings));
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right.width() as u16)])
            .areas(area);
    frame.render_widget(Paragraph::new(Line::from(left)), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

fn render_jobs(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let hint = t.dim();
    let centered_message = |lines: Vec<Line<'static>>| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, "Jobs"))
    };
    let lines = match &app.connection {
        ConnectionStatus::NotConfigured => vec![
            Line::raw(""),
            Line::styled("No Jenkins instance configured", Style::new().italic()),
            Line::styled("Press 0 to open the settings", hint),
        ],
        ConnectionStatus::Connecting { url } => vec![
            Line::raw(""),
            Line::styled(format!("Connecting to {url}…"), Style::new().italic()),
        ],
        ConnectionStatus::Failed { url, error } => vec![
            Line::raw(""),
            Line::styled(format!("Cannot reach {url}"), Style::new().italic()),
            Line::styled(error.clone(), Style::new().fg(t.error)),
            Line::styled("Check the settings (0), or press r to retry", hint),
        ],
        ConnectionStatus::Connected { .. } => match app.jobs.load() {
            JobsLoad::NotLoaded | JobsLoad::Loading => vec![
                Line::raw(""),
                Line::styled("Loading jobs…", Style::new().italic()),
            ],
            JobsLoad::Failed(error) => vec![
                Line::raw(""),
                Line::styled("Could not load jobs", Style::new().italic()),
                Line::styled(error.clone(), Style::new().fg(t.error)),
                Line::styled("Press r to retry", hint),
            ],
            JobsLoad::Loaded(_) => return render_job_list(frame, area, app),
        },
    };
    frame.render_widget(centered_message(lines), area);
}

fn render_job_list(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
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
    let block = view_block(t, &title);
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
        let mut spans = vec![Span::styled(" / ", Style::new().fg(t.highlight).bold())];
        let shown: Vec<char> = input.value().chars().collect();
        spans.extend(input_spans(t, &shown, input.cursor()));
        frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
    } else if show_filter {
        let line = Line::from(vec![
            Span::styled(" filter: ", t.dim()),
            Span::styled(jobs.filter.clone(), Style::new().fg(t.highlight)),
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

    // Only the rows on screen are built (big instances have thousands).
    let selected = jobs.selected.min(visible.len() - 1);
    let window = visible_window(visible.len(), selected, list_area.height, &jobs.offset);
    let rows = visible[window.clone()].iter().enumerate().map(|(i, job)| {
        let (symbol, mut color) = status_symbol(t, job.status);
        // Text in the highlight colour would vanish on the selected row.
        if window.start + i == selected {
            color = t.on_selected(color);
        }
        Row::new(vec![
            Cell::from(Span::styled(symbol, Style::new().fg(color))),
            Cell::from(Span::styled(job.status.label(), Style::new().fg(color))),
            Cell::from(if job.building {
                Span::styled("⟳", Style::new().fg(t.warning).bold())
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
    .row_highlight_style(Style::new().bg(t.selected_bg).bold())
    .highlight_symbol("▶ ")
    .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default().with_selected(Some(selected - window.start));
    frame.render_stateful_widget(table, list_area, &mut state);
    app.list_rows.set(list_area.height);
}

fn render_history(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let hint = t.dim();
    let message = |lines: Vec<Line<'static>>| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, "Builds"))
    };
    let lines = match (&app.connection, &app.history.load) {
        (ConnectionStatus::Connected { .. }, HistoryLoad::Loaded) => {
            return render_history_list(frame, area, app);
        }
        (ConnectionStatus::Connected { .. }, HistoryLoad::Failed(error)) => vec![
            Line::raw(""),
            Line::styled("Could not load the build history", Style::new().italic()),
            Line::styled(error.clone(), Style::new().fg(t.error)),
            Line::styled("Press r to retry", hint),
        ],
        (ConnectionStatus::Connected { .. }, _) => vec![
            Line::raw(""),
            Line::styled("Loading build history…", Style::new().italic()),
        ],
        _ => vec![
            Line::raw(""),
            Line::styled("Not connected", Style::new().italic()),
            Line::styled("See the Jobs tab (1) or the settings (0)", hint),
        ],
    };
    frame.render_widget(message(lines), area);
}

fn render_history_list(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let history = &app.history;
    let visible = history.visible();
    let mut title = format!("Builds ({} newest", visible.len());
    if !history.active_filter().trim().is_empty() {
        title.push_str(" matching");
    }
    title.push(')');
    if history.has_more() {
        title.push_str(" · ↓ at the end loads more");
    }
    if history.refreshing {
        title.push_str(" · refreshing…");
    }
    let block = view_block(t, &title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let show_filter = history.filter_input.is_some() || !history.filter.is_empty();
    let [filter_area, list_area] = Layout::vertical([
        Constraint::Length(u16::from(show_filter)),
        Constraint::Min(0),
    ])
    .areas(inner);
    app.list_rows.set(list_area.height);
    if let Some(input) = &history.filter_input {
        let mut spans = vec![Span::styled(" / ", Style::new().fg(t.highlight).bold())];
        let shown: Vec<char> = input.value().chars().collect();
        spans.extend(input_spans(t, &shown, input.cursor()));
        frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
    } else if show_filter {
        let line = Line::from(vec![
            Span::styled(" filter: ", t.dim()),
            Span::styled(history.filter.clone(), Style::new().fg(t.highlight)),
        ]);
        frame.render_widget(Paragraph::new(line), filter_area);
    }

    if visible.is_empty() {
        let text = if history.active_filter().trim().is_empty() {
            "No builds yet".to_owned()
        } else {
            format!(
                "No builds of jobs matching \"{}\"",
                history.active_filter().trim()
            )
        };
        let message = Paragraph::new(vec![
            Line::raw(""),
            Line::styled(text, Style::new().italic()),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(message, list_area);
        return;
    }

    let rows = visible.iter().enumerate().map(|(i, entry)| {
        let (symbol, mut color, label) = if entry.building {
            ("⟳", t.warning, "running")
        } else {
            let status = entry.result.unwrap_or(JobStatus::Unknown);
            let (symbol, color) = status_symbol(t, status);
            (symbol, color, status.label())
        };
        // Text in the highlight colour would vanish on the selected row.
        let selected = i == history.selected;
        let mut dim = t.dim;
        if selected {
            color = t.on_selected(color);
            dim = t.on_selected(dim);
        }
        let dim = Style::new().fg(dim);
        let age = app
            .wall_now
            .duration_since(entry.started)
            .unwrap_or_default();
        let took = if entry.building {
            format!("{}…", format_duration(age))
        } else {
            format_duration(entry.duration)
        };
        Row::new(vec![
            Cell::from(Span::styled(symbol, Style::new().fg(color))),
            Cell::from(Span::styled(label, Style::new().fg(color))),
            Cell::from(Line::from(vec![
                Span::raw(entry.job.clone()),
                Span::styled(format!(" #{}", entry.number), dim),
            ])),
            Cell::from(Line::from(started_text(app, entry.started)).right_aligned()),
            Cell::from(Line::from(took).right_aligned()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(9),
            Constraint::Min(0),
            Constraint::Length(started_width(app)),
            Constraint::Length(9),
        ],
    )
    .column_spacing(1)
    .row_highlight_style(Style::new().bg(t.selected_bg).add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ")
    .highlight_spacing(HighlightSpacing::Always);
    let mut state = TableState::default().with_selected(Some(history.selected));
    frame.render_stateful_widget(table, list_area, &mut state);
}

/// The Pipelines and Runs tabs before their data is there: not connected,
/// loading, failed. `None` when loaded.
fn pipelines_message(app: &App, what: &str) -> Option<Vec<Line<'static>>> {
    let t = app.theme();
    let hint = t.dim();
    Some(match (&app.connection, &app.pipelines.load) {
        (ConnectionStatus::Connected { .. }, PipelineLoad::Loaded) => return None,
        (ConnectionStatus::Connected { .. }, PipelineLoad::Failed(error)) => vec![
            Line::raw(""),
            Line::styled(format!("Could not load the {what}"), Style::new().italic()),
            Line::styled(error.clone(), Style::new().fg(t.error)),
            Line::styled("Press r to retry", hint),
        ],
        (ConnectionStatus::Connected { .. }, _) => vec![
            Line::raw(""),
            Line::styled(format!("Loading {what}…"), Style::new().italic()),
        ],
        _ => vec![
            Line::raw(""),
            Line::styled("Not connected", Style::new().italic()),
            Line::styled("See the Jobs tab (1) or the settings (0)", hint),
        ],
    })
}

/// The filter line of a tab (while typing or when applied); returns the
/// area left for the content.
fn filter_line(
    frame: &mut Frame,
    area: Rect,
    t: &Theme,
    input: Option<&crate::input::TextInput>,
    applied: &str,
) -> Rect {
    let show = input.is_some() || !applied.is_empty();
    let [filter_area, rest] =
        Layout::vertical([Constraint::Length(u16::from(show)), Constraint::Min(0)]).areas(area);
    if let Some(input) = input {
        let mut spans = vec![Span::styled(" / ", Style::new().fg(t.highlight).bold())];
        let shown: Vec<char> = input.value().chars().collect();
        spans.extend(input_spans(t, &shown, input.cursor()));
        frame.render_widget(Paragraph::new(Line::from(spans)), filter_area);
    } else if show {
        let line = Line::from(vec![
            Span::styled(" filter: ", t.dim()),
            Span::styled(applied.to_owned(), Style::new().fg(t.highlight)),
        ]);
        frame.render_widget(Paragraph::new(line), filter_area);
    }
    rest
}

/// Symbol and colour of a run status.
fn run_status_style(t: &Theme, status: RunStatus, running: bool) -> (&'static str, Color) {
    let color = match status {
        RunStatus::Success => t.success,
        RunStatus::Partial => t.highlight,
        RunStatus::Aborted => t.neutral,
        RunStatus::Unstable => t.warning,
        RunStatus::Failure => t.error,
    };
    if running {
        ("⟳", color)
    } else {
        ("●", color)
    }
}

/// `2m 13s` from `start` to `end` (running: until now, with `…`).
fn span_text(app: &App, start: SystemTime, end: Option<SystemTime>) -> String {
    match end {
        Some(end) => format_duration(end.duration_since(start).unwrap_or_default()),
        None => format!(
            "{}…",
            format_duration(app.wall_now.duration_since(start).unwrap_or_default())
        ),
    }
}

/// The rows of a long list that fit in `height`, keeping `selected` in view
/// and moving the recorded `offset` as little as possible (like a scrolled
/// table, but without building the rows off screen).
fn visible_window(
    len: usize,
    selected: usize,
    height: u16,
    offset: &std::cell::Cell<usize>,
) -> std::ops::Range<usize> {
    let height = usize::from(height).max(1);
    let mut start = offset.get().min(len.saturating_sub(height));
    if selected < start {
        start = selected;
    } else if selected >= start + height {
        start = selected + 1 - height;
    }
    offset.set(start);
    start..(start + height).min(len)
}

/// One row of the Pipelines or Runs list.
struct RunRow<'a> {
    status: RunStatus,
    running: bool,
    /// Spans of the name column.
    name: Vec<Span<'a>>,
    start: SystemTime,
    /// `None` while running.
    end: Option<SystemTime>,
}

/// A list of runs as table rows: status, name, age, how long it took.
fn run_table<'a>(app: &App, rows: Vec<RunRow<'a>>, selected: usize) -> Table<'a> {
    let t = app.theme();
    let table_rows = rows.into_iter().enumerate().map(|(i, row)| {
        let RunRow {
            status,
            running,
            name,
            start,
            end,
        } = row;
        let (symbol, mut color) = run_status_style(t, status, running);
        let mut name = name;
        if i == selected {
            color = t.on_selected(color);
            // Dim parts of the name would vanish on the selection.
            for span in &mut name {
                if let Some(fg) = span.style.fg {
                    span.style.fg = Some(t.on_selected(fg));
                }
            }
        }
        Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(symbol, Style::new().fg(color)),
                Span::raw(" "),
                Span::styled(status.label(), Style::new().fg(color)),
            ])),
            Cell::from(Line::from(name)),
            Cell::from(Line::from(started_text(app, start)).right_aligned()),
            Cell::from(Line::from(span_text(app, start, end)).right_aligned()),
        ])
    });
    Table::new(
        table_rows,
        [
            Constraint::Length(10),
            Constraint::Min(0),
            Constraint::Length(started_width(app)),
            Constraint::Length(9),
        ],
    )
    .column_spacing(1)
    .row_highlight_style(Style::new().bg(t.selected_bg).bold())
    .highlight_symbol("▶ ")
    .highlight_spacing(HighlightSpacing::Always)
}

fn render_pipelines(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    if let Some(lines) = pipelines_message(app, "pipelines") {
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, "Pipelines"));
        return frame.render_widget(message, area);
    }
    let state = &app.pipelines;
    let pipelines = state.visible_pipelines();
    let mut title = format!("Pipelines ({})", pipelines.len());
    if state.refreshing {
        title.push_str(" · refreshing…");
    }
    let block = view_block(t, &title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let list_area = filter_line(
        frame,
        inner,
        t,
        state.list.filter_input.as_ref(),
        &state.list.filter,
    );
    app.list_rows.set(list_area.height);

    if pipelines.is_empty() {
        let filter = state.list.active_filter().trim();
        let lines = if filter.is_empty() {
            vec![
                Line::raw(""),
                Line::styled("No pipelines", Style::new().italic()),
                Line::styled(
                    format!("No job triggered another in its last {RUN_BUILDS} builds"),
                    t.dim(),
                ),
            ]
        } else {
            vec![
                Line::raw(""),
                Line::styled(
                    format!("No pipelines matching \"{filter}\""),
                    Style::new().italic(),
                ),
            ]
        };
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        return frame.render_widget(message, list_area);
    }

    let selected = state.list.selected.min(pipelines.len() - 1);
    let window = visible_window(
        pipelines.len(),
        selected,
        list_area.height,
        &state.list.offset,
    );
    let rows = pipelines[window.clone()]
        .iter()
        .map(|p| {
            let run = &p.runs[0];
            let (status, running) = (run.status, run.running);
            let (start, end) = run_span(state.data(), run);
            let mut name = vec![Span::raw(p.name.clone())];
            if p.name != p.first_job {
                name.push(Span::styled(format!("  {}", p.first_job), t.dim()));
            }
            RunRow {
                status,
                running,
                name,
                start,
                end,
            }
        })
        .collect();
    let selected = selected - window.start;
    let mut table_state = TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(run_table(app, rows, selected), list_area, &mut table_state);
}

fn render_runs(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    if let Some(lines) = pipelines_message(app, "runs") {
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, "Pipeline runs"));
        return frame.render_widget(message, area);
    }
    let state = &app.pipelines;
    let pipelines = state.pipelines();
    let runs = state.visible_runs();
    let mut title = format!("Pipeline runs ({})", runs.len());
    if state.refreshing {
        title.push_str(" · refreshing…");
    }
    let block = view_block(t, &title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let list_area = filter_line(
        frame,
        inner,
        t,
        state.runs.filter_input.as_ref(),
        &state.runs.filter,
    );
    app.list_rows.set(list_area.height);

    if runs.is_empty() {
        let filter = state.runs.active_filter().trim();
        let lines = if filter.is_empty() {
            vec![
                Line::raw(""),
                Line::styled("No pipeline runs", Style::new().italic()),
                Line::styled(
                    format!("None of the last {RUN_BUILDS} builds of any job triggered another"),
                    t.dim(),
                ),
            ]
        } else {
            vec![
                Line::raw(""),
                Line::styled(
                    format!("No pipeline runs matching \"{filter}\""),
                    Style::new().italic(),
                ),
            ]
        };
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        return frame.render_widget(message, list_area);
    }

    let selected = state.runs.selected.min(runs.len() - 1);
    let window = visible_window(runs.len(), selected, list_area.height, &state.runs.offset);
    let rows = runs[window.clone()]
        .iter()
        .map(|&(p, r)| {
            let pipeline = &pipelines[p];
            let run = &pipeline.runs[r];
            let (status, running) = (run.status, run.running);
            let (start, end) = run_span(state.data(), run);
            let name = vec![
                Span::raw(pipeline.name.clone()),
                Span::styled(format!(" #{}", state.run_number(run)), t.dim()),
            ];
            RunRow {
                status,
                running,
                name,
                start,
                end,
            }
        })
        .collect();
    let selected = selected - window.start;
    let mut table_state = TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(run_table(app, rows, selected), list_area, &mut table_state);
}

/// Status symbol and colour of a build in a run.
fn build_symbol(t: &Theme, build: &crate::pipelines::RunBuild) -> (&'static str, Color) {
    if build.building {
        ("⟳", t.warning)
    } else {
        status_symbol(t, build.result.unwrap_or(JobStatus::Unknown))
    }
}

fn render_run(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let Some(view) = &app.run else {
        return;
    };
    if let Some(lines) = pipelines_message(app, "run") {
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, "Run"));
        return frame.render_widget(message, area);
    }
    let state = &app.pipelines;
    let Some((pipeline, index)) = state.find_run(view) else {
        let lines = vec![
            Line::raw(""),
            Line::styled("This run is no longer loaded", Style::new().italic()),
            Line::styled(
                format!(
                    "Only runs started within the last {RUN_BUILDS} builds of a job are \
                     shown: ←/→ to move on, End for the latest"
                ),
                t.dim(),
            ),
        ];
        let message = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, &view.first_job));
        return frame.render_widget(message, area);
    };
    let run = &pipeline.runs[index];
    let (status, running) = (run.status, run.running);
    let mut title = format!(
        "{} · #{} · {} of {} · {}",
        pipeline.name,
        state.run_number(run),
        pipeline.runs.len() - index,
        pipeline.runs.len(),
        status.label()
    );
    if running {
        title.push_str(" · running");
    }
    if state.refreshing {
        title.push_str(" · refreshing…");
    }
    let selected = view.selected.min(run.builds.len() - 1);
    let inner = view_block(t, "").inner(area);
    // Builds plus the parts the run never reached, where they would have run
    // (why a run whose builds all succeeded can still be partial).
    let tree = run_tree(state.data(), pipeline, run);
    let selected_row = tree
        .iter()
        .position(|row| row.item == TreeItem::Build(selected))
        .unwrap_or(0);
    let unreached = |disabled: bool| if disabled { "disabled" } else { "not reached" };

    if view.tree {
        frame.render_widget(view_block(t, &title), area);
        let rows = tree.iter().enumerate().map(|(i, row)| {
            let mut dim = t.dim;
            if i == selected_row {
                dim = t.on_selected(dim);
            }
            let dim = Style::new().fg(dim);
            let prefix = Span::styled(row.prefix.clone(), dim);
            match &row.item {
                TreeItem::Missing { job, disabled } => Row::new(vec![
                    Cell::from(Line::from(vec![
                        prefix,
                        Span::styled(format!("○ {job}"), dim),
                        Span::styled(format!("  {}", unreached(*disabled)), dim.italic()),
                    ])),
                    Cell::from(""),
                    Cell::from(""),
                ]),
                TreeItem::Build(b) => {
                    let build = &state.data().builds[run.builds[*b]];
                    let (symbol, mut color) = build_symbol(t, build);
                    if i == selected_row {
                        color = t.on_selected(color);
                    }
                    let age = app
                        .wall_now
                        .duration_since(build.started)
                        .unwrap_or_default();
                    let took = if build.building {
                        format!("{}…", format_duration(age))
                    } else {
                        format_duration(build.duration)
                    };
                    Row::new(vec![
                        Cell::from(Line::from(vec![
                            prefix,
                            Span::styled(symbol, Style::new().fg(color)),
                            Span::raw(" "),
                            Span::raw(build.job.clone()),
                            Span::styled(format!(" #{}", build.number), dim),
                        ])),
                        Cell::from(Line::from(started_text(app, build.started)).right_aligned()),
                        Cell::from(Line::from(took).right_aligned()),
                    ])
                }
            }
        });
        let table = Table::new(
            rows,
            [
                Constraint::Min(0),
                Constraint::Length(started_width(app)),
                Constraint::Length(9),
            ],
        )
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(t.selected_bg).bold())
        .highlight_symbol("▶ ")
        .highlight_spacing(HighlightSpacing::Always);
        let mut table_state = TableState::default().with_selected(Some(selected_row));
        return frame.render_stateful_widget(table, inner, &mut table_state);
    }

    // Boxes: stacked like the tree, each under the build that triggered it;
    // unreached parts as dim boxes.
    let boxes: Vec<(String, &'static str, Color)> = tree
        .iter()
        .map(|row| match &row.item {
            TreeItem::Build(b) => {
                let build = &state.data().builds[run.builds[*b]];
                let (symbol, color) = build_symbol(t, build);
                (format!("{} #{}", build.job, build.number), symbol, color)
            }
            TreeItem::Missing { job, disabled } => {
                (format!("{job} ({})", unreached(*disabled)), "○", t.dim)
            }
        })
        .collect();
    let widths: Vec<usize> = boxes.iter().map(|b| b.0.chars().count() + 2).collect();
    let parents: Vec<Option<usize>> = tree.iter().map(|row| row.parent).collect();
    let layout = graph::tree_layout(&widths, &parents);
    let more = paint_boxes(frame, inner, t, &layout, &boxes, selected_row, &view.scroll);
    if !more.is_empty() {
        title.push_str(&format!(" · more {more}"));
    }
    frame.render_widget(view_block(t, &title), area);
}

/// Draw a box layout into `area`, scrolled so box `selected` is fully
/// visible (the position is kept in `scroll`). Boxes are `(label, symbol,
/// status colour)`; borders take the status colour. Returns the directions
/// in which the drawing continues off screen (e.g. `"→↓"`).
fn paint_boxes(
    frame: &mut Frame,
    area: Rect,
    t: &Theme,
    layout: &graph::Layout,
    boxes: &[(String, &'static str, Color)],
    selected: usize,
    scroll: &std::cell::Cell<(usize, usize)>,
) -> String {
    let (view_w, view_h) = (usize::from(area.width), usize::from(area.height));
    let (mut sx, mut sy) = scroll.get();
    if let Some(b) = layout.boxes.get(selected) {
        let (bx, by, bw, bh) = (b.x, b.y, b.width, graph::BOX_HEIGHT);
        if bx + bw > sx + view_w {
            sx = (bx + bw).saturating_sub(view_w);
        }
        if bx < sx {
            sx = bx;
        }
        if by + bh > sy + view_h {
            sy = (by + bh).saturating_sub(view_h);
        }
        if by < sy {
            sy = by;
        }
    }
    sx = sx.min(layout.width.saturating_sub(view_w));
    sy = sy.min(layout.height.saturating_sub(view_h));
    scroll.set((sx, sy));

    let edge = t.dim();
    let mut grid: Vec<Vec<(char, Style)>> = layout
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match *cell {
                    GraphCell::Empty => (' ', Style::new()),
                    GraphCell::Line(dirs) => (graph::line_char(dirs), edge),
                    GraphCell::Arrow => ('▶', edge),
                    GraphCell::Border { node, dirs } => {
                        let mut style = Style::new().fg(boxes[node].2);
                        if node == selected {
                            style = style.bold();
                        }
                        (graph::line_char(dirs), style)
                    }
                })
                .collect()
        })
        .collect();
    for (i, b) in layout.boxes.iter().enumerate() {
        let (label, symbol, color) = &boxes[i];
        let row = &mut grid[b.y + 1];
        let base = if i == selected {
            Style::new().bg(t.selected_bg).bold()
        } else {
            Style::new()
        };
        // The inside of the box, padding included, so the selection fills it.
        for cell in &mut row[b.x + 1..b.x + b.width - 1] {
            *cell = (' ', base);
        }
        let color = if i == selected {
            t.on_selected(*color)
        } else {
            *color
        };
        row[b.x + 2] = (symbol.chars().next().unwrap_or(' '), base.fg(color));
        for (dx, c) in label.chars().enumerate() {
            row[b.x + 4 + dx] = (c, base);
        }
    }
    let lines: Vec<Line> = grid
        .iter()
        .skip(sy)
        .take(view_h)
        .map(|row| {
            let mut spans: Vec<Span> = Vec::new();
            let mut text = String::new();
            let mut style = Style::new();
            for &(c, st) in row.iter().skip(sx).take(view_w) {
                if st != style && !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), style));
                }
                style = st;
                text.push(c);
            }
            if !text.is_empty() {
                spans.push(Span::styled(text, style));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    [
        (sx > 0, '←'),
        (sx + view_w < layout.width, '→'),
        (sy > 0, '↑'),
        (sy + view_h < layout.height, '↓'),
    ]
    .iter()
    .filter(|(more, _)| *more)
    .map(|(_, arrow)| *arrow)
    .collect()
}

fn render_build(frame: &mut Frame, area: Rect, app: &App) {
    let Some(view) = &app.build else {
        return;
    };
    let t = app.theme();
    let hint = t.dim();
    let message = |lines: Vec<Line<'static>>, title: &str| {
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(view_block(t, title))
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
                Line::styled(error.clone(), Style::new().fg(t.error)),
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
    let block = view_block(t, &title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label = |text: &str| Span::styled(format!(" {text:<12}"), hint);
    let mut lines = Vec::new();

    lines.push(Line::from(vec![label("Result"), result_span(t, build)]));
    let age = app
        .wall_now
        .duration_since(build.started)
        .unwrap_or_default();
    let started = if app.absolute_times {
        local_time(app, build.started, "%Y-%m-%d %H:%M:%S")
    } else {
        format!("{} ago", format_age(age))
    };
    lines.push(Line::from(vec![label("Started"), Span::raw(started)]));
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
            lines.push(Line::from(vec![label(""), progress_bar(t, age, estimated)]));
        }
    } else {
        lines.push(Line::from(vec![
            label("Duration"),
            Span::raw(format_duration(build.duration)),
        ]));
    }
    if let Some(stages) = build.stages.as_ref().filter(|s| !s.is_empty()) {
        let width = usize::from(inner.width).saturating_sub(13).max(20);
        for (i, line) in stage_lines(t, stages, width).into_iter().enumerate() {
            let mut spans = vec![label(if i == 0 { "Stages" } else { "" })];
            spans.extend(line);
            lines.push(Line::from(spans));
        }
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
                spans.push(Span::styled(
                    format!("{commit} "),
                    Style::new().fg(t.highlight),
                ));
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
    let t = app.theme();
    let hint = t.dim();
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
    let block = view_block(t, &title);
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
                Line::styled(error.clone(), Style::new().fg(t.error)),
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

fn result_span(t: &Theme, build: &Build) -> Span<'static> {
    if build.building {
        return Span::styled("⟳ running", Style::new().fg(t.warning).bold());
    }
    match build.result {
        Some(status) => {
            let (symbol, color) = status_symbol(t, status);
            Span::styled(
                format!("{symbol} {}", status.label()),
                Style::new().fg(color),
            )
        }
        None => Span::styled("unknown", t.dim()),
    }
}

/// Stages as a chain, `● Checkout 4s ─▶ ✕ Test 38s ─▶ ○ Deploy`, wrapped
/// to `width` columns (a stage is never split).
fn stage_lines(t: &Theme, stages: &[Stage], width: usize) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0;
    for (i, stage) in stages.iter().enumerate() {
        let (symbol, color) = match stage.status {
            StageStatus::Success => ("●", t.success),
            StageStatus::Failed => ("✕", t.error),
            StageStatus::Unstable => ("●", t.warning),
            StageStatus::Aborted => ("●", t.neutral),
            StageStatus::Running => ("⟳", t.warning),
            StageStatus::Paused => ("⏸", t.warning),
            StageStatus::NotRun | StageStatus::Unknown => ("○", t.dim),
        };
        let mut text = stage.name.clone();
        let ran = !matches!(stage.status, StageStatus::NotRun | StageStatus::Unknown);
        if ran && !stage.duration.is_zero() {
            text.push(' ');
            text.push_str(&format_duration(stage.duration));
        }
        if stage.status == StageStatus::Paused {
            text.push_str(" (waiting for input)");
        }
        let arrow = if i == 0 { "" } else { " ─▶ " };
        let piece = arrow.chars().count() + 2 + text.chars().count();
        if used > 0 && used + piece > width {
            lines.push(Vec::new());
            used = 0;
        }
        let line = lines.last_mut().expect("never empty");
        if used > 0 {
            line.push(Span::styled(arrow, t.dim()));
        }
        let name_style = if ran { Style::new() } else { t.dim() };
        line.push(Span::styled(format!("{symbol} "), Style::new().fg(color)));
        line.push(Span::styled(text, name_style));
        used += piece;
    }
    lines
}

/// `████████░░░░░░░░ 50%`, or full and red once past the estimate.
fn progress_bar(
    t: &Theme,
    elapsed: std::time::Duration,
    estimated: std::time::Duration,
) -> Span<'static> {
    const WIDTH: usize = 30;
    let ratio = elapsed.as_secs_f64() / estimated.as_secs_f64().max(1.0);
    let filled = ((ratio.min(1.0)) * WIDTH as f64).round() as usize;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled));
    if ratio > 1.0 {
        Span::styled(format!("{bar} longer than usual"), Style::new().fg(t.error))
    } else {
        Span::styled(
            format!("{bar} {:>3.0}%", ratio * 100.0),
            Style::new().fg(t.warning),
        )
    }
}

/// Symbol + color per status; the status word is shown too (not color alone).
fn status_symbol(t: &Theme, status: JobStatus) -> (&'static str, Color) {
    match status {
        JobStatus::Success => ("●", t.success),
        JobStatus::Unstable => ("●", t.warning),
        JobStatus::Failed => ("●", t.error),
        JobStatus::Aborted => ("●", t.neutral),
        JobStatus::NotBuilt => ("○", t.dim),
        JobStatus::Disabled => ("⊘", t.dim),
        JobStatus::Unknown => ("?", t.dim),
    }
}

const SECRET_MASK: &str = "••••••••";
const LABEL_WIDTH: usize = 16;

fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let s = &app.settings;
    let t = app.theme();
    let dim = t.dim();
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
        let previous = i.checked_sub(1).map(|p| &rows[p]);
        // Section heading where the section changes.
        if previous.is_none_or(|p| p.section() != row.section()) {
            if previous.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::styled(
                format!(" {}", row.section().title()),
                Style::new().fg(t.accent).bold(),
            ));
        }
        // Headers sub-heading, within the Jenkins section.
        if matches!(row, SettingsRow::Header(_) | SettingsRow::AddHeader)
            && matches!(previous, Some(SettingsRow::Setting(_)))
        {
            lines.push(Line::raw(""));
            lines.push(Line::styled("   Headers", Style::new().bold()));
        }
        let selected = i == selected_index;
        let marker = if selected { "▶" } else { " " };
        let label_style = if selected {
            Style::new().fg(t.highlight).bold()
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
            spans.extend(input_spans(t, &shown, input.cursor()));
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
                            Span::styled("on (insecure)", Style::new().fg(t.error).bold())
                        }
                        true => Span::raw("on"),
                        false => Span::raw("off"),
                    },
                    None if key == SettingKey::Theme => {
                        Span::styled(auto_theme_note(app.terminal_appearance), dim)
                    }
                    None if key.choices().is_some() => Span::styled(
                        format!("{} (default)", key.choices().unwrap_or_default()[0]),
                        dim,
                    ),
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

    let block = view_block(t, "Settings");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // A blank line before the docs, unless it would push the status message
    // ("Saved to …") off a small screen.
    let spacer = u16::from(usize::from(inner.height) > lines.len() + 1);
    let [top, _, docs] = Layout::vertical([
        Constraint::Length(lines.len() as u16),
        Constraint::Length(spacer),
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
            StatusMessage::Info(text) => Line::styled(text.as_str(), Style::new().fg(t.success)),
            StatusMessage::Error(text) => Line::styled(text.as_str(), Style::new().fg(t.error)),
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
fn input_spans(t: &Theme, shown: &[char], cursor: usize) -> Vec<Span<'static>> {
    let (before, after) = shown.split_at(cursor.min(shown.len()));
    let text = Style::new().fg(t.highlight).underlined();
    let mut spans = vec![Span::styled(before.iter().collect::<String>(), text)];
    match after.split_first() {
        Some((under, rest)) => {
            spans.push(Span::styled(under.to_string(), t.cursor));
            spans.push(Span::styled(rest.iter().collect::<String>(), text));
        }
        None => spans.push(Span::styled("█", Style::new().fg(t.highlight))),
    }
    spans
}

/// The unset theme setting: `auto`, and what it picked.
fn auto_theme_note(detected: Option<Appearance>) -> String {
    let picked = match detected {
        Some(Appearance::Dark) => "dark terminal detected",
        Some(Appearance::Light) => "light terminal detected",
        None => "terminal didn't say: dark",
    };
    format!("auto (default; {picked})")
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
    // The selection gray as background, hotkeys in the active tab's colour.
    let t = app.theme();
    // The outcome of the last action, for a few seconds, at the right end.
    let notice = app
        .notice
        .as_ref()
        .filter(|n| app.now.saturating_duration_since(n.at) < NOTICE_FOR)
        .map(|n| (format!(" {} ", n.text), n.error));
    let notice_width = notice
        .as_ref()
        .map_or(0, |(text, _)| (text.chars().count() as u16).min(area.width));
    // Navigation keys are left to the help popup. Hints are kept while they
    // fit whole next to the notice (they're ordered by importance): a hint
    // cut off mid-word reads as garbage.
    let gap = u16::from(notice_width > 0);
    let room = usize::from(area.width.saturating_sub(notice_width + gap));
    let mut used = 0;
    let bindings: Vec<Binding> = context_bindings(app.context())
        .iter()
        .copied()
        .filter(|b| b.in_bar)
        .take_while(|b| {
            // " key " + " desc" (+ two spaces before the next one).
            let width = b.key.chars().count() + 2 + b.desc.chars().count() + 1;
            let fits = used + width <= room;
            used += width + 2;
            fits
        })
        .collect();
    let spans = binding_spans(&bindings, t.on_accent);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(t.context_bar), area);
    if let Some((text, error)) = notice {
        let color = if error { t.error } else { t.success };
        let [_, right] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(notice_width)]).areas(area);
        frame.render_widget(
            Paragraph::new(Span::styled(text, t.context_bar.fg(color).bold())),
            right,
        );
    }
}

/// `⟳ 4s`: how old the data is; the icon is green while auto-refresh is on,
/// gray while it's off. `⟳ …` while fetching, `✕` after a failed refresh.
/// Only shown when connected (there's nothing to refresh otherwise).
fn refresh_status(app: &App) -> Line<'static> {
    if !matches!(app.connection, ConnectionStatus::Connected { .. }) {
        return Line::default();
    }
    // One block, coloured as a whole: green while auto-refresh is on, gray
    // while it's off (a coloured icon alone doesn't show up on the white bar).
    let t = app.theme();
    let block = if app.auto_refresh {
        t.refresh_on
    } else {
        t.refresh_off
    };
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
        (View::Builds, _) => (
            app.history.fetch_in_flight(),
            app.history.fetched_at,
            matches!(app.history.load, HistoryLoad::Failed(_)),
        ),
        (View::Pipelines | View::Runs | View::Run, _) => (
            app.pipelines.fetch_in_flight(),
            app.pipelines.fetched_at,
            matches!(app.pipelines.load, PipelineLoad::Failed(_)),
        ),
        (View::Build, Some(build)) => (
            build.fetch_in_flight(),
            build.fetched_at,
            matches!(build.load, BuildLoad::Failed(_)),
        ),
        _ => (
            app.jobs.fetch_in_flight(),
            app.jobs.fetched_at,
            matches!(app.jobs.load(), JobsLoad::Failed(_)),
        ),
    };
    let mut text = String::from(" ⟳");
    if busy {
        text.push_str(" …");
    } else if let Some(at) = fetched_at {
        text.push(' ');
        text.push_str(&format_age(app.now.saturating_duration_since(at)));
    }
    if failed && !busy {
        text.push_str(" ✕");
    }
    text.push(' ');
    // Leading space outside the block: a gap even when the connection text
    // is cut off.
    Line::from(vec![Span::raw(" "), Span::styled(text, block)])
}

/// When a build started: `17m ago`, or with `t` the local date and time
/// (`2026-09-29 14:03`).
fn started_text(app: &App, started: SystemTime) -> String {
    if app.absolute_times {
        local_time(app, started, "%Y-%m-%d %H:%M")
    } else {
        let age = app.wall_now.duration_since(started).unwrap_or_default();
        format!("{} ago", format_age(age))
    }
}

/// Width of the [`started_text`] column.
fn started_width(app: &App) -> u16 {
    if app.absolute_times { 16 } else { 8 }
}

/// `at` in the local time zone ([`App::time_zone`]), `strftime`-formatted.
fn local_time(app: &App, at: SystemTime, format: &str) -> String {
    match jiff::Timestamp::try_from(at) {
        Ok(ts) => ts
            .to_zoned(app.time_zone.clone())
            .strftime(format)
            .to_string(),
        Err(_) => "?".into(),
    }
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

fn render_confirm_quit(frame: &mut Frame, area: Rect, t: &Theme) {
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
        .border_style(Style::new().fg(t.error));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The `i` overlay: the connected instance and how Leeroy reaches it.
fn render_info(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let dim = t.dim();
    let row = |label: &str, value: Span<'static>| {
        Line::from(vec![Span::styled(format!(" {label:<12}"), dim), value])
    };
    let mut lines = vec![Line::raw("")];
    match &app.connection {
        ConnectionStatus::Connected { url, info } => {
            lines.push(row("URL", Span::raw(url.clone())));
            let version = info.version.clone().unwrap_or_else(|| "unknown".into());
            lines.push(row("Version", Span::raw(format!("Jenkins {version}"))));
            lines.push(row("User", Span::raw(info.user.clone())));
        }
        _ => {
            lines.push(Line::styled(" Not connected", Style::new().italic()));
            lines.push(Line::styled(" See the settings (0)", dim));
        }
    }
    match &app.info {
        InfoLoad::Loaded(info) => {
            lines.push(row(
                "Security",
                Span::raw(if info.security {
                    "on"
                } else {
                    "off (anyone can do anything)"
                }),
            ));
            lines.push(row(
                "Nodes",
                Span::raw(format!(
                    "{} of {} online",
                    info.nodes_online, info.nodes_total
                )),
            ));
            lines.push(row(
                "Executors",
                Span::raw(format!(
                    "{} of {} busy",
                    info.busy_executors, info.total_executors
                )),
            ));
            lines.push(row("Queue", Span::raw(format!("{} waiting", info.queued))));
            if info.quieting_down {
                lines.push(row(
                    "Status",
                    Span::styled(
                        "⚠ quieting down: no new builds start",
                        Style::new().fg(t.warning).bold(),
                    ),
                ));
            }
            if let Some(description) = &info.description {
                lines.push(row("About", Span::raw(description.clone())));
            }
        }
        InfoLoad::Loading => lines.push(Line::styled(" Loading…", Style::new().italic())),
        InfoLoad::Failed(error) => {
            lines.push(Line::styled(format!(" {error}"), Style::new().fg(t.error)))
        }
        InfoLoad::NotLoaded => {}
    }
    if let Some(config) = app.settings.connection_config() {
        lines.push(Line::raw(""));
        let tls = if config.skip_tls_verify {
            Span::styled("NOT VERIFIED (insecure)", Style::new().fg(t.error).bold())
        } else {
            Span::raw("verified")
        };
        lines.push(row("TLS", tls));
        let proxy = match &config.proxy {
            Some(proxy) => redact_url(proxy),
            None => "none (direct)".into(),
        };
        lines.push(row("Proxy", Span::raw(proxy)));
        let headers: Vec<&str> = config.headers.iter().map(|(n, _)| n.as_str()).collect();
        let headers = if headers.is_empty() {
            "none".to_owned()
        } else {
            headers.join(", ") // names only: values are credentials
        };
        lines.push(row("Headers", Span::raw(headers)));
    }
    lines.push(Line::raw(""));

    let width = (area.width.saturating_sub(4)).min(64);
    let popup = centered(area, width, lines.len() as u16 + 2);
    let block = Block::bordered()
        .title(" Jenkins instance ")
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(t.accent));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The promotions overlay: the run's manual steps, ticked with Space.
fn render_promote(frame: &mut Frame, area: Rect, app: &App) {
    let Some(list) = &app.promote else {
        return;
    };
    let t = app.theme();
    let mut lines = vec![Line::raw("")];
    if list.promotions.is_empty() {
        lines.push(Line::styled(
            "  Nothing to promote in this run",
            Style::new().italic(),
        ));
        lines.push(Line::styled(
            "  (no manual steps after its successful builds)",
            t.dim(),
        ));
    }
    for (i, promotion) in list.promotions.iter().enumerate() {
        let selected = i == list.selected;
        let tick = if list.ticked.contains(&i) {
            "[x]"
        } else {
            "[ ]"
        };
        let base = if selected {
            Style::new().bg(t.selected_bg).bold()
        } else {
            Style::new()
        };
        let dim = Style::new().fg(if selected {
            t.on_selected(t.dim)
        } else {
            t.dim
        });
        lines.push(Line::from(vec![
            Span::styled(if selected { " ▶ " } else { "   " }, base),
            Span::styled(format!("{tick} "), base.fg(t.highlight)),
            Span::styled(
                format!("{} #{}", promotion.from_job, promotion.from_number),
                base.patch(dim),
            ),
            Span::styled(" → ", base.patch(dim)),
            Span::styled(format!("{} ", promotion.job), base),
        ]));
    }
    lines.push(Line::raw(""));
    let width = lines
        .iter()
        .map(|l| l.width() as u16 + 4)
        .max()
        .unwrap_or(40)
        .clamp(44, area.width.saturating_sub(4));
    let popup = centered(area, width, lines.len() as u16 + 2);
    let block = Block::bordered()
        .title(" Promote ")
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(t.accent));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn render_confirm_start(frame: &mut Frame, area: Rect, app: &App) {
    let Some(start) = &app.confirm_start else {
        return;
    };
    let t = app.theme();
    let key = Style::new().bold();
    let mut name = vec![Span::raw("  "), Span::styled(start.name.clone(), key)];
    if start.pipeline && start.name != start.job {
        name.push(Span::styled(format!("  ({})", start.job), t.dim()));
    }
    let how = match start.rerun {
        Some(rerun) if rerun.in_run => "  with the same parameters, in the same run",
        Some(_) => "  with the same parameters",
        None => "  with its default parameters",
    };
    let lines = vec![
        Line::raw(""),
        Line::from(name),
        Line::styled(how, t.dim()),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("y", key),
            Span::raw(" / "),
            Span::styled("Enter", key),
            Span::raw(" start    "),
            Span::styled("n", key),
            Span::raw(" / "),
            Span::styled("Esc", key),
            Span::raw(" cancel"),
        ]),
    ];
    let width = lines
        .iter()
        .map(|l| l.width() as u16 + 4)
        .max()
        .unwrap_or(40)
        .clamp(40, area.width.saturating_sub(4));
    let popup = centered(area, width, lines.len() as u16 + 2);
    let title = match (start.rerun.is_some(), start.pipeline) {
        (true, _) => " Re-run a build? ",
        (false, true) => " Start a new pipeline run? ",
        (false, false) => " Start a new build? ",
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(t.warning));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme();
    let key_style = Style::new().add_modifier(Modifier::BOLD);
    let section =
        |title: &str| Line::styled(format!(" {title}"), Style::new().fg(t.highlight).bold());
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

    // Two per row: the popup has to fit on a 24-line terminal.
    let pairs = |items: Vec<(&str, &str)>| -> Vec<Line<'static>> {
        items
            .chunks(2)
            .map(|pair| {
                let mut spans = Vec::new();
                for (i, (key, desc)) in pair.iter().enumerate() {
                    let key = if i == 0 {
                        format!("{key:>8}")
                    } else {
                        format!("{key:>3}")
                    };
                    spans.push(Span::styled(key, key_style));
                    spans.push(Span::raw(format!("  {desc:<16}")));
                }
                Line::from(spans)
            })
            .collect()
    };

    // Describe the view underneath the popup, not the popup itself.
    let view_context = app.view.context();
    let mut lines = vec![section("Global")];
    lines.extend(pairs(
        GLOBAL_BINDINGS.iter().map(|b| (b.key, b.desc)).collect(),
    ));
    lines.push(Line::raw(""));
    lines.push(section("Tabs"));
    lines.extend(pairs(
        Tab::ALL
            .iter()
            .map(|t| (t.key_label(), t.title()))
            .collect(),
    ));
    // The view's own keys; global ones (listed above) aren't repeated.
    let view_bindings: Vec<Binding> = context_bindings(view_context)
        .iter()
        .copied()
        .filter(|b| !GLOBAL_BINDINGS.iter().any(|g| g.key == b.key))
        .collect();
    if !view_bindings.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section("Navigation"));
        lines.extend(rows(&view_bindings));
    }

    let popup = centered(area, 54, lines.len() as u16 + 2);
    let mut block = Block::bordered()
        .title(" Help ")
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(t.highlight));
    // Version and build time in the bottom border: no extra row needed.
    if !app.about.is_empty() {
        block = block.title_bottom(Line::styled(format!(" {} ", app.about), t.dim()).centered());
    }
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
