//! Snapshot tests: render the UI into an in-memory buffer and compare to
//! `tests/snapshots/*.snap`. Review changes with `cargo insta review`.

use leeroy::{
    app::{Action, App, ConnectionStatus, SettingsState},
    config::{SettingKey, Settings},
    jenkins::ServerInfo,
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
    insta::assert_snapshot!(header.trim_end(), @" Leeroy  ● https://jenkins.example.com · Jenkins 2.504.1 · hbro");
}

#[test]
fn settings_empty() {
    insta::assert_snapshot!(render_after(&[Action::OpenSettings]).backend());
}

#[test]
fn settings_editing_token() {
    let mut actions = vec![
        Action::OpenSettings,
        Action::SelectNext,
        Action::SelectNext, // the token
        Action::StartEdit,
    ];
    actions.extend(type_str("s3cret"));
    let terminal = render_after(&actions);
    let screen = format!("{}", terminal.backend());
    assert!(!screen.contains("s3cret"), "token shown in clear");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_saved_with_env_override() {
    let mut app = test_app();
    app.settings
        .env
        .set(SettingKey::JenkinsUsername, Some("ci-bot".into()));
    let mut actions = vec![Action::OpenSettings, Action::StartEdit];
    actions.extend(type_str("https://ci.example.com"));
    actions.extend([
        Action::ConfirmEdit,
        Action::SettingsSaved(Ok(())),
        Action::SelectNext,
    ]);
    apply(&mut app, &actions);
    app.settings
        .file
        .set(SettingKey::JenkinsToken, Some("s3cret".into()));
    insta::assert_snapshot!(render(&app).backend());
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
    apply(&mut app, &[Action::OpenSettings, Action::SelectPrev]);
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("hunter2"), "proxy password shown");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_invalid_proxy() {
    let mut actions = vec![Action::OpenSettings, Action::SelectPrev, Action::StartEdit];
    actions.extend(type_str("ftp://proxy:21"));
    actions.push(Action::ConfirmEdit);
    insta::assert_snapshot!(render_after(&actions).backend());
}

#[test]
fn settings_editing_proxy_cursor_mid_text() {
    let mut actions = vec![Action::OpenSettings, Action::SelectPrev, Action::StartEdit];
    actions.extend(type_str("socks5h://me:hunter2@bastion:1080"));
    actions.extend([Action::CursorHome, Action::CursorRight, Action::CursorRight]);
    let screen = format!("{}", render_after(&actions).backend());
    assert!(
        !screen.contains("hunter2"),
        "proxy password shown while editing"
    );
    assert!(
        screen.contains("socks5h://me:•••••••@bastion:1080"),
        "{screen}"
    );
    assert!(
        !screen.contains('█'),
        "cursor block shown although cursor is mid-text"
    );
}
