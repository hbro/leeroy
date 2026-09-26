//! Leeroy — a terminal UI for Jenkins.
//!
//! The core is pure and terminal-free so it can be tested headlessly:
//! - [`app`]: state + `update(Action)`
//! - [`event`]: terminal input -> [`app::Action`]
//! - [`ui`]: `render(frame, &app)`

pub mod app;
pub mod event;
pub mod ui;
