use std::{
    path::PathBuf,
    time::{Instant, SystemTime},
};

use crate::{
    builds::{BuildLoad, BuildPage, BuildRef, BuildStep, BuildView},
    config::{Section, SettingKey, Settings, header_env_var, parse_header, redact_url},
    console::{ConsoleChunk, ConsoleLoad, ConsoleView},
    history::{HistoryEntry, HistoryLoad, HistoryState},
    input::TextInput,
    jenkins::{ConnectionConfig, ServerInfo},
    jobs::{Job, JobsLoad, JobsState},
    pipelines::{PipelineData, PipelineLoad, PipelineState, RunRef, RunView},
    proxy::{ProxyEnv, SystemProxy},
    theme::{Appearance, Theme},
};

/// Everything that can change application state.
///
/// Key presses, timer ticks and results of [`Effect`]s are all translated
/// into actions, so every state transition goes through [`App::update`] and
/// can be unit tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Quit right away (Ctrl-C, or confirming the quit prompt).
    Quit,
    /// `q`: quit, after confirmation if the `ui.confirm_quit` setting is on.
    RequestQuit,
    ToggleHelp,
    /// Close the current context (overlay, edit, sub-view). No-op at the root view.
    Back,
    /// Timer tick with the current time, monotonic (auto-refresh, data age)
    /// and wall clock (build start times). The only way time enters the
    /// core, so tests control it.
    Tick(Instant, SystemTime),
    SwitchTab(Tab),
    SelectNext,
    SelectPrev,
    SelectPageDown,
    SelectPageUp,
    SelectFirst,
    SelectLast,
    /// Jobs tab: open the `/` filter input.
    StartFilter,
    /// Jobs tab: show the selected job's most recent build.
    OpenBuild,
    /// Build view: go to another build of the same job.
    BuildStep(BuildStep),
    /// Build view: open the build's console output.
    OpenConsole,
    /// Run view: switch between the tree and boxes.
    ToggleRunView,
    /// Console: scroll sideways.
    ScrollLeft,
    ScrollRight,
    /// Reload the job list (or reconnect if the connection failed).
    Refresh,
    /// `R`: turn auto-refresh on/off for this session.
    ToggleAutoRefresh,
    /// Start editing the selected setting.
    StartEdit,
    /// Insert a char at the cursor. Never logged: may be part of a secret.
    Input(char),
    /// Delete the char before the cursor (Backspace).
    DeleteChar,
    /// Delete the char under the cursor (Delete).
    DeleteForward,
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
    ClearInput,
    /// Store the edited value and save the config file.
    ConfirmEdit,
    /// Outcome of [`Effect::SaveSettings`]; `Err` holds a printable message.
    SettingsSaved(Result<(), String>),
    /// Outcome of [`Effect::Connect`] attempt `generation`.
    ConnectFinished {
        generation: u64,
        result: Result<ServerInfo, String>,
    },
    /// Outcome of [`Effect::FetchJobs`] for connection `generation`.
    JobsFetched {
        generation: u64,
        result: Result<Vec<Job>, String>,
    },
    /// Outcome of [`Effect::FetchHistory`] (made with `limit` builds per job).
    HistoryFetched {
        generation: u64,
        limit: usize,
        result: Result<Vec<HistoryEntry>, String>,
    },
    /// Outcome of [`Effect::FetchPipelines`].
    PipelinesFetched {
        generation: u64,
        result: Result<PipelineData, String>,
    },
    /// Outcome of [`Effect::FetchConsole`] (the chunk from `start`).
    ConsoleFetched {
        generation: u64,
        job: String,
        number: u64,
        start: u64,
        result: Result<ConsoleChunk, String>,
    },
    /// Outcome of [`Effect::FetchBuild`].
    BuildFetched {
        generation: u64,
        job: String,
        which: BuildRef,
        result: Result<BuildPage, String>,
    },
}

/// Side effects requested by [`App::update`]. The IO shell (`main.rs`) runs
/// them and reports back with an [`Action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    SaveSettings {
        path: PathBuf,
        settings: Settings,
    },
    /// Check the connection; answer with [`Action::ConnectFinished`]. A newer
    /// `Connect` supersedes (and may cancel) older ones.
    Connect {
        generation: u64,
        config: ConnectionConfig,
    },
    /// Load the job list; answer with [`Action::JobsFetched`]. `generation`
    /// is the connection it belongs to.
    FetchJobs {
        generation: u64,
        config: ConnectionConfig,
    },
    /// Load console output of build `number` of `job` from byte `start`;
    /// answer with [`Action::ConsoleFetched`].
    FetchConsole {
        generation: u64,
        job: String,
        number: u64,
        start: u64,
        config: ConnectionConfig,
    },
    /// Load `which` build of `job`; answer with [`Action::BuildFetched`].
    /// `want_numbers`: also get the job's build numbers (for a numbered build).
    FetchBuild {
        generation: u64,
        job: String,
        which: BuildRef,
        want_numbers: bool,
        config: ConnectionConfig,
    },
    /// Load the build history: every job's newest `limit` builds; answer with
    /// [`Action::HistoryFetched`].
    FetchHistory {
        generation: u64,
        limit: usize,
        config: ConnectionConfig,
    },
    /// Load job relations and recent builds' causes (Pipelines and Runs
    /// tabs); answer with [`Action::PipelinesFetched`].
    FetchPipelines {
        generation: u64,
        config: ConnectionConfig,
    },
}

/// Tabs, selected with digit keys (`1` = first, `0` = Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Jobs,
    Builds,
    Pipelines,
    Runs,
    Settings,
}

impl Tab {
    /// In tab-bar order.
    pub const ALL: [Tab; 5] = [
        Tab::Jobs,
        Tab::Builds,
        Tab::Pipelines,
        Tab::Runs,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Jobs => "Jobs",
            Tab::Builds => "Builds",
            Tab::Pipelines => "Pipelines",
            Tab::Runs => "Runs",
            Tab::Settings => "Settings",
        }
    }

    /// The key selecting this tab. Content tabs are `1`–`9` in order;
    /// Settings is `0`, shown at the far right of the tab bar.
    pub fn key(self) -> char {
        match self {
            Tab::Jobs => '1',
            Tab::Builds => '2',
            Tab::Pipelines => '3',
            Tab::Runs => '4',
            Tab::Settings => '0',
        }
    }

    /// [`Self::key`] as a label.
    pub fn key_label(self) -> &'static str {
        match self {
            Tab::Jobs => "1",
            Tab::Builds => "2",
            Tab::Pipelines => "3",
            Tab::Runs => "4",
            Tab::Settings => "0",
        }
    }

    pub fn from_key(c: char) -> Option<Tab> {
        Tab::ALL.into_iter().find(|tab| tab.key() == c)
    }

    fn view(self) -> View {
        match self {
            Tab::Jobs => View::Jobs,
            Tab::Builds => View::Builds,
            Tab::Pipelines => View::Pipelines,
            Tab::Runs => View::Runs,
            Tab::Settings => View::Settings,
        }
    }
}

/// How often a running build's console output is polled while shown.
pub const CONSOLE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// Builds per page before the renderer has measured the screen.
pub const DEFAULT_HISTORY_PAGE: usize = 30;

/// Rows moved by PgUp/PgDn.
pub const PAGE: isize = 10;

/// The main view being displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Jobs,
    /// Build history across all jobs.
    Builds,
    /// How jobs trigger each other, as a graph.
    Pipelines,
    /// Runs of all pipelines.
    Runs,
    /// One run of a pipeline, as boxes or a tree (from Pipelines or Runs).
    Run,
    /// A job's most recent build (within the Jobs tab).
    Build,
    /// A build's console output (within the Jobs tab).
    Console,
    Settings,
}

impl View {
    /// The tab this view belongs to.
    pub fn tab(self) -> Option<Tab> {
        match self {
            View::Builds => Some(Tab::Builds),
            View::Pipelines => Some(Tab::Pipelines),
            View::Runs => Some(Tab::Runs),
            // Opened from Pipelines unless said otherwise (see `App::tab`).
            View::Run => Some(Tab::Pipelines),
            // Opened from the Jobs tab unless said otherwise (see `App::tab`).
            View::Jobs | View::Build | View::Console => Some(Tab::Jobs),
            View::Settings => Some(Tab::Settings),
        }
    }

    /// The context this view provides when nothing is overlaid on it.
    pub fn context(self) -> Context {
        match self {
            View::Jobs => Context::Jobs,
            View::Builds => Context::Builds,
            View::Pipelines => Context::Pipelines,
            View::Runs => Context::Runs,
            View::Run => Context::Run,
            View::Build => Context::Build,
            View::Console => Context::Console,
            View::Settings => Context::Settings,
        }
    }
}

/// What currently has focus: decides which context keybindings apply and
/// what the context bar shows. Overlays take precedence over the view below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// "Quit Leeroy?" prompt, on top of everything else.
    ConfirmQuit,
    Jobs,
    /// Jobs tab with a filter applied (Esc clears it).
    JobsFiltered,
    /// Typing the `/` filter.
    JobsFilter,
    Builds,
    /// Builds tab with a filter applied.
    BuildsFiltered,
    /// Typing the Builds tab's `/` filter.
    BuildsFilter,
    Pipelines,
    PipelinesFiltered,
    PipelinesFilter,
    Runs,
    RunsFiltered,
    RunsFilter,
    Run,
    Build,
    Console,
    Settings,
    EditSetting,
    Help,
}

impl Context {
    pub fn title(self) -> &'static str {
        match self {
            Context::Jobs | Context::JobsFiltered => "Jobs",
            Context::JobsFilter
            | Context::BuildsFilter
            | Context::PipelinesFilter
            | Context::RunsFilter => "Filter",
            Context::Builds | Context::BuildsFiltered => "Builds",
            Context::Pipelines | Context::PipelinesFiltered => "Pipelines",
            Context::Runs | Context::RunsFiltered => "Runs",
            Context::Run => "Run",
            Context::ConfirmQuit => "Quit?",
            Context::Build => "Build",
            Context::Console => "Console",
            Context::Settings => "Settings",
            Context::EditSetting => "Edit",
            Context::Help => "Help",
        }
    }

    /// Whether [`Action::Back`] can close this context.
    pub fn closable(self) -> bool {
        match self {
            // Tabs aren't closed with Esc; they're switched with F-keys.
            Context::Jobs
            | Context::Builds
            | Context::Pipelines
            | Context::Runs
            | Context::Settings => false,
            Context::JobsFiltered
            | Context::JobsFilter
            | Context::BuildsFiltered
            | Context::BuildsFilter
            | Context::PipelinesFiltered
            | Context::PipelinesFilter
            | Context::RunsFiltered
            | Context::RunsFilter
            | Context::Run
            | Context::ConfirmQuit
            | Context::Build
            | Context::Console
            | Context::EditSetting
            | Context::Help => true,
        }
    }

    /// Text input: all printable keys go to the input, global keys are off.
    pub fn captures_input(self) -> bool {
        matches!(
            self,
            Context::EditSetting
                | Context::JobsFilter
                | Context::BuildsFilter
                | Context::PipelinesFilter
                | Context::RunsFilter
        )
    }

    /// Global keys (bottom bar) don't work here: text input, or a prompt
    /// that only takes its own answers. The global bar is dimmed then.
    pub fn global_keys_off(self) -> bool {
        self.captures_input() || self == Context::ConfirmQuit
    }
}

/// Connection to the Jenkins instance, shown in the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    NotConfigured,
    Connecting { url: String },
    Connected { url: String, info: ServerInfo },
    Failed { url: String, error: String },
}

/// One row of the settings view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsRow {
    Setting(SettingKey),
    /// A custom HTTP header, by (effective) name.
    Header(String),
    /// "+ add header".
    AddHeader,
}

impl SettingsRow {
    pub fn section(&self) -> Section {
        match self {
            SettingsRow::Setting(key) => key.section(),
            SettingsRow::Header(_) | SettingsRow::AddHeader => Section::Jenkins,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMessage {
    Info(String),
    Error(String),
}

/// Settings as loaded at startup plus the settings view's UI state.
#[derive(Debug)]
pub struct SettingsState {
    /// Config file location.
    pub path: PathBuf,
    /// Values from (and saved to) the config file.
    pub file: Settings,
    /// Values from env vars. They win over `file` and can't be edited here.
    pub env: Settings,
    /// Conventional proxy env vars, the fallback when no proxy is configured.
    pub proxy_env: ProxyEnv,
    pub selected: usize,
    /// Input while editing the selected setting.
    pub editing: Option<TextInput>,
    pub message: Option<StatusMessage>,
}

impl SettingsState {
    pub fn new(path: PathBuf, file: Settings, env: Settings) -> Self {
        Self {
            path,
            file,
            env,
            proxy_env: ProxyEnv::default(),
            selected: 0,
            editing: None,
            message: None,
        }
    }

    /// Rows in display order: fixed settings, then headers, then "+ add header".
    pub fn rows(&self) -> Vec<SettingsRow> {
        let in_section = |section: Section| {
            SettingKey::ALL
                .into_iter()
                .filter(move |key| key.section() == section)
                .map(SettingsRow::Setting)
        };
        // Jenkins connection (with its headers), then the application settings.
        let mut rows: Vec<SettingsRow> = in_section(Section::Jenkins).collect();
        rows.extend(
            self.effective()
                .jenkins
                .headers
                .into_keys()
                .map(SettingsRow::Header),
        );
        rows.push(SettingsRow::AddHeader);
        rows.extend(in_section(Section::Application));
        rows
    }

    pub fn selected_row(&self) -> SettingsRow {
        let rows = self.rows();
        rows[self.selected.min(rows.len() - 1)].clone()
    }

    fn select(&mut self, row: &SettingsRow) {
        if let Some(i) = self.rows().iter().position(|r| r == row) {
            self.selected = i;
        }
    }

    /// Whether header `name` comes from an env var (and so is read-only here).
    pub fn is_header_overridden(&self, name: &str) -> bool {
        self.env.header(name).is_some()
    }

    /// What the app should actually use: env vars over the file.
    pub fn effective(&self) -> Settings {
        self.file.overlaid(&self.env)
    }

    pub fn is_overridden(&self, key: SettingKey) -> bool {
        self.env.get(key).is_some()
    }

    /// The system proxy that applies when no proxy setting is set.
    pub fn system_proxy(&self) -> SystemProxy {
        self.proxy_env
            .for_target(self.effective().get(SettingKey::JenkinsUrl))
    }

    /// How to reach Jenkins with the current settings; `None` without a URL.
    pub fn connection_config(&self) -> Option<ConnectionConfig> {
        let settings = self.effective();
        let url = settings.get(SettingKey::JenkinsUrl)?.to_owned();
        let proxy = match settings.get(SettingKey::ProxyUrl) {
            Some(proxy) => Some(proxy.to_owned()),
            None => match self.system_proxy() {
                SystemProxy::Proxy { url, .. } => Some(url),
                SystemProxy::Bypassed { .. } | SystemProxy::None => None,
            },
        };
        let headers = settings.jenkins.headers.clone().into_iter().collect();
        Some(ConnectionConfig {
            url,
            headers,
            proxy,
            skip_tls_verify: settings.is_on(SettingKey::JenkinsSkipTlsVerify),
        })
    }
}

#[derive(Debug)]
pub struct App {
    pub running: bool,
    /// The quit confirmation prompt is open.
    pub confirm_quit: bool,
    pub view: View,
    pub show_help: bool,
    pub connection: ConnectionStatus,
    /// Id of the latest connection attempt; results of older ones are stale.
    pub connection_generation: u64,
    pub settings: SettingsState,
    pub jobs: JobsState,
    pub history: HistoryState,
    /// Pipelines and Runs tabs (one shared fetch).
    pub pipelines: PipelineState,
    /// The open run view (with [`View::Run`]).
    pub run: Option<RunView>,
    /// Height of the list area, recorded by the renderer: how many builds the
    /// Builds tab asks for per page.
    pub list_rows: std::cell::Cell<u16>,
    /// The open build view (with [`View::Build`] and [`View::Console`]).
    pub build: Option<BuildView>,
    /// The open console view (with [`View::Console`]).
    pub console: Option<ConsoleView>,
    /// Time of the latest [`Action::Tick`].
    pub now: Instant,
    /// Wall-clock time of the latest [`Action::Tick`].
    pub wall_now: SystemTime,
    /// Auto-refresh for this session; starts from the `refresh.auto` setting.
    pub auto_refresh: bool,
    /// Terminal background as reported at startup (`None`: unknown); picks
    /// the theme when `ui.theme` is `auto`.
    pub terminal_appearance: Option<Appearance>,
}

impl Default for App {
    fn default() -> Self {
        Self::new(SettingsState::new(
            PathBuf::from(crate::config::FILE_NAME),
            Settings::default(),
            Settings::default(),
        ))
    }
}

impl App {
    pub fn new(settings: SettingsState) -> Self {
        Self {
            running: true,
            confirm_quit: false,
            view: View::Jobs,
            show_help: false,
            connection: ConnectionStatus::NotConfigured,
            connection_generation: 0,
            auto_refresh: settings.effective().is_on(SettingKey::RefreshAuto),
            settings,
            jobs: JobsState::default(),
            history: HistoryState::default(),
            pipelines: PipelineState::default(),
            run: None,
            list_rows: std::cell::Cell::new(0),
            build: None,
            console: None,
            now: Instant::now(),
            wall_now: SystemTime::now(),
            terminal_appearance: None,
        }
    }

    /// The colours to draw with: the `ui.theme` setting, `auto` resolved
    /// against the detected terminal background. Read on every frame, so a
    /// changed setting shows right away.
    pub fn theme(&self) -> &'static Theme {
        self.settings
            .effective()
            .theme()
            .resolve(self.terminal_appearance)
    }

    /// Effects to run once at startup: connect if configured.
    pub fn start(&mut self) -> Vec<Effect> {
        self.connect().into_iter().collect()
    }

    /// Start a new connection attempt with the current settings.
    fn connect(&mut self) -> Option<Effect> {
        self.connection_generation += 1;
        // Jobs of the previous connection may belong to another instance.
        self.jobs.load = JobsLoad::NotLoaded;
        self.jobs.refreshing = false;
        self.history.load = HistoryLoad::NotLoaded;
        self.history.refreshing = false;
        self.pipelines.load = PipelineLoad::NotLoaded;
        self.pipelines.refreshing = false;
        let run_origin = self.run_origin();
        if matches!(self.view, View::Build | View::Console) {
            self.view = self.build_origin();
        }
        if self.view == View::Run {
            self.view = run_origin;
        }
        self.build = None;
        self.console = None;
        self.run = None;
        let Some(config) = self.settings.connection_config() else {
            self.connection = ConnectionStatus::NotConfigured;
            return None;
        };
        // Display-only copy: credentials in the URL are never shown.
        self.connection = ConnectionStatus::Connecting {
            url: redact_url(&config.url),
        };
        Some(Effect::Connect {
            generation: self.connection_generation,
            config,
        })
    }

    pub fn context(&self) -> Context {
        if self.confirm_quit {
            Context::ConfirmQuit
        } else if self.show_help {
            Context::Help
        } else if self.view == View::Settings && self.settings.editing.is_some() {
            Context::EditSetting
        } else if self.view == View::Jobs && self.jobs.filter_input.is_some() {
            Context::JobsFilter
        } else if self.view == View::Jobs && !self.jobs.filter.is_empty() {
            Context::JobsFiltered
        } else if self.view == View::Builds && self.history.filter_input.is_some() {
            Context::BuildsFilter
        } else if self.view == View::Builds && !self.history.filter.is_empty() {
            Context::BuildsFiltered
        } else if self.view == View::Pipelines && self.pipelines.list.filter_input.is_some() {
            Context::PipelinesFilter
        } else if self.view == View::Pipelines && !self.pipelines.list.filter.is_empty() {
            Context::PipelinesFiltered
        } else if self.view == View::Runs && self.pipelines.runs.filter_input.is_some() {
            Context::RunsFilter
        } else if self.view == View::Runs && !self.pipelines.runs.filter.is_empty() {
            Context::RunsFiltered
        } else {
            self.view.context()
        }
    }

    /// Apply an action. Must stay free of IO so tests can drive it directly;
    /// IO is requested by returning [`Effect`]s.
    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let context = self.context();
        let s = &mut self.settings;
        match action {
            Action::Quit => self.running = false,
            Action::RequestQuit => {
                if self.confirm_quit || !self.settings.effective().is_on(SettingKey::ConfirmQuit) {
                    self.running = false; // q q, or no confirmation wanted
                } else {
                    self.confirm_quit = true;
                }
            }
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Back => self.back(),
            Action::Tick(now, wall) => {
                self.now = now;
                self.wall_now = wall;
                let mut effects = self.auto_refresh_if_due();
                effects.extend(self.console_poll_if_due());
                return effects;
            }
            Action::ToggleAutoRefresh => {
                self.auto_refresh = !self.auto_refresh;
                // Turning it on refreshes right away when the data is stale.
                return self.auto_refresh_if_due();
            }
            Action::SwitchTab(tab) => {
                if self.view == View::Settings && tab != Tab::Settings {
                    self.settings.message = None; // "Saved to …" is stale by now
                }
                self.view = tab.view();
                self.show_help = false;
                if tab == Tab::Builds && self.history.load == HistoryLoad::NotLoaded {
                    return self.fetch_history(self.history_page());
                }
                if matches!(tab, Tab::Pipelines | Tab::Runs)
                    && self.pipelines.load == PipelineLoad::NotLoaded
                {
                    return self.fetch_pipelines();
                }
            }
            Action::StartFilter if self.view == View::Pipelines => {
                let list = &mut self.pipelines.list;
                list.filter_input = Some(TextInput::new(&list.filter));
            }
            Action::StartFilter if self.view == View::Runs => {
                let runs = &mut self.pipelines.runs;
                runs.filter_input = Some(TextInput::new(&runs.filter));
            }
            Action::StartFilter if self.view == View::Builds => {
                self.history.filter_input = Some(TextInput::new(&self.history.filter));
            }
            Action::StartFilter => {
                self.jobs.filter_input = Some(TextInput::new(&self.jobs.filter));
            }
            Action::OpenBuild if self.view == View::Pipelines => {
                // The pipeline's latest run.
                let pipelines = self.pipelines.visible_pipelines();
                if let Some(p) = pipelines.get(self.pipelines.list.selected) {
                    self.run = Some(RunView::new(
                        p.first_job.clone(),
                        RunRef::Latest,
                        View::Pipelines,
                    ));
                    self.view = View::Run;
                }
            }
            Action::OpenBuild if self.view == View::Runs => {
                let pipelines = self.pipelines.pipelines();
                let runs = self.pipelines.visible_runs(&pipelines);
                if let Some(&(p, r)) = runs.get(self.pipelines.runs.selected) {
                    let number = self.pipelines.run_number(&pipelines[p].runs[r]);
                    self.run = Some(RunView::new(
                        pipelines[p].first_job.clone(),
                        RunRef::Number(number),
                        View::Runs,
                    ));
                    self.view = View::Run;
                }
            }
            Action::OpenBuild if self.view == View::Run => {
                // The selected build of the run, at that number.
                let Some((job, number)) = self.selected_run_build() else {
                    return Vec::new();
                };
                self.view = View::Build;
                let mut view = BuildView::new(job);
                view.target = BuildRef::Number(number);
                view.origin = View::Run;
                self.build = Some(view);
                return self.build_effect().into_iter().collect();
            }
            Action::ToggleRunView => {
                if let Some(run) = self.run.as_mut().filter(|_| self.view == View::Run) {
                    run.tree = !run.tree;
                }
            }
            Action::OpenBuild if self.view == View::Builds => {
                let Some((job, number)) = self
                    .history
                    .visible()
                    .get(self.history.selected)
                    .map(|e| (e.job.clone(), e.number))
                else {
                    return Vec::new();
                };
                // Straight to that build; Esc comes back here.
                self.view = View::Build;
                let mut view = BuildView::new(job);
                view.target = BuildRef::Number(number);
                view.origin = View::Builds;
                self.build = Some(view);
                return self.build_effect().into_iter().collect();
            }
            Action::OpenBuild => {
                let Some(job) = self
                    .jobs
                    .visible()
                    .get(self.jobs.selected)
                    .map(|j| j.full_name.clone())
                else {
                    return Vec::new();
                };
                self.view = View::Build;
                // A new view starts out loading: its first fetch goes out
                // unconditionally (an older job's fetch is simply ignored).
                self.build = Some(BuildView::new(job));
                return self.build_effect().into_iter().collect();
            }
            Action::Refresh => return self.refresh(),
            Action::SelectNext
            | Action::SelectPrev
            | Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast
                if self.view == View::Jobs =>
            {
                let delta = match action {
                    Action::SelectNext => 1,
                    Action::SelectPrev => -1,
                    Action::SelectPageDown => PAGE,
                    Action::SelectPageUp => -PAGE,
                    Action::SelectFirst => isize::MIN / 2,
                    _ => isize::MAX / 2,
                };
                self.jobs.move_selection(delta);
            }
            Action::SelectNext
            | Action::SelectPrev
            | Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast
                if self.view == View::Builds =>
            {
                let page = self.history_page() as isize;
                let delta = match action {
                    Action::SelectNext => 1,
                    Action::SelectPrev => -1,
                    Action::SelectPageDown => page,
                    Action::SelectPageUp => -page,
                    Action::SelectFirst => isize::MIN / 2,
                    _ => isize::MAX / 2,
                };
                let was_at_end = self.history.at_end();
                self.history.move_selection(delta);
                // Moving down at the last row loads the next page.
                if delta > 0 && was_at_end && self.history.has_more() {
                    return self.fetch_history(self.history.limit + self.history_page());
                }
            }
            Action::SelectNext
            | Action::SelectPrev
            | Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast
                if matches!(self.view, View::Pipelines | View::Runs | View::Run) =>
            {
                let page = self.history_page() as isize;
                let delta = match action {
                    Action::SelectNext => 1,
                    Action::SelectPrev => -1,
                    Action::SelectPageDown => page,
                    Action::SelectPageUp => -page,
                    Action::SelectFirst => isize::MIN / 2,
                    _ => isize::MAX / 2,
                };
                let pipelines = self.pipelines.pipelines();
                match self.view {
                    View::Pipelines => {
                        let len = self.pipelines.visible_pipelines().len();
                        self.pipelines.list.move_selection(delta, len);
                    }
                    View::Runs => {
                        let len = self.pipelines.visible_runs(&pipelines).len();
                        self.pipelines.runs.move_selection(delta, len);
                    }
                    _ => {
                        let Some(view) = self.run.as_ref() else {
                            return Vec::new();
                        };
                        let len = self
                            .pipelines
                            .find_run(&pipelines, view)
                            .map_or(0, |(p, r)| p.runs[r].builds.len());
                        if let Some(view) = self.run.as_mut() {
                            view.selected = (view.selected as isize + delta)
                                .clamp(0, len.saturating_sub(1) as isize)
                                as usize;
                        }
                    }
                }
            }
            Action::SelectNext
            | Action::SelectPrev
            | Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast
                if self.view == View::Build =>
            {
                // Scrolling; the renderer clamps to the content height.
                if let Some(build) = &mut self.build {
                    let scroll = match action {
                        Action::SelectNext => build.scroll.saturating_add(1),
                        Action::SelectPrev => build.scroll.saturating_sub(1),
                        Action::SelectPageDown => build.scroll.saturating_add(PAGE as u16),
                        Action::SelectPageUp => build.scroll.saturating_sub(PAGE as u16),
                        Action::SelectFirst => 0,
                        _ => u16::MAX,
                    };
                    build.scroll = scroll.min(build.max_scroll.get());
                }
            }
            Action::SelectNext
            | Action::SelectPrev
            | Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast
            | Action::ScrollLeft
            | Action::ScrollRight
                if self.view == View::Console =>
            {
                if let Some(console) = &mut self.console {
                    match action {
                        Action::SelectNext => console.scroll_by(1),
                        Action::SelectPrev => console.scroll_by(-1),
                        Action::SelectPageDown => console.page_by(1),
                        Action::SelectPageUp => console.page_by(-1),
                        Action::SelectFirst => console.scroll_home(),
                        Action::SelectLast => console.scroll_end(),
                        Action::ScrollRight => {
                            console.left += crate::console::HSCROLL_STEP;
                        }
                        _ => {
                            console.left =
                                console.left.saturating_sub(crate::console::HSCROLL_STEP);
                        }
                    }
                }
            }
            Action::ScrollLeft | Action::ScrollRight => {}
            Action::OpenConsole => {
                let Some(number) = self
                    .build
                    .as_ref()
                    .filter(|_| self.view == View::Build)
                    .and_then(|b| match &b.load {
                        BuildLoad::Loaded(Some(build)) => Some(build.number),
                        _ => None,
                    })
                else {
                    return Vec::new(); // no build to show the output of
                };
                let job = self
                    .build
                    .as_ref()
                    .map(|b| b.job.clone())
                    .unwrap_or_default();
                self.view = View::Console;
                self.console = Some(ConsoleView::new(job, number));
                return self.console_effect().into_iter().collect();
            }
            Action::ConsoleFetched {
                generation,
                job,
                number,
                start,
                result,
            } => {
                let now = self.now;
                let Some(console) = self.console.as_mut().filter(|c| {
                    c.job == job
                        && c.number == number
                        && c.offset == start
                        && generation == self.connection_generation
                }) else {
                    return Vec::new(); // another build, a stale chunk, or reconnected
                };
                console.in_flight = false;
                console.attempted_at = Some(now);
                match result {
                    Ok(chunk) => {
                        console.push(chunk);
                        console.fetched_at = Some(now);
                        console.last_error = None;
                        console.load = ConsoleLoad::Loaded;
                    }
                    Err(error) if console.load == ConsoleLoad::Loaded => {
                        // Keep what we have; the next poll retries.
                        console.last_error = Some(error);
                    }
                    Err(error) => console.load = ConsoleLoad::Failed(error),
                }
            }
            Action::SelectPageDown
            | Action::SelectPageUp
            | Action::SelectFirst
            | Action::SelectLast => {}
            Action::SelectNext | Action::SelectPrev => {
                // While editing, moving away saves the field first; an invalid
                // value keeps the edit open instead of moving.
                let effects = self.confirm_edit();
                let s = &mut self.settings;
                if s.editing.is_none() {
                    let len = s.rows().len();
                    s.selected = s.selected.min(len - 1);
                    s.selected = match action {
                        Action::SelectNext => (s.selected + 1) % len,
                        _ => (s.selected + len - 1) % len,
                    };
                }
                return effects;
            }
            Action::StartEdit => {
                s.message = None;
                match s.selected_row() {
                    SettingsRow::Setting(key) if s.is_overridden(key) => {
                        s.message = Some(read_only(key.label(), key.env_var()));
                    }
                    SettingsRow::Setting(key) if key.choices().is_some() => {
                        // Next value, wrapping; the first (default) = unset.
                        let choices = key.choices().expect("checked");
                        let current = s.file.get(key).unwrap_or(choices[0]);
                        let i = choices.iter().position(|c| *c == current).unwrap_or(0);
                        let next = choices[(i + 1) % choices.len()];
                        let value = (next != choices[0]).then(|| next.to_owned());
                        return self.commit(|settings| settings.set(key, value));
                    }
                    SettingsRow::Setting(key) if key.is_bool() => {
                        // On/off settings toggle directly. Back to the default =
                        // unset, so the file only holds deviations from it.
                        let on = !s.file.is_on(key);
                        let value = (on != key.default_on()).then(|| on.to_string());
                        return self.commit(|settings| settings.set(key, value));
                    }
                    SettingsRow::Setting(key) => {
                        s.editing = Some(TextInput::new(s.file.get(key).unwrap_or_default()));
                    }
                    SettingsRow::Header(name) if s.is_header_overridden(&name) => {
                        s.message = Some(read_only(&name, &header_env_var(&name)));
                    }
                    SettingsRow::Header(name) => {
                        let value = s.file.header(&name).map_or("", |(_, v)| v);
                        s.editing = Some(TextInput::new(&format!("{name}: {value}")));
                    }
                    SettingsRow::AddHeader => s.editing = Some(TextInput::default()),
                }
            }
            Action::Input(_)
            | Action::DeleteChar
            | Action::DeleteForward
            | Action::CursorLeft
            | Action::CursorRight
            | Action::CursorHome
            | Action::CursorEnd
            | Action::ClearInput => {
                let filtering = matches!(
                    context,
                    Context::JobsFilter
                        | Context::BuildsFilter
                        | Context::PipelinesFilter
                        | Context::RunsFilter
                );
                let target = match context {
                    Context::JobsFilter => self.jobs.filter_input.as_mut(),
                    Context::BuildsFilter => self.history.filter_input.as_mut(),
                    Context::PipelinesFilter => self.pipelines.list.filter_input.as_mut(),
                    Context::RunsFilter => self.pipelines.runs.filter_input.as_mut(),
                    _ => self.settings.editing.as_mut(),
                };
                if let Some(input) = target {
                    let before = input.value().to_owned();
                    match action {
                        Action::Input(c) => input.insert(c),
                        Action::DeleteChar => input.backspace(),
                        Action::DeleteForward => input.delete(),
                        Action::CursorLeft => input.left(),
                        Action::CursorRight => input.right(),
                        Action::CursorHome => input.home(),
                        Action::CursorEnd => input.end(),
                        _ => input.clear(),
                    }
                    // A changed filter starts from the top, like fzf.
                    if filtering && input.value() != before {
                        self.jobs.selected = 0;
                        self.history.selected = 0;
                        self.pipelines.list.selected = 0;
                        self.pipelines.runs.selected = 0;
                    }
                }
            }
            Action::ConfirmEdit if context == Context::PipelinesFilter => {
                let list = &mut self.pipelines.list;
                if let Some(input) = list.filter_input.take() {
                    list.filter = input.value().trim().to_owned();
                }
            }
            Action::ConfirmEdit if context == Context::RunsFilter => {
                if let Some(input) = self.pipelines.runs.filter_input.take() {
                    self.pipelines.runs.filter = input.value().trim().to_owned();
                }
            }
            Action::ConfirmEdit if context == Context::BuildsFilter => {
                if let Some(input) = self.history.filter_input.take() {
                    self.history.filter = input.value().trim().to_owned();
                    self.history.clamp_selection();
                }
            }
            Action::ConfirmEdit if context == Context::JobsFilter => {
                if let Some(input) = self.jobs.filter_input.take() {
                    self.jobs.filter = input.value().trim().to_owned();
                    self.jobs.clamp_selection();
                }
            }
            Action::ConfirmEdit => return self.confirm_edit(),
            Action::SettingsSaved(Ok(())) => {
                s.message = Some(StatusMessage::Info(format!(
                    "Saved to {}",
                    s.path.display()
                )));
            }
            Action::SettingsSaved(Err(err)) => {
                s.message = Some(StatusMessage::Error(format!("Save failed: {err}")));
            }
            Action::ConnectFinished { generation, result } => {
                if generation != self.connection_generation {
                    return Vec::new(); // superseded by a newer attempt
                }
                let url = match &self.connection {
                    ConnectionStatus::Connecting { url } => url.clone(),
                    _ => return Vec::new(),
                };
                self.connection = match result {
                    Ok(info) => ConnectionStatus::Connected { url, info },
                    Err(error) => ConnectionStatus::Failed { url, error },
                };
                if matches!(self.connection, ConnectionStatus::Connected { .. }) {
                    let mut effects = self.fetch_jobs();
                    if self.view == View::Builds {
                        effects.extend(self.fetch_history(self.history_page()));
                    }
                    if matches!(self.view, View::Pipelines | View::Runs | View::Run) {
                        effects.extend(self.fetch_pipelines());
                    }
                    return effects;
                }
            }
            Action::BuildFetched {
                generation,
                job,
                which,
                result,
            } => {
                let now = self.now;
                let Some(build) = self.build.as_mut().filter(|b| {
                    b.job == job && b.target == which && generation == self.connection_generation
                }) else {
                    return Vec::new(); // another job, build or connection by now
                };
                build.refreshing = false;
                build.attempted_at = Some(now);
                build.load = match result {
                    Ok(page) => {
                        build.fetched_at = Some(now);
                        if let Some(numbers) = page.numbers {
                            build.numbers = Some(numbers);
                        }
                        BuildLoad::Loaded(page.build)
                    }
                    Err(error) => BuildLoad::Failed(error),
                };
            }
            Action::BuildStep(step) if self.view == View::Run => {
                // Another run of the same pipeline.
                let Some(view) = self.run.as_ref() else {
                    return Vec::new();
                };
                if let Some(target) = self.pipelines.step_run(view, step)
                    && let Some(view) = self.run.as_mut()
                {
                    view.target = target;
                    view.selected = 0;
                }
            }
            Action::BuildStep(step) => {
                let Some(build) = self.build.as_mut().filter(|_| self.view == View::Build) else {
                    return Vec::new();
                };
                let Some(target) = build.step(step) else {
                    return Vec::new();
                };
                // Another build: load it right away. A fetch still running for
                // the previous one is cancelled / its answer ignored.
                build.target = target;
                build.load = BuildLoad::Loading;
                build.refreshing = false;
                build.fetched_at = None;
                build.attempted_at = None;
                build.scroll = 0;
                return self.build_effect().into_iter().collect();
            }
            Action::HistoryFetched {
                generation,
                limit,
                result,
            } => {
                if generation != self.connection_generation {
                    return Vec::new();
                }
                let now = self.now;
                let history = &mut self.history;
                let previous = history
                    .visible()
                    .get(history.selected)
                    .map(|e| (e.job.clone(), e.number));
                history.refreshing = false;
                history.attempted_at = Some(now);
                match result {
                    Ok(entries) => {
                        history.set_entries(entries, limit);
                        history.fetched_at = Some(now);
                        history.load = HistoryLoad::Loaded;
                        // Keep the same build selected when it's still listed.
                        let visible = history.visible();
                        if let Some(i) = previous.and_then(|(job, number)| {
                            visible
                                .iter()
                                .position(|e| e.job == job && e.number == number)
                        }) {
                            history.selected = i;
                        }
                        history.clamp_selection();
                    }
                    Err(error) => history.load = HistoryLoad::Failed(error),
                }
            }
            Action::PipelinesFetched { generation, result } => {
                if generation != self.connection_generation {
                    return Vec::new();
                }
                let now = self.now;
                // Keep the same pipeline and run selected across reloads.
                let (previous_pipeline, previous_run) = self.selected_list_keys();
                let state = &mut self.pipelines;
                state.refreshing = false;
                state.attempted_at = Some(now);
                match result {
                    Ok(data) => {
                        state.data = data;
                        state.fetched_at = Some(now);
                        state.load = PipelineLoad::Loaded;
                        let pipelines = state.visible_pipelines();
                        if let Some(i) = previous_pipeline
                            .and_then(|job| pipelines.iter().position(|p| p.first_job == job))
                        {
                            state.list.selected = i;
                        }
                        state.list.move_selection(0, pipelines.len());
                        let all = state.pipelines();
                        let runs = state.visible_runs(&all);
                        if let Some(i) = previous_run.and_then(|(job, number)| {
                            runs.iter().position(|&(p, r)| {
                                all[p].first_job == job
                                    && state.run_number(&all[p].runs[r]) == number
                            })
                        }) {
                            state.runs.selected = i;
                        }
                        state.runs.move_selection(0, runs.len());
                    }
                    Err(error) => state.load = PipelineLoad::Failed(error),
                }
            }
            Action::JobsFetched { generation, result } => {
                if generation != self.connection_generation {
                    return Vec::new(); // belongs to an older connection
                }
                // Keep the same job selected across reloads when possible.
                let previous = self
                    .jobs
                    .visible()
                    .get(self.jobs.selected)
                    .map(|job| job.full_name.clone());
                self.jobs.refreshing = false;
                self.jobs.attempted_at = Some(self.now);
                if result.is_ok() {
                    self.jobs.fetched_at = Some(self.now);
                }
                self.jobs.load = match result {
                    Ok(jobs) => JobsLoad::Loaded(jobs),
                    Err(error) => JobsLoad::Failed(error),
                };
                let visible = self.jobs.visible();
                self.jobs.selected = previous
                    .and_then(|name| visible.iter().position(|job| job.full_name == name))
                    .unwrap_or(self.jobs.selected);
                self.jobs.clamp_selection();
            }
        }
        Vec::new()
    }

    /// Load the job list for the current connection.
    /// Load the job list for the current connection, unless a fetch is
    /// already in flight: then this is a no-op and the running one is left to
    /// finish, so slow fetches never pile up or get restarted forever. (A new
    /// connection resets the in-flight state; see [`Self::connect`].)
    fn fetch_jobs(&mut self) -> Vec<Effect> {
        if self.jobs.fetch_in_flight() {
            return Vec::new();
        }
        let Some(config) = self.settings.connection_config() else {
            return Vec::new();
        };
        // Keep showing the current list while it reloads (no flicker, and the
        // selection can be restored afterwards).
        if matches!(self.jobs.load, JobsLoad::Loaded(_)) {
            self.jobs.refreshing = true;
        } else {
            self.jobs.load = JobsLoad::Loading;
        }
        vec![Effect::FetchJobs {
            generation: self.connection_generation,
            config,
        }]
    }

    /// Reload the open build view's data, unless its fetch is in flight
    /// (same no-pile-up rule as for jobs).
    fn fetch_build(&mut self) -> Vec<Effect> {
        let Some(build) = self.build.as_mut().filter(|b| !b.fetch_in_flight()) else {
            return Vec::new();
        };
        if matches!(build.load, BuildLoad::Loaded(_)) {
            build.refreshing = true;
        } else {
            build.load = BuildLoad::Loading;
        }
        self.build_effect().into_iter().collect()
    }

    /// The fetch for the open build view (no in-flight check).
    fn build_effect(&self) -> Option<Effect> {
        let build = self.build.as_ref()?;
        Some(Effect::FetchBuild {
            generation: self.connection_generation,
            job: build.job.clone(),
            which: build.target,
            want_numbers: build.numbers.is_none() && matches!(build.target, BuildRef::Number(_)),
            config: self.settings.connection_config()?,
        })
    }

    /// The next chunk for the open console view (no in-flight check).
    fn console_effect(&self) -> Option<Effect> {
        let console = self.console.as_ref()?;
        Some(Effect::FetchConsole {
            generation: self.connection_generation,
            job: console.job.clone(),
            number: console.number,
            start: console.offset,
            config: self.settings.connection_config()?,
        })
    }

    /// Fetch more console output unless a fetch is running.
    fn fetch_console(&mut self) -> Vec<Effect> {
        let Some(console) = self.console.as_mut().filter(|c| !c.in_flight) else {
            return Vec::new();
        };
        console.in_flight = true;
        self.console_effect().into_iter().collect()
    }

    /// Tail a running build's output: poll every [`CONSOLE_POLL`] while the
    /// console is on screen, independent of the auto-refresh setting.
    fn console_poll_if_due(&mut self) -> Vec<Effect> {
        let connected = matches!(self.connection, ConnectionStatus::Connected { .. });
        let due = self
            .console
            .as_ref()
            .is_some_and(|c| c.poll_due(self.now, CONSOLE_POLL));
        if connected && self.view == View::Console && due {
            self.fetch_console()
        } else {
            Vec::new()
        }
    }

    /// Start a fetch of what's on screen when auto-refresh is on, we're
    /// connected, nothing is in flight and the last attempt is at least one
    /// interval old.
    fn auto_refresh_if_due(&mut self) -> Vec<Effect> {
        let connected = matches!(self.connection, ConnectionStatus::Connected { .. });
        if !self.auto_refresh || !connected {
            return Vec::new();
        }
        let interval = self.settings.effective().refresh_interval();
        let due = |last: Option<Instant>| {
            last.is_none_or(|last| self.now.saturating_duration_since(last) >= interval)
        };
        match (&self.view, &self.build) {
            // Tailed separately (see `console_poll_if_due`).
            (View::Console, _) => Vec::new(),
            (View::Builds, _) => {
                if !self.history.fetch_in_flight() && due(self.history.attempted_at) {
                    return self.fetch_history(self.history.limit.max(self.history_page()));
                }
                Vec::new()
            }
            (View::Pipelines | View::Runs | View::Run, _) => {
                if !self.pipelines.fetch_in_flight() && due(self.pipelines.attempted_at) {
                    return self.fetch_pipelines();
                }
                Vec::new()
            }
            (View::Build, Some(build)) => {
                if !build.fetch_in_flight() && due(build.attempted_at) {
                    return self.fetch_build();
                }
                Vec::new()
            }
            _ if !self.jobs.fetch_in_flight() && due(self.jobs.attempted_at) => self.fetch_jobs(),
            _ => Vec::new(),
        }
    }

    /// Builds per page: the rows that fit (recorded by the renderer).
    fn history_page(&self) -> usize {
        match self.list_rows.get() {
            0 => DEFAULT_HISTORY_PAGE,
            rows => usize::from(rows),
        }
    }

    /// Fetch the build history with `limit` builds per job, unless a fetch is
    /// already in flight (same no-pile-up rule as elsewhere).
    fn fetch_history(&mut self, limit: usize) -> Vec<Effect> {
        let connected = matches!(self.connection, ConnectionStatus::Connected { .. });
        if !connected || self.history.fetch_in_flight() {
            return Vec::new();
        }
        let Some(config) = self.settings.connection_config() else {
            return Vec::new();
        };
        if self.history.load == HistoryLoad::Loaded {
            self.history.refreshing = true;
        } else {
            self.history.load = HistoryLoad::Loading;
        }
        vec![Effect::FetchHistory {
            generation: self.connection_generation,
            limit,
            config,
        }]
    }

    /// Fetch job relations and runs, unless a fetch is already in flight
    /// (same no-pile-up rule as elsewhere).
    fn fetch_pipelines(&mut self) -> Vec<Effect> {
        let connected = matches!(self.connection, ConnectionStatus::Connected { .. });
        if !connected || self.pipelines.fetch_in_flight() {
            return Vec::new();
        }
        let Some(config) = self.settings.connection_config() else {
            return Vec::new();
        };
        if self.pipelines.load == PipelineLoad::Loaded {
            self.pipelines.refreshing = true;
        } else {
            self.pipelines.load = PipelineLoad::Loading;
        }
        vec![Effect::FetchPipelines {
            generation: self.connection_generation,
            config,
        }]
    }

    /// First job of the selected pipeline and `(first job, number)` of the
    /// selected run, to keep them selected when the lists reload.
    fn selected_list_keys(&self) -> (Option<String>, Option<(String, u64)>) {
        let state = &self.pipelines;
        let pipeline = state
            .visible_pipelines()
            .get(state.list.selected)
            .map(|p| p.first_job.clone());
        let all = state.pipelines();
        let run = state
            .visible_runs(&all)
            .get(state.runs.selected)
            .map(|&(p, r)| (all[p].first_job.clone(), state.run_number(&all[p].runs[r])));
        (pipeline, run)
    }

    /// `(job, number)` of the build selected in the run view.
    pub fn selected_run_build(&self) -> Option<(String, u64)> {
        let view = self.run.as_ref()?;
        let pipelines = self.pipelines.pipelines();
        let (pipeline, index) = self.pipelines.find_run(&pipelines, view)?;
        let run = &pipeline.runs[index];
        let build = &self.pipelines.data.builds[*run.builds.get(view.selected)?];
        Some((build.job.clone(), build.number))
    }

    /// Where a run view goes back to.
    fn run_origin(&self) -> View {
        self.run.as_ref().map_or(View::Pipelines, |r| r.origin)
    }

    /// Where a build view goes back to.
    fn build_origin(&self) -> View {
        self.build.as_ref().map_or(View::Jobs, |b| b.origin)
    }

    /// The tab the current view belongs to (a build view opened from the
    /// Builds tab belongs to Builds).
    pub fn tab(&self) -> Option<Tab> {
        let view = match self.view {
            View::Build | View::Console => self.build_origin(),
            view => view,
        };
        match view {
            View::Run => self.run_origin().tab(),
            view => view.tab(),
        }
    }

    /// `r`: reload what's on screen when connected; otherwise (re)connect.
    fn refresh(&mut self) -> Vec<Effect> {
        match self.connection {
            ConnectionStatus::Connected { .. } if self.view == View::Console => {
                self.fetch_console()
            }
            ConnectionStatus::Connected { .. } if self.view == View::Build => self.fetch_build(),
            ConnectionStatus::Connected { .. } if self.view == View::Builds => {
                self.fetch_history(self.history.limit.max(self.history_page()))
            }
            ConnectionStatus::Connected { .. }
                if matches!(self.view, View::Pipelines | View::Runs | View::Run) =>
            {
                self.fetch_pipelines()
            }
            ConnectionStatus::Connected { .. } => self.fetch_jobs(),
            ConnectionStatus::Connecting { .. } => Vec::new(),
            ConnectionStatus::NotConfigured | ConnectionStatus::Failed { .. } => {
                self.connect().into_iter().collect()
            }
        }
    }

    /// Validate and store the value being edited. An invalid value keeps the
    /// edit open (with an error) so the typo can be fixed. No-op when not editing.
    fn confirm_edit(&mut self) -> Vec<Effect> {
        let s = &mut self.settings;
        let Some(input) = s.editing.take() else {
            return Vec::new();
        };
        let text = input.value().trim().to_owned();
        let result = match s.selected_row() {
            SettingsRow::Setting(key) if text.is_empty() => Ok(Change::Set(key, None)),
            SettingsRow::Setting(key) => key.validate(&text).map(|v| Change::Set(key, Some(v))),
            // Clearing a header's field removes it.
            SettingsRow::Header(old) if text.is_empty() => Ok(Change::Header {
                old: Some(old),
                new: None,
            }),
            SettingsRow::AddHeader if text.is_empty() => return Vec::new(),
            row => parse_header(&text).and_then(|(name, value)| {
                let old = match row {
                    SettingsRow::Header(old) => Some(old),
                    _ => None,
                };
                let renamed = old.as_ref().is_none_or(|o| !o.eq_ignore_ascii_case(&name));
                if renamed && s.is_header_overridden(&name) {
                    Err(format!(
                        "{name} is set by ${}; unset it to change it here",
                        header_env_var(&name)
                    ))
                } else if renamed && s.file.header(&name).is_some() {
                    Err(format!(
                        "header {name} already exists; edit that row instead"
                    ))
                } else {
                    Ok(Change::Header {
                        old,
                        new: Some((name, value)),
                    })
                }
            }),
        };
        let change = match result {
            Ok(change) => change,
            Err(err) => {
                s.editing = Some(input);
                s.message = Some(StatusMessage::Error(err));
                return Vec::new();
            }
        };
        match change {
            Change::Set(key, value) => self.commit(|settings| settings.set(key, value)),
            Change::Header { old, new } => {
                let select = new
                    .as_ref()
                    .map(|(name, _)| SettingsRow::Header(name.clone()));
                let effects = self.commit(|settings| {
                    if let Some(old) = &old {
                        settings.set_header(old, None);
                    }
                    if let Some((name, value)) = new {
                        settings.set_header(&name, Some(value));
                    }
                });
                let s = &mut self.settings;
                match select {
                    Some(row) => s.select(&row),
                    None => s.selected = s.selected.min(s.rows().len() - 1),
                }
                effects
            }
        }
    }

    /// Apply `change` to the file layer, save, and reconnect if that changed
    /// how Jenkins is reached.
    fn commit(&mut self, change: impl FnOnce(&mut Settings)) -> Vec<Effect> {
        let s = &mut self.settings;
        let before = s.connection_config();
        let auto_before = s.effective().is_on(SettingKey::RefreshAuto);
        change(&mut s.file);
        // Changing the default applies to the running session right away.
        let auto_after = s.effective().is_on(SettingKey::RefreshAuto);
        if auto_after != auto_before {
            self.auto_refresh = auto_after;
        }
        let s = &mut self.settings;
        s.message = Some(StatusMessage::Info("Saving…".into()));
        let mut effects = vec![Effect::SaveSettings {
            path: s.path.clone(),
            settings: s.file.clone(),
        }];
        if s.connection_config() != before {
            effects.extend(self.connect());
        }
        effects
    }

    /// Close whatever is on top: help popup, then an edit, then the settings view.
    fn back(&mut self) {
        match self.context() {
            Context::ConfirmQuit => self.confirm_quit = false,
            Context::Help => self.show_help = false,
            Context::EditSetting => self.settings.editing = None,
            // Cancel typing: the previously applied filter stays.
            Context::JobsFilter => self.jobs.filter_input = None,
            Context::JobsFiltered => {
                self.jobs.filter.clear();
                self.jobs.clamp_selection();
            }
            Context::Build => {
                self.view = self.build_origin();
                self.build = None;
            }
            Context::BuildsFilter => self.history.filter_input = None,
            Context::BuildsFiltered => {
                self.history.filter.clear();
                self.history.clamp_selection();
            }
            Context::Console => {
                self.view = View::Build;
                self.console = None;
            }
            Context::PipelinesFilter => self.pipelines.list.filter_input = None,
            Context::PipelinesFiltered => self.pipelines.list.filter.clear(),
            Context::RunsFilter => self.pipelines.runs.filter_input = None,
            Context::RunsFiltered => self.pipelines.runs.filter.clear(),
            Context::Run => {
                self.view = self.run_origin();
                self.run = None;
            }
            Context::Jobs
            | Context::Builds
            | Context::Pipelines
            | Context::Runs
            | Context::Settings => {}
        }
    }
}

/// A validated edit from the settings view.
enum Change {
    Set(SettingKey, Option<String>),
    /// Remove `old` (if any), then add `new` (if any): add, edit, rename, delete.
    Header {
        old: Option<String>,
        new: Option<(String, String)>,
    },
}

fn read_only(what: &str, var: &str) -> StatusMessage {
    StatusMessage::Error(format!("{what} is set by ${var}; unset it to edit here"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_app() -> App {
        let mut app = App::default();
        app.update(Action::SwitchTab(Tab::Settings));
        app
    }

    fn select(app: &mut App, row: SettingsRow) {
        app.settings.select(&row);
        assert_eq!(app.settings.selected_row(), row);
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            app.update(Action::Input(c));
        }
    }

    #[test]
    fn quit_stops_running() {
        let mut app = App::default();
        app.update(Action::Quit);
        assert!(!app.running);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = App::default();
        app.update(Action::ToggleHelp);
        assert!(app.show_help);
        app.update(Action::ToggleHelp);
        assert!(!app.show_help);
    }

    #[test]
    fn back_closes_help() {
        let mut app = App::default();
        app.update(Action::ToggleHelp);
        app.update(Action::Back);
        assert!(!app.show_help);
        assert_eq!(app.context(), Context::Jobs);
    }

    #[test]
    fn back_at_root_keeps_running() {
        let mut app = App::default();
        app.update(Action::Back);
        assert!(app.running);
        assert_eq!(app.context(), Context::Jobs);
    }

    #[test]
    fn help_overlay_takes_context() {
        let mut app = App::default();
        assert_eq!(app.context(), Context::Jobs);
        app.update(Action::ToggleHelp);
        assert_eq!(app.context(), Context::Help);
    }

    #[test]
    fn back_unwinds_edit_then_settings() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        assert_eq!(app.context(), Context::EditSetting);
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Settings);
        // Settings is a tab: Esc doesn't leave it (F1 does).
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Settings);
        app.update(Action::SwitchTab(Tab::Jobs));
        assert_eq!(app.context(), Context::Jobs);
    }

    #[test]
    fn selection_wraps() {
        let mut app = settings_app();
        app.update(Action::SelectPrev);
        assert_eq!(
            app.settings.selected_row(),
            SettingsRow::Setting(SettingKey::Theme),
            "last row: the Application section follows the headers"
        );
        app.update(Action::SelectNext);
        assert_eq!(
            app.settings.selected_row(),
            SettingsRow::Setting(SettingKey::JenkinsUrl)
        );
    }

    #[test]
    fn edit_and_confirm_requests_save() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, " https://cii ");
        app.update(Action::DeleteChar); // trailing space
        app.update(Action::DeleteChar); // extra 'i'
        let effects = app.update(Action::ConfirmEdit);

        let mut expected = Settings::default();
        expected.set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        assert_eq!(
            effects[0],
            Effect::SaveSettings {
                path: app.settings.path.clone(),
                settings: expected
            }
        );
        assert_eq!(app.context(), Context::Settings);
    }

    #[test]
    fn empty_value_unsets() {
        let mut app = settings_app();
        app.settings
            .file
            .set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.editing.as_ref().map(|i| i.value()),
            Some("https://ci")
        );
        app.update(Action::ClearInput);
        app.update(Action::ConfirmEdit);
        assert_eq!(app.settings.file.get(SettingKey::JenkinsUrl), None);
    }

    #[test]
    fn invalid_value_keeps_editing() {
        let mut app = settings_app();
        select(&mut app, SettingsRow::Setting(SettingKey::ProxyUrl));
        app.update(Action::StartEdit);
        type_str(&mut app, "ftp://proxy:21");
        assert_eq!(app.update(Action::ConfirmEdit), Vec::new());
        assert_eq!(app.context(), Context::EditSetting);
        assert_eq!(
            app.settings.editing.as_ref().map(|i| i.value()),
            Some("ftp://proxy:21")
        );
        assert!(matches!(
            app.settings.message,
            Some(StatusMessage::Error(ref m)) if m.contains("socks5h")
        ));
        assert_eq!(app.settings.file.get(SettingKey::ProxyUrl), None);
    }

    #[test]
    fn cancel_edit_keeps_value() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, "https://ci");
        app.update(Action::Back);
        assert_eq!(app.settings.file.get(SettingKey::JenkinsUrl), None);
    }

    #[test]
    fn env_overridden_setting_is_read_only() {
        let mut app = settings_app();
        app.settings
            .env
            .set(SettingKey::JenkinsUrl, Some("https://env".into()));
        app.update(Action::StartEdit);
        assert_eq!(app.context(), Context::Settings);
        assert!(matches!(
            app.settings.message,
            Some(StatusMessage::Error(ref m)) if m.contains("LEEROY_JENKINS_URL")
        ));
    }

    #[test]
    fn save_result_is_reported() {
        let mut app = settings_app();
        app.update(Action::SettingsSaved(Err("disk full".into())));
        assert_eq!(
            app.settings.message,
            Some(StatusMessage::Error("Save failed: disk full".into()))
        );
        app.update(Action::SettingsSaved(Ok(())));
        assert!(matches!(app.settings.message, Some(StatusMessage::Info(_))));
    }

    fn set_url(app: &mut App, url: &str) -> Vec<Effect> {
        app.settings.selected = 0;
        app.update(Action::StartEdit);
        app.update(Action::ClearInput);
        type_str(app, url);
        app.update(Action::ConfirmEdit)
    }

    fn connect_generation(effects: &[Effect]) -> Option<u64> {
        effects.iter().find_map(|e| match e {
            Effect::Connect { generation, .. } => Some(*generation),
            _ => None,
        })
    }

    fn info(user: &str) -> ServerInfo {
        ServerInfo {
            version: Some("2.504".into()),
            user: user.into(),
        }
    }

    #[test]
    fn start_connects_only_when_configured() {
        let mut app = App::default();
        assert_eq!(app.start(), Vec::new());
        assert_eq!(app.connection, ConnectionStatus::NotConfigured);

        app.settings
            .file
            .set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        let effects = app.start();
        assert!(connect_generation(&effects).is_some());
        assert_eq!(
            app.connection,
            ConnectionStatus::Connecting {
                url: "https://ci".into()
            }
        );
    }

    #[test]
    fn changing_url_reconnects_and_reports_result() {
        let mut app = settings_app();
        let generation = connect_generation(&set_url(&mut app, "https://ci")).unwrap();
        app.update(Action::ConnectFinished {
            generation,
            result: Ok(info("me")),
        });
        assert_eq!(
            app.connection,
            ConnectionStatus::Connected {
                url: "https://ci".into(),
                info: info("me")
            }
        );

        let generation = connect_generation(&set_url(&mut app, "https://other")).unwrap();
        app.update(Action::ConnectFinished {
            generation,
            result: Err("timed out".into()),
        });
        assert_eq!(
            app.connection,
            ConnectionStatus::Failed {
                url: "https://other".into(),
                error: "timed out".into()
            }
        );
    }

    #[test]
    fn stale_results_are_ignored() {
        let mut app = settings_app();
        let old = connect_generation(&set_url(&mut app, "https://old")).unwrap();
        let new = connect_generation(&set_url(&mut app, "https://new")).unwrap();
        app.update(Action::ConnectFinished {
            generation: old,
            result: Ok(info("old")),
        });
        assert!(matches!(
            app.connection,
            ConnectionStatus::Connecting { .. }
        ));
        app.update(Action::ConnectFinished {
            generation: new,
            result: Ok(info("new")),
        });
        assert!(matches!(
            app.connection,
            ConnectionStatus::Connected { ref url, .. } if url == "https://new"
        ));
    }

    #[test]
    fn unchanged_connection_does_not_reconnect() {
        let mut app = settings_app();
        set_url(&mut app, "https://ci");
        assert_eq!(connect_generation(&set_url(&mut app, "https://ci")), None);
        // A new header changes the connection: reconnect.
        assert!(connect_generation(&add_header(&mut app, "Authorization: Bearer abc")).is_some());
        // Re-saving the same header value doesn't.
        select(&mut app, SettingsRow::Header("Authorization".into()));
        app.update(Action::StartEdit);
        assert_eq!(connect_generation(&app.update(Action::ConfirmEdit)), None);
    }

    fn add_header(app: &mut App, line: &str) -> Vec<Effect> {
        select(app, SettingsRow::AddHeader);
        app.update(Action::StartEdit);
        type_str(app, line);
        app.update(Action::ConfirmEdit)
    }

    #[test]
    fn header_add_edit_rename_delete() {
        let mut app = settings_app();
        add_header(&mut app, "X-Token: one");
        assert_eq!(
            app.settings.file.header("x-token"),
            Some(("X-Token", "one"))
        );
        assert_eq!(
            app.settings.selected_row(),
            SettingsRow::Header("X-Token".into()),
            "new header selected"
        );

        // Edit: prefilled with "Name: value".
        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.editing.as_ref().map(|i| i.value()),
            Some("X-Token: one")
        );
        app.update(Action::ClearInput);
        type_str(&mut app, "X-Renamed: two");
        app.update(Action::ConfirmEdit);
        assert_eq!(app.settings.file.header("X-Token"), None);
        assert_eq!(
            app.settings.file.header("X-Renamed"),
            Some(("X-Renamed", "two"))
        );

        // Clearing the field deletes it.
        app.update(Action::StartEdit);
        app.update(Action::ClearInput);
        let effects = app.update(Action::ConfirmEdit);
        assert!(matches!(effects.first(), Some(Effect::SaveSettings { .. })));
        assert!(app.settings.file.jenkins.headers.is_empty());
        assert_eq!(app.settings.selected_row(), SettingsRow::AddHeader);
    }

    #[test]
    fn header_errors_keep_editing() {
        let mut app = settings_app();
        add_header(&mut app, "X-Token: one");
        for bad in ["no colon", "X-Empty:", "x-token: dup"] {
            let effects = add_header(&mut app, bad);
            assert_eq!(effects, Vec::new(), "{bad}");
            assert_eq!(app.context(), Context::EditSetting, "{bad}");
            assert!(
                matches!(app.settings.message, Some(StatusMessage::Error(_))),
                "{bad}"
            );
            app.update(Action::Back);
        }
        // Empty "+ add header" just cancels.
        assert_eq!(add_header(&mut app, ""), Vec::new());
        assert_eq!(app.context(), Context::Settings);
    }

    #[test]
    fn env_headers_are_read_only_and_used() {
        let mut app = settings_app();
        app.settings
            .env
            .set_header("Authorization", Some("Bearer env".into()));
        set_url(&mut app, "https://ci");
        select(&mut app, SettingsRow::Header("Authorization".into()));
        app.update(Action::StartEdit);
        assert_eq!(app.context(), Context::Settings);
        assert!(matches!(
            app.settings.message,
            Some(StatusMessage::Error(ref m)) if m.contains("LEEROY_JENKINS_HEADERS_AUTHORIZATION")
        ));
        // Can't shadow it with a file header either.
        let effects = add_header(&mut app, "authorization: Basic file");
        assert_eq!(effects, Vec::new());
        app.update(Action::Back);
        assert_eq!(
            app.settings.connection_config().unwrap().headers,
            vec![("Authorization".to_owned(), "Bearer env".to_owned())]
        );
    }

    #[test]
    fn url_credentials_never_shown_in_connection_status() {
        let mut app = settings_app();
        set_url(&mut app, "https://me:s3cret@ci");
        assert_eq!(
            app.connection,
            ConnectionStatus::Connecting {
                url: "https://me:••••@ci".into()
            }
        );
        // The real URL is still used to connect.
        assert_eq!(
            app.settings.connection_config().unwrap().url,
            "https://me:s3cret@ci"
        );
    }

    #[test]
    fn clearing_url_disconnects() {
        let mut app = settings_app();
        set_url(&mut app, "https://ci");
        assert_eq!(connect_generation(&set_url(&mut app, "")), None);
        assert_eq!(app.connection, ConnectionStatus::NotConfigured);
    }

    #[test]
    fn connection_uses_proxy_setting_over_system_proxy() {
        let mut app = App::default();
        app.settings.proxy_env =
            ProxyEnv::from_env(|name| (name == "HTTPS_PROXY").then(|| "http://system:3128".into()));
        app.settings
            .file
            .set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        assert_eq!(
            app.settings.connection_config().unwrap().proxy.as_deref(),
            Some("http://system:3128")
        );
        app.settings
            .file
            .set(SettingKey::ProxyUrl, Some("socks5h://bastion:1080".into()));
        assert_eq!(
            app.settings.connection_config().unwrap().proxy.as_deref(),
            Some("socks5h://bastion:1080")
        );
    }

    #[test]
    fn enter_toggles_skip_tls_verify_and_reconnects() {
        let mut app = settings_app();
        set_url(&mut app, "https://ci");
        select(
            &mut app,
            SettingsRow::Setting(SettingKey::JenkinsSkipTlsVerify),
        );

        let effects = app.update(Action::StartEdit);
        assert_eq!(
            app.context(),
            Context::Settings,
            "no text edit for a toggle"
        );
        assert!(app.settings.file.is_on(SettingKey::JenkinsSkipTlsVerify));
        assert!(matches!(
            effects.last(),
            Some(Effect::Connect { config, .. }) if config.skip_tls_verify
        ));

        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.file.get(SettingKey::JenkinsSkipTlsVerify),
            None
        );
        assert!(!app.settings.connection_config().unwrap().skip_tls_verify);
    }

    #[test]
    fn cursor_editing() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, "https://ci");
        app.update(Action::CursorHome);
        app.update(Action::DeleteForward); // h
        app.update(Action::Input('H'));
        app.update(Action::CursorEnd);
        app.update(Action::CursorLeft);
        app.update(Action::DeleteChar); // c
        let input = app.settings.editing.as_ref().unwrap();
        assert_eq!((input.value(), input.cursor()), ("Https://i", 8));
    }

    #[test]
    fn arrows_while_editing_save_and_move() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, "https://ci");
        let effects = app.update(Action::SelectNext);
        assert!(matches!(effects.first(), Some(Effect::SaveSettings { .. })));
        assert_eq!(
            app.settings.file.get(SettingKey::JenkinsUrl),
            Some("https://ci")
        );
        assert_eq!(
            app.settings.selected_row(),
            SettingsRow::Setting(SettingKey::JenkinsSkipTlsVerify)
        );
        assert_eq!(app.context(), Context::Settings);
    }

    #[test]
    fn arrows_while_editing_invalid_value_stay() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, "not a url");
        assert_eq!(app.update(Action::SelectPrev), Vec::new());
        assert_eq!(
            app.settings.selected_row(),
            SettingsRow::Setting(SettingKey::JenkinsUrl)
        );
        assert_eq!(app.context(), Context::EditSetting);
        assert!(matches!(
            app.settings.message,
            Some(StatusMessage::Error(_))
        ));
    }

    fn job(name: &str) -> Job {
        Job {
            full_name: name.into(),
            url: String::new(),
            status: crate::jobs::JobStatus::Success,
            building: false,
        }
    }

    /// An app connected to https://ci, with the given jobs loaded.
    fn connected_with_jobs(names: &[&str]) -> App {
        let mut app = App::default();
        app.settings
            .file
            .set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        app.start();
        let generation = app.connection_generation;
        let effects = app.update(Action::ConnectFinished {
            generation,
            result: Ok(info("me")),
        });
        assert!(matches!(effects.as_slice(), [Effect::FetchJobs { .. }]));
        assert_eq!(app.jobs.load, JobsLoad::Loading);
        app.update(Action::JobsFetched {
            generation,
            result: Ok(names.iter().map(|n| job(n)).collect()),
        });
        app
    }

    fn visible_names(app: &App) -> Vec<String> {
        app.jobs
            .visible()
            .iter()
            .map(|j| j.full_name.clone())
            .collect()
    }

    #[test]
    fn jobs_are_fetched_after_connecting() {
        let app = connected_with_jobs(&["a", "b"]);
        assert_eq!(visible_names(&app), ["a", "b"]);
    }

    #[test]
    fn jobs_of_an_old_connection_are_ignored_and_cleared() {
        let mut app = connected_with_jobs(&["old"]);
        let old = app.connection_generation;
        app.update(Action::SwitchTab(Tab::Settings));
        set_url(&mut app, "https://other");
        assert_eq!(app.jobs.load, JobsLoad::NotLoaded, "cleared on reconnect");
        app.update(Action::JobsFetched {
            generation: old,
            result: Ok(vec![job("stale")]),
        });
        assert_eq!(app.jobs.load, JobsLoad::NotLoaded);
    }

    #[test]
    fn refresh_keeps_the_selected_job() {
        let mut app = connected_with_jobs(&["a", "b", "c"]);
        app.update(Action::SelectNext);
        app.update(Action::SelectNext); // "c"
        let effects = app.update(Action::Refresh);
        assert!(matches!(effects.as_slice(), [Effect::FetchJobs { .. }]));
        assert!(app.jobs.refreshing);
        assert_eq!(
            visible_names(&app).len(),
            3,
            "old list stays while reloading"
        );
        app.update(Action::JobsFetched {
            generation: app.connection_generation,
            result: Ok(vec![job("0-new"), job("a"), job("b"), job("c")]),
        });
        assert_eq!(app.jobs.visible()[app.jobs.selected].full_name, "c");
    }

    #[test]
    fn refresh_reconnects_after_a_failure() {
        let mut app = App::default();
        app.settings
            .file
            .set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        app.start();
        app.update(Action::ConnectFinished {
            generation: app.connection_generation,
            result: Err("down".into()),
        });
        let effects = app.update(Action::Refresh);
        assert!(matches!(effects.as_slice(), [Effect::Connect { .. }]));
    }

    #[test]
    fn fetch_failure_is_shown() {
        let mut app = connected_with_jobs(&[]);
        app.update(Action::Refresh);
        app.update(Action::JobsFetched {
            generation: app.connection_generation,
            result: Err("HTTP 500".into()),
        });
        assert_eq!(app.jobs.load, JobsLoad::Failed("HTTP 500".into()));
    }

    #[test]
    fn filter_flow() {
        let mut app = connected_with_jobs(&["api/main", "api/release", "web/main"]);
        app.update(Action::SelectLast);
        app.update(Action::StartFilter);
        assert_eq!(app.context(), Context::JobsFilter);
        type_str(&mut app, "main");
        assert_eq!(visible_names(&app), ["api/main", "web/main"], "live");
        assert_eq!(app.jobs.selected, 0, "typing resets the selection");
        app.update(Action::SelectNext); // arrows move the list while typing
        assert_eq!(app.jobs.selected, 1);

        app.update(Action::ConfirmEdit);
        assert_eq!(app.context(), Context::JobsFiltered);
        assert_eq!(app.jobs.filter, "main");

        // Editing again and cancelling keeps the applied filter.
        app.update(Action::StartFilter);
        type_str(&mut app, " api");
        assert_eq!(visible_names(&app), ["api/main"]);
        app.update(Action::Back);
        assert_eq!(app.jobs.filter, "main");
        assert_eq!(visible_names(&app).len(), 2);

        // Esc in the list clears it.
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Jobs);
        assert_eq!(visible_names(&app).len(), 3);
    }

    #[test]
    fn page_and_edge_selection() {
        let names: Vec<String> = (0..25).map(|i| format!("job-{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut app = connected_with_jobs(&refs);
        app.update(Action::SelectPageDown);
        assert_eq!(app.jobs.selected, PAGE as usize);
        app.update(Action::SelectLast);
        assert_eq!(app.jobs.selected, 24);
        app.update(Action::SelectPageDown);
        assert_eq!(app.jobs.selected, 24, "clamped");
        app.update(Action::SelectFirst);
        assert_eq!(app.jobs.selected, 0);
    }

    #[test]
    fn tab_keys() {
        let mut app = settings_app();
        app.update(Action::SwitchTab(Tab::Jobs));
        assert_eq!(app.view, View::Jobs);
        assert_eq!(Tab::from_key('1'), Some(Tab::Jobs));
        assert_eq!(Tab::from_key('0'), Some(Tab::Settings));
        assert_eq!(Tab::from_key('2'), Some(Tab::Builds));
        assert_eq!(Tab::from_key('3'), Some(Tab::Pipelines));
        assert_eq!(Tab::from_key('4'), Some(Tab::Runs));
        assert_eq!(Tab::from_key('5'), None);
        for tab in Tab::ALL {
            assert_eq!(tab.key_label(), tab.key().to_string());
        }
    }

    use std::time::Duration;

    fn fetches(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::FetchJobs { .. }))
            .count()
    }

    fn tick(app: &mut App, after: Duration) -> Vec<Effect> {
        let now = app.now + after;
        let wall = app.wall_now + after;
        app.update(Action::Tick(now, wall))
    }

    fn finish_fetch(app: &mut App, result: Result<Vec<Job>, String>) {
        app.update(Action::JobsFetched {
            generation: app.connection_generation,
            result,
        });
    }

    #[test]
    fn auto_refresh_on_by_default_every_10s() {
        let mut app = connected_with_jobs(&["a"]);
        assert!(app.auto_refresh);
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(9))), 0);
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(1))), 1);
    }

    #[test]
    fn auto_refresh_every_interval_without_overlap() {
        let mut app = connected_with_jobs(&["a"]);
        app.settings
            .file
            .set(SettingKey::RefreshInterval, Some("5".into()));
        app.auto_refresh = true;

        assert_eq!(
            fetches(&tick(&mut app, Duration::from_secs(4))),
            0,
            "not yet due"
        );
        assert_eq!(
            fetches(&tick(&mut app, Duration::from_secs(1))),
            1,
            "due at 5s"
        );
        assert_eq!(
            fetches(&tick(&mut app, Duration::from_secs(60))),
            0,
            "one in flight: no overlap"
        );
        finish_fetch(&mut app, Ok(vec![job("a")]));
        assert_eq!(app.jobs.fetched_at, Some(app.now));
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(4))), 0);
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(1))), 1);
    }

    #[test]
    fn failed_refresh_waits_a_full_interval() {
        let mut app = connected_with_jobs(&["a"]);
        let good = app.jobs.fetched_at;
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(10))), 1);
        finish_fetch(&mut app, Err("HTTP 502".into()));
        assert_eq!(app.jobs.fetched_at, good, "last good fetch time kept");
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(9))), 0);
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(1))), 1);
    }

    #[test]
    fn toggling_on_refreshes_stale_data_right_away() {
        let mut app = connected_with_jobs(&["a"]);
        assert_eq!(app.update(Action::ToggleAutoRefresh), Vec::new(), "off");
        assert!(!app.auto_refresh);
        assert_eq!(fetches(&tick(&mut app, Duration::from_secs(120))), 0);
        assert_eq!(
            fetches(&app.update(Action::ToggleAutoRefresh)),
            1,
            "on + stale"
        );
        assert!(app.auto_refresh);
    }

    #[test]
    fn auto_refresh_setting_applies_to_running_session() {
        let mut app = settings_app();
        assert!(app.auto_refresh, "default on");
        select(&mut app, SettingsRow::Setting(SettingKey::RefreshAuto));
        app.update(Action::StartEdit); // off: stored explicitly
        assert_eq!(
            app.settings.file.get(SettingKey::RefreshAuto),
            Some("false")
        );
        assert!(!app.auto_refresh);
        app.update(Action::StartEdit); // back to the default: unset
        assert_eq!(app.settings.file.get(SettingKey::RefreshAuto), None);
        assert!(app.auto_refresh);
    }

    #[test]
    fn skip_tls_toggle_still_defaults_off() {
        let mut app = settings_app();
        select(
            &mut app,
            SettingsRow::Setting(SettingKey::JenkinsSkipTlsVerify),
        );
        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.file.get(SettingKey::JenkinsSkipTlsVerify),
            Some("true")
        );
        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.file.get(SettingKey::JenkinsSkipTlsVerify),
            None
        );
    }

    #[test]
    fn no_auto_refresh_while_disconnected() {
        let mut app = App {
            auto_refresh: true,
            ..App::default()
        };
        assert_eq!(tick(&mut app, Duration::from_secs(3600)), Vec::new());
    }

    #[test]
    fn refresh_while_fetching_waits_for_the_running_fetch() {
        let mut app = connected_with_jobs(&["a"]);
        assert_eq!(fetches(&app.update(Action::Refresh)), 1);
        // Hammering r (or auto-refresh coming due) while it runs: nothing new.
        for _ in 0..5 {
            assert_eq!(app.update(Action::Refresh), Vec::new());
        }
        app.auto_refresh = true;
        assert_eq!(tick(&mut app, Duration::from_secs(3600)), Vec::new());
        // Once it finishes, refreshing works again.
        finish_fetch(&mut app, Ok(vec![job("a")]));
        assert_eq!(fetches(&app.update(Action::Refresh)), 1);
    }

    #[test]
    fn new_connection_does_not_wait_for_the_old_fetch() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::Refresh); // in flight, for the old connection
        app.update(Action::SwitchTab(Tab::Settings));
        set_url(&mut app, "https://other");
        let effects = app.update(Action::ConnectFinished {
            generation: app.connection_generation,
            result: Ok(info("me")),
        });
        assert_eq!(fetches(&effects), 1);
    }

    use crate::builds::Build;

    /// A `Latest` answer: the build plus numbers 1..=its number.
    fn latest_page(build: Option<Build>) -> BuildPage {
        let numbers = build.as_ref().map_or(vec![], |b| (1..=b.number).collect());
        BuildPage {
            build,
            numbers: Some(numbers),
        }
    }

    fn sample_build(number: u64) -> Build {
        Build {
            number,
            display_name: format!("#{number}"),
            result: Some(crate::jobs::JobStatus::Success),
            building: false,
            started: SystemTime::UNIX_EPOCH,
            duration: Duration::from_secs(60),
            estimated: None,
            description: None,
            causes: vec![],
            parameters: vec![],
            changes: vec![],
            pipeline: false,
            stages: None,
        }
    }

    fn build_fetch(effects: &[Effect]) -> Option<(u64, String)> {
        effects.iter().find_map(|e| match e {
            Effect::FetchBuild {
                generation, job, ..
            } => Some((*generation, job.clone())),
            _ => None,
        })
    }

    #[test]
    fn enter_opens_the_selected_jobs_last_build() {
        let mut app = connected_with_jobs(&["a", "b"]);
        app.update(Action::SelectNext);
        let (generation, job) = build_fetch(&app.update(Action::OpenBuild)).unwrap();
        assert_eq!(job, "b");
        assert_eq!(app.view, View::Build);
        assert_eq!(app.context(), Context::Build);
        assert_eq!(app.view.tab(), Some(Tab::Jobs), "still the Jobs tab");

        app.update(Action::BuildFetched {
            generation,
            job: "b".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(Some(sample_build(7)))),
        });
        let build = app.build.as_ref().unwrap();
        assert_eq!(build.load, BuildLoad::Loaded(Some(sample_build(7))));
        assert_eq!(build.numbers, Some((1..=7).collect()));
        assert_eq!(build.fetched_at, Some(app.now));
    }

    #[test]
    fn no_builds_and_failures_are_states() {
        let mut app = connected_with_jobs(&["a"]);
        let (generation, _) = build_fetch(&app.update(Action::OpenBuild)).unwrap();
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(None)),
        });
        assert_eq!(app.build.as_ref().unwrap().load, BuildLoad::Loaded(None));
        app.update(Action::Refresh);
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Err("HTTP 500".into()),
        });
        assert_eq!(
            app.build.as_ref().unwrap().load,
            BuildLoad::Failed("HTTP 500".into())
        );
    }

    #[test]
    fn answers_for_another_job_or_connection_are_ignored() {
        let mut app = connected_with_jobs(&["a", "b"]);
        let (generation, _) = build_fetch(&app.update(Action::OpenBuild)).unwrap();
        app.update(Action::Back);
        app.update(Action::SelectNext);
        app.update(Action::OpenBuild); // "b"
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(Some(sample_build(1)))),
        });
        assert_eq!(app.build.as_ref().unwrap().load, BuildLoad::Loading);
        app.update(Action::BuildFetched {
            generation: generation + 99,
            job: "b".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(Some(sample_build(1)))),
        });
        assert_eq!(app.build.as_ref().unwrap().load, BuildLoad::Loading);
    }

    #[test]
    fn refresh_and_auto_refresh_follow_the_build_view_without_pile_up() {
        let mut app = connected_with_jobs(&["a"]);
        let (generation, _) = build_fetch(&app.update(Action::OpenBuild)).unwrap();
        assert_eq!(
            app.update(Action::Refresh),
            Vec::new(),
            "first fetch in flight"
        );
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(Some(sample_build(1)))),
        });
        // r: the build, not the job list.
        let effects = app.update(Action::Refresh);
        assert!(build_fetch(&effects).is_some());
        assert_eq!(fetches(&effects), 0);
        assert!(
            app.build.as_ref().unwrap().refreshing,
            "old details stay visible"
        );
        assert_eq!(app.update(Action::Refresh), Vec::new(), "in flight");
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(latest_page(Some(sample_build(2)))),
        });
        // Auto-refresh (on by default) re-fetches the build after the interval.
        assert_eq!(tick(&mut app, Duration::from_secs(9)), Vec::new());
        let effects = tick(&mut app, Duration::from_secs(1));
        assert!(build_fetch(&effects).is_some());
        assert_eq!(
            fetches(&effects),
            0,
            "job list not polled from the build view"
        );
    }

    #[test]
    fn esc_returns_to_the_list_with_selection() {
        let mut app = connected_with_jobs(&["a", "b", "c"]);
        app.update(Action::SelectLast);
        app.update(Action::OpenBuild);
        app.update(Action::Back);
        assert_eq!(app.view, View::Jobs);
        assert!(app.build.is_none());
        assert_eq!(app.jobs.selected, 2);
    }

    #[test]
    fn build_scroll_is_clamped_to_what_the_renderer_allows() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::OpenBuild);
        app.build.as_ref().unwrap().max_scroll.set(5);
        app.update(Action::SelectLast);
        assert_eq!(app.build.as_ref().unwrap().scroll, 5);
        app.update(Action::SelectPrev);
        assert_eq!(
            app.build.as_ref().unwrap().scroll,
            4,
            "no overshoot after G"
        );
        app.update(Action::SelectFirst);
        assert_eq!(app.build.as_ref().unwrap().scroll, 0);
    }

    #[test]
    fn enter_with_no_visible_jobs_does_nothing() {
        let mut app = connected_with_jobs(&[]);
        assert_eq!(app.update(Action::OpenBuild), Vec::new());
        assert_eq!(app.view, View::Jobs);
    }

    fn build_fetch_which(effects: &[Effect]) -> Option<BuildRef> {
        effects.iter().find_map(|e| match e {
            Effect::FetchBuild { which, .. } => Some(*which),
            _ => None,
        })
    }

    /// Build view of job "a" showing latest #8, with builds 3, 5, 7, 8.
    fn app_on_latest_build() -> App {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::OpenBuild);
        let generation = app.connection_generation;
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(BuildPage {
                build: Some(sample_build(8)),
                numbers: Some(vec![3, 5, 7, 8]),
            }),
        });
        app
    }

    fn arrive(app: &mut App, which: BuildRef, build: Option<Build>) {
        let generation = app.connection_generation;
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which,
            result: Ok(BuildPage {
                build,
                numbers: None,
            }),
        });
    }

    #[test]
    fn stepping_through_builds() {
        let mut app = app_on_latest_build();
        let effects = app.update(Action::BuildStep(BuildStep::Older));
        assert_eq!(build_fetch_which(&effects), Some(BuildRef::Number(7)));
        let view = app.build.as_ref().unwrap();
        assert_eq!(view.load, BuildLoad::Loading);
        assert_eq!(
            view.position(),
            Some((3, 4)),
            "position known while loading"
        );
        arrive(&mut app, BuildRef::Number(7), Some(sample_build(7)));

        assert_eq!(
            build_fetch_which(&app.update(Action::BuildStep(BuildStep::First))),
            Some(BuildRef::Number(3))
        );
        arrive(&mut app, BuildRef::Number(3), Some(sample_build(3)));
        assert_eq!(
            app.update(Action::BuildStep(BuildStep::Older)),
            Vec::new(),
            "oldest"
        );
        assert_eq!(
            build_fetch_which(&app.update(Action::BuildStep(BuildStep::Last))),
            Some(BuildRef::Latest)
        );
        assert_eq!(
            app.build.as_ref().unwrap().numbers,
            Some(vec![3, 5, 7, 8]),
            "kept"
        );
    }

    #[test]
    fn quick_steps_ignore_answers_for_builds_left_behind() {
        let mut app = app_on_latest_build();
        app.update(Action::BuildStep(BuildStep::Older)); // 7
        app.update(Action::BuildStep(BuildStep::Older)); // 5, while 7 is loading
        assert_eq!(app.build.as_ref().unwrap().target, BuildRef::Number(5));
        arrive(&mut app, BuildRef::Number(7), Some(sample_build(7)));
        assert_eq!(
            app.build.as_ref().unwrap().load,
            BuildLoad::Loading,
            "7 ignored"
        );
        arrive(&mut app, BuildRef::Number(5), Some(sample_build(5)));
        assert_eq!(
            app.build.as_ref().unwrap().load,
            BuildLoad::Loaded(Some(sample_build(5)))
        );
    }

    #[test]
    fn refresh_follows_latest_but_pins_older_builds() {
        let mut app = app_on_latest_build();
        assert_eq!(
            build_fetch_which(&app.update(Action::Refresh)),
            Some(BuildRef::Latest)
        );
        let generation = app.connection_generation;
        // A new build #9 appeared: latest follows it, numbers grow.
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            which: BuildRef::Latest,
            result: Ok(BuildPage {
                build: Some(sample_build(9)),
                numbers: Some(vec![3, 5, 7, 8, 9]),
            }),
        });
        assert_eq!(app.build.as_ref().unwrap().position(), Some((5, 5)));

        app.update(Action::BuildStep(BuildStep::Older)); // 8
        arrive(&mut app, BuildRef::Number(8), Some(sample_build(8)));
        assert_eq!(
            build_fetch_which(&app.update(Action::Refresh)),
            Some(BuildRef::Number(8)),
            "an older build stays pinned"
        );
    }

    #[test]
    fn a_deleted_build_is_a_state_not_an_error() {
        let mut app = app_on_latest_build();
        app.update(Action::BuildStep(BuildStep::Older));
        arrive(&mut app, BuildRef::Number(7), None);
        let view = app.build.as_ref().unwrap();
        assert_eq!(view.load, BuildLoad::Loaded(None));
        // Can still move on from there.
        assert_eq!(view.step(BuildStep::Older), Some(BuildRef::Number(5)));
    }

    #[test]
    fn steps_scroll_back_to_the_top() {
        let mut app = app_on_latest_build();
        app.build.as_ref().unwrap().max_scroll.set(5);
        app.update(Action::SelectLast);
        app.update(Action::BuildStep(BuildStep::Older));
        assert_eq!(app.build.as_ref().unwrap().scroll, 0);
    }

    #[test]
    fn quit_asks_for_confirmation_by_default() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::RequestQuit);
        assert!(app.running);
        assert_eq!(app.context(), Context::ConfirmQuit);
        // Esc / n: stay.
        app.update(Action::Back);
        assert!(app.running);
        assert_eq!(app.context(), Context::Jobs);
        // q q: quit.
        app.update(Action::RequestQuit);
        app.update(Action::RequestQuit);
        assert!(!app.running);
    }

    #[test]
    fn prompt_sits_on_top_of_other_contexts() {
        let mut app = settings_app();
        app.update(Action::ToggleHelp);
        app.update(Action::RequestQuit);
        assert_eq!(app.context(), Context::ConfirmQuit);
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Help, "back to where we were");
    }

    #[test]
    fn quit_without_confirmation_when_disabled() {
        let mut app = App::default();
        app.settings
            .file
            .set(SettingKey::ConfirmQuit, Some("false".into()));
        app.update(Action::RequestQuit);
        assert!(!app.running);
    }

    #[test]
    fn confirm_quit_setting_defaults_on_and_toggles_to_explicit_off() {
        let mut app = settings_app();
        assert!(app.settings.effective().is_on(SettingKey::ConfirmQuit));
        select(&mut app, SettingsRow::Setting(SettingKey::ConfirmQuit));
        app.update(Action::StartEdit);
        assert_eq!(
            app.settings.file.get(SettingKey::ConfirmQuit),
            Some("false")
        );
    }

    #[test]
    fn theme_setting_cycles_and_applies_right_away() {
        let mut app = settings_app();
        app.terminal_appearance = Some(Appearance::Light);
        assert_eq!(app.theme().name, "light", "auto follows the terminal");
        select(&mut app, SettingsRow::Setting(SettingKey::Theme));

        let mut seen = Vec::new();
        for _ in 0..3 {
            let effects = app.update(Action::StartEdit);
            assert!(
                effects
                    .iter()
                    .any(|e| matches!(e, Effect::SaveSettings { .. })),
                "each change is saved"
            );
            seen.push((
                app.settings.file.get(SettingKey::Theme).map(str::to_owned),
                app.theme().name,
            ));
        }
        assert_eq!(
            seen,
            [
                (Some("dark".into()), "dark"),
                (Some("light".into()), "light"),
                (None, "light"), // back to auto = unset
            ]
        );
    }

    #[test]
    fn auto_theme_is_dark_when_the_terminal_did_not_say() {
        let app = App::default();
        assert_eq!(app.terminal_appearance, None);
        assert_eq!(app.theme().name, "dark");
    }

    use crate::console::ConsoleChunk;

    fn console_fetch(effects: &[Effect]) -> Option<(u64, u64)> {
        effects.iter().find_map(|e| match e {
            Effect::FetchConsole { number, start, .. } => Some((*number, *start)),
            _ => None,
        })
    }

    fn chunk_arrives(app: &mut App, start: u64, text: &str, more: bool) {
        let generation = app.connection_generation;
        let number = app.console.as_ref().unwrap().number;
        app.update(Action::ConsoleFetched {
            generation,
            job: "a".into(),
            number,
            start,
            result: Ok(ConsoleChunk {
                bytes: text.as_bytes().to_vec(),
                next: start + text.len() as u64,
                more,
            }),
        });
    }

    #[test]
    fn c_opens_the_console_of_the_shown_build() {
        let mut app = app_on_latest_build(); // #8
        let effects = app.update(Action::OpenConsole);
        assert_eq!(console_fetch(&effects), Some((8, 0)));
        assert_eq!(app.view, View::Console);
        assert_eq!(app.context(), Context::Console);
        assert_eq!(app.view.tab(), Some(Tab::Jobs));
        chunk_arrives(&mut app, 0, "line 1\nline 2\n", false);
        let console = app.console.as_ref().unwrap();
        assert_eq!(console.load, ConsoleLoad::Loaded);
        assert_eq!(console.line_count(), 2);
        // Esc: back to the build.
        app.update(Action::Back);
        assert_eq!(app.view, View::Build);
        assert!(app.console.is_none());
    }

    #[test]
    fn no_console_without_a_loaded_build() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::OpenBuild); // still loading
        assert_eq!(app.update(Action::OpenConsole), Vec::new());
        assert_eq!(app.view, View::Build);
    }

    #[test]
    fn running_build_output_is_tailed_every_second() {
        let mut app = app_on_latest_build();
        app.update(Action::OpenConsole);
        assert_eq!(
            tick(&mut app, Duration::from_secs(5)),
            Vec::new(),
            "first fetch in flight"
        );
        chunk_arrives(&mut app, 0, "a\n", true);
        assert_eq!(
            tick(&mut app, Duration::from_millis(500)),
            Vec::new(),
            "not due"
        );
        let effects = tick(&mut app, Duration::from_millis(500));
        assert_eq!(
            console_fetch(&effects),
            Some((8, 2)),
            "continues at the offset"
        );
        assert_eq!(app.update(Action::Refresh), Vec::new(), "one in flight");
        chunk_arrives(&mut app, 2, "b\n", false);
        assert_eq!(app.console.as_ref().unwrap().line_count(), 2);
        assert_eq!(
            tick(&mut app, Duration::from_secs(60)),
            Vec::new(),
            "complete: polling stops"
        );
    }

    #[test]
    fn tailing_ignores_auto_refresh_setting() {
        let mut app = app_on_latest_build();
        app.auto_refresh = false;
        app.update(Action::OpenConsole);
        chunk_arrives(&mut app, 0, "a\n", true);
        assert!(console_fetch(&tick(&mut app, Duration::from_secs(1))).is_some());
    }

    #[test]
    fn stale_or_duplicate_chunks_are_ignored() {
        let mut app = app_on_latest_build();
        app.update(Action::OpenConsole);
        chunk_arrives(&mut app, 0, "a\n", true);
        chunk_arrives(&mut app, 0, "a\n", true); // same chunk again
        assert_eq!(app.console.as_ref().unwrap().line_count(), 1);
    }

    #[test]
    fn poll_errors_keep_the_output() {
        let mut app = app_on_latest_build();
        app.update(Action::OpenConsole);
        chunk_arrives(&mut app, 0, "a\n", true);
        tick(&mut app, Duration::from_secs(1));
        let generation = app.connection_generation;
        app.update(Action::ConsoleFetched {
            generation,
            job: "a".into(),
            number: 8,
            start: 2,
            result: Err("timed out".into()),
        });
        let console = app.console.as_ref().unwrap();
        assert_eq!(console.load, ConsoleLoad::Loaded);
        assert_eq!(console.last_error.as_deref(), Some("timed out"));
        assert_eq!(console.line_count(), 1);
    }

    #[test]
    fn console_scroll_actions() {
        let mut app = app_on_latest_build();
        app.update(Action::OpenConsole);
        let text: String = (0..50).map(|i| format!("{i}\n")).collect();
        chunk_arrives(&mut app, 0, &text, true);
        app.console.as_ref().unwrap().viewport.set(10);
        app.update(Action::SelectPrev);
        assert!(!app.console.as_ref().unwrap().following);
        app.update(Action::SelectFirst);
        assert_eq!(app.console.as_ref().unwrap().visible_top(), 0);
        app.update(Action::SelectPageDown);
        assert_eq!(app.console.as_ref().unwrap().visible_top(), 10);
        app.update(Action::SelectLast);
        assert!(app.console.as_ref().unwrap().following);
        app.update(Action::ScrollRight);
        app.update(Action::ScrollRight);
        app.update(Action::ScrollLeft);
        assert_eq!(
            app.console.as_ref().unwrap().left,
            crate::console::HSCROLL_STEP
        );
    }

    use crate::history::HistoryEntry;

    fn entry(job: &str, number: u64, age_secs: u64) -> HistoryEntry {
        HistoryEntry {
            job: job.into(),
            number,
            result: Some(crate::jobs::JobStatus::Success),
            building: false,
            started: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
            duration: Duration::from_secs(30),
        }
    }

    fn history_fetch(effects: &[Effect]) -> Option<usize> {
        effects.iter().find_map(|e| match e {
            Effect::FetchHistory { limit, .. } => Some(*limit),
            _ => None,
        })
    }

    /// Connected, on the Builds tab with a 3-row list and `entries` loaded.
    fn builds_tab(entries: Vec<HistoryEntry>) -> App {
        let mut app = connected_with_jobs(&["a", "b"]);
        app.list_rows.set(3);
        let limit = history_fetch(&app.update(Action::SwitchTab(Tab::Builds))).unwrap();
        assert_eq!(limit, 3, "as many as fit on screen");
        assert_eq!(app.context(), Context::Builds);
        let generation = app.connection_generation;
        app.update(Action::HistoryFetched {
            generation,
            limit,
            result: Ok(entries),
        });
        app
    }

    fn five_builds() -> Vec<HistoryEntry> {
        vec![
            entry("a", 5, 10),
            entry("b", 9, 20),
            entry("a", 4, 30),
            entry("b", 8, 40),
            entry("a", 3, 50),
        ]
    }

    #[test]
    fn builds_tab_fetches_one_page_and_only_once() {
        let mut app = builds_tab(five_builds());
        assert_eq!(app.history.visible().len(), 3, "only the guaranteed rows");
        app.update(Action::SwitchTab(Tab::Jobs));
        assert_eq!(
            history_fetch(&app.update(Action::SwitchTab(Tab::Builds))),
            None,
            "already loaded"
        );
    }

    #[test]
    fn moving_past_the_end_loads_the_next_page() {
        let mut app = builds_tab(five_builds());
        app.update(Action::SelectNext);
        app.update(Action::SelectNext);
        assert!(app.history.at_end());
        let effects = app.update(Action::SelectNext);
        assert_eq!(history_fetch(&effects), Some(6), "one more page");
        assert!(app.history.refreshing, "the loaded rows stay visible");
        assert_eq!(
            app.update(Action::SelectNext),
            Vec::new(),
            "one fetch at a time"
        );
    }

    #[test]
    fn builds_filter() {
        let mut app = builds_tab(five_builds());
        app.update(Action::StartFilter);
        assert_eq!(app.context(), Context::BuildsFilter);
        type_str(&mut app, "b");
        let jobs: Vec<&str> = app
            .history
            .visible()
            .iter()
            .map(|e| e.job.as_str())
            .collect();
        assert_eq!(jobs, ["b", "b"]);
        app.update(Action::ConfirmEdit);
        assert_eq!(app.context(), Context::BuildsFiltered);
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Builds);
        assert_eq!(app.history.visible().len(), 3);
    }

    #[test]
    fn enter_opens_that_build_and_esc_returns_to_builds() {
        let mut app = builds_tab(five_builds());
        app.update(Action::SelectNext); // b #9
        let effects = app.update(Action::OpenBuild);
        let fetch = effects.iter().find_map(|e| match e {
            Effect::FetchBuild {
                job,
                which,
                want_numbers,
                ..
            } => Some((job.clone(), *which, *want_numbers)),
            _ => None,
        });
        assert_eq!(fetch, Some(("b".into(), BuildRef::Number(9), true)));
        assert_eq!(app.view, View::Build);
        assert_eq!(
            app.tab(),
            Some(Tab::Builds),
            "the Builds tab stays highlighted"
        );
        app.update(Action::Back);
        assert_eq!(app.view, View::Builds);
        assert_eq!(app.history.selected, 1, "selection kept");
    }

    #[test]
    fn refresh_on_the_builds_tab_refetches_history() {
        let mut app = builds_tab(five_builds());
        let effects = app.update(Action::Refresh);
        assert_eq!(history_fetch(&effects), Some(3));
        assert_eq!(fetches(&effects), 0, "not the job list");
        assert_eq!(app.update(Action::Refresh), Vec::new(), "in flight");
    }

    #[test]
    fn history_of_an_old_connection_is_ignored() {
        let mut app = connected_with_jobs(&["a"]);
        app.list_rows.set(3);
        app.update(Action::SwitchTab(Tab::Builds));
        let old = app.connection_generation;
        app.update(Action::HistoryFetched {
            generation: old + 1,
            limit: 3,
            result: Ok(five_builds()),
        });
        assert_eq!(app.history.load, HistoryLoad::Loading);
    }

    fn pipelines_fetch(effects: &[Effect]) -> bool {
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPipelines { .. }))
    }

    fn pipeline_data() -> PipelineData {
        crate::pipelines::parse(
            r#"{"jobs": [
                {"fullName": "app", "color": "blue",
                 "downstreamProjects": [{"fullName": "deploy"}],
                 "builds": [{"number": 3, "result": "SUCCESS", "timestamp": 1000}]},
                {"fullName": "deploy", "color": "blue", "builds": [
                    {"number": 9, "result": "SUCCESS", "timestamp": 2000,
                     "actions": [{"causes": [{"upstreamProject": "app", "upstreamBuild": 3}]}]}]}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn pipelines_and_runs_share_one_fetch() {
        let mut app = connected_with_jobs(&["a"]);
        assert!(pipelines_fetch(
            &app.update(Action::SwitchTab(Tab::Pipelines))
        ));
        assert!(
            !pipelines_fetch(&app.update(Action::SwitchTab(Tab::Runs))),
            "already loading, for both tabs"
        );
        assert!(
            !pipelines_fetch(&app.update(Action::Refresh)),
            "one fetch in flight"
        );
        let generation = app.connection_generation;
        app.update(Action::PipelinesFetched {
            generation,
            result: Ok(pipeline_data()),
        });
        assert_eq!(app.pipelines.load, PipelineLoad::Loaded);
        assert!(pipelines_fetch(&app.update(Action::Refresh)), "r reloads");
        assert!(app.pipelines.refreshing, "old data stays visible");
    }

    #[test]
    fn pipelines_of_an_old_connection_are_ignored() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::SwitchTab(Tab::Pipelines));
        let old = app.connection_generation;
        app.update(Action::SwitchTab(Tab::Settings));
        set_url(&mut app, "https://other");
        app.update(Action::PipelinesFetched {
            generation: old,
            result: Ok(pipeline_data()),
        });
        assert_eq!(
            app.pipelines.load,
            PipelineLoad::NotLoaded,
            "reset, not filled"
        );
    }

    #[test]
    fn runs_open_the_run_then_the_exact_build() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::SwitchTab(Tab::Runs));
        let generation = app.connection_generation;
        app.update(Action::PipelinesFetched {
            generation,
            result: Ok(pipeline_data()),
        });
        assert!(
            app.update(Action::OpenBuild).is_empty(),
            "no fetch: data is here"
        );
        assert_eq!(app.view, View::Run);
        app.update(Action::SelectNext); // deploy #9, triggered by app #3
        let effects = app.update(Action::OpenBuild);
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::FetchBuild { job, which: BuildRef::Number(9), want_numbers: true, .. }
                if job == "deploy"
        )));
        assert_eq!(app.tab(), Some(Tab::Runs));
        app.update(Action::Back);
        assert_eq!(app.view, View::Run);
        assert!(app.run.as_ref().unwrap().tree, "tree by default");
        app.update(Action::ToggleRunView);
        assert!(!app.run.as_ref().unwrap().tree);
        app.update(Action::Back);
        assert_eq!(app.view, View::Runs);
        assert!(app.run.is_none());
    }

    #[test]
    fn reconnecting_leaves_the_run_view() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::SwitchTab(Tab::Pipelines));
        let generation = app.connection_generation;
        app.update(Action::PipelinesFetched {
            generation,
            result: Ok(pipeline_data()),
        });
        app.update(Action::OpenBuild);
        assert_eq!(app.view, View::Run);
        app.update(Action::SwitchTab(Tab::Settings));
        app.update(Action::SwitchTab(Tab::Pipelines));
        // (Switching tabs leaves the run view behind; a reconnect clears it.)
        set_url(&mut app, "https://other");
        assert!(app.run.is_none());
    }

    #[test]
    fn pipelines_filter_is_typed_and_cleared() {
        let mut app = connected_with_jobs(&["a"]);
        app.update(Action::SwitchTab(Tab::Pipelines));
        app.update(Action::StartFilter);
        assert_eq!(app.context(), Context::PipelinesFilter);
        type_str(&mut app, "dep");
        app.update(Action::ConfirmEdit);
        assert_eq!(app.context(), Context::PipelinesFiltered);
        assert_eq!(app.pipelines.list.filter, "dep");
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Pipelines, "Esc clears the filter");
    }

    #[test]
    fn each_tab_remembers_its_filter() {
        let mut app = connected_with_jobs(&["a"]);
        let filters = [
            (Tab::Jobs, "jobs"),
            (Tab::Builds, "builds"),
            (Tab::Pipelines, "pipes"),
            (Tab::Runs, "runs"),
        ];
        for (tab, text) in filters {
            app.update(Action::SwitchTab(tab));
            app.update(Action::StartFilter);
            type_str(&mut app, text);
            app.update(Action::ConfirmEdit);
        }
        for (tab, text) in filters {
            app.update(Action::SwitchTab(tab));
            let applied = match tab {
                Tab::Jobs => &app.jobs.filter,
                Tab::Builds => &app.history.filter,
                Tab::Pipelines => &app.pipelines.list.filter,
                _ => &app.pipelines.runs.filter,
            };
            assert_eq!(applied, text, "{tab:?}");
        }
    }
}
