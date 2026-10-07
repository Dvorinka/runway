//! runway — single binary for the platform.
//!
//! `runway serve` runs the full instance (API + embedded workers).
//! The same binary is the user-facing CLI (`deploy`, `logs`, `env`, `domains`, ...).

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "runway", version, about = "Self-hosted deployment platform")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Runway instance (API + workers + migrations).
    Serve,
    /// Apply database migrations only.
    Migrate,
    /// Deploy the current directory (or the linked repo).
    Deploy,
    /// Stream logs for a deployment.
    Logs {
        /// Deployment ID (defaults to latest of the linked project).
        deployment: Option<String>,
    },
    /// Authenticate this machine against an instance.
    Login,
    /// Link the current directory to a project.
    Link,
    /// Manage environment variables.
    Env,
    /// Manage domains.
    Domains,
    /// Open the project in the browser.
    Open,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve | Command::Migrate => {
            let settings = runway_core::Settings::from_env()?;
            let db = runway_core::db::connect(&settings).await?;
            runway_core::db::migrate(&db).await?;

            if matches!(cli.command, Command::Migrate) {
                tracing::info!("migrations applied");
                return Ok(());
            }

            // Workers run embedded in the same process.
            let worker_db = db.clone();
            let worker_settings = settings.clone();
            tokio::spawn(async move {
                if let Err(e) = runway_worker::run(worker_db, worker_settings).await {
                    tracing::error!(error = %e, "worker loop died");
                }
            });

            let state = runway_api::AppState {
                db,
                settings: settings.clone(),
            };
            let app = runway_api::router(state);
            let listener = tokio::net::TcpListener::bind(&settings.listen_addr).await?;
            tracing::info!(addr = %settings.listen_addr, "runway serving");
            axum::serve(listener, app).await?;
        }
        Command::Deploy => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Logs { .. } => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Login => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Link => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Env => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Domains => anyhow::bail!("not implemented yet (Phase 4)"),
        Command::Open => anyhow::bail!("not implemented yet (Phase 4)"),
    }

    Ok(())
}
