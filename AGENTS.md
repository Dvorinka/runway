# AI Agent Guidelines for Runway

Ground-up Rust rewrite of the devpush deployment platform. The Python
codebase at `~/Desktop/PROG+HTML/devpush` is the **reference
implementation and spec** — when porting behavior, read it; don't
redesign from memory.

## Principles

1. **Single binary.** API, workers, and CLI ship in `runway`. Workers are
   tokio tasks, not separate processes.
2. **Postgres is the queue.** `SELECT ... FOR UPDATE SKIP LOCKED`. Do not
   add Redis or another broker.
3. **Stdlib/established crates first.** No speculative abstractions, no
   framework-in-framework. Match dependency choices already in the
   workspace `Cargo.toml`.
4. **Port, don't redesign.** Schema, deployment lifecycle
   (`prepare → deploy → finalize → completed`), alias semantics, network
   topology, and edge cases mirror devpush unless a documented reason
   exists.
5. **Frontend-first product.** When two designs are equal, pick the one
   that serves static/SSR frontend deploys better.

## Conventions

- Rust 2021, `rustfmt` + `clippy` clean. `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` before pushing.
- Errors: `thiserror` in libs (`runway-core::Error`), `anyhow` at the edges (CLI, handlers).
- Async: tokio everywhere; no blocking calls in async paths (`tokio::fs`, `spawn_blocking` when needed).
- DB: sqlx with migrations in `migrations/` — numbered, idempotent, never edit applied migrations.
- Secrets: encrypted at rest (see devpush `get_fernet` pattern — design the Rust equivalent), never logged, never committed.
- Traefik configs: write atomically (temp + rename), as in `runway-core::traefik`.
- Types: prefer typed fields over loosely-typed `serde_json::Value` config bags, but project/deployment `config` stays a JSON map (parity with devpush semantics).

## Repo layout

- `crates/core` — settings, db, docker, traefik, cloudflare, models, detection
- `crates/api` — axum router: REST, webhooks, SSE, MCP, SPA host
- `crates/worker` — deploy pipeline, monitor, cron, reconcile
- `crates/cli` — the `runway` binary (serve + user CLI)
- `migrations/` — sqlx migrations
- `web/` — React dashboard

## Verification

- `cargo check && cargo clippy --workspace -- -D warnings && cargo test`
- `sqlx migrate run` against a local Postgres before touching migration code paths.
- Deploy-flow changes: verify against the devpush behavior, then a real container build.

## Hard rules

- No commits of secrets or `.env`.
- No new top-level dependencies without checking the workspace first.
- Don't rename existing types/functions without asking.
- Git: conventional commits (`feat(api): …`, `fix(worker): …`), PRs to `main`.
