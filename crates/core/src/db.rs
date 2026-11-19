//! Postgres pool + migration helpers.

use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::ConnectOptions;

use crate::config::Settings;
use crate::error::Result;

pub type Db = PgPool;

/// Connect to Postgres with a bounded pool.
pub async fn connect(settings: &Settings) -> Result<Db> {
    let options: PgConnectOptions = settings.database_url.parse()?;
    // Warn on statements over 1s — surfaced issue #89's observer latency.
    let options = options.log_slow_statements(log::LevelFilter::Warn, Duration::from_secs(1));
    let pool = PgPoolOptions::new()
        .max_connections(settings.db_max_connections)
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Apply pending migrations (sqlx migrate! at compile time).
pub async fn migrate(pool: &Db) -> Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
