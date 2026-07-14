//! Deterministic, model-free storage and serving primitives for prose.
#![forbid(unsafe_code)]

pub mod cli;
pub mod model;
pub mod store;

pub use model::*;
pub use store::{Error, Result, Store};
