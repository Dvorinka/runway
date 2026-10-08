<p align="center">
  <img src="web/public/runway-logo.svg" width="420" alt="Runway">
</p>

<h1 align="center">Runway</h1>

<p align="center">
  Self-hosted deployment platform — the Vercel experience without the meter.
  Git-push deploys, preview URLs, and real HTTPS on your own hardware.
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#features">Features</a> •
  <a href="#cli">CLI</a> •
  <a href="ARCHITECTURE.md">Architecture</a> •
  <a href="ROADMAP.md">Roadmap</a>
</p>

<p align="center">
  <a href="https://github.com/Dvorinka/runway/actions/workflows/ci.yml"><img src="https://github.com/Dvorinka/runway/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

Runway is an open-source control plane for deploying web apps to Docker.
Connect a repo, push, and get a deployed URL — with immutable, per-branch,
and per-environment aliases, instant rollback, and live build logs. One
Rust binary runs the API, the deploy workers, and the end-user CLI;
Postgres is the only datastore and doubles as the job queue.

It works where other platforms don't: an instance-level Cloudflare Tunnel
covers the dashboard and the wildcard deploy domain, so a box behind CGNAT
with no public IP is a first-class install. Direct-IP + ACME remains the
standard path.

> **Status:** active development, informed by
> [devpush](https://github.com/hunvreus/devpush) (schema, lifecycle, and
> integrations ported as spec). Phases 0–5 shipped — see
> [ROADMAP.md](ROADMAP.md).

## Features

- **Git-push deploys** — GitHub App, Gitea, GitLab, Bitbucket, GitHub Enterprise; webhooks, manual, API, CLI, and cron triggers
- **Preview deployments** — immutable `-id-<sha>`, `-branch-<name>`, and `-env-<slug>` URLs per deployment; per-PR commit status
- **Frontend-first** — static deploy mode (build → `output_directory` → static serve), framework detection for Next.js, Astro, SvelteKit, Nuxt, Remix, Vite/SPA, Hugo; Node 20/22, npm/pnpm/yarn/bun detection, build-cache volumes
- **Works behind CGNAT** — instance Cloudflare Tunnel covers the dashboard + `*.DEPLOY_DOMAIN`; per-team tunnels for custom domains; one-click DNS assign via the Cloudflare API
- **Managed storage** — Postgres, MongoDB, Redis, SQLite, and volumes per team; linked into deployments with environment filtering and private networks
- **Teams & access** — teams with owner/admin/member roles, email invites, audit log, notifications, email + password auth (Argon2id), optional OIDC/SSO, sign-up allowlist
- **CLI + API + MCP** — `runway` CLI over the same REST API, `ak_` API keys, OpenAPI at `/api/v1/openapi.json`, JSON-RPC MCP server for coding agents
- **Ops** — rollback, redeploy, cancel, live build + runtime logs (SSE), outbound webhooks with HMAC signatures, cron jobs, redirect rules, project export/import
- **Scale-out** — remote Docker nodes with a real return path (published host port → Traefik file provider), health probes, per-project node assignment
- **No artificial limits** — your hardware, your quotas

## Ingress modes

| Mode | Use when | What happens |
| --- | --- | --- |
| **Cloudflare Tunnel** | No public IP / CGNAT / home server | `cloudflared` sidecars dial out; Cloudflare edge terminates TLS → Traefik |
| **Direct IP + ACME** | VPS with ports 80/443 | Traefik gets Let's Encrypt certs automatically |

Both are orthogonal to custom domains — per-team tunnels or direct DNS,
your choice per domain.

## Quick Start

Requires Docker with the Compose plugin, `openssl`, `curl`, and `tar`:

```bash
curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | bash
```

Installs into `./runway`, generates secrets into `.env` (never committed),
builds the image, and starts the stack — dashboard at
`http://runway.localhost` on the published port. Non-interactive; re-runs
never overwrite `.env`.

Then create the first user — sign-in password plus a CLI API key:

```bash
docker compose --project-directory ./runway -f ./runway/compose/production.yml \
  exec runway runway bootstrap --email you@example.com --password 'change-me'
```

Alternatively register through the dashboard — the first account created
becomes the instance admin. Sign-in is email + password; no mail server
or OAuth app is required. To deploy from GitHub, open **Settings →
GitHub App → Register with GitHub** — one click creates a private app
with the permissions Runway needs and stores its credentials encrypted
in Postgres (no restart).

Override install defaults with env vars:

```bash
curl -fsSL https://raw.githubusercontent.com/Dvorinka/runway/main/install.sh | \
  RUNWAY_DIR=/opt/runway HTTP_PORT=8080 \
  APP_HOSTNAME=apps.example.com DEPLOY_DOMAIN=deploy.example.com bash
```

`RUNWAY_VERSION` pins a release tag, `RUNWAY_REF` selects any git ref.
From a git checkout, `./install.sh` installs in place.

To go fully online without a public IP, set `CF_API_TOKEN` +
`CF_ACCOUNT_ID` in `.env` and restart — the platform provisions the
instance tunnel, DNS records, and ingress rules covering `APP_HOSTNAME`
and `*.DEPLOY_DOMAIN`. For direct-IP hosting set `DISABLE_TLS=false` and
`ACME_EMAIL`, point DNS at the host, done.

## CLI

The same binary is the user-facing CLI — thin client over the REST API,
config in `~/.config/runway`, project link in `.runway/project.json`:

```bash
runway login                 # browser or API-key auth
runway link                  # link cwd to a project
runway deploy --follow       # upload source, stream build logs
runway logs --follow         # runtime logs
runway env set KEY=value
runway domains add app.example.com
runway domains assign-cf app.example.com   # one-click CF DNS via team tunnel
runway open                  # current deployment URL
```

Agents get the same surface: `ak_` API keys against `/api/v1`, or the MCP
server at `POST /api/mcp` (`initialize` / `tools/list` / `tools/call`).

## Development

```bash
# Rust workspace — API, workers, CLI
cargo build
docker compose -f compose/development.yml up -d --build

# Dashboard (web/)
cd web && pnpm install && pnpm dev

# Verify before pushing
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test
```

## Architecture

```
crates/
  core/    runway-core    — settings, db, docker, traefik, cloudflare, models, detection
  api/     runway-api     — axum router: REST, webhooks, SSE, MCP, SPA host
  worker/  runway-worker  — deploy pipeline, monitor, cron, reconcile
  cli/     runway         — single binary: serve + user CLI
migrations/               — sqlx migrations (Postgres is also the job queue)
web/                      — React + Vite + Tailwind dashboard
compose/                  — development and production stacks
```

Single process, no Redis: deployments are Postgres rows claimed with
`SELECT ... FOR UPDATE SKIP LOCKED`. Runner containers get per-deployment
edge networks; Traefik file-provider configs are written atomically; logs
are file-tailed, not Loki. See [ARCHITECTURE.md](ARCHITECTURE.md) for the
full model.

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — process model, deploy lifecycle, networking
- [ROADMAP.md](ROADMAP.md) — shipped phases and what remains
- `GET /api/v1/openapi.json` — live API contract
- `.env.example` — every configuration knob

## License

[MIT](LICENSE) © 2026 Tomas Dvorak
