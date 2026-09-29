//! Snapshot tests: render the UI into an in-memory buffer and compare to
//! `tests/snapshots/*.snap`. Review changes with `cargo insta review`.

use leeroy::{
    app::{Action, App, ConnectionStatus, Effect, SettingsState, Tab, View},
    builds::{Build, BuildPage, BuildRef, BuildStep, Change, Stage, StageStatus},
    config::{SettingKey, Settings},
    console::ConsoleChunk,
    history::HistoryEntry,
    jenkins::ServerInfo,
    jobs::{Job, JobStatus},
    pipelines,
    theme::Appearance,
    ui,
};
use ratatui::{Terminal, backend::TestBackend, style::Color};

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
fn help_fits_on_every_screen() {
    // The popup lists the keys of the view below it; at 80x24 it must not
    // lose its bottom border on any of them.
    let views = [
        Tab::Jobs,
        Tab::Builds,
        Tab::Pipelines,
        Tab::Runs,
        Tab::Settings,
    ];
    for tab in views {
        let terminal = render_after(&[Action::SwitchTab(tab), Action::ToggleHelp]);
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains('╚'), "{tab:?}:\n{screen}");
    }
    // The build and run views have the most keys.
    let mut app = app_with_build(Some(finished_build()));
    apply(&mut app, &[Action::ToggleHelp]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains('╚'), "build view:\n{screen}");
    let mut app = app_with_pipelines(Tab::Pipelines);
    apply(&mut app, &[Action::OpenBuild, Action::ToggleHelp]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains('╚'), "run view:\n{screen}");
}

#[test]
fn instance_info_overlay() {
    let mut app = app_with_jobs();
    app.update(Action::ToggleInfo);
    let generation = app.connection_generation;
    app.update(Action::InfoFetched {
        generation,
        result: Ok(leeroy::instance::InstanceInfo {
            mode: Some("NORMAL".into()),
            description: Some("Build farm".into()),
            security: true,
            quieting_down: true,
            nodes_online: 2,
            nodes_total: 3,
            busy_executors: 1,
            total_executors: 4,
            queued: 5,
        }),
    });
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("quieting down"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn start_run_prompt_and_notice() {
    let mut app = app_with_pipelines(Tab::Pipelines);
    apply(&mut app, &[Action::SelectNext, Action::RequestStartRun]); // shop
    let prompt = format!("{}", render(&app).backend());
    assert!(prompt.contains("shop  (shop/build)"), "{prompt}");
    insta::assert_snapshot!(prompt);

    apply(
        &mut app,
        &[
            Action::ConfirmStartRun,
            Action::BuildTriggered {
                name: "a run of shop".into(),
                result: Ok(()),
            },
        ],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Started a run of shop"), "{screen}");
    // Gone after a few seconds.
    let later = app.now + std::time::Duration::from_secs(6);
    let wall = app.wall_now;
    apply(&mut app, &[Action::Tick(later, wall)]);
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("Started a run"), "{screen}");
}

#[test]
fn promotions_overlay() {
    let mut app = app_with_pipelines(Tab::Pipelines);
    // libs/core's run: every manual step after a successful build was taken
    // (tests/e2e, with deploy/production after it, is still running).
    apply(&mut app, &[Action::OpenBuild, Action::OpenPromote]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Nothing to promote"), "{screen}");
    apply(&mut app, &[Action::Back, Action::Back]);

    // shop #7 → shop/test #3; its manual step to shop/deploy is open.
    apply(
        &mut app,
        &[
            Action::SelectNext,
            Action::OpenBuild,
            Action::BuildStep(BuildStep::Older),
            Action::OpenPromote,
            Action::TogglePromotion,
        ],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(
        screen.contains("[x] shop/test #3 → shop/deploy"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
    let effects = app.update(Action::ConfirmPromote);
    assert!(matches!(effects.as_slice(), [Effect::Promote { .. }]));
}

#[test]
fn start_build_prompt() {
    let mut app = app_with_jobs();
    apply(&mut app, &[Action::RequestStartRun]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Start a build?"), "{screen}");
    assert!(screen.contains("backend/api/main"), "{screen}");
    assert!(
        !screen.contains("(backend"),
        "no first job for a job: {screen}"
    );
}

#[test]
fn help_open() {
    insta::assert_snapshot!(render_after(&[Action::ToggleHelp]).backend());
}

/// Version and build time in the bottom border (fixed here: `main.rs` sets
/// the real ones).
#[test]
fn help_shows_version_and_build_time() {
    let mut app = test_app();
    app.about = "Leeroy 9.9.9 · built 2026-09-29 13:40 UTC";
    apply(&mut app, &[Action::ToggleHelp]);
    let screen = format!("{}", render(&app).backend());
    assert!(
        screen.contains("═ Leeroy 9.9.9 · built 2026-09-29 13:40 UTC ═"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
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
    insta::assert_snapshot!(header.trim_end(), @" Leeroy  ● https://jenkins.example.com · Jenkins 2.504.1 · hbro   h/?  help   ⟳");
}

#[test]
fn settings_empty() {
    insta::assert_snapshot!(render_after(&[Action::SwitchTab(Tab::Settings)]).backend());
}

#[test]
fn settings_theme_selected() {
    let mut app = test_app();
    app.terminal_appearance = Some(Appearance::Light);
    // Next to last: Theme, with its docs below.
    apply(
        &mut app,
        &[
            Action::SwitchTab(Tab::Settings),
            Action::SelectPrev,
            Action::SelectPrev,
        ],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(
        screen.contains("auto (default; light terminal detected)"),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn switching_theme_recolours_right_away() {
    let bar_bg = |app: &App| render(app).backend().buffer()[(0, 0)].bg;
    let mut app = test_app();
    app.terminal_appearance = Some(Appearance::Dark);
    apply(
        &mut app,
        &[
            Action::SwitchTab(Tab::Settings),
            Action::SelectPrev,
            Action::SelectPrev,
        ],
    );
    assert_eq!(bar_bg(&app), Color::White, "auto on a dark terminal");
    apply(&mut app, &[Action::StartEdit, Action::StartEdit]); // auto → dark → light
    assert_eq!(app.settings.file.get(SettingKey::Theme), Some("light"));
    assert_eq!(bar_bg(&app), Color::Black);
}

#[test]
fn settings_editing_header() {
    let mut actions = vec![
        Action::SwitchTab(Tab::Settings),
        Action::SelectNext,
        Action::SelectNext,
        Action::SelectNext, // "+ add header", after URL / TLS / proxy
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
    let mut actions = vec![Action::SwitchTab(Tab::Settings), Action::StartEdit];
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
    let mut actions = vec![Action::SwitchTab(Tab::Settings), Action::StartEdit];
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
        &[
            Action::SwitchTab(Tab::Settings),
            Action::SelectNext,
            Action::SelectNext,
        ],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("hunter2"), "proxy password shown");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_invalid_proxy() {
    let mut actions = vec![
        Action::SwitchTab(Tab::Settings),
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
        Action::SwitchTab(Tab::Settings),
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

/// Pipelines/Runs data. `libs/core` #12 → backend/api/main #56 →
/// deploy/staging #54 → tests/e2e #49 (running) + tests/smoke #51
/// (unstable); tests/e2e → deploy/production statically. `shop/build` #7 →
/// shop/test #3 (its manual step to shop/deploy not taken), and shop/build #8
/// failed without triggering anything.
const PIPELINES_JSON: &str = r#"{"jobs": [
    {"fullName": "backend/api/main", "color": "blue", "builds": [
        {"number": 56, "result": "SUCCESS", "timestamp": 1700000000000, "duration": 95000,
         "actions": [{"causes": [{"upstreamProject": "libs/core", "upstreamBuild": 12}]}]}]},
    {"fullName": "deploy/production", "color": "blue",
     "upstreamProjects": [{"fullName": "tests/e2e"}], "builds": []},
    {"fullName": "deploy/staging", "color": "blue",
     "upstreamProjects": [{"fullName": "backend/api/main"}],
     "downstreamProjects": [{"fullName": "tests/smoke"}],
     "builds": [
        {"number": 54, "result": "SUCCESS", "timestamp": 1700000240000, "duration": 61000,
         "actions": [{"causes": [{"upstreamProject": "backend/api/main", "upstreamBuild": 56}]}]}]},
    {"fullName": "docs", "color": "disabled", "builds": []},
    {"fullName": "libs/core", "color": "blue", "builds": [
        {"number": 12, "result": "SUCCESS", "timestamp": 1699999760000, "duration": 40000}]},
    {"fullName": "shop/build", "color": "red", "builds": [
        {"number": 8, "result": "FAILURE", "timestamp": 1700000500000, "duration": 20000},
        {"number": 7, "result": "SUCCESS", "timestamp": 1699990000000, "duration": 50000}]},
    {"fullName": "shop/deploy", "color": "notbuilt",
     "upstreamProjects": [{"fullName": "shop/test"}], "builds": []},
    {"fullName": "shop/test", "color": "blue",
     "downstreamProjects": [{"fullName": "shop/deploy"}], "builds": [
        {"number": 3, "result": "SUCCESS", "timestamp": 1699990060000, "duration": 30000,
         "actions": [{"causes": [{"upstreamProject": "shop/build", "upstreamBuild": 7}]}]}]},
    {"fullName": "tests/e2e", "color": "blue_anime", "builds": [
        {"number": 49, "result": null, "building": true, "timestamp": 1700000480000,
         "actions": [{"causes": [{"upstreamProject": "deploy/staging", "upstreamBuild": 54}]}]}]},
    {"fullName": "tests/smoke", "color": "yellow",
     "upstreamProjects": [{"fullName": "deploy/staging"}], "builds": [
        {"number": 51, "result": "UNSTABLE", "timestamp": 1700000400000, "duration": 30000,
         "actions": [{"causes": [{"upstreamProject": "deploy/staging", "upstreamBuild": 54}]}]}]}
]}"#;

/// Connected, with [`PIPELINES_JSON`] loaded, the clock 10 minutes after
/// the newest build started, on `tab`.
fn app_with_pipelines(tab: Tab) -> App {
    use std::time::{Duration, SystemTime};
    let mut app = app_with_jobs();
    app.wall_now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_001_080_000);
    let generation = app.connection_generation;
    app.update(Action::SwitchTab(tab));
    app.update(Action::PipelinesFetched {
        generation,
        result: pipelines::parse(PIPELINES_JSON),
    });
    app
}

#[test]
fn pipelines_list() {
    let app = app_with_pipelines(Tab::Pipelines);
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("docs"), "not a pipeline");
    assert!(screen.contains("shop"), "named by the common prefix");
    insta::assert_snapshot!(screen);
}

#[test]
fn pipeline_latest_run_as_tree_and_boxes() {
    let mut app = app_with_pipelines(Tab::Pipelines);
    // libs/core is the first pipeline (sorted by name).
    apply(&mut app, &[Action::OpenBuild]);
    assert_eq!(app.view, View::Run);
    assert_eq!(app.tab(), Some(Tab::Pipelines));
    let tree = format!("{}", render(&app).backend());
    assert!(
        tree.contains("└─ ● deploy/staging #54"),
        "tree first:\n{tree}"
    );
    insta::assert_snapshot!("pipeline_run_tree", tree);

    apply(&mut app, &[Action::ToggleRunView, Action::SelectNext]);
    let boxes = format!("{}", render(&app).backend());
    assert!(boxes.contains("─▶│ ● backend/api/main #56"), "{boxes}");
    insta::assert_snapshot!("pipeline_run_boxes", boxes);

    // Enter opens the selected build (backend/api/main #56); Esc comes back.
    let effects = app.update(Action::OpenBuild);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::FetchBuild { job, which: BuildRef::Number(56), .. } if job == "backend/api/main"
    )));
    assert_eq!(app.tab(), Some(Tab::Pipelines));
    apply(&mut app, &[Action::Back]);
    assert_eq!(app.view, View::Run);
    apply(&mut app, &[Action::Back]);
    assert_eq!(app.view, View::Pipelines);
}

#[test]
fn runs_list_opens_the_same_run_view() {
    let mut app = app_with_pipelines(Tab::Runs);
    let screen = format!("{}", render(&app).backend());
    insta::assert_snapshot!(screen);
    // The newest run is shop's failed #8, then libs/core #12.
    apply(&mut app, &[Action::SelectNext, Action::OpenBuild]);
    assert_eq!(app.view, View::Run);
    assert_eq!(app.tab(), Some(Tab::Runs));
    let run = format!("{}", render(&app).backend());
    assert!(run.contains("libs/core · #12"), "{run}");
    apply(&mut app, &[Action::Back]);
    assert_eq!(app.view, View::Runs);
}

#[test]
fn stepping_to_an_older_run() {
    let mut app = app_with_pipelines(Tab::Pipelines);
    apply(&mut app, &[Action::SelectNext, Action::OpenBuild]); // shop, latest = #8
    let latest = format!("{}", render(&app).backend());
    assert!(latest.contains("shop · #8 · 2 of 2 · failure"), "{latest}");
    apply(&mut app, &[Action::BuildStep(BuildStep::Older)]);
    let older = format!("{}", render(&app).backend());
    assert!(
        older.contains("shop · #7 · 1 of 2 · partial"),
        "shop/deploy not reached: {older}"
    );
}

#[test]
fn selected_run_rows_stay_readable() {
    for theme in ["dark", "light"] {
        for tab in [Tab::Pipelines, Tab::Runs] {
            let mut app = app_with_pipelines(tab);
            app.settings
                .file
                .set(SettingKey::Theme, Some(theme.to_owned()));
            for row in 0..3 {
                apply(&mut app, &[Action::SelectFirst]);
                for _ in 0..row {
                    apply(&mut app, &[Action::SelectNext]);
                }
                let terminal = render(&app);
                let buffer = terminal.backend().buffer();
                let y = (0..HEIGHT)
                    .find(|&y| buffer[(0, y)].symbol() == "▶")
                    .expect("selected row");
                for x in 0..WIDTH {
                    let cell = &buffer[(x, y)];
                    assert!(
                        cell.symbol() == " " || cell.fg != cell.bg,
                        "{theme} {tab:?} row {row}: {:?} unreadable at x={x}",
                        cell.symbol()
                    );
                }
            }
        }
    }
}

#[test]
fn pipelines_filter() {
    let mut app = app_with_pipelines(Tab::Pipelines);
    let mut actions = vec![Action::StartFilter];
    actions.extend(type_str("shop"));
    actions.push(Action::ConfirmEdit);
    apply(&mut app, &actions);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Pipelines (1)"), "{screen}");
    assert!(screen.contains("filter: shop"), "{screen}");
}

#[test]
fn pipelines_without_relations() {
    let mut app = app_with_jobs();
    let generation = app.connection_generation;
    app.update(Action::SwitchTab(Tab::Pipelines));
    app.update(Action::PipelinesFetched {
        generation,
        result: pipelines::parse(r#"{"jobs": [{"fullName": "docs", "color": "blue"}]}"#),
    });
    insta::assert_snapshot!(render(&app).backend());
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
fn tab_bar_digits_with_settings_on_the_right() {
    use ratatui::style::Color;
    let mut app = app_with_jobs();
    let row = |app: &App| -> (String, Color, Color) {
        let terminal = render(app);
        let buffer = terminal.backend().buffer();
        let text: String = (0..WIDTH).map(|x| buffer[(x, 1)].symbol()).collect();
        (text, buffer[(1, 1)].bg, buffer[(WIDTH - 2, 1)].bg)
    };
    let (text, jobs_bg, settings_bg) = row(&app);
    assert!(text.starts_with(" 1 Jobs "), "{text:?}");
    assert!(text.ends_with(" 0 Settings "), "right-aligned: {text:?}");
    assert_eq!(
        (jobs_bg, settings_bg),
        (Color::Blue, Color::Reset),
        "Jobs active"
    );
    apply(&mut app, &[Action::SwitchTab(Tab::Settings)]);
    let (_, jobs_bg, settings_bg) = row(&app);
    assert_eq!(
        (jobs_bg, settings_bg),
        (Color::Reset, Color::Blue),
        "Settings active"
    );
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
    ends(&app, "· me       h/?  help   ⟳ 0s ");

    // Same compact form with auto-refresh off (only the icon colour differs).
    apply(&mut app, &[Action::ToggleAutoRefresh]);
    let wall = app.wall_now + Duration::from_secs(75);
    apply(
        &mut app,
        &[Action::Tick(base + Duration::from_secs(75), wall)],
    );
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
    for theme in ["dark", "light"] {
        let mut app = app_with_jobs();
        app.settings
            .file
            .set(SettingKey::Theme, Some(theme.to_owned()));
        assert_eq!(app.theme().name, theme);
        selected_rows_readable(&mut app);
    }
}

fn selected_rows_readable(app: &mut App) {
    let rows = app.jobs.visible().len();
    for row in 0..rows {
        apply(app, &[Action::SelectFirst]);
        for _ in 0..row {
            apply(app, &[Action::SelectNext]);
        }
        let label = app.jobs.visible()[row].status.label();
        let terminal = render(app);
        let buffer = terminal.backend().buffer();
        // The selected row is marked with "▶"; check its status cells.
        let y = (0..HEIGHT)
            .find(|&y| (0..2).any(|x| buffer[(x, y)].symbol() == "▶"))
            .expect("selected row");
        let line: String = (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect();
        let start = line.find(label).expect("status label on the row");
        let x = line[..start].chars().count() as u16;
        for dx in 0..label.chars().count() as u16 {
            let cell = &buffer[(x + dx, y)];
            assert_ne!(
                cell.fg,
                cell.bg,
                "{}: {label:?} unreadable on the selected row (fg == bg == {:?})",
                app.theme().name,
                cell.bg
            );
        }
    }
}

/// App in the build view of `backend/api/release-1.2`, with a fixed wall clock
/// so ages in snapshots are stable.
fn app_with_build(build: Option<Build>) -> App {
    use std::time::{Duration, SystemTime};
    let mut app = app_with_jobs();
    app.wall_now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    apply(&mut app, &[Action::SelectNext, Action::OpenBuild]);
    let generation = app.connection_generation;
    let numbers = build
        .as_ref()
        .map_or(vec![], |b| vec![38, 40, 41, b.number]);
    apply(
        &mut app,
        &[Action::BuildFetched {
            generation,
            job: "backend/api/release-1.2".into(),
            which: BuildRef::Latest,
            result: Ok(BuildPage {
                build,
                numbers: Some(numbers),
            }),
        }],
    );
    app
}

fn finished_build() -> Build {
    use std::time::{Duration, SystemTime};
    Build {
        number: 42,
        display_name: "#42".into(),
        result: Some(JobStatus::Failed),
        building: false,
        // 5 minutes before the test's wall clock.
        started: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 - 300),
        duration: Duration::from_secs(133),
        estimated: Some(Duration::from_secs(120)),
        description: Some("Release candidate".into()),
        causes: vec!["Started by user Hans".into()],
        parameters: vec![
            ("ENV".into(), "prod".into()),
            ("DRY_RUN".into(), "false".into()),
        ],
        changes: vec![
            Change {
                commit: Some("0123abcd".into()),
                message: "Fix login redirect".into(),
                author: Some("Alice".into()),
            },
            Change {
                commit: None,
                message: "Bump version".into(),
                author: None,
            },
        ],
        pipeline: true,
        stages: Some(vec![
            stage("Checkout", StageStatus::Success, 4),
            stage("Build", StageStatus::Success, 62),
            stage("Test", StageStatus::Failed, 38),
            stage("Deploy", StageStatus::NotRun, 0),
        ]),
    }
}

fn stage(name: &str, status: StageStatus, secs: u64) -> Stage {
    Stage {
        name: name.into(),
        status,
        duration: std::time::Duration::from_secs(secs),
    }
}

#[test]
fn many_stages_wrap_under_the_label() {
    let mut build = finished_build();
    let names = [
        "Checkout",
        "Compile",
        "Unit tests",
        "Lint",
        "Package",
        "Integration",
        "Publish",
        "Deploy",
    ];
    build.stages = Some(
        names
            .iter()
            .map(|n| stage(n, StageStatus::Success, 75))
            .collect(),
    );
    let screen = format!("{}", render(&app_with_build(Some(build))).backend());
    let lines: Vec<&str> = screen.lines().collect();
    let first = lines.iter().position(|l| l.contains("Stages")).unwrap();
    // Every stage whole, on continuation lines aligned under the first.
    for name in names {
        assert!(
            screen.contains(&format!("● {name} 1m 15s")),
            "{name}:\n{screen}"
        );
    }
    // A wrapped line starts with a stage (no dangling arrow).
    assert!(
        lines[first + 1].starts_with("\"             ● "),
        "{screen}"
    );
}

#[test]
fn build_finished() {
    insta::assert_snapshot!(render(&app_with_build(Some(finished_build()))).backend());
}

#[test]
fn build_running_with_progress() {
    use std::time::Duration;
    let mut build = finished_build();
    build.building = true;
    build.result = None;
    build.duration = Duration::ZERO;
    build.estimated = Some(Duration::from_secs(600)); // 5m of ~10m
    build.changes.clear();
    let screen = format!("{}", render(&app_with_build(Some(build))).backend());
    assert!(screen.contains("⟳ running"), "{screen}");
    assert!(screen.contains("5m 00s of ~10m 00s"), "{screen}");
    assert!(
        screen.contains("███████████████░░░░░░░░░░░░░░░  50%"),
        "{screen}"
    );
}

#[test]
fn build_running_longer_than_usual() {
    use std::time::Duration;
    let mut build = finished_build();
    build.building = true;
    build.estimated = Some(Duration::from_secs(60));
    let screen = format!("{}", render(&app_with_build(Some(build))).backend());
    assert!(screen.contains("longer than usual"), "{screen}");
}

#[test]
fn build_never_run() {
    let screen = format!("{}", render(&app_with_build(None)).backend());
    assert!(screen.contains("No builds yet"), "{screen}");
    assert!(screen.contains("backend/api/release-1.2"), "{screen}");
}

#[test]
fn build_navigation_title_and_deleted_build() {
    let mut app = app_with_build(Some(finished_build()));
    let screen = format!("{}", render(&app).backend());
    assert!(
        screen.contains("backend/api/release-1.2 · #42 · 4 of 4"),
        "{screen}"
    );
    assert!(
        screen.contains("←/→  older/newer   c  console"),
        "Home/End only in the help: {screen}"
    );

    apply(&mut app, &[Action::BuildStep(BuildStep::Older)]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Loading build #41…"), "{screen}");

    let generation = app.connection_generation;
    apply(
        &mut app,
        &[Action::BuildFetched {
            generation,
            job: "backend/api/release-1.2".into(),
            which: BuildRef::Number(41),
            result: Ok(BuildPage {
                build: None,
                numbers: None,
            }),
        }],
    );
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("Build #41 no longer exists"), "{screen}");
}

#[test]
fn quit_confirmation_popup() {
    let mut app = app_with_jobs();
    apply(&mut app, &[Action::RequestQuit]);
    insta::assert_snapshot!(render(&app).backend());
}

/// Console view of the finished build with `lines` lines of output.
fn app_with_console(lines: usize, more: bool) -> App {
    let mut app = app_with_build(Some(finished_build()));
    apply(&mut app, &[Action::OpenConsole]);
    let text: String = (0..lines)
        .map(|i| format!("\x1b[32m[INFO]\x1b[0m\tline {i:03} of the build\n"))
        .collect();
    let generation = app.connection_generation;
    apply(
        &mut app,
        &[Action::ConsoleFetched {
            generation,
            job: "backend/api/release-1.2".into(),
            number: 42,
            start: 0,
            result: Ok(ConsoleChunk {
                bytes: text.clone().into_bytes(),
                next: text.len() as u64,
                more,
            }),
        }],
    );
    app
}

#[test]
fn console_following_a_running_build() {
    let app = app_with_console(100, true);
    insta::assert_snapshot!(render(&app).backend());
}

#[test]
fn console_paused_and_complete() {
    let mut app = app_with_console(100, true);
    render(&app); // records the viewport height
    apply(&mut app, &[Action::SelectFirst]);
    let screen = format!("{}", render(&app).backend());
    assert!(screen.contains("paused (End to follow)"), "{screen}");
    assert!(screen.contains("[INFO]  line 000 of the build"), "{screen}");
    assert!(!screen.contains('\x1b'), "escape codes stripped");

    let done = app_with_console(3, false);
    let screen = format!("{}", render(&done).backend());
    assert!(screen.contains("console · complete"), "{screen}");
}

/// Header and global bar: white across the full width; global hotkeys white
/// on black; the context bar has no context name.
#[test]
fn bar_colours() {
    use ratatui::style::Color;
    let app = app_with_jobs();
    let terminal = render(&app);
    let buffer = terminal.backend().buffer();
    // Header white up to the help hint, which is white on black. (Columns,
    // not byte offsets: the header has multi-byte symbols.)
    let hint = (0..WIDTH)
        .find(|&x| buffer[(x + 1, 0)].symbol() == "h" && buffer[(x + 2, 0)].symbol() == "/")
        .expect("help hint");
    for x in 0..hint {
        assert_eq!(buffer[(x, 0)].bg, Color::White, "header column {x}");
    }
    assert_eq!(
        (buffer[(hint + 1, 0)].fg, buffer[(hint + 1, 0)].bg),
        (Color::White, Color::Black)
    );

    // The last line is the context bar (no global bar): selection gray,
    // hotkeys in the tab blue, no context name.
    let y = HEIGHT - 1;
    let context: String = (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect();
    assert!(
        context.starts_with(" Enter "),
        "no context name: {context:?}"
    );
    assert!(!context.contains("↑/↓"), "navigation keys only in help");
    assert_eq!(buffer[(1, y)].bg, Color::Blue, "hotkey");
    assert_eq!(
        buffer[(WIDTH - 1, y)].bg,
        Color::DarkGray,
        "bar to the right edge"
    );
}

/// The refresh block is coloured as a whole: green = auto-refresh on.
#[test]
fn refresh_block_colour() {
    use ratatui::style::Color;
    let block_bg = |app: &App| {
        let terminal = render(app);
        let buffer = terminal.backend().buffer();
        let x = (0..WIDTH)
            .find(|&x| buffer[(x, 0)].symbol() == "⟳")
            .expect("refresh icon");
        (
            buffer[(x - 1, 0)].bg,
            buffer[(x, 0)].bg,
            buffer[(WIDTH - 1, 0)].bg,
        )
    };
    let mut app = app_with_jobs();
    assert_eq!(block_bg(&app), (Color::Green, Color::Green, Color::Green));
    apply(&mut app, &[Action::ToggleAutoRefresh]);
    assert_eq!(
        block_bg(&app),
        (Color::DarkGray, Color::DarkGray, Color::DarkGray)
    );
}

fn history_entry(
    job: &str,
    number: u64,
    status: Option<JobStatus>,
    age: u64,
    took: u64,
) -> HistoryEntry {
    use std::time::{Duration, SystemTime};
    HistoryEntry {
        job: job.into(),
        number,
        result: status,
        building: status.is_none(),
        started: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 - age),
        duration: Duration::from_secs(took),
    }
}

/// Builds tab with a fixed clock and a 5-row page.
fn app_on_builds_tab() -> App {
    use std::time::{Duration, SystemTime};
    let mut app = app_with_jobs();
    app.wall_now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    app.list_rows.set(5);
    apply(&mut app, &[Action::SwitchTab(Tab::Builds)]);
    let generation = app.connection_generation;
    apply(
        &mut app,
        &[Action::HistoryFetched {
            generation,
            limit: 5,
            result: Ok(vec![
                history_entry("backend/api/release-1.2", 46, None, 150, 0),
                history_entry("backend/api/main", 56, Some(JobStatus::Success), 600, 95),
                history_entry("frontend/web/main", 12, Some(JobStatus::Failed), 3_700, 312),
                history_entry("docs", 7, Some(JobStatus::Aborted), 90_000, 12),
                history_entry("backend/worker", 88, Some(JobStatus::Unstable), 95_000, 61),
                history_entry("backend/api/main", 55, Some(JobStatus::Success), 99_000, 90),
            ]),
        }],
    );
    app
}

#[test]
fn builds_tab_list() {
    let app = app_on_builds_tab();
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains("#55"), "only the guaranteed 5 rows");
    insta::assert_snapshot!(screen);
}

/// `t`: when builds started as local dates and times, in every list and
/// the build view.
#[test]
fn absolute_timestamps() {
    let mut app = app_on_builds_tab();
    app.time_zone = jiff::tz::TimeZone::fixed(jiff::tz::offset(2));
    apply(&mut app, &[Action::ToggleTimestamps]);
    let screen = format!("{}", render(&app).backend());
    assert!(!screen.contains(" ago"), "{screen}");
    insta::assert_snapshot!("builds_tab_absolute_times", screen);

    let mut app = app_with_pipelines(Tab::Runs);
    apply(&mut app, &[Action::ToggleTimestamps]);
    let runs = format!("{}", render(&app).backend());
    assert!(runs.contains("2023-11-14 22:"), "UTC here:\n{runs}");
    insta::assert_snapshot!("runs_absolute_times", runs);
    apply(&mut app, &[Action::OpenBuild]);
    insta::assert_snapshot!(
        "run_tree_absolute_times",
        format!("{}", render(&app).backend())
    );

    let mut app = app_with_build(Some(finished_build()));
    apply(&mut app, &[Action::ToggleTimestamps]);
    let build = format!("{}", render(&app).backend());
    assert!(
        build.contains("Started     2023-11-14 22:08:20"),
        "with seconds:\n{build}"
    );
    insta::assert_snapshot!("build_absolute_times", build);
}

/// The selected build row keeps its dim `#number` readable (dark gray on the
/// dark-gray highlight used to vanish).
#[test]
fn selected_build_row_number_stays_readable() {
    let app = app_on_builds_tab();
    let terminal = render(&app);
    let buffer = terminal.backend().buffer();
    let y = (0..HEIGHT)
        .find(|&y| buffer[(0, y)].symbol() == "▶")
        .expect("selected row");
    let line: String = (0..WIDTH).map(|x| buffer[(x, y)].symbol()).collect();
    let start = line.find("#46").expect("build number");
    let x = line[..start].chars().count() as u16;
    for dx in 0..3 {
        let cell = &buffer[(x + dx, y)];
        assert_ne!(cell.fg, cell.bg, "#46 unreadable on the selected row");
    }
}

/// A production-sized instance: 300 chains of 5 jobs, 20 builds per job.
fn big_pipeline_data() -> pipelines::PipelineData {
    use std::time::{Duration, SystemTime};
    let mut data = pipelines::PipelineData::default();
    for chain in 0..300 {
        let job = |step: usize| format!("team{chain:03}/step{step}");
        for step in 0..5 {
            data.jobs.push(pipelines::PipelineJob {
                name: job(step),
                status: JobStatus::Success,
                building: false,
            });
            if step > 0 {
                data.edges.push((job(step - 1), job(step)));
                data.static_edges.push((job(step - 1), job(step)));
            }
            for n in 1..=20u64 {
                data.builds.push(pipelines::RunBuild {
                    job: job(step),
                    number: n,
                    result: Some(JobStatus::Success),
                    building: false,
                    started: SystemTime::UNIX_EPOCH
                        + Duration::from_secs(1_700_000_000 + n * 3600 + step as u64 * 60),
                    duration: Duration::from_secs(50),
                    parent: (step > 0).then(|| (job(step - 1), n)),
                });
            }
        }
    }
    data.jobs.sort_by(|a, b| a.name.cmp(&b.name));
    data.edges.sort();
    data.static_edges.sort();
    data
}

#[test]
fn long_run_list_scrolls_with_the_selection() {
    let mut app = app_with_jobs();
    let generation = app.connection_generation;
    app.update(Action::SwitchTab(Tab::Runs));
    app.update(Action::PipelinesFetched {
        generation,
        result: Ok(big_pipeline_data()),
    });
    // The selected row is always on screen, and it's the selected run.
    let selected_line = |app: &App| {
        let screen = format!("{}", render(app).backend());
        screen
            .lines()
            .find(|l| l.starts_with("\"▶"))
            .map(str::to_owned)
            .expect("selection on screen")
    };
    let expected = |app: &App| {
        let all = app.pipelines.pipelines();
        let (p, r) = app.pipelines.visible_runs()[app.pipelines.runs.selected];
        format!(
            "{} #{}",
            all[p].name,
            app.pipelines.run_number(&all[p].runs[r])
        )
    };
    apply(&mut app, &[Action::SelectLast]);
    assert!(selected_line(&app).contains(&expected(&app)), "last row");
    for _ in 0..30 {
        apply(&mut app, &[Action::SelectPrev]);
    }
    assert!(
        selected_line(&app).contains(&expected(&app)),
        "scrolled back up"
    );
    apply(&mut app, &[Action::SelectFirst]);
    let screen = format!("{}", render(&app).backend());
    let first_row = screen.lines().nth(3).unwrap();
    assert!(first_row.starts_with("\"▶"), "top of the list: {first_row}");
}

/// Not a correctness test: how long a keypress + frame takes on the
/// Pipeline runs tab of a big instance. `cargo test --release --test render
/// -- --ignored --nocapture pipeline_runs_speed`
#[test]
#[ignore]
fn pipeline_runs_speed() {
    let mut app = app_with_jobs();
    let generation = app.connection_generation;
    app.update(Action::SwitchTab(Tab::Runs));
    app.update(Action::PipelinesFetched {
        generation,
        result: Ok(big_pipeline_data()),
    });
    let start = std::time::Instant::now();
    let rounds = 20;
    for _ in 0..rounds {
        app.update(Action::SelectNext);
        render(&app);
    }
    let per_key = start.elapsed() / rounds;
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let _ = pipelines::pipelines(app.pipelines.data());
    }
    let per_build = start.elapsed() / rounds;
    println!("keypress + frame: {per_key:?}; one pipelines() rebuild: {per_build:?}");
}

#[test]
fn long_job_list_scrolls_with_the_selection() {
    let mut app = app_with_jobs();
    let generation = app.connection_generation;
    app.update(Action::JobsFetched {
        generation,
        result: Ok((0..200)
            .map(|i| job(&format!("job-{i:03}"), JobStatus::Success, false))
            .collect()),
    });
    // The selected row is always on screen, and it's the selected job.
    let check = |app: &App, what: &str| {
        let screen = format!("{}", render(app).backend());
        let line = screen
            .lines()
            .find(|l| l.starts_with("\"▶"))
            .expect("selection on screen");
        let name = &app.jobs.visible()[app.jobs.selected].full_name;
        assert!(line.contains(name.as_str()), "{what}: {line}");
    };
    apply(&mut app, &[Action::SelectLast]);
    check(&app, "last row");
    for _ in 0..30 {
        apply(&mut app, &[Action::SelectPrev]);
    }
    check(&app, "scrolled back up");
    apply(&mut app, &[Action::SelectFirst]);
    let screen = format!("{}", render(&app).backend());
    let first_row = screen.lines().nth(3).unwrap();
    assert!(first_row.starts_with("\"▶"), "top of the list: {first_row}");
}

/// How long a keypress plus a frame takes on the other tabs of a big
/// instance: `cargo test --release --test render -- --ignored --nocapture
/// list_speed`
#[test]
#[ignore]
fn list_speed() {
    use std::time::{Duration, SystemTime};
    fn time(label: &str, app: &mut App, action: Action) {
        let rounds = 20;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            app.update(action.clone());
            render(app);
        }
        println!("{label}: {:?}", start.elapsed() / rounds);
    }
    let mut app = app_with_jobs();
    let generation = app.connection_generation;
    let names: Vec<String> = (0..5000)
        .map(|i| format!("team{:03}/service-{i}/main", i % 300))
        .collect();
    app.update(Action::JobsFetched {
        generation,
        result: Ok(names
            .iter()
            .map(|n| job(n, JobStatus::Success, false))
            .collect()),
    });
    time("jobs", &mut app, Action::SelectNext);
    apply(&mut app, &[Action::StartFilter]);
    apply(&mut app, &type_str("team1 main"));
    time("jobs filtered", &mut app, Action::SelectNext);
    apply(&mut app, &[Action::Back, Action::StartFilter]);
    apply(&mut app, &type_str("service-4999"));
    time("jobs filtered, one match", &mut app, Action::SelectNext);

    app.update(Action::SwitchTab(Tab::Builds));
    let mut entries = Vec::new();
    for (i, name) in names.iter().enumerate() {
        for n in 1..=20u64 {
            entries.push(HistoryEntry {
                job: name.clone(),
                number: n,
                result: Some(JobStatus::Success),
                building: false,
                started: SystemTime::UNIX_EPOCH + Duration::from_secs(n * 100_000 + i as u64),
                duration: Duration::from_secs(50),
            });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.started));
    let generation = app.connection_generation;
    app.update(Action::HistoryFetched {
        generation,
        limit: 20,
        result: Ok(entries),
    });
    time("builds", &mut app, Action::SelectNext);
    apply(&mut app, &[Action::StartFilter]);
    apply(&mut app, &type_str("team1 main"));
    time("builds filtered", &mut app, Action::SelectNext);
    apply(&mut app, &[Action::Back, Action::StartFilter]);
    apply(&mut app, &type_str("service-4999"));
    time("builds filtered, one match", &mut app, Action::SelectNext);

    // One wide run (a job fanning out to 200 steps, half never reached) in
    // an instance with many other edges.
    let mut data = big_pipeline_data();
    let root = "wide/root".to_owned();
    let steps: Vec<String> = (0..200).map(|i| format!("wide/step{i:03}")).collect();
    for name in std::iter::once(&root).chain(&steps) {
        data.jobs.push(pipelines::PipelineJob {
            name: name.clone(),
            status: JobStatus::Success,
            building: false,
        });
    }
    for (i, step) in steps.iter().enumerate() {
        data.edges.push((root.clone(), step.clone()));
        data.static_edges.push((root.clone(), step.clone()));
        if i % 2 == 0 {
            data.builds.push(pipelines::RunBuild {
                job: step.clone(),
                number: 1,
                result: Some(JobStatus::Success),
                building: false,
                started: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_060),
                duration: Duration::from_secs(50),
                parent: Some((root.clone(), 1)),
            });
        }
    }
    data.builds.push(pipelines::RunBuild {
        job: root.clone(),
        number: 1,
        result: Some(JobStatus::Success),
        building: false,
        started: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
        duration: Duration::from_secs(50),
        parent: None,
    });
    data.jobs.sort_by(|a, b| a.name.cmp(&b.name));
    data.edges.sort();
    data.static_edges.sort();
    app.update(Action::SwitchTab(Tab::Runs));
    let generation = app.connection_generation;
    app.update(Action::PipelinesFetched {
        generation,
        result: Ok(data),
    });
    app.update(Action::OpenBuild);
    assert_eq!(app.view, View::Run, "the newest run: the wide one");
    time("run view (tree)", &mut app, Action::SelectNext);
    app.update(Action::ToggleRunView);
    time("run view (boxes)", &mut app, Action::SelectNext);
}
