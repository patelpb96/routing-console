//! Routing console: an operator's view of tasks being matched to contributors.
//! `main.rs` only reads the environment and serves `http::router`; tests drive `app::App`.

pub mod app;
pub mod clock;
pub mod domain;
pub mod engine;
pub mod error;
pub mod http;
pub mod seed;
pub mod snapshot;
pub mod store;
