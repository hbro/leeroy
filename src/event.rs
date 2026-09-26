use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{Action, App, Context, Tab};

/// A keybinding as shown to the user. Display-only: keep in sync with
/// [`map_key`] (the tests below check the advertised keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub key: &'static str,
    pub desc: &'static str,
}

const fn bind(key: &'static str, desc: &'static str) -> Binding {
    Binding { key, desc }
}

/// Bindings that work everywhere (bottom bar), except while typing text.
pub const GLOBAL_BINDINGS: &[Binding] = &[
    bind("q", "quit"),
    bind("s", "settings"),
    bind("?", "help"),
    bind("r", "refresh"),
    bind("R", "toggle auto-refresh"),
];

/// Bindings that only apply in the given context (context bar).
pub fn context_bindings(context: Context) -> &'static [Binding] {
    const JOBS: &[Binding] = &[bind("↑/↓", "select"), bind("/", "filter")];
    const JOBS_FILTERED: &[Binding] = &[
        bind("↑/↓", "select"),
        bind("/", "filter"),
        bind("Esc", "clear filter"),
    ];
    const JOBS_FILTER: &[Binding] = &[
        bind("Enter", "apply"),
        bind("Esc", "cancel"),
        bind("↑/↓", "select"),
        bind("C-u", "clear"),
    ];
    const SETTINGS: &[Binding] = &[
        bind("↑/↓", "select"),
        bind("Enter", "edit/toggle"),
        bind("Esc", "back"),
    ];
    const EDIT: &[Binding] = &[
        bind("Enter", "save"),
        bind("Esc", "cancel"),
        bind("↑/↓", "save & move"),
        bind("←/→", "cursor"),
        bind("C-u", "clear"),
    ];
    const HELP: &[Binding] = &[bind("Esc", "close")];
    match context {
        Context::Jobs => JOBS,
        Context::JobsFiltered => JOBS_FILTERED,
        Context::JobsFilter => JOBS_FILTER,
        Context::Settings => SETTINGS,
        Context::EditSetting => EDIT,
        Context::Help => HELP,
    }
}

/// Translate a key press into an action, given the current state.
pub fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    // Windows reports both press and release; only act on press.
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    let context = app.context();

    // Text input swallows everything else, including global keys.
    if context.captures_input() {
        return match key.code {
            KeyCode::Enter => Some(Action::ConfirmEdit),
            KeyCode::Esc => Some(Action::Back),
            // Settings: save the field and move; filter: move the list selection.
            KeyCode::Up => Some(Action::SelectPrev),
            KeyCode::Down => Some(Action::SelectNext),
            KeyCode::PageUp if context == Context::JobsFilter => Some(Action::SelectPageUp),
            KeyCode::PageDown if context == Context::JobsFilter => Some(Action::SelectPageDown),
            KeyCode::Left => Some(Action::CursorLeft),
            KeyCode::Right => Some(Action::CursorRight),
            KeyCode::Home => Some(Action::CursorHome),
            KeyCode::End => Some(Action::CursorEnd),
            KeyCode::Backspace => Some(Action::DeleteChar),
            KeyCode::Delete => Some(Action::DeleteForward),
            KeyCode::Char('a') if ctrl => Some(Action::CursorHome),
            KeyCode::Char('e') if ctrl => Some(Action::CursorEnd),
            KeyCode::Char('u') if ctrl => Some(Action::ClearInput),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                Some(Action::Input(c))
            }
            _ => None,
        };
    }

    match key.code {
        KeyCode::F(n) => return Tab::from_f_key(n).map(Action::SwitchTab),
        KeyCode::Char('q') => return Some(Action::Quit),
        KeyCode::Char('s') => return Some(Action::OpenSettings),
        KeyCode::Char('?') => return Some(Action::ToggleHelp),
        // Some terminals report Shift+r as 'r' with SHIFT instead of 'R'.
        KeyCode::Char('R') => return Some(Action::ToggleAutoRefresh),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            return Some(Action::ToggleAutoRefresh);
        }
        KeyCode::Char('r') => return Some(Action::Refresh),
        _ => {}
    }
    match (context, key.code) {
        // Esc only ever closes the current context, never the app.
        (_, KeyCode::Esc) if context.closable() => Some(Action::Back),
        (Context::Settings, KeyCode::Down | KeyCode::Char('j')) => Some(Action::SelectNext),
        (Context::Settings, KeyCode::Up | KeyCode::Char('k')) => Some(Action::SelectPrev),
        (Context::Settings, KeyCode::Enter) => Some(Action::StartEdit),
        (Context::Jobs | Context::JobsFiltered, code) => match code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
            KeyCode::PageDown => Some(Action::SelectPageDown),
            KeyCode::PageUp => Some(Action::SelectPageUp),
            KeyCode::Home | KeyCode::Char('g') => Some(Action::SelectFirst),
            KeyCode::End | KeyCode::Char('G') => Some(Action::SelectLast),
            KeyCode::Char('/') => Some(Action::StartFilter),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;

    const ALL_CONTEXTS: [Context; 6] = [
        Context::Jobs,
        Context::JobsFiltered,
        Context::JobsFilter,
        Context::Settings,
        Context::EditSetting,
        Context::Help,
    ];

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn app_in(context: Context) -> App {
        let mut app = App::default();
        match context {
            Context::Jobs => {}
            Context::JobsFiltered => app.jobs.filter = "api".into(),
            Context::JobsFilter => app.jobs.filter_input = Some(Default::default()),
            Context::Settings => app.view = View::Settings,
            Context::EditSetting => {
                app.view = View::Settings;
                app.settings.editing = Some(Default::default());
            }
            Context::Help => app.show_help = true,
        }
        assert_eq!(app.context(), context);
        app
    }

    /// Parse a displayed key label back into the key events it stands for.
    fn keys_of(label: &str) -> Vec<KeyEvent> {
        if label == "/" {
            return vec![key(KeyCode::Char('/'))];
        }
        label
            .split('/')
            .map(|part| match part {
                "Esc" => key(KeyCode::Esc),
                "Enter" => key(KeyCode::Enter),
                "↑" => key(KeyCode::Up),
                "↓" => key(KeyCode::Down),
                "←" => key(KeyCode::Left),
                "→" => key(KeyCode::Right),
                s if s.starts_with("C-") && s.chars().count() == 3 => {
                    ctrl(s.chars().nth(2).unwrap())
                }
                s if s.chars().count() == 1 => key(KeyCode::Char(s.chars().next().unwrap())),
                other => panic!("unknown key label {other:?}; extend keys_of"),
            })
            .collect()
    }

    #[test]
    fn f_keys_switch_tabs() {
        for context in [Context::Jobs, Context::Settings, Context::Help] {
            let app = app_in(context);
            assert_eq!(
                map_key(&app, key(KeyCode::F(1))),
                Some(Action::SwitchTab(Tab::Jobs)),
                "{context:?}"
            );
            assert_eq!(map_key(&app, key(KeyCode::F(12))), None, "no such tab");
        }
    }

    #[test]
    fn refresh_keys_are_global() {
        for context in [
            Context::Jobs,
            Context::JobsFiltered,
            Context::Settings,
            Context::Help,
        ] {
            let app = app_in(context);
            assert_eq!(
                map_key(&app, key(KeyCode::Char('r'))),
                Some(Action::Refresh),
                "{context:?}"
            );
            assert_eq!(
                map_key(&app, key(KeyCode::Char('R'))),
                Some(Action::ToggleAutoRefresh),
                "{context:?}"
            );
        }
        // ...but typing in a field still types.
        for context in [Context::JobsFilter, Context::EditSetting] {
            let app = app_in(context);
            assert_eq!(
                map_key(&app, key(KeyCode::Char('r'))),
                Some(Action::Input('r'))
            );
            assert_eq!(
                map_key(&app, key(KeyCode::Char('R'))),
                Some(Action::Input('R'))
            );
        }
    }

    #[test]
    fn jobs_keys() {
        let app = app_in(Context::Jobs);
        for (code, action) in [
            (KeyCode::Char('/'), Action::StartFilter),
            (KeyCode::Char('r'), Action::Refresh),
            (KeyCode::Char('R'), Action::ToggleAutoRefresh),
            (KeyCode::Char('j'), Action::SelectNext),
            (KeyCode::PageDown, Action::SelectPageDown),
            (KeyCode::Char('G'), Action::SelectLast),
            (KeyCode::Home, Action::SelectFirst),
        ] {
            assert_eq!(map_key(&app, key(code)), Some(action), "{code:?}");
        }
        assert_eq!(
            map_key(&app, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::SHIFT)),
            Some(Action::ToggleAutoRefresh)
        );
        assert_eq!(map_key(&app, key(KeyCode::Esc)), None, "nothing to clear");
        let filtered = app_in(Context::JobsFiltered);
        assert_eq!(map_key(&filtered, key(KeyCode::Esc)), Some(Action::Back));
    }

    #[test]
    fn filter_input_takes_letters() {
        let app = app_in(Context::JobsFilter);
        for c in ['q', 's', 'r', 'j', '/'] {
            assert_eq!(map_key(&app, key(KeyCode::Char(c))), Some(Action::Input(c)));
        }
        assert_eq!(map_key(&app, key(KeyCode::Down)), Some(Action::SelectNext));
        assert_eq!(
            map_key(&app, key(KeyCode::PageDown)),
            Some(Action::SelectPageDown)
        );
        assert_eq!(
            map_key(&app, key(KeyCode::F(1))),
            None,
            "F-keys off while typing"
        );
    }

    #[test]
    fn editing_keys() {
        let app = app_in(Context::EditSetting);
        for (code, action) in [
            (KeyCode::Left, Action::CursorLeft),
            (KeyCode::Right, Action::CursorRight),
            (KeyCode::Home, Action::CursorHome),
            (KeyCode::End, Action::CursorEnd),
            (KeyCode::Delete, Action::DeleteForward),
            (KeyCode::Up, Action::SelectPrev),
            (KeyCode::Down, Action::SelectNext),
        ] {
            assert_eq!(map_key(&app, key(code)), Some(action), "{code:?}");
        }
        assert_eq!(map_key(&app, ctrl('a')), Some(Action::CursorHome));
        assert_eq!(map_key(&app, ctrl('e')), Some(Action::CursorEnd));
    }

    #[test]
    fn quit_keys() {
        let app = App::default();
        assert_eq!(map_key(&app, key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(map_key(&app, ctrl('c')), Some(Action::Quit));
    }

    #[test]
    fn esc_closes_help() {
        let app = app_in(Context::Help);
        assert_eq!(map_key(&app, key(KeyCode::Esc)), Some(Action::Back));
    }

    #[test]
    fn esc_never_quits() {
        for context in ALL_CONTEXTS {
            let app = app_in(context);
            assert_ne!(map_key(&app, key(KeyCode::Esc)), Some(Action::Quit));
        }
        assert_eq!(map_key(&app_in(Context::Jobs), key(KeyCode::Esc)), None);
    }

    #[test]
    fn help_key() {
        let app = App::default();
        assert_eq!(
            map_key(&app, key(KeyCode::Char('?'))),
            Some(Action::ToggleHelp)
        );
    }

    #[test]
    fn unmapped_key() {
        let app = App::default();
        assert_eq!(map_key(&app, key(KeyCode::Char('x'))), None);
    }

    #[test]
    fn typing_does_not_trigger_global_keys() {
        let app = app_in(Context::EditSetting);
        for c in ['q', 's', '?', 'j', 'k'] {
            assert_eq!(map_key(&app, key(KeyCode::Char(c))), Some(Action::Input(c)));
        }
        assert_eq!(map_key(&app, ctrl('c')), Some(Action::Quit));
    }

    #[test]
    fn advertised_bindings_are_mapped() {
        for context in ALL_CONTEXTS {
            let app = app_in(context);
            let globals = if context.captures_input() {
                &[][..]
            } else {
                GLOBAL_BINDINGS
            };
            for b in globals.iter().chain(context_bindings(context)) {
                for event in keys_of(b.key) {
                    assert!(
                        map_key(&app, event).is_some(),
                        "{context:?}: {:?} is shown but does nothing",
                        b.key
                    );
                }
            }
        }
    }
}
