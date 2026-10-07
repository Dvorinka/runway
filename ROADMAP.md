# Runway Roadmap

Goal: a self-hosted deployment platform that genuinely competes with
Vercel for frontend workloads — no limits, no public-IP requirement,
agent-native.

Reference implementation: `~/Desktop/PROG+HTML/devpush` (frozen fork —
its schema, lifecycle, and integrations are the spec; port features in
dependency order, don't redesign them).

Legend: `[ ]` todo · `[~]` in progress · `[x]` done

## Phase 0 — Skeleton

- [x] Cargo workspace: `core`, `api`, `worker`, `cli` (single binary)
- [ ] Postgres schema port from devpush models (~1:1)
- [ ] Settings, db pool, migrations, `/health`
- [ ] Docker connectivity via bollard (socket proxy aware)
- [ ] Traefik file-provider writer (atomic)
- [ ] Web app skeleton: Vite + React + Tailwind + shadcn, served by axum
- [ ] CI: fmt, clippy, test, build

## Phase 1 — Deploy loop MVP

*Prove the core loop end-to-end before any breadth. GitHub only.*

- [ ] Auth: magic link + GitHub OAuth; `ak_` API keys
- [ ] GitHub App: installation, repo list, webhook → deployment
- [ ] Project: environments + branch mapping, encrypted env vars, config
      (runner, commands, root_directory, port)
- [ ] Postgres job queue (SKIP LOCKED) + deploy worker
- [ ] Runner containers on edge/workspace networks, `$PORT` injection,
      dep-cache volumes, config-file overrides (`runway.json`)
- [ ] Monitor probe → finalize; aliases (immutable/branch/env);
      Traefik labels + dynamic config
- [ ] File-tailed logs → SSE (build + runtime)
- [ ] Rollback, redeploy, cancel, skip; computed/observed status
- [ ] `runway serve` production packaging (compose: app, pgsql, traefik)

## Phase 2 — Frontend-first

*The wedge. Nobody else does this well self-hosted.*

- [ ] Static deployment mode: build → extract `output_directory` →
      shared static file server → aliases (no long-lived process)
- [ ] Framework detection + presets: Next.js, Astro, SvelteKit, Nuxt,
      Remix, Vite/SPA, Hugo (+ existing backend presets)
- [ ] Node version matrix (20/22), package-manager detection
      (npm/pnpm/yarn/bun)
- [ ] Per-PR preview deployments + commit status/PR comment with URL
- [ ] Build-output caching (.next/cache-style, per project)
- [ ] SPA rewrites/redirects/headers config (`runway.json`)

## Phase 3 — Anywhere access (CGNAT)

*The personal pain point, done properly.*

- [ ] Instance-level Cloudflare Tunnel: admin-managed, covers
      `APP_HOSTNAME` + `*.DEPLOY_DOMAIN` — whole box reachable with
      no public IP
- [ ] `DISABLE_TLS` edge-termination mode honored end-to-end
- [ ] One-click custom domain assign (CF DNS API), multi-domain,
      apex + subdomain + redirect types
- [ ] Per-team tunnels for custom domains (port from devpush)
- [ ] Tunnel health/restart, ingress bookkeeping on assign/remove
- [ ] Fallback unchanged: direct IP + ACME (HTTP-01/DNS-01)

## Phase 4 — API, CLI, agentic

- [ ] Full REST `/api/v1` surface: projects, deployments, env vars,
      domains, logs, teams — OpenAPI documented
- [ ] Source-upload deploys (tarball → build, no git required)
- [ ] `runway` CLI: `login`, `link`, `deploy`, `logs`, `env`,
      `domains`, `open` — GitHub releases + npm + brew
- [ ] Real MCP server: JSON-RPC `tools/list` + `tools/call`,
      streamable HTTP, `ak_` auth
- [ ] Deploy hooks + outbound webhooks (port event types from devpush)

## Phase 5 — Breadth (port from devpush, in order)

- [ ] Teams, invitations, RBAC + per-project permissions
- [ ] Audit log, notifications, deploy button route
- [ ] Storage provisioning: SQLite, Postgres, Mongo, Redis + volumes
- [ ] Git providers: Gitea, GitLab, Bitbucket, GitHub Enterprise
- [ ] OIDC/SSO, allowlist
- [ ] Cron jobs (redeploy + HTTP-call mode)
- [ ] Redirect rules UI, project export/import
- [ ] Remote nodes — with a real return path (per-node cloudflared or
      WireGuard); devpush's version is ingress-broken, do not port the bug
- [ ] Optional: self-hosted runner registry (independence from
      devpushhq images)

## Non-goals

- Serverless function isolation, ISR, edge middleware runtime
- Self-built CDN (Cloudflare proxy covers this)
- Kubernetes orchestration
