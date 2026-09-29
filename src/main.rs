use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Parser;
use color_eyre::{Result, eyre::eyre};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use leeroy::{
    app::{Action, App, Effect, SettingsState},
    config::{self, Settings},
    event::map_key,
    jenkins,
    proxy::ProxyEnv,
    theme::Appearance,
    ui,
};
use ratatui::DefaultTerminal;
use terminal_colorsaurus::{QueryOptions, ThemeMode};
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    task::AbortHandle,
};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

const TICK_RATE: Duration = Duration::from_millis(250);

/// Leeroy — a terminal UI for Jenkins.
///
/// Every setting can also be set with an env var, which takes precedence over
/// the config file: LEEROY_JENKINS_URL, LEEROY_JENKINS_SKIP_TLS_VERIFY,
/// LEEROY_PROXY_URL, LEEROY_REFRESH_AUTO, LEEROY_REFRESH_INTERVAL, and
/// LEEROY_JENKINS_HEADERS_<NAME> per HTTP header (e.g.
/// LEEROY_JENKINS_HEADERS_AUTHORIZATION). Without a proxy setting, the usual
/// HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY env vars apply.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Config file to use. Overrides $LEEROY_CONFIG
    /// [default: $XDG_CONFIG_HOME/leeroy/config.toml (~/.config/leeroy/config.toml),
    /// or an existing ~/.leeroy/config.toml; on Windows %APPDATA%\leeroy\config.toml]
    // $LEEROY_CONFIG is handled by config::resolve_path rather than clap's
    // `env`, which rejects an empty value instead of ignoring it.
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse args and load config before touching the terminal, so errors
    // (and --help) print normally.
    let cli = Cli::parse();
    color_eyre::install()?;
    let _log_guard = init_logging()?;
    tracing::info!("starting Leeroy");

    let location = config::resolve_path(cli.config, |name| std::env::var_os(name), Path::exists)
        .ok_or_else(|| {
            eyre!(
                "cannot determine config location (no $HOME, or %APPDATA% on Windows): \
                 set LEEROY_CONFIG or pass --config"
            )
        })?;
    if let Some(ignored) = &location.ignored {
        // Printed before the alternate screen, so it's visible after quitting.
        eprintln!(
            "warning: both {} and {} exist; using {}",
            location.path.display(),
            ignored.display(),
            location.path.display()
        );
        tracing::warn!(used = %location.path.display(), ignored = %ignored.display(), "two config files found");
    }
    let path = location.path;
    let file = config::load(&path)?;
    let env = Settings::from_env(std::env::vars_os())?;
    tracing::info!(path = %path.display(), "loaded config");
    file.validate(|key| {
        let (table, name) = key.toml_path();
        format!("{table}.{name} in {}", path.display())
    })?;
    let mut settings = SettingsState::new(path, file, env);
    settings.proxy_env = ProxyEnv::from_env(|name| std::env::var_os(name));
    let mut app = App::new(settings);
    app.terminal_appearance = detect_appearance();
    // Absolute build times are shown in local time (falls back to UTC).
    app.time_zone = jiff::tz::TimeZone::system();

    // ratatui::init enters raw mode + alternate screen and installs a panic
    // hook that restores the terminal before the panic message is printed.
    let terminal = ratatui::init();
    let result = run(terminal, app).await;
    ratatui::restore();
    result
}

async fn run(mut terminal: DefaultTerminal, mut app: App) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(TICK_RATE);
    // Background tasks (e.g. Jenkins API calls) send their results here.
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();
    let mut executor = Executor {
        tx: action_tx,
        connect_task: None,
        fetch_task: None,
        build_task: None,
        console_task: None,
        history_task: None,
        pipelines_task: None,
    };
    for effect in app.start() {
        executor.run(effect);
    }

    while app.running {
        terminal.draw(|frame| ui::render(frame, &app))?;

        let action = tokio::select! {
            now = tick.tick() => Some(Action::Tick(now.into_std(), std::time::SystemTime::now())),
            Some(action) = action_rx.recv() => Some(action),
            maybe_event = events.next() => match maybe_event {
                Some(Ok(Event::Key(key))) => map_key(&app, key),
                // Resize and others: fall through to a redraw.
                Some(Ok(_)) => None,
                Some(Err(err)) => return Err(err.into()),
                None => Some(Action::Quit),
            },
        };

        if let Some(action) = action {
            // Input chars may be part of a secret: don't log them.
            if !matches!(action, Action::Tick(..) | Action::Input(_)) {
                tracing::debug!(?action, "update");
            }
            for effect in app.update(action) {
                executor.run(effect);
            }
        }
    }
    tracing::info!("exiting");
    Ok(())
}

/// Runs side effects requested by `App::update`, reporting back via `tx`.
struct Executor {
    tx: UnboundedSender<Action>,
    /// The in-flight connection check, aborted when a newer one starts.
    connect_task: Option<AbortHandle>,
    /// The in-flight job list fetch, aborted when a newer one starts.
    fetch_task: Option<AbortHandle>,
    /// The in-flight build fetch, aborted when one for another job starts.
    build_task: Option<AbortHandle>,
    /// The in-flight console fetch, aborted when another starts.
    console_task: Option<AbortHandle>,
    /// The in-flight build history fetch.
    history_task: Option<AbortHandle>,
    pipelines_task: Option<AbortHandle>,
}

impl Executor {
    fn run(&mut self, effect: Effect) {
        let tx = &self.tx;
        match effect {
            Effect::SaveSettings { path, settings } => {
                // Inline on purpose: a tiny local write, and running saves in
                // order means an older save can never overwrite a newer one.
                let result = config::save(&path, &settings).map_err(|err| format!("{err:#}"));
                match &result {
                    Ok(()) => tracing::info!(path = %path.display(), "saved config"),
                    Err(err) => tracing::error!(%err, "saving config failed"),
                }
                let _ = tx.send(Action::SettingsSaved(result));
            }
            Effect::Connect { generation, config } => {
                if let Some(previous) = self.connect_task.take() {
                    previous.abort();
                }
                tracing::info!(?config, generation, "connecting");
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::check(&config).await;
                    match &result {
                        Ok(info) => tracing::info!(?info, generation, "connected"),
                        Err(err) => tracing::warn!(%err, generation, "connection failed"),
                    }
                    let _ = tx.send(Action::ConnectFinished { generation, result });
                });
                self.connect_task = Some(task.abort_handle());
            }
            Effect::FetchJobs { generation, config } => {
                if let Some(previous) = self.fetch_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_jobs(&config).await;
                    match &result {
                        Ok(jobs) => tracing::info!(count = jobs.len(), generation, "fetched jobs"),
                        Err(err) => tracing::warn!(%err, generation, "fetching jobs failed"),
                    }
                    let _ = tx.send(Action::JobsFetched { generation, result });
                });
                self.fetch_task = Some(task.abort_handle());
            }
            Effect::FetchHistory {
                generation,
                limit,
                config,
            } => {
                if let Some(previous) = self.history_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_history(&config, limit).await;
                    match &result {
                        Ok(entries) => {
                            tracing::info!(limit, builds = entries.len(), "fetched history")
                        }
                        Err(err) => tracing::warn!(%err, "fetching history failed"),
                    }
                    let _ = tx.send(Action::HistoryFetched {
                        generation,
                        limit,
                        result,
                    });
                });
                self.history_task = Some(task.abort_handle());
            }
            Effect::TriggerBuild { job, name, config } => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = jenkins::trigger_build(&config, &job).await;
                    match &result {
                        Ok(()) => tracing::info!(job, "triggered a build"),
                        Err(err) => tracing::warn!(job, %err, "triggering a build failed"),
                    }
                    let _ = tx.send(Action::BuildTriggered { name, result });
                });
            }
            Effect::Promote { promotions, config } => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    // One after the other, like clicking them in turn.
                    let mut results = Vec::new();
                    for promotion in &promotions {
                        let result = jenkins::promote(&config, promotion).await;
                        match &result {
                            Ok(linked) => tracing::info!(to = promotion.job, linked, "promoted"),
                            Err(err) => {
                                tracing::warn!(to = promotion.job, %err, "promotion failed")
                            }
                        }
                        results.push((promotion.clone(), result));
                    }
                    let _ = tx.send(Action::Promoted(results));
                });
            }
            Effect::OpenBrowser { url } => {
                let result = open_url(&url);
                if let Err(err) = &result {
                    tracing::warn!(%err, "opening a browser failed");
                }
                let _ = tx.send(Action::BrowserOpened(result));
            }
            Effect::FetchInstanceInfo { generation, config } => {
                // Short-lived and harmless to overlap: not tracked or aborted.
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = jenkins::fetch_instance_info(&config).await;
                    if let Err(err) = &result {
                        tracing::warn!(%err, "fetching instance info failed");
                    }
                    let _ = tx.send(Action::InfoFetched { generation, result });
                });
            }
            Effect::FetchPipelines { generation, config } => {
                if let Some(previous) = self.pipelines_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_pipelines(&config).await;
                    match &result {
                        Ok(data) => tracing::info!(
                            jobs = data.jobs.len(),
                            edges = data.edges.len(),
                            builds = data.builds.len(),
                            "fetched pipelines"
                        ),
                        Err(err) => tracing::warn!(%err, "fetching pipelines failed"),
                    }
                    let _ = tx.send(Action::PipelinesFetched { generation, result });
                });
                self.pipelines_task = Some(task.abort_handle());
            }
            Effect::FetchConsole {
                generation,
                job,
                number,
                start,
                config,
            } => {
                if let Some(previous) = self.console_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_console(&config, &job, number, start).await;
                    match &result {
                        Ok(chunk) => tracing::debug!(
                            %job, number, start, bytes = chunk.bytes.len(), more = chunk.more,
                            "fetched console"
                        ),
                        Err(err) => tracing::warn!(%job, number, %err, "fetching console failed"),
                    }
                    let _ = tx.send(Action::ConsoleFetched {
                        generation,
                        job,
                        number,
                        start,
                        result,
                    });
                });
                self.console_task = Some(task.abort_handle());
            }
            Effect::FetchBuild {
                generation,
                job,
                which,
                want_numbers,
                config,
            } => {
                if let Some(previous) = self.build_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_build(&config, &job, which, want_numbers).await;
                    match &result {
                        Ok(page) => tracing::info!(
                            %job,
                            ?which,
                            number = ?page.build.as_ref().map(|b| b.number),
                            builds = ?page.numbers.as_ref().map(Vec::len),
                            "fetched build"
                        ),
                        Err(err) => tracing::warn!(%job, ?which, %err, "fetching build failed"),
                    }
                    let _ = tx.send(Action::BuildFetched {
                        generation,
                        job,
                        which,
                        result,
                    });
                });
                self.build_task = Some(task.abort_handle());
            }
        }
    }
}

/// The terminal's background brightness, for the `auto` theme. Asked before
/// raw mode and the event stream start: the answer arrives on stdin. Terminals
/// that don't support the query are recognised quickly; the timeout only
/// covers ones that don't answer at all.
fn detect_appearance() -> Option<Appearance> {
    let mut options = QueryOptions::default();
    options.timeout = Duration::from_millis(500);
    match terminal_colorsaurus::theme_mode(options) {
        Ok(ThemeMode::Dark) => Some(Appearance::Dark),
        Ok(ThemeMode::Light) => Some(Appearance::Light),
        Err(err) => {
            tracing::info!(%err, "terminal background unknown");
            None
        }
    }
    .inspect(|appearance| tracing::info!(?appearance, "terminal background detected"))
}

/// Open `url` with the system's handler, detached and silent (stdout and
/// stderr belong to the TUI).
fn open_url(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        // `start` via cmd would treat `&` in the URL as a command separator.
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    let mut child = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| err.to_string())?;
    // Reap it when it exits, without waiting here.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Log to a file: stdout belongs to the TUI. Location: see
/// `config::log_path_for`. Filter via `RUST_LOG` (default `info`).
fn init_logging() -> Result<WorkerGuard> {
    let path = config::log_path_for(config::Platform::current(), |name| std::env::var_os(name));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    Ok(guard)
}
