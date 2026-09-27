use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    app::{Action, App, Context, Tab},
    builds::BuildStep,
};

/// A keybinding as shown to the user. Display-only: keep in sync with
/// [`map_key`] (the tests below check the advertised keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub key: &'static str,
    pub desc: &'static str,
    /// Shown in the context bar; navigation keys everyone tries anyway
    /// (arrows, Home/End, PgUp/PgDn) are only listed in the help popup.
    pub in_bar: bool,
}

const fn bind(key: &'static str, desc: &'static str) -> Binding {
    Binding {
        key,
        desc,
        in_bar: true,
    }
}

/// A navigation key: in the help popup, not in the context bar.
const fn nav(key: &'static str, desc: &'static str) -> Binding {
    Binding {
        key,
        desc,
        in_bar: false,
    }
}

/// Bindings that work everywhere (bottom bar), except while typing text.
/// Global keys, listed in the help popup (the header only hints at `h/?`).
pub const GLOBAL_BINDINGS: &[Binding] = &[
    bind("q", "quit"),
    bind("h/?", "help"),
    bind("r", "refresh"),
    bind("R", "toggle auto-refresh"),
];

/// Bindings that only apply in the given context (context bar).
pub fn context_bindings(context: Context) -> &'static [Binding] {
    const JOBS: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "last build"),
        bind("/", "filter"),
    ];
    const JOBS_FILTERED: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "last build"),
        bind("/", "filter"),
        bind("Esc", "clear filter"),
    ];
    // Ordered by importance: at 80 columns the last one may be cut off.
    const BUILD: &[Binding] = &[
        bind("←/→", "older/newer"),
        nav("Home/End", "first/last"),
        bind("c", "console"),
        bind("Esc", "back"),
        nav("↑/↓", "scroll"),
    ];
    const CONSOLE: &[Binding] = &[
        nav("↑/↓", "line"),
        nav("PgUp/PgDn", "page"),
        nav("Home/End", "top/bottom"),
        bind("c/Esc", "back"),
        bind("←/→", "sideways"),
    ];
    const BUILDS: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "open build"),
        bind("/", "filter"),
    ];
    const BUILDS_FILTERED: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "open build"),
        bind("/", "filter"),
        bind("Esc", "clear filter"),
    ];
    const PIPELINES: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "latest run"),
        bind("/", "filter"),
    ];
    const PIPELINES_FILTERED: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "latest run"),
        bind("/", "filter"),
        bind("Esc", "clear filter"),
    ];
    const RUNS: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "open run"),
        bind("/", "filter"),
    ];
    const RUNS_FILTERED: &[Binding] = &[
        nav("↑/↓", "select"),
        bind("Enter", "open run"),
        bind("/", "filter"),
        bind("Esc", "clear filter"),
    ];
    // Ordered by importance: at 80 columns the last one may be cut off.
    const RUN: &[Binding] = &[
        bind("←/→", "older/newer"),
        bind("v", "tree/boxes"),
        bind("Enter", "open build"),
        bind("Esc", "back"),
        nav("↑/↓", "select"),
        nav("Home/End", "first/last"),
    ];
    const JOBS_FILTER: &[Binding] = &[
        bind("Enter", "apply"),
        bind("Esc", "cancel"),
        nav("↑/↓", "select"),
        bind("C-u", "clear"),
    ];
    const SETTINGS: &[Binding] = &[nav("↑/↓", "select"), bind("Enter", "edit/toggle")];
    const EDIT: &[Binding] = &[
        bind("Enter", "save"),
        bind("Esc", "cancel"),
        nav("↑/↓", "save & move"),
        bind("←/→", "cursor"),
        bind("C-u", "clear"),
    ];
    const HELP: &[Binding] = &[bind("Esc", "close")];
    const CONFIRM_QUIT: &[Binding] = &[bind("y/Enter", "quit"), bind("n/Esc", "stay")];
    match context {
        Context::ConfirmQuit => CONFIRM_QUIT,
        Context::Jobs => JOBS,
        Context::JobsFiltered => JOBS_FILTERED,
        Context::JobsFilter
        | Context::BuildsFilter
        | Context::PipelinesFilter
        | Context::RunsFilter => JOBS_FILTER,
        Context::Builds => BUILDS,
        Context::BuildsFiltered => BUILDS_FILTERED,
        Context::Runs => RUNS,
        Context::RunsFiltered => RUNS_FILTERED,
        Context::Run => RUN,
        Context::Pipelines => PIPELINES,
        Context::PipelinesFiltered => PIPELINES_FILTERED,
        Context::Build => BUILD,
        Context::Console => CONSOLE,
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

    // The quit prompt takes every key: only its own answers do something.
    if context == Context::ConfirmQuit {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(Action::Quit),
            KeyCode::Char('q') => Some(Action::RequestQuit),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(Action::Back),
            _ => None,
        };
    }

    // Text input swallows everything else, including global keys.
    if context.captures_input() {
        return match key.code {
            KeyCode::Enter => Some(Action::ConfirmEdit),
            KeyCode::Esc => Some(Action::Back),
            // Settings: save the field and move; filter: move the list selection.
            KeyCode::Up => Some(Action::SelectPrev),
            KeyCode::Down => Some(Action::SelectNext),
            KeyCode::PageUp if context != Context::EditSetting => Some(Action::SelectPageUp),
            KeyCode::PageDown if context != Context::EditSetting => Some(Action::SelectPageDown),
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
        KeyCode::Char(c @ '0'..='9') => return Tab::from_key(c).map(Action::SwitchTab),
        KeyCode::Char('q') => return Some(Action::RequestQuit),
        KeyCode::Char('?') | KeyCode::Char('h') => return Some(Action::ToggleHelp),
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
        (Context::Run, code) => match code {
            KeyCode::Left => Some(Action::BuildStep(BuildStep::Older)),
            KeyCode::Right => Some(Action::BuildStep(BuildStep::Newer)),
            KeyCode::Home => Some(Action::BuildStep(BuildStep::First)),
            KeyCode::End => Some(Action::BuildStep(BuildStep::Last)),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
            KeyCode::PageDown => Some(Action::SelectPageDown),
            KeyCode::PageUp => Some(Action::SelectPageUp),
            KeyCode::Char('g') => Some(Action::SelectFirst),
            KeyCode::Char('G') => Some(Action::SelectLast),
            KeyCode::Char('v') => Some(Action::ToggleRunView),
            KeyCode::Enter => Some(Action::OpenBuild),
            _ => None,
        },
        (
            Context::Jobs
            | Context::JobsFiltered
            | Context::Builds
            | Context::BuildsFiltered
            | Context::Pipelines
            | Context::PipelinesFiltered
            | Context::Runs
            | Context::RunsFiltered,
            code,
        ) => match code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
            KeyCode::PageDown => Some(Action::SelectPageDown),
            KeyCode::PageUp => Some(Action::SelectPageUp),
            KeyCode::Home | KeyCode::Char('g') => Some(Action::SelectFirst),
            KeyCode::End | KeyCode::Char('G') => Some(Action::SelectLast),
            KeyCode::Char('/') => Some(Action::StartFilter),
            KeyCode::Enter => Some(Action::OpenBuild),
            _ => None,
        },
        (Context::Build, code) => match code {
            KeyCode::Left => Some(Action::BuildStep(BuildStep::Older)),
            KeyCode::Right => Some(Action::BuildStep(BuildStep::Newer)),
            KeyCode::Home => Some(Action::BuildStep(BuildStep::First)),
            KeyCode::End => Some(Action::BuildStep(BuildStep::Last)),
            // Scrolling the details (Home/End go to the first/last build).
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
            KeyCode::PageDown => Some(Action::SelectPageDown),
            KeyCode::PageUp => Some(Action::SelectPageUp),
            KeyCode::Char('g') => Some(Action::SelectFirst),
            KeyCode::Char('G') => Some(Action::SelectLast),
            KeyCode::Char('c') => Some(Action::OpenConsole),
            _ => None,
        },
        (Context::Console, code) => match code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectPrev),
            KeyCode::PageDown => Some(Action::SelectPageDown),
            KeyCode::PageUp => Some(Action::SelectPageUp),
            KeyCode::Home | KeyCode::Char('g') => Some(Action::SelectFirst),
            KeyCode::End | KeyCode::Char('G') => Some(Action::SelectLast),
            KeyCode::Left => Some(Action::ScrollLeft),
            KeyCode::Right => Some(Action::ScrollRight),
            // c toggles: it opened the console from the build view.
            KeyCode::Char('c') => Some(Action::Back),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;

    const ALL_CONTEXTS: [Context; 19] = [
        Context::ConfirmQuit,
        Context::Jobs,
        Context::JobsFiltered,
        Context::JobsFilter,
        Context::Builds,
        Context::BuildsFiltered,
        Context::BuildsFilter,
        Context::Pipelines,
        Context::PipelinesFiltered,
        Context::PipelinesFilter,
        Context::Runs,
        Context::RunsFiltered,
        Context::RunsFilter,
        Context::Run,
        Context::Build,
        Context::Console,
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
            Context::ConfirmQuit => app.confirm_quit = true,
            Context::JobsFiltered => app.jobs.filter = "api".into(),
            Context::JobsFilter => app.jobs.filter_input = Some(Default::default()),
            Context::Builds => app.view = View::Builds,
            Context::BuildsFiltered => {
                app.view = View::Builds;
                app.history.filter = "api".into();
            }
            Context::BuildsFilter => {
                app.view = View::Builds;
                app.history.filter_input = Some(Default::default());
            }
            Context::Build => {
                app.view = View::Build;
                app.build = Some(crate::builds::BuildView::new("job".into()));
            }
            Context::Console => {
                app.view = View::Console;
                app.console = Some(crate::console::ConsoleView::new("job".into(), 1));
            }
            Context::Pipelines => app.view = View::Pipelines,
            Context::PipelinesFiltered => {
                app.view = View::Pipelines;
                app.pipelines.list.filter = "api".into();
            }
            Context::PipelinesFilter => {
                app.view = View::Pipelines;
                app.pipelines.list.filter_input = Some(Default::default());
            }
            Context::Runs => app.view = View::Runs,
            Context::RunsFiltered => {
                app.view = View::Runs;
                app.pipelines.runs.filter = "api".into();
            }
            Context::RunsFilter => {
                app.view = View::Runs;
                app.pipelines.runs.filter_input = Some(Default::default());
            }
            Context::Run => {
                app.view = View::Run;
                app.run = Some(crate::pipelines::RunView::new(
                    "job".into(),
                    crate::pipelines::RunRef::Latest,
                    View::Pipelines,
                ));
            }
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
                "Home" => key(KeyCode::Home),
                "y" => key(KeyCode::Char('y')),
                "h" => key(KeyCode::Char('h')),
                "n" => key(KeyCode::Char('n')),
                "End" => key(KeyCode::End),
                "PgUp" => key(KeyCode::PageUp),
                "PgDn" => key(KeyCode::PageDown),
                s if s.starts_with("C-") && s.chars().count() == 3 => {
                    ctrl(s.chars().nth(2).unwrap())
                }
                s if s.chars().count() == 1 => key(KeyCode::Char(s.chars().next().unwrap())),
                other => panic!("unknown key label {other:?}; extend keys_of"),
            })
            .collect()
    }

    #[test]
    fn digits_switch_tabs() {
        for context in [Context::Jobs, Context::Settings, Context::Help] {
            let app = app_in(context);
            assert_eq!(
                map_key(&app, key(KeyCode::Char('1'))),
                Some(Action::SwitchTab(Tab::Jobs)),
                "{context:?}"
            );
            assert_eq!(
                map_key(&app, key(KeyCode::Char('0'))),
                Some(Action::SwitchTab(Tab::Settings)),
                "{context:?}"
            );
            assert_eq!(map_key(&app, key(KeyCode::Char('5'))), None, "no such tab");
            assert_eq!(map_key(&app, key(KeyCode::F(1))), None, "F-keys unused");
        }
        assert_eq!(
            map_key(&app_in(Context::Jobs), key(KeyCode::Char('h'))),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            map_key(&app_in(Context::Settings), key(KeyCode::Esc)),
            None,
            "Settings is a tab: Esc doesn't leave it"
        );
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
            (KeyCode::Enter, Action::OpenBuild),
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
    fn console_keys() {
        let app = app_in(Context::Console);
        for (code, action) in [
            (KeyCode::Down, Action::SelectNext),
            (KeyCode::Up, Action::SelectPrev),
            (KeyCode::PageDown, Action::SelectPageDown),
            (KeyCode::PageUp, Action::SelectPageUp),
            (KeyCode::Home, Action::SelectFirst),
            (KeyCode::End, Action::SelectLast),
            (KeyCode::Left, Action::ScrollLeft),
            (KeyCode::Right, Action::ScrollRight),
            (KeyCode::Esc, Action::Back),
            (KeyCode::Char('c'), Action::Back),
        ] {
            assert_eq!(map_key(&app, key(code)), Some(action), "{code:?}");
        }
        assert_eq!(
            map_key(&app_in(Context::Build), key(KeyCode::Char('c'))),
            Some(Action::OpenConsole)
        );
    }

    #[test]
    fn quit_prompt_keys() {
        let app = app_in(Context::ConfirmQuit);
        for code in [KeyCode::Char('y'), KeyCode::Enter] {
            assert_eq!(map_key(&app, key(code)), Some(Action::Quit), "{code:?}");
        }
        assert_eq!(
            map_key(&app, key(KeyCode::Char('q'))),
            Some(Action::RequestQuit)
        );
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            assert_eq!(map_key(&app, key(code)), Some(Action::Back), "{code:?}");
        }
        // Everything else is ignored while asking, global keys included.
        for code in [
            KeyCode::Char('0'),
            KeyCode::Char('r'),
            KeyCode::Char('1'),
            KeyCode::Down,
        ] {
            assert_eq!(map_key(&app, key(code)), None, "{code:?}");
        }
        assert_eq!(
            map_key(&app, ctrl('c')),
            Some(Action::Quit),
            "Ctrl-C still quits"
        );
    }

    #[test]
    fn build_view_keys() {
        let app = app_in(Context::Build);
        for (code, step) in [
            (KeyCode::Left, BuildStep::Older),
            (KeyCode::Right, BuildStep::Newer),
            (KeyCode::Home, BuildStep::First),
            (KeyCode::End, BuildStep::Last),
        ] {
            assert_eq!(
                map_key(&app, key(code)),
                Some(Action::BuildStep(step)),
                "{code:?}"
            );
        }
        assert_eq!(
            map_key(&app, key(KeyCode::Char('G'))),
            Some(Action::SelectLast),
            "scroll"
        );
        assert_eq!(map_key(&app, key(KeyCode::Esc)), Some(Action::Back));
        assert_eq!(
            map_key(&app, key(KeyCode::Char('j'))),
            Some(Action::SelectNext)
        );
        assert_eq!(
            map_key(&app, key(KeyCode::Char('r'))),
            Some(Action::Refresh)
        );
        assert_eq!(map_key(&app, key(KeyCode::Enter)), None);
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
            map_key(&app, key(KeyCode::Char('1'))),
            Some(Action::Input('1')),
            "digits are typed, not tab keys, while typing"
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
        assert_eq!(
            map_key(&app, key(KeyCode::Char('q'))),
            Some(Action::RequestQuit)
        );
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
    fn settings_only_via_their_tab_key() {
        let app = app_in(Context::Jobs);
        assert_eq!(
            map_key(&app, key(KeyCode::Char('s'))),
            None,
            "no s shortcut"
        );
        assert_eq!(
            map_key(&app, key(KeyCode::Char('0'))),
            Some(Action::SwitchTab(Tab::Settings))
        );
    }

    #[test]
    fn advertised_bindings_are_mapped() {
        for context in ALL_CONTEXTS {
            let app = app_in(context);
            let globals = if context.global_keys_off() {
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
