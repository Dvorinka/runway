# Contributing

Runway is community-driven — self-hosting stays a first-class use case.
Bugs, features, docs, and preset additions are all welcome.

## Setup

```bash
git clone https://github.com/Dvorinka/runway.git && cd runway
cp .env.example .env          # set DEPLOY_DOMAIN, DATABASE_URL, etc.

# Rust workspace — API, workers, CLI ship in one binary
cargo build

# Local stack (Postgres, Traefik)
docker compose -f compose/development.yml up -d --build

# Dashboard (React + Vite)
cd web && pnpm install && pnpm dev
```

## Workflow

1. Fork, branch from `main` (`feature/name` or `issue/123-name`).
2. Make the change. Small, focused PRs merge fastest.
3. Open the PR against `main`.

## Before you push

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd web && pnpm build        # if you touched web/
```

- **Migrations** live in `migrations/` — numbered, idempotent, additive.
  Never edit an applied migration; add a new one.
- **Commits** are conventional: `feat(api): …`, `fix(worker): …`,
  `docs: …`, `refactor:`, `test:`, `chore:`.
- **Secrets** never get committed. `.env` is gitignored; add knobs to
  `.env.example` instead.

## Conventions

- Single binary — workers are tokio tasks, not separate processes.
- Postgres is the job queue (`FOR UPDATE SKIP LOCKED`). No Redis.
- Established crates over new dependencies; check the workspace
  `Cargo.toml` before adding anything.
- Errors: `thiserror` in `runway-core`, `anyhow` at edges.
- Traefik configs are written atomically (temp + rename).
- The Python project at `hunvreus/devpush` is the reference spec for
  lifecycle semantics — port behavior, don't redesign it.

See [AGENTS.md](AGENTS.md) for the full conventions doc (written for AI
agents, useful for humans too) and [ARCHITECTURE.md](ARCHITECTURE.md) for
the system model.

## Reporting bugs

File an [issue](https://github.com/Dvorinka/runway/issues/new/choose)
with logs, reproduction steps, and your install method. For security
issues, use a [private advisory](SECURITY.md) instead.
