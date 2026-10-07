# Runway

An open-source, self-hosted deployment platform — the Vercel experience without the meter.

Git-push deploys, preview URLs, instant rollback, custom domains, and real HTTPS on any box — including machines behind CGNAT with no public IP, via built-in Cloudflare Tunnel. One binary, one `runway` CLI.

> **Status:** early scaffold. This is a ground-up Rust rewrite informed by
> [devpush](https://github.com/hunvreus/devpush) (and the heavily extended
> Dvorinka fork), which serves as the reference implementation and spec.

## Design goals

- **Frontend-first.** Static sites and SSR frameworks (Next.js, Astro, SvelteKit, Nuxt, Vite/SPA, Hugo) are first-class citizens — framework detection, correct ports, build caching, per-PR previews. Backends still work; they're just not the wedge.
- **Works behind CGNAT.** An instance-level Cloudflare Tunnel covers the dashboard, the wildcard deploy domain, and custom domains. No public IP, no port-forwarding, no dynamic DNS. Direct-IP + ACME remains the standard path.
- **Single binary.** `runway serve` runs the whole platform — API, deployment workers, monitor, cron — with Postgres as the only datastore. No Redis, no Loki on single-node installs.
- **Agent-native.** Full REST API plus a real MCP server, so coding agents can deploy, inspect, and remediate. The CLI is a thin client over the same API.
- **No artificial limits.** Your hardware, your quotas.

## Stack

| Layer | Tech |
|---|---|
| Control plane | Rust (axum, sqlx, bollard, tokio) — workspace `crates/` |
| Dashboard | React + Vite + Tailwind + shadcn (`web/`) |
| CLI | clap, same binary — `runway` |
| Data | PostgreSQL (also the job queue — SKIP LOCKED) |
| Ingress | Traefik (Docker + file providers) |
| Logs | File-tailed container logs (no Loki/Alloy) |
| Tunnels | cloudflared sidecars managed by the platform |

## Quickstart (future)

```bash
# On any Linux box, including behind CGNAT:
curl -fsSL https://get.runway.sh | sh
runway serve

# From a project directory:
runway login && runway link && runway deploy
```

## Repo layout

```
crates/
  core/    runway-core    — settings, db, docker, traefik, cloudflare, models
  api/     runway-api     — axum router: REST, webhooks, SSE, MCP, SPA host
  worker/  runway-worker  — deploy pipeline, monitor, cron, reconcile
  cli/     runway         — single binary: serve + user CLI
migrations/               — sqlx migrations
web/                      — React dashboard (Phase 0+)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) and [ROADMAP.md](ROADMAP.md).

## License

MIT
