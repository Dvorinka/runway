//! runway-core: shared domain logic.
//!
//! Owns everything the API, worker, and CLI need: settings, the database
//! pool, Docker access, Traefik dynamic config, git provider clients, and
//! the Cloudflare DNS/Tunnel client.

pub mod access;
pub mod audit;
pub mod avatars;
pub mod cloudflare;
pub mod config;
pub mod cron;
pub mod crypto;
pub mod db;
pub mod deploy;
pub mod docker;
pub mod error;
pub mod events;
pub mod git_providers;
pub mod github;
pub mod logs;
pub mod mail;
pub mod models;
pub mod node_tls;
pub mod password;
pub mod pathmatch;
pub mod presets;
pub mod rum;
pub mod slugify;
pub mod traefik;
pub mod tunnel;
pub mod webhook;

pub use config::Settings;
pub use error::{Error, Result};
