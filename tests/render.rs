//! Snapshot tests: render the UI into an in-memory buffer and compare to
//! `tests/snapshots/*.snap`. Review changes with `cargo insta review`.

use leeroy::{
    app::{Action, App},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

/// Build an app, apply `actions` in order, and render one frame.
fn render_after(actions: &[Action]) -> Terminal<TestBackend> {
    let mut app = App::new();
    for action in actions {
        app.update(action.clone());
    }
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|frame| ui::render(frame, &app)).unwrap();
    terminal
}

#[test]
fn initial_screen() {
    insta::assert_snapshot!(render_after(&[]).backend());
}

#[test]
fn help_open() {
    insta::assert_snapshot!(render_after(&[Action::ToggleHelp]).backend());
}

#[test]
fn help_closed_again() {
    // Must match the initial screen exactly: the popup leaves no residue.
    let closed = render_after(&[Action::ToggleHelp, Action::ToggleHelp]);
    let initial = render_after(&[]);
    assert_eq!(closed.backend().buffer(), initial.backend().buffer());
}
