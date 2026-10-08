<p align="center">
  <img src="docs/assets/readme-logo.svg" alt="Runway" width="120">
</p>

<h1 align="center">Runway</h1>

<p align="center">
  Open-source, self-hosted deployment platform.<br>
  The Vercel experience — git-push deploys, preview URLs, real HTTPS — on your own hardware.
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="ARCHITECTURE.md">Documentation</a> ·
  <a href="https://github.com/Dvorinka/runway/releases">Releases</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/Dvorinka/runway/actions/workflows/ci.yml"><img src="https://github.com/Dvorinka/runway/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Dvorinka/runway/releases"><img src="https://img.shields.io/github/v/release/Dvorinka/runway" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/Dvorinka/runway" alt="License"></a>
</p>

## What is Runway?

Runway is an open-source control plane for deploying web apps to Docker.
Connect a repo, push, and get a deployed URL — with immutable, per-branch,
and per-environment aliases, instant rollback, and live build logs. One
Rust binary runs the API, the deploy workers, and the end-user CLI;
Postgres is the only datastore and doubles as the job queue. No Redis, no
vendor lock-in, no meter.

It works where other platforms don't: an instance-level Cloudflare Tunnel
covers the dashboard and the wildcard deploy domain, so a box behind CGNAT
with no public IP is a first-class install. Direct-IP + ACME remains the
standard path.

> **Status:** active development, informed by
> [devpush](https://github.com/hunvreus/devpush) (schema, lifecycle, and
> integrations ported as spec). See [ROADMAP.md](ROADMAP.md).

## Features

**Deploy pipeline**

- **Git-push deploys** — GitHub App, Gitea, GitLab, Bitbucket, GitHub Enterprise; webhooks, manual, API, CLI, cron, and deploy-hook (`dp_` token) triggers
- **Preview deployments** — immutable `-id-<sha>`, `-branch-<name>`, and `-env-<slug>` URLs; preview environments for unmatched branches (opt-in); per-PR commit status and upserted preview comments; teardown on PR close
- **Frontend-first** — static deploy mode, 24 framework presets (Next.js, Astro, SvelteKit, Nuxt, Remix, Vite, Hugo, Angular, Qwik, Eleventy, Gatsby, Docusaurus, SolidStart…), Node 20/22 + npm/pnpm/yarn/bun detection, build-cache volumes, real `Dockerfile` builds, monorepo `root_directory`
- **Deploy rules** — branch patterns, path-scoped glob filters (`apps/web/**` skips doc-only pushes), commit-message skip flags, environment quotas
- **Instant rollback** — alias re-point to any retained deployment; per-environment retention policy keeps rollback targets

**Runtime & edge**

- **Health checks** — HTTP probes → `unhealthy` observed status → crash notification and auto-requeue
- **Deployment protection** — Traefik basicAuth on non-production routers; the `environment/prod` alias stays public
- **Edge firewall** — IP allowlist and rate limiting as Traefik middlewares, ordered before protection and CDN headers
- **Analytics** — first-party privacy-preserving web analytics (daily-rotating visitor HMAC, no IPs stored) + RUM speed insights; optional Rybbit/Umami/GSC injection
- **Status pages** — opt-in public `/{slug}` status page per project plus a shields-style SVG badge for READMEs

**Accounts & teams**

- **Auth** — email + password (Argon2id), magic links, email verification, TOTP 2FA with recovery codes, OIDC/SSO, dedicated GitHub and Google OAuth, sign-up allowlist
- **Teams & RBAC** — owner/admin/member roles, email invites, per-project environment variables, audit log, notifications
- **Managed storage** — Postgres, MongoDB, Redis, SQLite, and volumes per team, linked into deployments on private networks

**Ops & platform**

- **Works behind CGNAT** — instance Cloudflare Tunnel covers the dashboard + `*.DEPLOY_DOMAIN`; per-team tunnels for custom domains; one-click DNS assign via the Cloudflare API
- **Scale-out** — remote Docker nodes with per-node mTLS provisioning (one-click CA + cert bundles) and a real return path
- **CLI + API + MCP** — `runway` CLI over the same REST API, `ak_` API keys, OpenAPI at `/api/v1/openapi.json`, JSON-RPC MCP server for coding agents
- **Observability** — live build + runtime logs (SSE), deployment metrics, outbound webhooks with HMAC signatures, admin job-queue inspection and retry, project export/import

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

Release tags build the container image (`ghcr.io/dvorinka/runway`) and
per-platform CLI binaries; the CLI also ships as an npm wrapper
(`npm i -g @runway-deploy/cli`) and a Homebrew formula (`homebrew/runway.rb`).

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
runway env pull              # write .env.local (decrypted, 0600)
runway domains add app.example.com
runway domains assign-cf app.example.com   # one-click CF DNS via team tunnel
runway open                  # current deployment URL
```

Agents get the same surface: `ak_` API keys against `/api/v1`, or the MCP
server at `POST /api/mcp` (`initialize` / `tools/list` / `tools/call`).

## Architecture

```
Browser ──▶ runway (Rust + axum) ──▶ PostgreSQL (data + job queue)
   web       │                ──▶ Docker / remote mTLS nodes
   cli       ├── SSE logs & events
   agents    └── Traefik file provider ──▶ Cloudflare Tunnel or ACME
```

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
`SELECT ... FOR UPDATE SKIP LOCKED` — orphaned jobs requeue on startup and
panics land as retries, not wedges. Runner containers get per-deployment
edge networks; Traefik file-provider configs are written atomically; logs
are file-tailed, not Loki. See [ARCHITECTURE.md](ARCHITECTURE.md) for the
full model.

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

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — process model, deploy lifecycle, networking
- [ROADMAP.md](ROADMAP.md) — shipped phases and what remains
- [`docs/`](docs/README.md) — self-hosting, preset matrix, `runway.json` reference, agent surface
- `GET /api/v1/openapi.json` — live API contract
- `.env.example` — every configuration knob

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) — fork, branch from `development`,
conventional commits, gates before push. Good first issues are labeled.

## Security

See [SECURITY.md](SECURITY.md) — private reporting via GitHub advisories
plus the self-hosting hardening checklist.

## License

[MIT](LICENSE) © 2026 Tomas Dvorak
