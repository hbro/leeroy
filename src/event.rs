use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{Action, App};

/// Translate a key press into an action, given the current state.
pub fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    // Windows reports both press and release; only act on press.
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Char('q') => Some(Action::Quit),
        // Esc closes the help popup first, quits otherwise.
        KeyCode::Esc if app.show_help => Some(Action::ToggleHelp),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn quit_keys() {
        let app = App::new();
        assert_eq!(map_key(&app, key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(map_key(&app, key(KeyCode::Esc)), Some(Action::Quit));
        assert_eq!(
            map_key(
                &app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(Action::Quit)
        );
    }

    #[test]
    fn esc_closes_help_before_quitting() {
        let mut app = App::new();
        app.show_help = true;
        assert_eq!(map_key(&app, key(KeyCode::Esc)), Some(Action::ToggleHelp));
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
}
