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

- [x] Auth: email + password (Argon2id, `/api/auth/login` +
      `/api/auth/register`), `ak_` API keys, JWT session cookies;
      optional OIDC/SSO
- [x] GitHub App: installation, repo list, webhook → deployment;
      in-dashboard registration via the app-manifest flow
      (`/api/v1/github/app/*`, credentials AES-GCM in Postgres, hot-load
      no restart, env `GITHUB_APP_*` as override)
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
- [x] Per-PR preview deployments + commit status with URL
      (`pull_request` webhook → deploy on head branch, `runway/deploy`
      commit status with target URL, `preview_prs` config opt-out)
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
      `logs [--follow] [--project]`, `deployments`, `rollback`,
      `stats`, `env list|set|unset`, `domains list|add|remove|assign-cf`,
      `open` — config in `~/.config/runway`, link in
      `.runway/project.json` (releases/npm/brew publishing deferred)
- [x] Real MCP server: JSON-RPC 2.0 `initialize`/`tools/list`/`tools/call`
      on `POST /api/mcp`, `ak_` auth — 12 tools: reads (projects,
      deployments, logs, domains) + writes (`deploy_project`,
      `cancel_deployment`, `rollback_environment`, `list_env`,
      `set_env`, `deployment_stats`)
- [x] Outbound webhooks: `project_webhook` + `team_webhook` tables,
      `deployment.{started,succeeded,failed,canceled,skipped,crashed}`
      events, `X-Runway-Signature` HMAC + delivery ids (verified live)

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
      Bitbucket = full project linkage beyond devpush parity —
      `bitbucket_connection_id` on project, create verifies repo access
      and resolves repo uuid, commit resolution + clone via
      workspace-username app password against `bitbucket.org`;
      GitHub Enterprise via `GITHUB_API_URL` + `repo_base_url` clone URL
- [x] OIDC/SSO + allowlist: `allowlist` table (email/domain/pattern
      rules, empty = open signup, `user.id==1` superadmin CRUD under
      `/api/v1/admin/allowlist`), enforced on password registration and
      OIDC signup (`ACCESS_DENIED_MESSAGE` +
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
      Return path: container publishes an allocated host port
      (`49152+`, `alloc_remote_port`), `deployment.remote_port` is
      persisted, and Traefik file-provider `loadBalancer` services
      route `http://{node.host}:{port}` — no extra infra on the node.
      Monitor/cleanup/teardown use the node's own daemon; local
      bind-mounted storage is skipped on remote (artifacts/uploads
      stay local). **Caveats**: upload + static deployments are
      rejected on remote (local artifacts), TLS/mTLS columns exist
      for parity but are unused
- [x] Runner image overrides: `data_dir/runner-overrides.json` maps
      runner slugs to replacement images (or `enabled:false` to block
      one) — decouples self-hosted instances from ghcr.io/devpushhq
      without a full dynamic catalog; read per deploy so edits need
      no restart
- [x] Dashboard UI for the new surface: nav layout + Settings page
      (git provider connect/list, allowlist CRUD, node CRUD + health —
      admin sections gated on `user.id==1`), OIDC button on Login
      driven by `/api/auth/oidc/info`, project create form with
      provider/connection/repo pickers (GitHub installations +
      gitea/gitlab/bitbucket connections), project tabs for env vars,
      cron, redirects, domains, webhooks, and settings (node assign +
      export/import)

## Phase 6 — polish

- [x] One-line install: `install.sh` (served from GitHub raw — downloads a
      repo tarball, generates `.env` secrets, `compose up --build`) +
      `compose/production.yml` (80/443, ACME + websecure router,
      env-required secrets, `runway bootstrap` first-user flow)
- [x] Dashboard: teams UI (list/create, members + roles, invites,
      webhooks, audit log, rename/delete), storage UI (create/reset/
      delete, project link/unlink — `links` added to storage list
      response), notifications feed + unread badge, invite-accept page

## Phase 7 — Production hardening

*Beyond devpush parity — the operational layer.*

- [x] Dockerfile builds: `config.dockerfile_path` (auto-detected on
      project create) → fetch container stages source to a host context
      dir, `docker build` with the real repo as context (devpush built
      with an empty context — `COPY` could never work), built image
      serves directly. Settings card + tarball-deploy support verified
      end-to-end
- [x] CDN asset caching: hashed-asset paths get `immutable` + long
      `max-age`, HTML `must-revalidate` — Traefik headers middleware on
      static-output projects
- [x] Edge firewall: `config.firewall` → Traefik `ipAllowList` +
      `rateLimit` middlewares, applied on `PATCH` immediately
      (no redeploy). Settings card
- [x] Runtime observability: monitor observes container state →
      `observed_status`/`observed_exit_code`/`observed_missing_count`,
      `computed_status` (`crashed`/`missing`/`stopped`) surfaced in API
      + UI badges, manual `POST /deployments/{id}/reconcile`,
      crash → team notification + `crashed` webhook event
      (deduped on transition), Overview "Needs attention" section
- [x] Container metrics: `deployment_metric` table, 30s sampling in the
      monitor sweep, p-series endpoint + sparkline, one-shot
      `GET /deployments/{id}/stats` (live CPU/mem/net/disk),
      `runway stats` + MCP `deployment_stats`
- [x] Logs: per-deployment file logs + SSE, project-level merged tail
      (`GET /projects/{id}/logs`, Logs tab, `runway logs --project`),
      30-day file retention prune
- [x] Cascading deletes: `delete_project`/`delete_team`/`delete_user`
      queue jobs — containers, aliases, domains, storage links, Traefik
      config, memberships; name-confirmation on project delete
- [x] Account surface: `PATCH /api/auth/me`, `POST /api/auth/password`
      (bumps `tokens_invalid_before` → all sessions revoked except
      current), `DELETE /api/auth/me` with cleanup job
- [x] RBAC gate: project mutations require team `creator`+
      (devpush parity), deploys `member`+, storage `admin`+ — verified
      with a second user
- [x] Environments management: full-array `environments` PATCH +
      Settings UI card
- [x] Avatars: user/team/project upload + public serve
      (`has_avatar` flags, normalized files under `data_dir`,
      dashboard pickers + display everywhere)
- [x] Speed insights (RUM): `rum_event` table, same-origin
      `/_runway-rum` beacon routed through Traefik to the API
      (ad-blocker resistant, works without public dashboard), ~1KB
      `PerformanceObserver` script injected at deploy, p75 cards
      (LCP/INP/CLS/FCP/TTFB) + top paths — verified end-to-end
- [x] Analytics injection: `config.analytics` provider presets
      (Umami/Rybbit/Plausible/custom) + Google Search Console meta —
      scripts before `</body>`, meta in `<head>` — verified in served
      HTML
- [x] 7 new frontend presets: `static` (bare HTML), Angular,
      SolidStart, Qwik, Eleventy, Gatsby, Docusaurus — 25 total
- [x] Cloudflare UI: Team page connect card (token → account + tunnel
      status), Domains tab Verify + Assign-via-Cloudflare buttons
- [x] Dashboard rebuild: Overview at `/` (stats, project cards, recent
      deployments, needs-attention), Deployments index across
      projects, Vercel-grade project cards (search, grid/list, preset
      tags, latest deploy), drag-and-drop instant deploy (in-browser
      tar.gz, drop → upload → live logs)

## Phase 8 — Vercel-class polish

*Ordered by leverage. Each item names its gap.*

- [x] **Preview environment fallback** — unmatched branches synthesize
      a `pv{sha6}` env cloned from `config.preview_template` (default
      prod), env vars scoped via the template slug; opt out with
      `config.preview_environments=false` (settings toggle added)
- [x] **PR lifecycle** — `pull_request.closed` drops branch +
      preview-env aliases and enqueues container teardown; upserted
      `runway-preview` PR comment posts the branch URL at create and
      updates to ready/failed on finalize (`issues: write` on the app
      manifest; opt out `deployment_rules.preview_comment=false`)
- [x] **Bitbucket push webhook** — `POST /api/bitbucket/webhook`
      (`repo:push`). The payload's claimed sha is never trusted — the
      real branch head resolves via the project's connection API before
      deploying (closes devpush's forged-sha hole); optional
      `BITBUCKET_WEBHOOK_SECRET` via `?secret=` on the hook URL
- [x] **HTTP health checks** — `config.health_check` (path or
      `{path, interval_seconds, failures}`) probed by the monitor on
      running containers; consecutive failures set
      `observed_status=unhealthy` → `computed_status` + badge +
      crash notification, recovery self-heals. Settings card;
      `deployment_observed_status_check` widened (migration 0017)
- [x] **Deployment protection** — `basicAuth` on every non-prod router
      (branch, env-id, preview aliases + non-prod domains); write-only
      `protection_password` PATCH field → bcrypt `protection.users`;
      prod env alias + prod domains stay public; edge rewrite on
      PATCH; middleware order firewall → protect → cdn
- [x] **Deployment retention policy** — opt-in
      `config.deployment_retention` keeps the newest N completed
      deployments per environment; older unreferenced rows pruned with
      metrics, logs, and artifacts after each deploy; alias-referenced
      deployments (incl. `previous_deployment_id` rollback targets)
      always spared. Settings card added
- [ ] **TOTP 2FA** — `otpauth://` enrollment + recovery codes on the
      account surface
- [x] **First-party web analytics** — `web_analytics.enabled` gates
      `pageview`/`custom` beacons on the same `/_runway-rum` endpoint;
      snippet fires pv on load + SPA navs and exposes
      `window.rw.event()`; visitors are a daily-rotating HMAC (no IPs,
      no cookies), referrers stored as hostname only. Aggregates
      endpoint + dashboard card (migration 0018)
- [ ] **Public status pages** — per-project uptime view from
      `deployment.observed_*` + metric samples; opt-in public route
- [ ] **Remote-node mTLS** — columns exist; needs cert provisioning
      flow
- [ ] **Bitbucket deploys tested against a real workspace**
- [ ] **Release packaging** — `runway` npm wrapper/brew formula,
      published container images, versioned releases
- [ ] **Docs site** — preset matrix, config reference (`runway.json`,
      firewall, health checks), self-host install, agent/MCP guide

## Non-goals

- Serverless function isolation, ISR, edge middleware runtime
- Self-built CDN (Cloudflare proxy covers this)
- Kubernetes orchestration
