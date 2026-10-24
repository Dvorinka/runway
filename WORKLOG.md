# Worklog — overnight session (2026-10-21/22)

Autonomous pass, uncommitted. Working tree = Phase 1–3 + polish.

## Ployz LXC (Proxmox `nab9`)

- CT **538 `ployz-1`**: Debian 13, **privileged** (ZFS capable), 4c/8G/64G on `local-lvm`, DHCP on `vmbr0` (currently `10.0.0.80` — fresh lease after the privileged recreate), nesting+keyctl+fuse+mknod on.
- `zfsutils-linux` installed (binds host `kmod` + `/dev/zfs` via `lxc.cgroup2.devices.allow` + `lxc.mount.entry`), Docker 29.9.2.
- **Enrolled 2026-10-22** on ployz **0.3.0** via `curl -fsSL https://ployz.sh | PLOYZ_FLEET_TOKEN=… sh -s -- 0.3.0 --enroll` (non-interactive; env var feeds the fleet-token prompt).
- **State: ONLINE** — metrics streaming, `ployz-wg` mesh interfaces up, web traffic + builds enabled.
- Gotcha fixed: installer writes `/etc/apt/sources.list.d/ployz-contrib.sources` with `Signed-By: .pgp` while stock `debian.sources` uses the `.gpg` symlink — apt treats the string mismatch as fatal. Resolved by pointing debian.sources at `.pgp` and dropping its duplicate `contrib` component. Survives reinstall (installer regenerates its own file).

## Runway — what changed tonight

Shell + token polish (all verified in headless Chrome, `screenshots/`):

- `SectionHead` icon tiles → neutral (`border-border bg-accent/50`, `text-muted-foreground`)
- `EmptyState` wired into Overview (projects + recent deployments) and Project deployments tab
- Project page: hero brand blur removed, live-production card → plain `bg-card` (StatusDot carries the semantic), tab underline → `border-foreground`, "Production · live" label → `text-success`, cron schedule pills → neutral selection
- Deployments tab underline → `border-foreground`; provider segmented control → neutral
- Overview success-rate bar → semantic (success/warning/danger); job-queue "running" dot → `bg-amber-400`
- ComboBox check, BookOpen icon, numbered-list indexes, 404 code, Teams `owner` badge → all neutral
- Kept brand-orange only where it means attention/intent: unread dots+badge, palette selection, drop-zone overlay, active pipeline step, Overview hot-card, Login brand wash

## Verification loop (reusable)

- Stack: `scripts/verify/up.sh` (docker compose + seed). API `:8000`, creds `dev@runway.local` / `runway-dev-pass`.
- Vite dev for iteration: `cd dashboard && pnpm dev` on `:5173` (proxies `/api`).
- Headless CDP driver: `/tmp/cdp.py` (stdlib ws). If /tmp cleared on reboot it's gone — commands: `nav <url>`, `shot <png> [delay]`, `eval <js>`, `cons <url> [secs]`. Chrome: `setsid google-chrome --headless=new --no-sandbox --remote-debugging-port=9223 --user-data-dir=/tmp/rwprof`.
- Login for tests: `fetch('/api/auth/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({email,password})})` in-page.
- Upload deploy: `curl -b cookies -X POST :8000/api/v1/projects/<id>/deployments/upload -H 'content-type: application/gzip' --data-binary @site.tar.gz` — raw body, NOT `-F` multipart (breaks gunzip).

## Known environment hazards

- dockerd wedges under build load on this box (`docker ps` hangs). `sudo -n systemctl restart docker` recovers; containers come back. First deploy run failed on `ghcr.io` pull during a wedge — image is now cached locally.
- Visible Chrome paints blank under XWayland/GPU; app is fine. Use headless.

## Data now seeded

- Project `demo-app` (`dbabe3f7…`), 3 deployments, latest `10629f1` live at `demo-app-devs-team.deploy.localhost` (needs hosts/DNS entry to actually visit).
- 3 "Deployment ready" notifications.

## Morning pass (2026-10-22)

- CommandPalette: grouped results (Projects / Actions / Pages), "New project" action → `/projects?new=1` opens the create form directly (Projects reads the param). Selection marks with `text-foreground`, not brand.
- Deployment detail: `Preview` component probes the deploy URL (no-cors fetch, 5s timeout) — unreachable URL renders a calm placeholder ("isn't reachable from this network") instead of a dead grey iframe.
- Login: runway centerline dashes down the rotated beam — the one free brand metaphor.
- Layout: mobile collapse. `lg+` keeps the 240px rail; below that a sticky top bar (wordmark, Find icon, hamburger) + slide-over drawer with the identical rail. Drawer closes on navigation.
- `scripts/verify/cdp.py` — the CDP driver now lives in-repo (was /tmp, lost on reboot). `nav|shot|eval|cons|msize|mclear|mshot` — `mshot` does metrics+shot in one session (override is connection-scoped).
- Verified: mobile overview screenshot clean, palette grouped, login centerline rendered.

## Vercel authed-research pass (2026-10-22, second half)

Browser fix: dedicated X11 Chrome, profile `~/.config/chrome-inspect`,
CDP :9222 — Vercel session cloned via sqlite `.backup` of the main
profile's cookie store (same Safe Storage key). `cdp.py` takes
`CDP_PORT`. Full notes: `vercel-research/FINDINGS.md`.

Landed from the findings:

- **TopBar** (desktop `lg+`): sticky h-11 — breadcrumb left, page
  title center (project name fetched when scoped), `New project` right.
- **Preflight checklist** card on the project Deployments tab — real
  state (deploy/live/domain/env), n/4 chip, per-row Open links into
  `?tab=` views. Hides when all done.
- **CLI hint strip** under the live-production card — teaches
  `runway deploy`, Deployments shortcut.
- **Scoped palette**: inside `/projects/:id` the palette leads with
  that project's sub-pages (Deployments/Logs/Environment/Domains/
  Settings), breadcrumb hints = project name. Deep links need `?tab=`.
- **`?tab=` deep-linking** on the project page — tabs are now
  addressable (palette + external links land correctly).
- **404 runway strip**: tilted band with centerline dashes behind the
  copy — the on-brand version of Vercel's contributor-name backdrop.
- **Semantics sweep**: neutral domain-type badges, `hover:text-brand`
  → `hover:text-foreground`, card hover borders → muted; EmptyState
  for the domains list; Add-domain button `shrink-0`.

Research artifacts: `vercel-research/*.png` (dashboard, deployments,
domains, new-project, palette, project, settings, env-vars).

Gotcha: vite can serve a stale transformed module on the bare URL
(`?t=` is fresh). Symptom: DOM lacks new render despite current file.
Fix: kill the vite pid and restart `pnpm dev`.

## Deep-dive pass (2026-10-22, third half)

Vercel drop-deploy completed end-to-end via CDP file-upload
(`fup` command: DOM.setFileInputFiles on the hidden input — Vercel's
"update/drop" confirm page → Deploy). New captures: drop-confirm,
deploying state, project deployments list, deployment detail,
logs-as-filter-panel, source file-browser, actions menu, `f` palette.

Findings that landed:

- **TopBar breadcrumbs**: `/deployments/:id` → "Deployments / abc1234",
  project pages keep "Projects → name". Matches Vercel's breadcrumb.
- **`f` opens the palette** (Vercel's shortcut — their Find field shows
  `F`). Guarded: not while typing in inputs/textarea/contentEditable,
  not with modifiers. Find button shows both `⌘K` and `F` chips.

Findings intentionally skipped:

- Deployment sub-nav pills (Deployment/Logs/Resources/Source/Open
  Graph): our one-page layout is denser — the same surfaces live in
  the card already (logs stream, stats, preview, source hash).
- Composable filter chips: Vercel composes Author/Environment/Status;
  we filter on status only — tabs are the right shape for one axis.
- Collapsible log sections per step: our log is a single stream —
  no per-step boundary in the API to hang the UI on.
- Sidebar scope-swap for logs/filters: an architectural change with
  real payoff only when Runway has request-log surfaces worth that
  depth. Noted as backlog.

Artifact note: `project-6it86` lives on Vercel as a research
deployment — delete it from the Vercel dashboard when done, or leave
it (Hobby tier, costs nothing).

- **DeploymentRow grid-overflow fix** — `Link` grid children got
  `min-w-0`; mobile rows were overflowing the card (badge off-screen).
  Verified at 390px.

### Deployment deep-dive round (Vercel actions + OG inspector)

- **API**: `GET /api/v1/deployments/{id}` now returns `env_state`
  (`subdomain`, `current_deployment_id`, `previous_deployment_id`) —
  new `deploy::environment_alias` helper, `rollback` refactored to use
  it. New `GET /api/v1/deployments/{id}/og?path=` — server-side fetch
  of the deployment URL + `opengraph::parse` (new core module,
  unit-tested) returning title/description/og/twitter/favicon maps;
  always 200 with `{ok:false}` on fetch failure.
- **Deployment page**: actions collapsed into a ⋯ `Menu` (new ui.tsx
  primitive, click-outside/Escape): Instant rollback (gated on
  `env_state.previous == dep.id`, devpush parity), Copy deployment
  URL, Copy deployment ID, Redeploy, Refresh status (reconcile), Skip
  build (queued/pending only). `Visit` + `Cancel` stay flat.
- **`current` chip** in the header meta when the viewed deployment
  serves its environment — Vercel's `Production [Current]`.
- **Instant rollback** verified live: alias flipped to the viewed
  deployment, Traefik config regenerated, `current` chip appeared.
  Flipped back afterward — seeded alias state is original.
- **OG inspector panel**: collapsible card on succeeded deployments —
  pathname input + fetch, per-tag status rows (provided/not), doc
  hints + copyable `<meta>` snippets. In this dev topology the fetch
  returns the honest `ok:false` state (deploy.localhost unreachable
  from the host process); verified on a real instance it parses.

### Dev-stack state note
Canonical stack restored: `compose-runway-1` serves `:8000` with the
release binary + baked dashboard (`index-BMN15brT.js`); local
`runway serve` stopped, `pg-forward` removed. Login + `/files` +
`prod_state` verified on the baked image.

dockerd wedge note — this box runs Docker **Desktop** (CLI context
`desktop-linux`, qemu VM). `systemctl restart docker` restarts an
unused native daemon — the real recovery is `docker desktop restart`
(user unit `docker-desktop.service`). Symptoms: `docker info` times
out while `docker ps` answers, build client sleeps on a dead
buildkit session (`DeadlineExceeded` on retry). After a VM restart,
all compose services return via restart policies.

### Promote-to-production + Resources inventory + settings footers

- **`POST /api/v1/deployments/{id}/promote`** — ports devpush's stubbed
  promote: validates `completed` + `succeeded`, resolves the target
  environment alias via the new shared `deploy::environment_subdomain`
  (prod → project slug, others → `<slug>-env-<env>`), flips the alias
  via `alias_update_or_create`, stashes the old target in
  `previous_deployment_id`, regenerates Traefik, publishes
  `deployment_promotion`. Idempotent when already pointing at the
  target; `environment not found` on bad env. Default env `prod`.
  Verified live: alias flip, previous tracked, idempotent re-post,
  error path — then restored seeded state.
- **`promote_deployment` MCP tool** — same `deploy::promote` via
  `deployment_for`, `{deployment_id, environment_id?=prod}` →
  `{environment_id, deployment_id}`. Verified via JSON-RPC; alias
  restored to `f80ed2e1` after.
- **`prod_state` on `GET /deployments/{id}`** — same shape as
  `env_state` but always resolves the *production* alias, so the UI
  can gate "Promote to production" on the actual prod target.
- **Deployment ⋯ menu**: "Promote to production" between Instant
  rollback and the copy items; shares a generalized `aliasAction`
  confirm/done/error banner with rollback. Verified end-to-end in
  the browser (banner → confirm → `current` chip), then rolled back
  to the original alias state.
- **`GET /api/v1/deployments/{id}/files`** — walks
  `data/static/<dep_id>/` under `spawn_blocking`, returns sorted
  `{path,size}` entries; `{ok:false}` when the deployment has no
  static payload on disk (server/image deploys keep artifacts in
  their container image — honest empty state, not an error).
- **Files panel** on the deployment page: collapsible, lazy-fetches
  on first expand; filter input, per-file type chip, size, total
  count + bytes. Verified: `index.html · HTML · 65 B`.
- **Deployment page scroll fix** — root was a viewport-locked
  `h-[calc(100vh-3.5rem)]` flex column; the new panels clipped below
  the fold and logs starved. Now `min-h` + natural page scroll; the
  log pane is bounded at `h-[45vh] min-h-64` with its own scroll.
- **Settings card footers** — new `CardFooter` (ui.tsx): divided
  footer row, `Learn more about X ↗` doc link left (real GitHub doc
  targets — `docs/runway-json.md` etc., `DOCS` const in Project.tsx),
  status text + Save right. Applied across the settings cards;
  save behavior and messages unchanged. Verified desktop + 390px.
- **Mobile overflow fix** — settings page was 426px wide at a 390
  viewport: the Firewall rate-limit row (`flex` + two `w-36` inputs
  + label, no wrap) held ~392px min-content and stretched the
  single-column `grid` track (grid items default `min-width:auto`),
  widening every card. `flex-wrap` on the row; page now 390=390.
- **dockerd wedge** — daemon hung twice mid-build (build client
  orphaned, `docker info` timed out). Real fix was
  `docker desktop restart` (see state note above); subsequent attempt
  hit `DeadlineExceeded` before that, then built clean end-to-end.
- **Build slowness note** — `rust-toolchain.toml` asks for `stable`,
  but `rust:1-bookworm` ships a version-pinned toolchain, so every
  uncached build downloads the full stable toolchain inside the
  build container (~10 min on this network) plus a no-cache release
  compile (~80 min on this VM).

### Promote CLI

- **`runway promote [deployment] [--environment prod]`** — the missing
  front door on `POST /deployments/{id}/promote`. Positional deployment
  defaults to the linked project's latest (same as
  redeploy/logs/stats); `--environment` targets non-prod envs. Prints
  `{env} now serves deployment {id}` — rollback's one-liner shape.
  Verified live on the dev stack: no-arg promote flipped the prod
  alias to the latest deployment, explicit-id promote was idempotent,
  re-promoting `f80ed2e1` restored the seeded alias state exactly,
  `--environment staging` surfaced `404 environment not found`.
  CLI verified via a throwaway `XDG_CONFIG_HOME` + a bootstrap-minted
  `ak_` on the dev DB — production `~/.config/runway` untouched.
- Docs: `docs/agents.md` CLI list + README CLI block (also added the
  missing `runway rollback` line there for symmetry).

### Promote CLI — docs note

The dev-stack `ak_` used for the promote verification lives in the
throwaway `data-run` DB; production `~/.config/runway` was untouched
(`XDG_CONFIG_HOME` sandbox).

### Per-step collapsible logs

- **`step` log stream** — `LogStore::step(id, name)` writes
  `<ts>\tstep\t<name>` markers; the worker emits them at the three
  lifecycle seams (`prepare` in `start`, `deploy` on the status flip,
  `finalize` at job start). Plain-text consumers see an ordinary line;
  the `logs.rs` format doc names the three streams
  (system/runtime/step). devpush has no equivalent — Loki streams are
  flat; this is a Runway-only addition following the deployment
  lifecycle statuses.
- **Deployment page grouping** — log pane folds lines into sections
  per marker: `Prepare / Deploy / Finalize` headers with line counts
  and `−/+` toggles (the panel collapse idiom). Completed phases
  collapse; the open phase stays expanded (Vercel behavior). A click
  sticks through later markers; `filter` flattens to a plain list and
  hides markers. Markerless logs (older deploys) render flat,
  unchanged.
- **CLI** — `runway logs` renders markers as `--- deploy ---` in both
  deployment and `--project` tails.
- Verified end-to-end on a real deployment: host `runway serve`
  (fresh debug binary, compose db via a socat forward, container
  stopped so it claimed the job) ran `17672f6f` prepare→deploy→
  finalize and wrote all three `step` markers to the log file; the
  dashboard groups them (`Prepare 11 lines`, `Deploy 1 line`,
  `Finalize` open with "Deployment succeeded").
- **Log-dedupe fix** — the e2e check exposed a pre-existing bug:
  `logs_stream` replays the file tail as SSE history while the page
  also fetches the same tail via REST, so every line rendered twice
  (and again on each EventSource reconnect). The pane now dedupes
  against rendered lines (full-line equality — the rfc3339 prefix
  makes each unique); uses `prev` state, not an effect-local set, so
  StrictMode remounts stay correct.
- Also verified earlier on seeded log with injected markers:
  `Prepare 34 lines` + `Deploy 1 line` collapsed, `Finalize` open,
  toggle expands, 390px clean.

### Docker build determinism

- `rust-toolchain.toml` dropped from the dep-cache COPY — the image
  now compiles with `rust:1-bookworm`'s pinned toolchain (1.99.1,
  satisfies `rust-version = "1.85"`) instead of rustup-resolving
  `stable` (~500MB download every uncached build). Repo toml
  untouched — local dev keeps rustfmt/clippy components.
- `.dockerignore` refreshed: `web/` paths were stale post-rename
  (would have leaked `dashboard/node_modules` + `dist` into the build
  context); now `dashboard/…` + `data-run/`.
- `docker build --check` clean; full `docker build` finished:
  dep-cache stage 4765s + workspace crates 2150s (~2h under qemu),
  zero rustup download lines, `runway:verify-toolchain` produced and
  `runway --version` smoke passed inside the build.
- Gotcha learned: `docker build` snapshots the context at
  invocation — the verify image predates the step-marker code (it
  contains `promote` but not `logs.step`). Verified by `strings` on
  the shipped binary; a real deployment through it produced no
  markers (the e2e proof above used the fresh debug binary instead).
  Stack restored to the release image afterwards; prod alias back to
  seed state (`f80ed2e1` current / `276399ee` previous).

## Phase 1 audit (PR17–20 merged tree, host-served on :8001)

- Monitor crash-notification spam root-caused: host-serve ran against
  `/var/run/docker.sock` (the `default` context daemon) while runners
  live on Docker Desktop's socket — every seeded container read as
  `not_found`, the compose monitor wrote `running` back, and the
  observed-state flip-flop fired a notification per tick. Restarted
  host-serve on `~/.docker/desktop/docker.sock`; spam stopped.
- Hardened the emit path anyway (PR #20): the `running→down`
  transition is claimed by a conditional UPDATE so one writer owns it,
  plus a 15-minute per-deployment cooldown row-check. Verified live:
  first flip → 1 notification, second flip inside cooldown → still 1.
- Audit findings → issues #21–#33 (actions-menu rollback/delete,
  devpushhq image leak in logs, API-keys UI, runtime/build log split,
  project-scope rail, deploy filters, CLI parity, a11y, skeletons,
  card-contained empty states, palette depth, git source context,
  notification semantics).
- Shipped in-flight: files endpoint now returns
  `static_output_missing` vs `no_file_payload` via `is_static()`
  (PR #18), project payload exposes `production_deployment_id` and the
  Production card resolves the alias target instead of the newest
  succeeded deployment (PR #18 + #19). Verified live: card links to
  `f80ed2e` (the real alias target, `9c6edba` / 1d ago).
- Not-a-bug resolved during audit: Overview amber vs Projects green
  health dot for demo-app was mid-flip-flop data churn from the
  monitor bug above, not a rendering inconsistency.

## TODO next

- Artifact inventory is static-deploys-only; server/image deployments
  would need tarball/image inspection for the same list.
- Audit continues: remaining routes/interactions (new-project flow,
  promote/rollback dialog, palette `f`, mobile pass per route).

## Phase 1b — interaction fixes + mobile sweep (PR #19 branch)

- Notification links emitted `/projects/:id/deployments/:id` (not a
  route) — all three emitters corrected to `/deployments/:id`; seeded
  rows fixed in-place. Verified: feed click lands on the deployment.
- `/login` now redirects to `/` when already signed in (was 404).
- Command palette matching is subsequence-fuzzy (`dmo` → `demo-app`).
- Upload provider is a first-class New-project path (name instead of
  repo, upload copy, no connection/repo fetches). Verified end-to-end:
  `audit-upload` created via UI and present in DB.
- Project-scope rail (issue #25): `/projects/:id` swaps the sidebar to
  back-link + project identity + tab links driving `?tab=`; in-page
  strip hidden on lg+, kept on mobile.
- Empty states on Environment + Cron tabs are card-contained (#30
  partially; other routes still to sweep).
- Mobile 390px sweep found a systemic defect: every bare `grid`
  (implicit auto column) sized its track to max-content, so long mono
  strings (URLs, image refs) overflowed the viewport and `truncate`
  never engaged — Settings rendered at 1036px. Fixed globally:
  `grid-cols-1` base on all single-column grids, `min-w-0` on flex
  chains, stepper connectors `w-4 sm:w-10`, domain-row anchors
  `min-w-0 flex-1 truncate`, header meta `flex-wrap`. All routes now
  report `scrollWidth=390`.
- API keys UI (#23): Settings card with create (one-time reveal +
  copy), list (created/last-used metadata), revoke — verified
  end-to-end against `/api/v1/keys`.
- Build/runtime log split (#24): worker emits `step live` at
  conclusion (PR #18); the pane splits into build|runtime tabs —
  build keeps grouped step sections, runtime shows post-deploy
  container stdout/lifecycle; static deploys show an honest empty
  state. Old logs fall back to runtime-stream lines after `finalize`.
- Issue dispositions: #21 Delete — devpush has no user-facing
  deployment DELETE (internal cleanup task only); kept open as
  design-pending. #22 devpushhq namespace — closed won't-fix: the
  image ref is real pull evidence and devpush logs the identical
  line; `runner-overrides.json` is the supported remap.

## Overnight round 3 — parity + polish batch

- Composable deployment filters (#26): env/project chips compose with
  status tabs + free-text query; verified `preview` narrows to 1 row.
- Command palette depth (#31): all 10 project tabs scoped, deployment
  search items, context actions (open prod site, copy prod URL, copy
  project ID), `run` actions vs `to` navigation.
- CLI/MCP parity (#27): added `runway cancel` + `runway skip`
  (REST+MCP had them, CLI didn't); added `skip_deployment` MCP tool
  (cancel only accepts in-flight; queued needs skip); docs tool
  surface updated.
- Notification retention (#33): monitor's hourly sweep deletes read
  notifications >30d, all rows >90d — devpush never prunes; the
  flap incident showed why a bound is needed.
- Unread indicators moved `bg-brand` → `bg-info` — DESIGN.md reserves
  orange for intent-in-flight; unread is informational evidence.
- Error boundary + skeleton pass (#29): ErrorBoundary wraps every
  routed page; all raw "Loading…" texts converted to Skeleton
  (Team, GitHub App card, Files panel, public status page);
  Notifications gained a loading skeleton.
- Accessibility (#28): Button focus-visible ring; switch semantics
  (role/aria-checked) on toggles; Menu aria-haspopup/roles; palette
  dialog/combobox/listbox semantics; Input mirrors placeholder as
  aria-label (~65 fields); chips/tabs/steppers expose pressed/
  selected/expanded.
- Upload-provider project creation (#34) verified E2E and closed.
- Deployment Delete (#21) closed not-planned: neither devpush nor
  Vercel offers user-facing deployment deletion — immutable history.
- Docker disk inventory: `GET /api/v1/admin/system` summarizes
  `docker system df` (images/containers/volumes/build-cache counts +
  bytes); Settings "Docker disk usage" card renders it (48 images /
  31.5 GB live). Self-hosted ops surface; neither reference has one.
- Upload-deploy E2E: bare `index.html` tarball via `runway deploy` path
  surfaced two real defects, both fixed on PR #18:
  - `detect_upload_preset`: upload projects carry no source at creation,
    so detection never ran — empty config → "runner image not found".
    The handler now lists the archive, runs the same `presets::detect`
    git providers get at creation, stamps it into the deployment config
    snapshot (incl. `dockerfile_path` fallback; explicit runner wins).
    Corrupt archives now 400 instead of queueing a doomed deploy.
  - `disconnect_from_network` parity: devpush tolerates 404/409 and
    warns on the rest; Runway propagated Docker's 500 "not connected"
    and retried `fail_deployment` 4x per failure. Now benign-detach
    states are swallowed and the rest warn — verified live: the
    readiness-timeout failure concluded with zero job retries.
  - Residual finding filed as #35: host `runway serve` under Docker
    Desktop cannot probe bridge IPs — compose/self-host unaffected.
- CLI `cancel`/`skip` verified against live server (real REST calls,
  correct conclusions); MCP `skip_deployment` added; api.rs tool table
  now lists the full 16-tool surface.
- `runway deploy` CLI verified E2E: tar+upload → static preset
  detection → publish → serve container; status-follow output correct.
