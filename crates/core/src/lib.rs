//! runway-core: shared domain logic.
//!
//! Owns everything the API, worker, and CLI need: settings, the database
//! pool, Docker access, Traefik dynamic config, git provider clients, and
//! the Cloudflare DNS/Tunnel client.

pub mod cloudflare;
pub mod config;
pub mod db;
pub mod docker;
pub mod error;
pub mod models;
pub mod traefik;

pub use config::Settings;
pub use error::{Error, Result};
