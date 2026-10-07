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
- [x] Postgres schema port from devpush models (~1:1)
- [x] Settings, db pool, migrations, `/health`
- [x] Docker connectivity via bollard (socket proxy aware)
- [x] Traefik file-provider writer (atomic)
- [x] Web app skeleton: Vite + React + Tailwind + shadcn, served by axum
- [x] CI: fmt, clippy, test, build

## Phase 1 — Deploy loop MVP

*Prove the core loop end-to-end before any breadth. GitHub only.*

- [x] Auth: GitHub OAuth + `ak_` API keys + magic link (SMTP or dev-log)
- [x] GitHub App: installation, repo list, webhook → deployment
- [x] Project: environments + branch mapping, encrypted env vars, config
      (runner, commands, root_directory, port)
- [x] Postgres job queue (SKIP LOCKED) + deploy worker
- [x] Runner containers on edge/workspace networks, `$PORT` injection,
      dep-cache volumes, config-file overrides (`runway.json`)
- [x] Monitor probe → finalize; aliases (immutable/branch/env);
      Traefik labels + dynamic config
- [x] File-tailed logs → SSE (build + runtime)
- [x] Rollback, redeploy, cancel, skip; computed/observed status
- [x] `runway serve` production packaging (compose: app, pgsql, traefik)

## Phase 2 — Frontend-first

*The wedge. Nobody else does this well self-hosted.*

- [x] Static deployment mode: build → extract `output_directory` →
      `static-web` serve container → aliases (no long-lived process)
- [x] Framework detection + presets: Next.js, Astro (static+SSR),
      SvelteKit, Nuxt, Remix, Vite/SPA, Hugo (+ backend presets ported)
- [x] Node version matrix (20/22), package-manager detection
      (npm/pnpm/yarn/bun lockfiles → command rewrite)
- [~] Per-PR preview deployments + commit status with URL
      (PR comment deferred — commit status carries the link)
- [x] Build-output caching (`.next/cache`, `.turbo` volumes, per project)
- [x] SPA rewrites/redirects/headers via `runway.json`
      (translated to static-web-server config)

## Phase 3 — Anywhere access (CGNAT)

*The personal pain point, done properly.*

- [x] Instance-level Cloudflare Tunnel: `CF_API_TOKEN` + `CF_ACCOUNT_ID`
      → `ensure_instance_tunnel` job at serve; covers `APP_HOSTNAME` +
      `*.DEPLOY_DOMAIN` — whole box reachable with no public IP
- [x] `DISABLE_TLS` edge-termination mode honored end-to-end
      (web-only labels; tunnel ingress → traefik:80)
- [x] One-click custom domain assign (CF DNS API)
      `POST .../domains/{id}/assign-cloudflare`; multi-domain,
      apex + subdomain + redirect types in schema/Traefik
- [x] Per-team tunnels: `cloudflare_connection` table, token connect
      (`POST /teams/{id}/cloudflare/connect`), disconnect with cleanup,
      per-team `cloudflared-<team>` containers + health loop, zone list;
      domain assign/remove prefers the team connection, falls back to
      instance CF config. Team OAuth connect deferred (needs CF OAuth
      client creds)
- [x] Tunnel health/restart (5-min reconcile loop + restart policy),
      ingress bookkeeping on domain assign/remove
- [x] Fallback unchanged: direct IP + ACME (`le` TLS-challenge
      resolver configured; publish :443 in production)

## Phase 4 — API, CLI, agentic

- [x] Full REST `/api/v1` surface: projects, deployments, env vars
      (PUT replace + PATCH upsert), domains, webhooks, teams read,
      API keys, logs — documented at `GET /api/v1/openapi.json`
- [x] Source-upload deploys: `POST .../deployments/upload` (raw gzip,
      streamed to `data/uploads`, sha256 commit id, `source_archive`
      pipeline branch — skips git clone; tarball retained for redeploy)
- [x] `runway` CLI: `login`, `link`, `deploy --follow`,
      `logs --follow`, `env list|set|unset`,
      `domains list|add|remove|assign-cf`, `open` — config in
      `~/.config/runway`, link in `.runway/project.json`
      (releases/npm/brew publishing deferred)
- [x] Real MCP server: JSON-RPC 2.0 `initialize`/`tools/list`/`tools/call`
      on `POST /api/mcp`, `ak_` auth — 7 tools (project/deployment reads,
      logs, domains, redeploy)
- [x] Outbound webhooks: `project_webhook` + `team_webhook` tables,
      `deployment.{started,succeeded,failed,canceled,skipped}` events,
      `X-Runway-Signature` HMAC + delivery ids (verified live)

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
