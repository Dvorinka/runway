# Architecture

Runway is a single-binary deployment platform. One `runway` executable runs
the API, the deployment workers, and serves as the end-user CLI. Postgres is
the only datastore — it doubles as the job queue.

Informed by the devpush reference implementation (`~/Desktop/PROG+HTML/devpush`),
whose schema, deployment lifecycle, and edge cases are the spec for this rewrite.

## Process model

```
runway serve
├── axum API          — REST, webhooks, SSE, MCP, hosts the React SPA
├── deploy worker     — claims queued deployments (SKIP LOCKED)
├── monitor loop      — probes containers, reconciles observed state
├── cron tick         — scheduled jobs
└── (sidecars)        — traefik, cloudflared, pgsql run as containers
```

No Redis. Jobs are Postgres rows claimed with
`SELECT ... FOR UPDATE SKIP LOCKED`; multi-node scaling can add a real
broker later without changing semantics.

## Crates

- **runway-core** — Settings (env), sqlx pool + migrations, bollard Docker
  client, Traefik dynamic-config writer, Cloudflare client, domain models,
  framework detection, log tailing.
- **runway-api** — axum router. REST under `/api/v1`, deploy tokens under
  `/api/deploy`, git webhooks, SSE streams, MCP endpoint (`/api/mcp`),
  and the compiled React SPA as fallback.
- **runway-worker** — the deployment pipeline and background loops.
- **runway** (cli) — `serve`, `migrate`, `login`, `link`, `deploy`, `logs`,
  `env`, `domains`, `open`. Remote commands are thin calls to the REST API.

## Deployment flow (Phase 1 spec, ported from devpush)

1. **Trigger** — GitHub webhook / manual / API / CLI upload → `deployment`
   row (`status=prepare`) enqueued in Postgres.
2. **Claim** — deploy worker locks the row, snapshots project config
   (runner image, commands, env vars, port, root directory).
3. **Build & run** — runner container created on a per-deployment edge
   network (`runway_edge_<id>`) + per-team workspace network
   (`runway_ws_<team>`); inside: clone at commit → `devpush.json`-style
   config file overrides → build → pre-deploy → start on `$PORT`.
4. **Probe** — monitor polls the configured port until ready or timeout.
5. **Finalize** — mark `completed/succeeded`, swap aliases (branch,
   environment, environment_id), regenerate Traefik file config
   (atomically), fire webhooks, clean up superseded containers.
6. **Fail** — stop/remove container, mark `failed`, emit events.

Statuses: `prepare → deploy → finalize → completed` with conclusion
`succeeded | failed | canceled | skipped`.

## Networking

- `runway_default` — Traefik, app, public-facing services.
- `runway_internal` — Postgres, Docker socket proxy (internal-only).
- `runway_edge_<deployment-id>` — Traefik ⇄ runner, per deployment.
- `runway_ws_<team-id>` — east/west between a team's deployments.
- No shared runner network; lateral movement is denied by topology.

## Domains & TLS

- Aliases: immutable `…-id-<sha7>`, branch `…-branch-<name>`,
  environment `…-env-<slug>` hostnames under `DEPLOY_DOMAIN`.
- Custom domains: `Domain` rows → Traefik file-provider routers
  (`Host()` rules, redirect types 301/302/307/308).
- TLS: ACME via Traefik (HTTP-01 or DNS-01 provider matrix), or
  `DISABLE_TLS` when an upstream edge terminates TLS.

## Access paths (the CGNAT story)

Two ingress modes, orthogonal to each other:

1. **Direct IP** — standard: A record → server, ACME certs, ports 80/443.
2. **Cloudflare Tunnel** — `cloudflared` sidecar(s) make outbound
   connections; Cloudflare edge terminates TLS and forwards to Traefik.

Tunnel scope levels:

- **Instance tunnel** (admin, new in Runway): one tunnel covering
  `APP_HOSTNAME` + `*.DEPLOY_DOMAIN`. Set up during install or from admin
  settings — makes a home server fully usable with no public IP.
- **Team tunnel** (from devpush): per-team tunnel for custom domains;
  domains get CNAME → `<tunnel-id>.cfargotunnel.com`, ingress updated on
  every assign/remove.

The platform manages cloudflared container lifecycle, tunnel credentials
(encrypted at rest), DNS records, and ingress rules — one-click per domain.

## Logs

File-tailed, not Loki:

- Runner containers log to Docker's json-file driver (default).
- The worker streams `docker logs --follow` for live SSE and appends to
  `data/logs/<deployment_id>.log` for history.
- Log queries = file reads; no extra containers, no retention daemon.

Trade-off: no LokiQL. Acceptable at this product's scale; a Loki backend
can be reintroduced behind a `LogStore` trait if ever needed.

## Static deployments (the frontend wedge)

Two deployment modes, same pipeline:

- **Service** — long-lived container behind Traefik (today's model).
- **Static** — build in the runner, extract `output_directory` (e.g.
  `dist/`), publish to `data/static/<deployment_id>`, serve via a shared
  static file server (or Traefik `fileServer` middleware per alias).
  Cheaper, faster, and the right primitive for most frontend work.

## API surface (Phase 4 spec)

- `ak_` API keys → full REST (`/api/v1/*`), OpenAPI via utoipa/aide.
- `dp_` deploy tokens → `POST /api/deploy` (env-scoped).
- CLI: tarball upload deploy (`POST /api/v1/deployments` with a source
  archive — bypasses git clone), plus login/link/logs/env/domains.
- MCP: real protocol — JSON-RPC 2.0 `initialize`, `tools/list`,
  `tools/call` on streamable HTTP at `/api/mcp`, `ak_` auth.

## Data model (ports from devpush ~1:1)

user, user_identity, team, team_member, team_invite, project, deployment,
alias, domain, redirect_rule, github_installation, gitea/gitlab/bitbucket
_connection, cloudflare_connection, storage, storage_project, deploy_token,
api_key, webhook (team/project), notification, audit_log, cron_job,
project_permission, queue_worker, remote_node.

## Explicit non-goals

- Serverless function isolation, ISR, edge-runtime middleware — out of
  scope by design; Next.js standalone output covers realistic SSR needs.
- Self-built CDN — Cloudflare proxied mode is the CDN.
- Kubernetes — remote Docker nodes (with a real return path) cover scale-out.
