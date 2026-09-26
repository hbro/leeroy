//! Snapshot tests: render the UI into an in-memory buffer and compare to
//! `tests/snapshots/*.snap`. Review changes with `cargo insta review`.

use leeroy::{
    app::{Action, App, ConnectionStatus, SettingsState},
    config::{SettingKey, Settings},
    jenkins::ServerInfo,
    jobs::{Job, JobStatus},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

/// Fixed so snapshots don't depend on the machine running them.
fn test_app() -> App {
    App::new(SettingsState::new(
        "/home/user/.config/leeroy/config.toml".into(),
        Settings::default(),
        Settings::default(),
    ))
}

fn apply(app: &mut App, actions: &[Action]) {
    for action in actions {
        app.update(action.clone());
    }
}

/// Build an app, apply `actions` in order, and render one frame.
fn render_after(actions: &[Action]) -> Terminal<TestBackend> {
    let mut app = test_app();
    apply(&mut app, actions);
    render(&app)
}

fn render(app: &App) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal
}

fn type_str(text: &str) -> Vec<Action> {
    text.chars().map(Action::Input).collect()
}

#[test]
fn initial_screen() {
    insta::assert_snapshot!(render_after(&[]).backend());
}

#[test]
fn help_open() {
    insta::assert_snapshot!(render_after(&[Action::ToggleHelp]).backend());
}

#[test]
fn help_closed_again() {
    // Must match the initial screen exactly: the popup leaves no residue.
    let closed = render_after(&[Action::ToggleHelp, Action::ToggleHelp]);
    let initial = render_after(&[]);
    assert_eq!(closed.backend().buffer(), initial.backend().buffer());
}

#[test]
fn header_shows_connected_instance() {
    let mut app = test_app();
    app.connection = ConnectionStatus::Connected {
        url: "https://jenkins.example.com".into(),
        info: ServerInfo {
            version: Some("2.504.1".into()),
            user: "hbro".into(),
        },
    };
    let terminal = render(&app);
    let header: String = terminal.backend().buffer().content()[..WIDTH as usize]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    insta::assert_snapshot!(header.trim_end(), @" Leeroy  ● https://jenkins.example.com · Jenkins 2.504.1 · hbro               ⟳");
}

#[test]
fn settings_empty() {
    insta::assert_snapshot!(render_after(&[Action::OpenSettings]).backend());
}

#[test]
fn settings_editing_header() {
    let mut actions = vec![
        Action::OpenSettings,
        Action::SelectPrev, // wraps to "+ add header"
        Action::StartEdit,
    ];
    actions.extend(type_str("Authorization: Basic s3cret"));
    let terminal = render_after(&actions);
    let screen = format!("{}", terminal.backend());
    // While editing, the value is shown as typed; masking returns afterwards.
    assert!(screen.contains("Authorization: Basic s3cret"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_saved_with_env_header() {
    let mut app = test_app();
    app.settings
        .env
        .set_header("Authorization", Some("Basic from-env".into()));
    app.settings
        .file
        .set_header("X-Forwarded-User", Some("me".into()));
    let mut actions = vec![Action::OpenSettings, Action::StartEdit];
    actions.extend(type_str("https://ci.example.com"));
    actions.extend([Action::ConfirmEdit, Action::SettingsSaved(Ok(()))]);
    apply(&mut app, &actions);
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("from-env"), "env header value shown");
    insta::assert_snapshot!(screen);
}

#[test]
fn url_credentials_masked_except_while_editing() {
    let mut app = test_app();
    let mut actions = vec![Action::OpenSettings, Action::StartEdit];
    actions.extend(type_str("https://me:s3cret@ci.example.com"));
    apply(&mut app, &actions);
    let editing = format!("{}", render(&app).backend());
    assert!(
        editing.contains("https://me:s3cret@ci.example.com"),
        "shown as typed while editing: {editing}"
    );

    apply(&mut app, &[Action::ConfirmEdit]);
    let settings = format!("{}", render(&app).backend());
    apply(&mut app, &[Action::Back]);
    let jobs = format!("{}", render(&app).backend());
    for screen in [&settings, &jobs] {
        assert!(!screen.contains("s3cret"), "{screen}");
    }
    assert!(jobs.contains("https://me:••••@ci.example.com"), "{jobs}");
}

#[test]
fn jobs_connecting() {
    let mut app = test_app();
    app.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    app.start();
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn jobs_connection_failed() {
    let mut app = test_app();
    app.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    app.start();
    app.update(Action::ConnectFinished {
        generation: app.connection_generation,
        result: Err("cannot connect: dns error: failed to lookup address information".into()),
    });
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn jobs_connected_anonymously_with_tls_unverified() {
    let mut app = test_app();
    app.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    app.settings
        .file
        .set(SettingKey::JenkinsSkipTlsVerify, Some("true".into()));
    app.start();
    app.update(Action::ConnectFinished {
        generation: app.connection_generation,
        result: Ok(ServerInfo {
            version: Some("2.504.1".into()),
            user: "anonymous".into(),
        }),
    });
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn settings_proxy_selected_with_system_fallback() {
    let mut app = test_app();
    app.settings.proxy_env = leeroy::proxy::ProxyEnv::from_env(|name| {
        (name == "HTTPS_PROXY").then(|| "http://me:hunter2@corp-proxy:3128".into())
    });
    apply(
        &mut app,
        &[Action::OpenSettings, Action::SelectNext, Action::SelectNext],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("hunter2"), "proxy password shown");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_invalid_proxy() {
    let mut actions = vec![
        Action::OpenSettings,
        Action::SelectNext,
        Action::SelectNext, // proxy
        Action::StartEdit,
    ];
    actions.extend(type_str("ftp://proxy:21"));
    actions.push(Action::ConfirmEdit);
    insta::assert_snapshot!(render_after(&actions).backend());
}

#[test]
fn settings_editing_proxy_cursor_mid_text() {
    let mut actions = vec![
        Action::OpenSettings,
        Action::SelectNext,
        Action::SelectNext, // proxy
        Action::StartEdit,
    ];
    actions.extend(type_str("socks5h://me:hunter2@bastion:1080"));
    actions.extend([Action::CursorHome, Action::CursorRight, Action::CursorRight]);
    let screen = format!("{}", render_after(&actions).backend());
    assert!(
        screen.contains("socks5h://me:hunter2@bastion:1080"),
        "shown as typed while editing: {screen}"
    );
    assert!(
        !screen.contains('█'),
        "cursor block shown although cursor is mid-text"
    );
}

fn job(name: &str, status: JobStatus, building: bool) -> Job {
    Job {
        full_name: name.into(),
        url: String::new(),
        status,
        building,
    }
}

/// Connected to a fake instance with a mix of job states.
fn app_with_jobs() -> App {
    let mut app = test_app();
    app.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    app.start();
    let generation = app.connection_generation;
    app.update(Action::ConnectFinished {
        generation,
        result: Ok(ServerInfo {
            version: Some("2.504.1".into()),
            user: "me".into(),
        }),
    });
    app.update(Action::JobsFetched {
        generation,
        result: Ok(vec![
            job("backend/api/main", JobStatus::Success, false),
            job("backend/api/release-1.2", JobStatus::Failed, true),
            job("backend/worker", JobStatus::Unstable, false),
            job("docs", JobStatus::Disabled, false),
            job("frontend/web/main", JobStatus::Aborted, false),
            job("frontend/web/pr-42", JobStatus::NotBuilt, false),
        ]),
    });
    app
}

#[test]
fn jobs_list() {
    let mut app = app_with_jobs();
    apply(&mut app, &[Action::SelectNext]);
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn jobs_loading() {
    let mut app = app_with_jobs();
    // First load: placeholder.
    let mut fresh = test_app();
    fresh.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    fresh.start();
    let generation = fresh.connection_generation;
    apply(
        &mut fresh,
        &[Action::ConnectFinished {
            generation,
            result: Ok(ServerInfo {
                version: None,
                user: "me".into(),
            }),
        }],
    );
    let screen = format!("{}", render(&fresh).backend());
    assert!(screen.contains("Loading jobs…"), "{screen}");

    // Reload: list stays, title says so.
    apply(&mut app, &[Action::Refresh]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Jobs (6) · refreshing…"), "{screen}");
    assert!(screen.contains("backend/api/main"), "{screen}");
}

#[test]
fn jobs_filter_typing() {
    let mut app = app_with_jobs();
    let mut actions = vec![Action::StartFilter];
    actions.extend(type_str("api"));
    apply(&mut app, &actions);
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn jobs_filter_applied_no_match() {
    let mut app = app_with_jobs();
    let mut actions = vec![Action::StartFilter];
    actions.extend(type_str("nope"));
    actions.push(Action::ConfirmEdit);
    apply(&mut app, &actions);
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn tab_bar_marks_settings_while_open() {
    let mut app = app_with_jobs();
    let row = |app: &App| -> String {
        let terminal = render(app);
        terminal.backend().buffer().content()[WIDTH as usize..2 * WIDTH as usize]
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    };
    assert_eq!(row(&app), "  F1 Jobs");
    apply(&mut app, &[Action::OpenSettings]);
    assert_eq!(row(&app), "  F1 Jobs   Settings");
}

/// First line of the screen (the header).
fn header_line(app: &App) -> String {
    let terminal = render(app);
    terminal.backend().buffer().content()[..WIDTH as usize]
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn refresh_status_in_header() {
    use std::time::Duration;
    let mut app = app_with_jobs();
    let base = app.now;
    let ends = |app: &App, tail: &str| {
        let line = header_line(app);
        assert!(line.ends_with(tail), "expected …{tail:?} in {line:?}");
    };
    ends(&app, "· me                   ⟳ 0s ");

    // Same compact form with auto-refresh off (only the icon colour differs).
    apply(&mut app, &[Action::ToggleAutoRefresh]);
    apply(&mut app, &[Action::Tick(base + Duration::from_secs(75))]);
    ends(&app, "⟳ 1m ");

    // R again: stale data is refreshed right away.
    apply(&mut app, &[Action::ToggleAutoRefresh]);
    ends(&app, "⟳ … ");

    // A failed refresh keeps the last good age and adds a marker.
    let generation = app.connection_generation;
    apply(
        &mut app,
        &[Action::JobsFetched {
            generation,
            result: Err("HTTP 502".into()),
        }],
    );
    ends(&app, "⟳ 1m ✕ ");
    insta::assert_snapshot!(render(&app).backend());

    // Not connected: no refresh status at all.
    let line = header_line(&test_app());
    assert!(!line.contains('⟳'), "{line}");
}

/// Every status must stay readable on the selected row: its colour may not
/// equal the highlight background (dark-gray statuses used to vanish there).
#[test]
fn selected_row_status_stays_readable() {
    let mut app = app_with_jobs();
    let rows = app.jobs.visible().len();
    for row in 0..rows {
        apply(&mut app, &[Action::SelectFirst]);
        for _ in 0..row {
            apply(&mut app, &[Action::SelectNext]);
        }
        let label = app.jobs.visible()[row].status.label();
        let terminal = render(&app);
        let buffer = terminal.backend().buffer();
        // The selected row is marked with "▶"; check its status cells.
        let y = (0..HEIGHT)
            .find(|&y| buffer[(1, y)].symbol() == "▶")
            .expect("selected row");
        let line: String = (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect();
        let start = line.find(label).expect("status label on the row");
        let x = line[..start].chars().count() as u16;
        for dx in 0..label.chars().count() as u16 {
            let cell = &buffer[(x + dx, y)];
            assert_ne!(
                cell.fg, cell.bg,
                "{label:?} unreadable on the selected row (fg == bg == {:?})",
                cell.bg
            );
        }
    }
}
