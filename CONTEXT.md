# Runway — glossary

Canonical terms. Use these in code, UI copy, docs, and conversation — not
synonyms. Where a term matches Ployz's glossary the meaning should too.

**Instance** — one running `runway` deployment: binary + Postgres +
Traefik. First registered user is its admin.

**Project** — the deployable unit. One repo/source, many Deployments,
one Production alias per Environment. Vercel: "project".

**Deployment** — one build+release of a Project. Lifecycle:
`prepare → deploy → finalize → completed` (or `failed` / `cancelled`).
A Deployment is immutable; rollback creates a new one. Vercel: "deployment".

**Environment** — `production` or a named preview context. Runway's
preview granularity is per-deployment, not per-PR-set like Ployz's.

**Alias** — a hostname that points at a Deployment. `*.DEPLOY_DOMAIN`
generated aliases plus custom Domains. Assigning an alias is atomic.

**Domain** — a user-owned hostname attached to a Project, verified then
routed. Ployz says "domain"; same thing.

**Build** — the prepare phase's artifact work (detect → install → build).
Output becomes the Deployment's static/server payload.

**Observed status** — what runtime evidence says: health checks, container
state. Never a desired-state guess. Reports `unhealthy`, `crashed`,
`building`, etc. (See DESIGN.md bet 4.)

**Team** — a billing/membership boundary owning Projects and Tunnels.
Personal accounts are single-member Teams. Vercel: "team" / Ployz: "org".

**Tunnel** — a Cloudflare tunnel owned by a Team, terminating TLS at the
edge for hosts without a public IP. `assign-cf` binds a Domain to one.

**API key** — `ak_`-prefixed agent/CLI credential, sha256-hashed at rest.
The MCP server and CLI authenticate with it; the dashboard uses sessions.

**Worker** — a tokio task inside the binary: deploy pipeline, monitor,
cron, reconcile. Not a process, not a queue consumer of anything but
Postgres.
