# Integration & CI/CD roadmap — GitHub, build runners, KERN, agent-first

Audited 2026-11-19 against: the live Vercel GitHub integration surface,
KERN (`~/Desktop/PROG+HTML/KERN`, v0.5.x self-hosted GH Actions
replacement), and Runway's own code. This is the plan that closes the
"Vercel-connected" loop and makes Runway's build/dispatch layer
user- and agent-first.

## What already exists (verified in code)

| Capability | Where | Notes |
|---|---|---|
| GitHub App manifest registration | `api::routes::github` | one-click app creation, hot-patched creds |
| Installation tokens + lifecycle | `core::github` | install/delete/suspend/repo-add-remove tracked |
| Push → deploy w/ rules | `github.rs::handle_push` | branches, authors, paths, merge-skip, auto_deploy |
| PR preview deploys | `github.rs::handle_pull_request` | opened/sync/reopened → deploy; closed → teardown |
| PR comment upsert | `post_preview_comment` | single marked comment, preview URL |
| Commit statuses | `deploy.rs::post_commit_status` | `runway/deploy` context on success/failure |
| Deployment rules | `deployment_rules` config | `auto_deploy`, `preview_prs`, `preview_comment` |
| Remote nodes | `remote_node` | token-gated enroll, mTLS, last-seen — runtime only |
| Team webhooks (outbound) | `routes::teams` | deploy events fan out |
| Gitea/Forgejo/GitLab webhooks | `git_providers.rs` | all legs verified E2E 2026-11-19 (#41 closed): GitLab + Bitbucket simulated, GitHub App real repo |
| MCP server | `routes::mcp` | ~20 read/deploy tools |
| Audit export, activity log, `--json` CLI | recent PRs | agent-consumable |
| Automation bypass secret | #78 | Vercel deployment-protection equivalent |

## Gaps vs Vercel's GitHub integration

1. **No GitHub Checks API** — Vercel posts Check Runs with details,
   annotations, and a "re-run" action. Commit statuses can't be
   re-requested from the GitHub UI; checks can.
2. **Preview policy is binary** — Vercel is *all-or-nothing* on previews
   (a real complaint); Runway already does better with `preview_prs`/
   `preview_comment`, but is missing: **draft-PR policy** (currently
   deploys drafts — wasted builds), **fork-PR policy** (untrusted code
   needs opt-in + env scrub), **label-gated previews** (`deploy-preview`
   label opt-in/opt-out), **path-scoped previews** (reuse `rules.paths`
   for PRs — currently skipped), **preview TTL** (auto-teardown after N
   days even if PR stays open).
3. **No superseded-build cancellation** — rapid pushes queue every SHA.
   Vercel has no cancel either — easy "better than Vercel" win: cancel
   pending preview builds when a newer commit arrives on the same ref.
4. **No deploy hooks** — URL-triggered deploys (Vercel's Deploy Hooks
   equivalent) for CMS/external triggers.
5. **No `/deploy` PR commands** — comment-triggered redeploys
   (`@runway deploy`, `@runway redeploy`) — cheap, very agent-friendly.
6. **No Check-run-driven rerun** — "Re-run" in GitHub UI should requeue
   the deployment (`check_run`/`check_suite` webhook events).
7. **Commit-status context is fixed** — one `runway/deploy` context per
   repo even with multiple projects on it. Scope contexts per project
   slug (`runway/<slug>`) so required-status checks work in monorepos.
8. **No preview env-var scoping** — env vars already scope to
   environments; preview deployments should resolve a `preview`
   environment by default (Vercel parity + secret hygiene).
9. **PR comment is URL-only** — add build state, duration, commit, and
   update it on success/failure. Verbosity levels: off | url | full.

## Self-hosted build runners (own architecture)

Today `remote_node` = **runtime** target only; builds always run where
the Runway process lives (`static`/`dockerfile` explicitly bail on
remote nodes). To let users build on their own hardware/arch
(ARM64 boxes, Apple Silicon, GPU hosts):

- **Executor nodes** — extend `remote_node` with a `capabilities`
  field (`build`, `run`, or both) + labels (`arch=arm64`,
  `os=macos`, `gpu`). Build jobs carry a `node_selector`; the node
  claims via the same SKIP LOCKED queue, runs the `runner-*` container
  locally, and ships the artifact back (static dir tarball / image
  push to a registry Runway can pull from).
- **Registry story** — originally assumed for image-based remote
  builds. Turned out unnecessary: the dockerfile fetch stages the repo
  in-container, the context is pulled back over the archive API, and
  `build_image` runs on the node — the image never leaves the daemon
  that serves it. A registry only becomes relevant if we ever want the
  build daemon and the serve daemon to differ.
- **Fallback policy** — `node_selector` unmatched → queue holds or
  falls back to local, per team setting.
- This also fixes the current sharp edges where `static` and
  `dockerfile` deploys hard-fail on remote nodes instead of scheduling
  elsewhere.

## KERN bridge (optional, feature-flagged)

KERN = user's other project: Go control plane + runner agents that
execute GitHub Actions workflow YAML on self-hosted hardware
(macOS/Windows host executor included). Natural complement — KERN owns
generic CI capacity; Runway owns deploy.

Non-goal: no hard dependency. Integration is a per-team opt-in.

- **Level 0 — today, docs only** (free): enroll the same machines as
  both KERN runners and Runway remote nodes. One-line installers
  already exist on both sides. Write the recipe.
- **Level 1 — `build_executor: "kern"` — SHIPPED** (verified end-to-end
  against kern.tdvorak.dev). `project.config.build_executor = "kern"`
  plus `RUNWAY_KERN_URL`/`RUNWAY_KERN_TOKEN`/`RUNWAY_KERN_PROJECT`:
  1. the source ships to KERN as a content-addressed bundle
     (`PUT /api/v1/projects/{p}/source`, resolve-then-upload dedupe)
     which **replaces the repo checkout** — upload deploys send the
     deployment tarball, git deploys send a marker bundle and the
     in-workflow clone does the real fetch,
  2. a generated workflow (`POST /api/v1/projects/{p}/runs` with inline
     `workflow_yaml`, `trust_level: trusted`) runs the *same*
     `commands.join(" && ")` script as the local runner — readcfg
     overrides, presets, snippets — under `container.image` = the
     resolved runner image, on `kern_labels` (default
     `self-hosted,linux,docker`),
  3. poll `GET /api/v1/runs/{id}` → conclusion; deployment cancel
     propagates `POST /runs/{id}/cancel`; run id is recorded on the
     deployment (`config.kern_run_id`),
  4. the `runway-out` artifact (`actions/upload-artifact`,
     `path: runway-out/**`) is downloaded
     (`GET /api/v1/artifacts?run_id=` → `/{id}/download`) into the
     static dir — SWS serve and `.runway.json` handling are unchanged.
  Gotchas encoded: a bare `path: dir` glob yields an empty artifact
  (must be `dir/**`); publish uses `tar --exclude` because the
  workspace `runway-out/` would self-copy when `output_directory=.`.
  Scope: static (output_directory) builds only; token is a single
  instance-level PAT today — per-team scoped tokens are the upgrade.
- **Level 2 — shared pool** (later): Runway build jobs claimable by
  KERN runners through a claim-shim adapter, or joint runner labels.
  Defer until Level 1 proves demand.
- Bonus: KERN's trust-tier isolation model (trusted / same-repo PR /
  fork with egress proxy) is a ready-made spec for Runway's fork-PR
  preview isolation if fork previews ever land.

## Agent-first surface

Agents are the primary operator — the API is the product.

| Now | Needed |
|---|---|
| ~20 MCP tools incl. deploy/cancel/redeploy/set_env/cron | Missing write tools: project create/update + deploy-rules config, domain add/remove/verify, storage create/link, node approve/rm/enroll-token, webhook endpoint CRUD |
| REST API, undocumented shape | OpenAPI spec (`utoipa` codegen or maintained `openapi.yaml` like KERN's) published at `/api/openapi.json` |
| Ad-hoc discovery | `llms.txt` at repo root + `/.well-known/`; an `AGENTS.md` quickstart; a `.agents/skills/deploy-with-runway` skill |
| No dry run | `runway deploy --dry-run` / `?dry_run=1` — resolves preset, env, rules, outputs the plan JSON |
| No idempotency | `Idempotency-Key` header on deploy/env/domain mutations |
| Polling | SSE already exists for logs; add deployment-state SSE + `runway deploy --wait` exit codes mapped to states |
| Errors as strings | Stable error `code` field on every 4xx/5xx |
| No `doctor` command | `runway doctor` — instance health/preflight with `--json` + check IDs (self-host debugging + agent gating) |

KERN's `openapi.yaml` is the bar — Runway should match it.

## Phasing

**P0 — shipped 2026-11-19 (PR #97):**
- ~~Draft-PR skip~~ `preview_drafts` + ~~fork-PR opt-in~~ `preview_forks`
  (secure default: off) + ~~label gate~~ `preview_label`
- ~~Cancel superseded preview builds~~ (`cancel_superseded`, default on)
- ~~Per-project status context~~ `runway/<slug>`
- ~~Preview env scope~~ — `preview`-scoped env vars overlay the template
- ~~PR comment upgrades~~ — `minimal`/`full` verbosity + conclusion
  (ready/failed + duration) updates
- ~~Deploy hooks~~ — `/api/deploy-hook/{token}` + CRUD + MCP
- ~~`runway doctor` + `deploy --dry-run` + `llms.txt`~~
- ~~vercel.json import~~ — upload archives auto-translate to runway.json
- ~~Node 24 runner~~ — `node-24` → `node:24-bookworm`
- ~~`Idempotency-Key`~~ on deploy endpoints + MCP arg
- ~~Preview TTL teardown~~ — `preview_ttl_hours`, reconcile sweep
- ~~Structured API error codes~~ — `code` field on error bodies
- ~~Traefik edge-network reattach on restart~~ (#98) — container-id
  watcher + malformed dynamic-config sweep at startup

**P1 — first half shipped in PR #97:**
- ~~GitHub Checks API~~ — check runs with `external_id`, status fallback
- ~~`check_run` re-requested → redeploy~~
- ~~`@runway deploy` PR comment commands~~
- ~~Node selector~~ — `project.config.node_selector` label match,
  least-loaded pick (executor-node half of remote builds)
- ~~MCP write batch~~ — config, hooks, domains, storage (31 tools)
- ~~Executor builds on remote nodes (static)~~ — `/out` pulled back
  over the docker archive API, shipped to a node volume for SWS
- ~~KERN bridge Level 1~~ — `build_executor: "kern"` verified live
  against kern.tdvorak.dev (bundle dispatch + artifact return)

- ~~Remote dockerfile builds~~ — remote fetch + context pull-back +
  `build_image` on the node; verified live, no registry needed

**P2:**
- KERN Level 2 — shared runner pool, per-team tokens
- Fork-preview isolation — env scrub shipped (`pr_fork` → no project
  env in build/runtime, unit-tested); clone tokens moved out of
  container env into a deleted-before-build askpass file (all builds,
  not just forks). Remaining: egress/network isolation (KERN trust-tier)
- ~~Usage metering v1~~ — `GET /api/v1/projects/{id}/usage`, `runway
  usage`, MCP `get_project_usage`: deploy counts, build seconds,
  artifact/image bytes per project over N days. Remaining: bandwidth
  (needs Traefik access-log accounting), team rollup, billing hooks
- ~~GitHub App webhook E2E~~ — `runway-deploys` app (env creds:
  `GITHUB_APP_ID`/`_NAME`/`_PRIVATE_KEY`/`_WEBHOOK_SECRET`),
  signed push → deploy → private clone → serve → check run. Live
  inbound delivery still needs a public webhook URL.

## Cross-links

- ~~#41 git-provider verification~~ — closed, all legs verified
- #86 serverless runtime — shim shipped (PR #101): static `api/*`
  handlers bundle into a node:http dispatcher co-served with the
  static output; per-function isolation stays under enterprise gates
- #87 vercel.json import — `deploymentEnabled` maps to deploy rules
- KERN repo: `~/Desktop/PROG+HTML/KERN` — runner contract:
  `docs/runner-contract.md`, API: `openapi.yaml`, compat matrix:
  `docs/compatibility.md`
