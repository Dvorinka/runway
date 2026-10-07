//! runway — single binary for the platform.
//!
//! `runway serve` runs the full instance (API + embedded workers).
//! The same binary is the user-facing CLI (`deploy`, `logs`, `env`, `domains`, ...).

use clap::{Parser, Subcommand};

mod client;
mod commands;

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
    /// Create the first user + team and mint an API key (local bootstrap).
    Bootstrap {
        /// Email for the admin user.
        #[arg(long)]
        email: String,
        /// Display name / username base.
        #[arg(long)]
        username: Option<String>,
    },
    /// Deploy the current directory (upload tarball to the linked project).
    Deploy {
        /// Poll until the deployment reaches a terminal status.
        #[arg(long)]
        follow: bool,
    },
    /// Stream logs for a deployment.
    Logs {
        /// Deployment ID (defaults to latest of the linked project).
        deployment: Option<String>,
        /// Keep tailing until the deployment concludes.
        #[arg(long)]
        follow: bool,
    },
    /// Authenticate this machine against an instance.
    Login {
        /// Instance URL (e.g. http://localhost:8000).
        #[arg(long)]
        server: Option<String>,
        /// API key (ak_...).
        #[arg(long)]
        key: Option<String>,
    },
    /// Link the current directory to a project.
    Link {
        /// Project ID (interactive list if omitted).
        project: Option<String>,
    },
    /// Manage environment variables: env [list|set KEY=VAL|unset KEY].
    Env {
        /// list | set KEY=VALUE | unset KEY
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Environment slug to scope the variable to.
        #[arg(long)]
        environment: Option<String>,
    },
    /// Manage domains: domains [list|add <host>|remove <host>|assign-cf <host>].
    Domains {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Open the project's deployment URL in the browser.
    Open {
        /// Print the URL instead of opening a browser.
        #[arg(long)]
        print: bool,
    },
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

            let crypto = runway_core::crypto::Crypto::new(&settings.encryption_key)?;
            let bus = runway_core::events::EventBus::new();

            // Workers run embedded in the same process.
            {
                let db = db.clone();
                let settings = settings.clone();
                let bus = bus.clone();
                let crypto = crypto.clone();
                tokio::spawn(async move {
                    if let Err(e) = runway_worker::run(db, settings, bus, crypto).await {
                        tracing::error!(error = %e, "worker init failed");
                    }
                });
            }

            let state = runway_api::AppState {
                db,
                settings: settings.clone(),
                bus: bus.clone(),
                crypto: crypto.clone(),
                github: runway_core::github::GithubService::from_settings(&settings),
                github_oauth: runway_core::github::GithubService::oauth_only(&settings),
                logs: runway_core::logs::LogStore::new(&settings.data_dir, bus),
                docker: runway_core::docker::connect(&settings).ok(),
            };
            let app = runway_api::router(state);
            let listener = tokio::net::TcpListener::bind(&settings.listen_addr).await?;
            tracing::info!(addr = %settings.listen_addr, "runway serving");
            axum::serve(listener, app).await?;
        }
        Command::Bootstrap { email, username } => {
            let settings = runway_core::Settings::from_env()?;
            let db = runway_core::db::connect(&settings).await?;
            runway_core::db::migrate(&db).await?;
            let crypto = runway_core::crypto::Crypto::new(&settings.encryption_key)?;
            bootstrap(&db, &crypto, &email, username.as_deref()).await?;
        }
        Command::Login { server, key } => commands::login(server, key).await?,
        Command::Link { project } => commands::link(project).await?,
        Command::Deploy { follow } => commands::deploy(follow).await?,
        Command::Logs { deployment, follow } => commands::logs(deployment, follow).await?,
        Command::Env { args, environment } => commands::env(args, environment).await?,
        Command::Domains { args } => commands::domains(args).await?,
        Command::Open { print } => commands::open(print).await?,
    }

    Ok(())
}

/// First-run setup: user + team + API key, printed once to stdout.
async fn bootstrap(
    db: &sqlx::PgPool,
    crypto: &runway_core::crypto::Crypto,
    email: &str,
    username: Option<&str>,
) -> anyhow::Result<()> {
    use runway_core::models::{ApiKey, Team};
    use runway_core::slugify::{slugify, token_hex};

    let username = username
        .map(|u| slugify(u, 50))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| {
            let local = email.split('@').next().unwrap_or("admin");
            let s = slugify(local, 50);
            if s.is_empty() {
                format!("user-{}", token_hex(4))
            } else {
                s
            }
        });

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"user\" (email, username, email_verified)
         VALUES ($1, $2, true)
         ON CONFLICT (email) DO UPDATE SET email = EXCLUDED.email
         RETURNING id",
    )
    .bind(email)
    .bind(&username)
    .fetch_one(db)
    .await?;

    let has_team: Option<(String,)> =
        sqlx::query_as("SELECT team_id FROM team_member WHERE user_id = $1 LIMIT 1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    if has_team.is_none() {
        let team = Team::new(&format!("{username}'s team"), user_id);
        sqlx::query("INSERT INTO team (id, name, created_by_user_id) VALUES ($1,$2,$3)")
            .bind(&team.id)
            .bind(&team.name)
            .bind(user_id)
            .execute(db)
            .await?;
        runway_core::models::Project::assign_team_slug(db, &team).await?;
        sqlx::query("INSERT INTO team_member (team_id, user_id, role) VALUES ($1,$2,'owner')")
            .bind(&team.id)
            .bind(user_id)
            .execute(db)
            .await?;
        sqlx::query("UPDATE \"user\" SET default_team_id = $1 WHERE id = $2")
            .bind(&team.id)
            .bind(user_id)
            .execute(db)
            .await?;
    }

    let (raw, hash) = ApiKey::generate();
    sqlx::query("INSERT INTO api_key (id, user_id, name, token) VALUES ($1,$2,'bootstrap',$3)")
        .bind(token_hex(16))
        .bind(user_id)
        .bind(&hash)
        .execute(db)
        .await?;

    let _ = crypto;
    println!("user_id:  {user_id}");
    println!("api_key:  {raw}");
    println!("(shown once — store it safely)");
    Ok(())
}
