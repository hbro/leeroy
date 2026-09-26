/// Everything that can change application state.
///
/// Key presses, timer ticks and (later) async Jenkins responses are all
/// translated into actions, so every state transition goes through
/// [`App::update`] and can be unit tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    Tick,
}

#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub show_help: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            running: true,
            show_help: false,
        }
    }

    /// Apply an action. Must stay free of IO so tests can drive it directly.
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Tick => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_stops_running() {
        let mut app = App::new();
        app.update(Action::Quit);
        assert!(!app.running);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = App::new();
        app.update(Action::ToggleHelp);
        assert!(app.show_help);
        app.update(Action::ToggleHelp);
        assert!(!app.show_help);
    }
}
