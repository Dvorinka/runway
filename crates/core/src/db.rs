//! Postgres pool + migration helpers.

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::config::Settings;
use crate::error::Result;

pub type Db = PgPool;

/// Connect to Postgres with a bounded pool.
pub async fn connect(settings: &Settings) -> Result<Db> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&settings.database_url)
        .await?;
    Ok(pool)
}

/// Apply pending migrations (sqlx migrate! at compile time).
pub async fn migrate(pool: &Db) -> Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
