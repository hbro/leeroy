use std::{path::PathBuf, time::Duration};

use color_eyre::Result;
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use leeroy::{
    app::{Action, App},
    event::map_key,
    ui,
};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

const TICK_RATE: Duration = Duration::from_millis(250);

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let _log_guard = init_logging()?;
    tracing::info!("starting leeroy");

    // ratatui::init enters raw mode + alternate screen and installs a panic
    // hook that restores the terminal before the panic message is printed.
    let terminal = ratatui::init();
    let result = run(terminal).await;
    ratatui::restore();
    result
}

async fn run(mut terminal: DefaultTerminal) -> Result<()> {
    let mut app = App::new();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(TICK_RATE);
    // Background tasks (e.g. Jenkins API calls) send their results here.
    let (_action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    while app.running {
        terminal.draw(|frame| ui::render(frame, &app))?;

        let action = tokio::select! {
            _ = tick.tick() => Some(Action::Tick),
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
            if action != Action::Tick {
                tracing::debug!(?action, "update");
            }
            app.update(action);
        }
    }
    tracing::info!("exiting");
    Ok(())
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
