//! Leeroy — a terminal UI for Jenkins.
//!
//! The core is pure and terminal-free so it can be tested headlessly:
//! - [`app`]: state + `update(Action) -> Vec<Effect>`
//! - [`config`]: config file location, load/save, env overrides
//! - [`event`]: terminal input -> [`app::Action`]
//! - [`jenkins`]: HTTP client + connection check (the only IO module besides `config`)
//! - [`proxy`]: which proxy applies (setting vs. system env vars)
//! - [`ui`]: `render(frame, &app)`

pub mod app;
pub mod auth;
pub mod builds;
pub mod config;
pub mod console;
pub mod credentials;
pub mod detect;
pub mod event;
pub mod graph;
pub mod history;
pub mod input;
pub mod instance;
pub mod jenkins;
pub mod jobs;
pub mod oidc;
pub mod pipelines;
pub mod promote;
pub mod proxy;
pub mod secrets;
pub mod theme;
pub mod ui;
