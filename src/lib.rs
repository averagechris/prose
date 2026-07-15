//! Deterministic, model-free storage and serving primitives for prose.
#![forbid(unsafe_code)]

pub mod app;
pub mod cli;
pub mod mcp;
pub mod model;
pub mod server;
pub mod store;

pub use app::*;
pub use model::*;
pub use store::{Error, Result, Store};
