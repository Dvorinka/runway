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
| Gitea/Forgejo/GitLab webhooks | `git_providers.rs` | #41 still open for live verification |
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
- **Registry story** — image-based remote builds need somewhere to
  push. Phase A: static-only remote builds (tarball return is trivial).
  Phase B: optional embedded registry or user-supplied one.
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
- **Level 1 — `build_executor: "kern"`** on project/team config:
  1. Deploy time → Runway generates a minimal build workflow
     (checkout → install/build → `runway upload` the output dir as a
     tarball to `POST /deployments/upload` or a dedicated
     artifact-return endpoint),
  2. dispatches it to a KERN project via KERN's REST API
     (`/projects/{p}/workflows/{id}/runs` dispatch),
  3. polls run status (`/runs/{id}`), maps KERN conclusion →
     deployment status, streams step logs into the deployment log.
  Needs on KERN's side: dispatch-with-inputs + artifact return (both
  exist in spirit via `workflow_dispatch` + artifacts; verify exact
  surface). Token: a scoped KERN PAT stored encrypted per team.
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

**P0 (next sprint, all small):**
- Draft-PR skip + fork-PR opt-in + label gate for previews
- Cancel superseded preview builds on same ref
- Per-project commit-status context (`runway/<slug>`)
- Preview deployments default to `preview` env scope
- PR comment: status/duration update + verbosity setting
- Deploy hooks (tokenized URL trigger)

**P1:**
- GitHub Checks API (replaces statuses where installed)
- `check_run` re-requested → redeploy
- `@runway deploy` PR comment commands
- Executor nodes (static builds on remote nodes, labels, selector)
- MCP mutating tools batch + OpenAPI + llms.txt + skill
- KERN Level 0 docs recipe; spec KERN dispatch/artifact surface

**P2:**
- Remote image builds + registry story
- `build_executor: kern` end-to-end
- Preview TTL teardown
- Fork-preview isolation (KERN trust-tier model)
- Usage metering per project/team (enterprise gate from #roadmap)

## Cross-links

- #41 git-provider verification — gate for all of the above
- #86 serverless runtime — separate parity blocker
- #87 vercel.json import — `deploymentEnabled` maps to deploy rules
- KERN repo: `~/Desktop/PROG+HTML/KERN` — runner contract:
  `docs/runner-contract.md`, API: `openapi.yaml`, compat matrix:
  `docs/compatibility.md`
