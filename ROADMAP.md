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

- [x] Teams CRUD + invitations + RBAC: create/rename/soft-delete,
      member role change + removal (last-owner protected, self-leave),
      `team_invite` (30-day expiry, email via SMTP, accept by address
      match). Project permissions are team-role based (devpush parity)
- [x] Audit log (`audit_log`, writes on team/member/invite/deploy
      mutations, `GET /teams/{id}/audit` admin+), notifications
      (`notification` table, deploy succeeded/failed fan-out to team,
      `GET /api/v1/notifications` + mark-read), deploy button
      (`GET /api/deploy` → provider/repo prefill JSON)
- [x] Storage provisioning: sqlite/postgres/mongo/redis + volumes —
      `storage`/`storage_project` tables, provision/deprovision/reset
      jobs, per-storage containers + networks, `/data` binds, link with
      environment filter; deploy containers join storage networks
      (fixes devpush's unreachable-DB gap); container-assisted dir
      wipe for engine-owned files; `PGDATA=/data/pg` fix; password
      AES-GCM in config (`password_enc`)
- [x] Git providers: Gitea + GitLab fully wired —
      `*_connection` tables (AES-GCM tokens, token probe on connect,
      upsert per base_url/workspace), connection CRUD +
      repos/branches discovery under `/api/v1/git/{provider}/`,
      `POST /projects` accepts `provider`+`connection_id` (repo_id
      resolved from provider, preset auto-detect via root listing +
      package.json), manual deploy commit resolution +
      `start_deployment` clone via connection tokens (GIT_ASKPASS
      against `repo_base_url`), `POST /api/gitea/webhook`
      (X-Gitea-Signature) + `POST /api/gitlab/webhook`
      (X-Gitlab-Token) push → deploy with the shared rules filter;
      Bitbucket = connection CRUD + discovery only (devpush never
      links projects to it — no deploy path); GitHub Enterprise via
      `GITHUB_API_URL` + `repo_base_url` clone URL. Bitbucket
      project linkage + webhook still open (parity: devpush lacks it)
- [x] OIDC/SSO + allowlist: `allowlist` table (email/domain/pattern
      rules, empty = open signup, `user.id==1` superadmin CRUD under
      `/api/v1/admin/allowlist`), enforced on magic-link request +
      verify, GitHub OAuth, OIDC signup (`ACCESS_DENIED_MESSAGE` +
      `ACCESS_DENIED_WEBHOOK`); OIDC login + session-linking superset
      of devpush's link-only flow (`OIDC_*` settings, state cookie,
      encrypted access token, `GET /api/auth/oidc/info`)
- [x] Cron jobs: `cron_job` table + API CRUD
      (`/projects/{id}/cron`), `parse_schedule` (every N
      minutes|hours / `*/N` / minutes), worker tick loop resolves the
      real branch head (devpush uses a `cron-trigger` placeholder sha
      — we do better) and creates `trigger='cron'` deployments;
      `next_run_at` advances even on failure
- [x] Redirect rules + project export/import: `redirect_rule` CRUD
      writes Traefik `redirectRegex` middlewares onto deployment
      routers (regex fixed for full-URL matching — devpush's `^/path`
      never matches); `GET /projects/{id}/export` +
      `POST /projects/{id}/import` (config merge, env dedupe on
      key+environment, rules appended)
- [x] Remote nodes: `remote_node` CRUD + health probe +
      `project.remote_node_id` assignment; worker resolves a per-node
      Docker client (tcp/http) at deploy time with local fallback.
      **Caveat**: same ingress gap as devpush — Traefik only watches
      the local daemon, so remote containers get no routes until a
      return path lands (per-node cloudflared or Traefik provider);
      TLS columns exist for parity but are unused (devpush admin UI
      never sets them)
- [ ] Remote-node return path — per-node cloudflared or a Traefik
      docker provider per node (the big remaining item)
- [ ] Optional: self-hosted runner registry (independence from
      devpushhq images)

## Non-goals

- Serverless function isolation, ISR, edge middleware runtime
- Self-built CDN (Cloudflare proxy covers this)
- Kubernetes orchestration
