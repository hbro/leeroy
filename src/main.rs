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
    ui,
};
use ratatui::DefaultTerminal;
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
    /// or an existing ~/.leeroy/config.toml]
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
            eyre!("cannot determine config location: set $HOME or $LEEROY_CONFIG, or pass --config")
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
    let app = App::new(settings);

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
            Effect::FetchBuild {
                generation,
                job,
                which,
                config,
            } => {
                if let Some(previous) = self.build_task.take() {
                    previous.abort();
                }
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = jenkins::fetch_build(&config, &job, which).await;
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

/// Log to a file: stdout belongs to the TUI.
/// Path: `$LEEROY_LOG`, else `$XDG_STATE_HOME/leeroy/leeroy.log`
/// (defaulting to `~/.local/state`). Filter via `RUST_LOG` (default `info`).
fn init_logging() -> Result<WorkerGuard> {
    let path = match std::env::var_os("LEEROY_LOG") {
        Some(p) => PathBuf::from(p),
        None => std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(std::env::temp_dir)
            .join("leeroy/leeroy.log"),
    };
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
