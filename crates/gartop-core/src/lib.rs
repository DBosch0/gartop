//! Shared building blocks for the `gartop` viewer and the `gartop-fetch` fetcher:
//! configuration and paths, domain models, response parsers and SQLite storage.

pub mod config;
pub mod model;
pub mod parse;
pub mod store;

pub use config::{Config, Paths};
pub use store::Store;
