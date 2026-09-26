use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{Action, App, Context};

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

/// Bindings that work everywhere (bottom bar).
pub const GLOBAL_BINDINGS: &[Binding] = &[bind("q", "quit"), bind("?", "help")];

/// Bindings that only apply in the given context (context bar).
pub fn context_bindings(context: Context) -> &'static [Binding] {
    const JOBS: &[Binding] = &[];
    const HELP: &[Binding] = &[bind("Esc", "close")];
    match context {
        Context::Jobs => JOBS,
        Context::Help => HELP,
    }
}

/// Translate a key press into an action, given the current state.
pub fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    // Windows reports both press and release; only act on press.
    if key.kind != KeyEventKind::Press {
        return None;
    }
    // Global keys first, then keys of whatever has focus.
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Action::Quit);
        }
        KeyCode::Char('q') => return Some(Action::Quit),
        KeyCode::Char('?') => return Some(Action::ToggleHelp),
        _ => {}
    }
    let context = app.context();
    match (context, key.code) {
        // Esc only ever closes the current context, never the app.
        (_, KeyCode::Esc) if context.closable() => Some(Action::Back),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app_in(context: Context) -> App {
        let mut app = App::new();
        app.show_help = context == Context::Help;
        assert_eq!(app.context(), context);
        app
    }

    /// Parse a displayed key label back into a key code.
    fn code_of(label: &str) -> KeyCode {
        match label {
            "Esc" => KeyCode::Esc,
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
            other => panic!("unknown key label {other:?}; extend code_of"),
        }
    }

    #[test]
    fn quit_keys() {
        let app = App::new();
        assert_eq!(map_key(&app, key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(
            map_key(
                &app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(Action::Quit)
        );
    }

    #[test]
    fn esc_closes_help() {
        let app = app_in(Context::Help);
        assert_eq!(map_key(&app, key(KeyCode::Esc)), Some(Action::Back));
    }

    #[test]
    fn esc_never_quits() {
        let app = app_in(Context::Jobs);
        assert_eq!(map_key(&app, key(KeyCode::Esc)), None);
    }

    #[test]
    fn help_key() {
        let app = App::new();
        assert_eq!(
            map_key(&app, key(KeyCode::Char('?'))),
            Some(Action::ToggleHelp)
        );
    }

    #[test]
    fn unmapped_key() {
        let app = App::new();
        assert_eq!(map_key(&app, key(KeyCode::Char('x'))), None);
    }

    #[test]
    fn advertised_bindings_are_mapped() {
        for context in [Context::Jobs, Context::Help] {
            let app = app_in(context);
            for b in GLOBAL_BINDINGS.iter().chain(context_bindings(context)) {
                assert!(
                    map_key(&app, key(code_of(b.key))).is_some(),
                    "{context:?}: {:?} is shown but does nothing",
                    b.key
                );
            }
        }
    }
}
