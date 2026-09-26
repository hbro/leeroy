use std::{
    path::PathBuf,
    time::{Instant, SystemTime},
};

use crate::{
    builds::{Build, BuildLoad, BuildView},
    config::{SettingKey, Settings, header_env_var, parse_header, redact_url},
    input::TextInput,
    jenkins::{ConnectionConfig, ServerInfo},
    jobs::{Job, JobsLoad, JobsState},
    proxy::{ProxyEnv, SystemProxy},
};

/// Everything that can change application state.
///
/// Key presses, timer ticks and results of [`Effect`]s are all translated
/// into actions, so every state transition goes through [`App::update`] and
/// can be unit tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    /// Close the current context (overlay, edit, sub-view). No-op at the root view.
    Back,
    /// Timer tick with the current time, monotonic (auto-refresh, data age)
    /// and wall clock (build start times). The only way time enters the
    /// core, so tests control it.
    Tick(Instant, SystemTime),
    OpenSettings,
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
    /// Outcome of [`Effect::FetchBuild`]; `Ok(None)`: never built.
    BuildFetched {
        generation: u64,
        job: String,
        result: Result<Option<Build>, String>,
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
    /// Load `job`'s last build; answer with [`Action::BuildFetched`].
    FetchBuild {
        generation: u64,
        job: String,
        config: ConnectionConfig,
    },
}

/// Content tabs, switched with F-keys (`F1` = first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Jobs,
}

impl Tab {
    pub const ALL: [Tab; 1] = [Tab::Jobs];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Jobs => "Jobs",
        }
    }

    /// The F-key number: `Tab::ALL[n - 1]`.
    pub fn f_key(self) -> u8 {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0) as u8 + 1
    }

    pub fn from_f_key(n: u8) -> Option<Tab> {
        Tab::ALL.get(usize::from(n).checked_sub(1)?).copied()
    }

    fn view(self) -> View {
        match self {
            Tab::Jobs => View::Jobs,
        }
    }
}

/// Rows moved by PgUp/PgDn.
pub const PAGE: isize = 10;

/// The main view being displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Jobs,
    /// A job's most recent build (within the Jobs tab).
    Build,
    Settings,
}

impl View {
    /// The tab this view belongs to (`None` for settings).
    pub fn tab(self) -> Option<Tab> {
        match self {
            View::Jobs | View::Build => Some(Tab::Jobs),
            View::Settings => None,
        }
    }

    /// The context this view provides when nothing is overlaid on it.
    pub fn context(self) -> Context {
        match self {
            View::Jobs => Context::Jobs,
            View::Build => Context::Build,
            View::Settings => Context::Settings,
        }
    }
}

/// What currently has focus: decides which context keybindings apply and
/// what the context bar shows. Overlays take precedence over the view below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Jobs,
    /// Jobs tab with a filter applied (Esc clears it).
    JobsFiltered,
    /// Typing the `/` filter.
    JobsFilter,
    Build,
    Settings,
    EditSetting,
    Help,
}

impl Context {
    pub fn title(self) -> &'static str {
        match self {
            Context::Jobs | Context::JobsFiltered => "Jobs",
            Context::JobsFilter => "Filter",
            Context::Build => "Build",
            Context::Settings => "Settings",
            Context::EditSetting => "Edit",
            Context::Help => "Help",
        }
    }

    /// Whether [`Action::Back`] can close this context.
    pub fn closable(self) -> bool {
        match self {
            Context::Jobs => false,
            Context::JobsFiltered
            | Context::JobsFilter
            | Context::Build
            | Context::Settings
            | Context::EditSetting
            | Context::Help => true,
        }
    }

    /// Text input: all printable keys go to the input, global keys are off.
    pub fn captures_input(self) -> bool {
        matches!(self, Context::EditSetting | Context::JobsFilter)
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
        let mut rows: Vec<SettingsRow> = SettingKey::ALL.map(SettingsRow::Setting).into();
        rows.extend(
            self.effective()
                .jenkins
                .headers
                .into_keys()
                .map(SettingsRow::Header),
        );
        rows.push(SettingsRow::AddHeader);
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
    pub view: View,
    pub show_help: bool,
    pub connection: ConnectionStatus,
    /// Id of the latest connection attempt; results of older ones are stale.
    pub connection_generation: u64,
    pub settings: SettingsState,
    pub jobs: JobsState,
    /// The open build view (with [`View::Build`]).
    pub build: Option<BuildView>,
    /// Time of the latest [`Action::Tick`].
    pub now: Instant,
    /// Wall-clock time of the latest [`Action::Tick`].
    pub wall_now: SystemTime,
    /// Auto-refresh for this session; starts from the `refresh.auto` setting.
    pub auto_refresh: bool,
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
            view: View::Jobs,
            show_help: false,
            connection: ConnectionStatus::NotConfigured,
            connection_generation: 0,
            auto_refresh: settings.effective().is_on(SettingKey::RefreshAuto),
            settings,
            jobs: JobsState::default(),
            build: None,
            now: Instant::now(),
            wall_now: SystemTime::now(),
        }
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
        self.build = None;
        if self.view == View::Build {
            self.view = View::Jobs;
        }
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
        if self.show_help {
            Context::Help
        } else if self.view == View::Settings && self.settings.editing.is_some() {
            Context::EditSetting
        } else if self.view == View::Jobs && self.jobs.filter_input.is_some() {
            Context::JobsFilter
        } else if self.view == View::Jobs && !self.jobs.filter.is_empty() {
            Context::JobsFiltered
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
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Back => self.back(),
            Action::Tick(now, wall) => {
                self.now = now;
                self.wall_now = wall;
                return self.auto_refresh_if_due();
            }
            Action::ToggleAutoRefresh => {
                self.auto_refresh = !self.auto_refresh;
                // Turning it on refreshes right away when the data is stale.
                return self.auto_refresh_if_due();
            }
            Action::OpenSettings => {
                self.view = View::Settings;
                self.show_help = false;
            }
            Action::SwitchTab(tab) => {
                self.view = tab.view();
                self.show_help = false;
            }
            Action::StartFilter => {
                self.jobs.filter_input = Some(TextInput::new(&self.jobs.filter));
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
                let filtering = context == Context::JobsFilter;
                let target = if filtering {
                    self.jobs.filter_input.as_mut()
                } else {
                    self.settings.editing.as_mut()
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
                    }
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
                    return self.fetch_jobs();
                }
            }
            Action::BuildFetched {
                generation,
                job,
                result,
            } => {
                let now = self.now;
                let Some(build) = self
                    .build
                    .as_mut()
                    .filter(|b| b.job == job && generation == self.connection_generation)
                else {
                    return Vec::new(); // another job or connection by now
                };
                build.refreshing = false;
                build.attempted_at = Some(now);
                if result.is_ok() {
                    build.fetched_at = Some(now);
                }
                build.load = match result {
                    Ok(found) => BuildLoad::Loaded(found),
                    Err(error) => BuildLoad::Failed(error),
                };
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
        Some(Effect::FetchBuild {
            generation: self.connection_generation,
            job: self.build.as_ref()?.job.clone(),
            config: self.settings.connection_config()?,
        })
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

    /// `r`: reload what's on screen when connected; otherwise (re)connect.
    fn refresh(&mut self) -> Vec<Effect> {
        match self.connection {
            ConnectionStatus::Connected { .. } if self.view == View::Build => self.fetch_build(),
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
            Context::Help => self.show_help = false,
            Context::EditSetting => self.settings.editing = None,
            // Cancel typing: the previously applied filter stays.
            Context::JobsFilter => self.jobs.filter_input = None,
            Context::JobsFiltered => {
                self.jobs.filter.clear();
                self.jobs.clamp_selection();
            }
            Context::Build => {
                self.view = View::Jobs;
                self.build = None;
            }
            Context::Settings => {
                self.view = View::Jobs;
                self.settings.message = None;
            }
            Context::Jobs => {}
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
        app.update(Action::OpenSettings);
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
        app.update(Action::Back);
        assert_eq!(app.context(), Context::Jobs);
    }

    #[test]
    fn selection_wraps() {
        let mut app = settings_app();
        app.update(Action::SelectPrev);
        assert_eq!(app.settings.selected_row(), SettingsRow::AddHeader);
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
        app.update(Action::OpenSettings);
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
    fn f_key_leaves_settings() {
        let mut app = settings_app();
        app.update(Action::SwitchTab(Tab::Jobs));
        assert_eq!(app.view, View::Jobs);
        assert_eq!(Tab::from_f_key(1), Some(Tab::Jobs));
        assert_eq!(Tab::from_f_key(0), None);
        assert_eq!(Tab::Jobs.f_key(), 1);
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
        app.update(Action::OpenSettings);
        set_url(&mut app, "https://other");
        let effects = app.update(Action::ConnectFinished {
            generation: app.connection_generation,
            result: Ok(info("me")),
        });
        assert_eq!(fetches(&effects), 1);
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
            result: Ok(Some(sample_build(7))),
        });
        let build = app.build.as_ref().unwrap();
        assert_eq!(build.load, BuildLoad::Loaded(Some(sample_build(7))));
        assert_eq!(build.fetched_at, Some(app.now));
    }

    #[test]
    fn no_builds_and_failures_are_states() {
        let mut app = connected_with_jobs(&["a"]);
        let (generation, _) = build_fetch(&app.update(Action::OpenBuild)).unwrap();
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
            result: Ok(None),
        });
        assert_eq!(app.build.as_ref().unwrap().load, BuildLoad::Loaded(None));
        app.update(Action::Refresh);
        app.update(Action::BuildFetched {
            generation,
            job: "a".into(),
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
            result: Ok(Some(sample_build(1))),
        });
        assert_eq!(app.build.as_ref().unwrap().load, BuildLoad::Loading);
        app.update(Action::BuildFetched {
            generation: generation + 99,
            job: "b".into(),
            result: Ok(Some(sample_build(1))),
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
            result: Ok(Some(sample_build(1))),
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
            result: Ok(Some(sample_build(2))),
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
}
