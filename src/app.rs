use std::path::PathBuf;

use crate::{
    config::{SettingKey, Settings},
    input::TextInput,
    jenkins::{ConnectionConfig, ServerInfo},
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
    Tick,
    OpenSettings,
    SelectNext,
    SelectPrev,
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
}

/// The main view being displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Jobs,
    Settings,
}

impl View {
    /// The context this view provides when nothing is overlaid on it.
    pub fn context(self) -> Context {
        match self {
            View::Jobs => Context::Jobs,
            View::Settings => Context::Settings,
        }
    }
}

/// What currently has focus: decides which context keybindings apply and
/// what the context bar shows. Overlays take precedence over the view below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Jobs,
    Settings,
    EditSetting,
    Help,
}

impl Context {
    pub fn title(self) -> &'static str {
        match self {
            Context::Jobs => "Jobs",
            Context::Settings => "Settings",
            Context::EditSetting => "Edit",
            Context::Help => "Help",
        }
    }

    /// Whether [`Action::Back`] can close this context.
    pub fn closable(self) -> bool {
        match self {
            Context::Jobs => false,
            Context::Settings | Context::EditSetting | Context::Help => true,
        }
    }

    /// Text input: all printable keys go to the input, global keys are off.
    pub fn captures_input(self) -> bool {
        matches!(self, Context::EditSetting)
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

    pub fn selected_key(&self) -> SettingKey {
        SettingKey::ALL[self.selected]
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
        let credentials = match (
            settings.get(SettingKey::JenkinsUsername),
            settings.get(SettingKey::JenkinsToken),
        ) {
            (Some(user), Some(token)) => Some((user.to_owned(), token.to_owned())),
            _ => None,
        };
        let proxy = match settings.get(SettingKey::ProxyUrl) {
            Some(proxy) => Some(proxy.to_owned()),
            None => match self.system_proxy() {
                SystemProxy::Proxy { url, .. } => Some(url),
                SystemProxy::Bypassed { .. } | SystemProxy::None => None,
            },
        };
        Some(ConnectionConfig {
            url,
            credentials,
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
            settings,
        }
    }

    /// Effects to run once at startup: connect if configured.
    pub fn start(&mut self) -> Vec<Effect> {
        self.connect().into_iter().collect()
    }

    /// Start a new connection attempt with the current settings.
    fn connect(&mut self) -> Option<Effect> {
        self.connection_generation += 1;
        let Some(config) = self.settings.connection_config() else {
            self.connection = ConnectionStatus::NotConfigured;
            return None;
        };
        self.connection = ConnectionStatus::Connecting {
            url: config.url.clone(),
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
        } else {
            self.view.context()
        }
    }

    /// Apply an action. Must stay free of IO so tests can drive it directly;
    /// IO is requested by returning [`Effect`]s.
    pub fn update(&mut self, action: Action) -> Vec<Effect> {
        let s = &mut self.settings;
        match action {
            Action::Quit => self.running = false,
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Back => self.back(),
            Action::Tick => {}
            Action::OpenSettings => {
                self.view = View::Settings;
                self.show_help = false;
            }
            Action::SelectNext | Action::SelectPrev => {
                // While editing, moving away saves the field first; an invalid
                // value keeps the edit open instead of moving.
                let effects = self.confirm_edit();
                let s = &mut self.settings;
                if s.editing.is_none() {
                    let len = SettingKey::ALL.len();
                    s.selected = match action {
                        Action::SelectNext => (s.selected + 1) % len,
                        _ => (s.selected + len - 1) % len,
                    };
                }
                return effects;
            }
            Action::StartEdit => {
                let key = s.selected_key();
                if s.is_overridden(key) {
                    s.message = Some(StatusMessage::Error(format!(
                        "{} is set by ${}; unset it to edit here",
                        key.label(),
                        key.env_var()
                    )));
                } else if key.is_bool() {
                    // On/off settings toggle directly; off = unset (the default).
                    let value = (!s.file.is_on(key)).then(|| "true".to_owned());
                    return self.store(key, value);
                } else {
                    s.editing = Some(TextInput::new(s.file.get(key).unwrap_or_default()));
                    s.message = None;
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
                if let Some(input) = &mut s.editing {
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
            }
        }
        Vec::new()
    }

    /// Validate and store the value being edited. An invalid value keeps the
    /// edit open (with an error) so the typo can be fixed. No-op when not editing.
    fn confirm_edit(&mut self) -> Vec<Effect> {
        let s = &mut self.settings;
        let Some(input) = s.editing.take() else {
            return Vec::new();
        };
        let key = s.selected_key();
        let value = if input.value().trim().is_empty() {
            None
        } else {
            match key.validate(input.value()) {
                Ok(value) => Some(value),
                Err(err) => {
                    s.editing = Some(input);
                    s.message = Some(StatusMessage::Error(err));
                    return Vec::new();
                }
            }
        };
        self.store(key, value)
    }

    /// Store a (validated) setting in the file layer, save, and reconnect if
    /// that changed how Jenkins is reached.
    fn store(&mut self, key: SettingKey, value: Option<String>) -> Vec<Effect> {
        let s = &mut self.settings;
        let before = s.connection_config();
        s.file.set(key, value);
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
            Context::Settings => {
                self.view = View::Jobs;
                self.settings.message = None;
            }
            Context::Jobs => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_app() -> App {
        let mut app = App::default();
        app.update(Action::OpenSettings);
        app
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
        assert_eq!(app.settings.selected_key(), SettingKey::ProxyUrl);
        app.update(Action::SelectNext);
        assert_eq!(app.settings.selected_key(), SettingKey::JenkinsUrl);
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
        app.update(Action::SelectPrev); // proxy URL
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
        // Username alone doesn't enable auth (needs a token too).
        app.update(Action::SelectNext);
        app.update(Action::StartEdit);
        type_str(&mut app, "me");
        assert_eq!(connect_generation(&app.update(Action::ConfirmEdit)), None);
        // The token completes the credentials: reconnect.
        app.update(Action::SelectNext);
        app.update(Action::StartEdit);
        type_str(&mut app, "tok");
        assert!(connect_generation(&app.update(Action::ConfirmEdit)).is_some());
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
        app.settings.selected = 3;
        assert_eq!(
            app.settings.selected_key(),
            SettingKey::JenkinsSkipTlsVerify
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
        assert_eq!(app.settings.selected_key(), SettingKey::JenkinsUsername);
        assert_eq!(app.context(), Context::Settings);
    }

    #[test]
    fn arrows_while_editing_invalid_value_stay() {
        let mut app = settings_app();
        app.update(Action::StartEdit);
        type_str(&mut app, "not a url");
        assert_eq!(app.update(Action::SelectPrev), Vec::new());
        assert_eq!(app.settings.selected_key(), SettingKey::JenkinsUrl);
        assert_eq!(app.context(), Context::EditSetting);
        assert!(matches!(
            app.settings.message,
            Some(StatusMessage::Error(_))
        ));
    }
}
