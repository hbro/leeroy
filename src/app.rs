use std::path::PathBuf;

use crate::config::{SettingKey, Settings};

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
    Input(char),
    DeleteChar,
    ClearInput,
    /// Store the edited value and save the config file.
    ConfirmEdit,
    /// Outcome of [`Effect::SaveSettings`]; `Err` holds a printable message.
    SettingsSaved(Result<(), String>),
}

/// Side effects requested by [`App::update`]. The IO shell (`main.rs`) runs
/// them and reports back with an [`Action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    SaveSettings { path: PathBuf, settings: Settings },
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
    Connected { url: String },
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
    pub selected: usize,
    /// Input buffer while editing the selected setting.
    pub editing: Option<String>,
    pub message: Option<StatusMessage>,
}

impl SettingsState {
    pub fn new(path: PathBuf, file: Settings, env: Settings) -> Self {
        Self {
            path,
            file,
            env,
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
}

#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub view: View,
    pub show_help: bool,
    pub connection: ConnectionStatus,
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
            settings,
        }
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
    /// IO is requested by returning an [`Effect`].
    pub fn update(&mut self, action: Action) -> Option<Effect> {
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
            Action::SelectNext => s.selected = (s.selected + 1) % SettingKey::ALL.len(),
            Action::SelectPrev => {
                s.selected = (s.selected + SettingKey::ALL.len() - 1) % SettingKey::ALL.len()
            }
            Action::StartEdit => {
                let key = s.selected_key();
                if s.is_overridden(key) {
                    s.message = Some(StatusMessage::Error(format!(
                        "{} is set by ${}; unset it to edit here",
                        key.label(),
                        key.env_var()
                    )));
                } else {
                    s.editing = Some(s.file.get(key).unwrap_or_default().to_owned());
                    s.message = None;
                }
            }
            Action::Input(c) => {
                if let Some(buf) = &mut s.editing {
                    buf.push(c);
                }
            }
            Action::DeleteChar => {
                if let Some(buf) = &mut s.editing {
                    buf.pop();
                }
            }
            Action::ClearInput => {
                if let Some(buf) = &mut s.editing {
                    buf.clear();
                }
            }
            Action::ConfirmEdit => {
                if let Some(buf) = s.editing.take() {
                    let value = buf.trim();
                    let value = (!value.is_empty()).then(|| value.to_owned());
                    s.file.set(s.selected_key(), value);
                    s.message = Some(StatusMessage::Info("Saving…".into()));
                    return Some(Effect::SaveSettings {
                        path: s.path.clone(),
                        settings: s.file.clone(),
                    });
                }
            }
            Action::SettingsSaved(Ok(())) => {
                s.message = Some(StatusMessage::Info(format!(
                    "Saved to {}",
                    s.path.display()
                )));
            }
            Action::SettingsSaved(Err(err)) => {
                s.message = Some(StatusMessage::Error(format!("Save failed: {err}")));
            }
        }
        None
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
        assert_eq!(app.settings.selected_key(), SettingKey::JenkinsToken);
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
        let effect = app.update(Action::ConfirmEdit);

        let mut expected = Settings::default();
        expected.set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        assert_eq!(
            effect,
            Some(Effect::SaveSettings {
                path: app.settings.path.clone(),
                settings: expected
            })
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
        assert_eq!(app.settings.editing.as_deref(), Some("https://ci"));
        app.update(Action::ClearInput);
        app.update(Action::ConfirmEdit);
        assert_eq!(app.settings.file.get(SettingKey::JenkinsUrl), None);
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
}
