//! Snapshot tests: render the UI into an in-memory buffer and compare to
//! `tests/snapshots/*.snap`. Review changes with `cargo insta review`.

use leeroy::{
    app::{Action, App, ConnectionStatus, SettingsState},
    config::{SettingKey, Settings},
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
    };
    let terminal = render(&app);
    let header: String = terminal.backend().buffer().content()[..WIDTH as usize]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    insta::assert_snapshot!(header.trim_end(), @" Leeroy  ● https://jenkins.example.com");
}

#[test]
fn settings_empty() {
    insta::assert_snapshot!(render_after(&[Action::OpenSettings]).backend());
}

#[test]
fn settings_editing_token() {
    let mut actions = vec![
        Action::OpenSettings,
        Action::SelectPrev, // wraps to the token
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
fn jobs_shows_configured_instance() {
    let mut app = test_app();
    app.settings.file.set(
        SettingKey::JenkinsUrl,
        Some("https://ci.example.com".into()),
    );
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Jenkins instance: https://ci.example.com"));
}
