# Vercel research — 2026-10-22 (authed session, vercel.com/td-vorak)

Browser: dedicated X11 Chrome, profile `~/.config/chrome-inspect`, CDP :9222.
Session was seeded by sqlite-backup of the main profile's cookie store.

## Sidebar
- Team chip top: avatar + name + plan badge ("Hobby"), chevron switcher.
- Find field shows `F` key hint (kbd chip), not ⌘K.
- Expandable nav groups carry a right chevron (Logs, Observability,
  Firewall, Agent, AI Gateway, Sandboxes, Workflows, Images, Daemons).
- Bottom cluster after divider: Usage, Support, Settings — then user
  chip row: avatar + name, `⋯` menu, notification bell.
- **Project scope**: inside a project the rail's items swap to
  project-scoped (Overview, Deployments, Logs, ..., Settings). The
  top-left chip becomes project avatar + name. No tab bar — the rail
  items ARE the tabs. Settings goes one level deeper: rail shows
  "‹ Settings" back-link + sub-nav list.

## Top bar (persistent, every page)
- Left: scope selector ("All Projects" or project name).
- Center: page title ("Overview", "Domains", "New Project").
- Right: `+ Create New` split + `Agent` button.
- Runway gap: no persistent top bar; page titles live in content.

## Page content patterns
- Toolbar row: filter icon button + search input left, action buttons
  right ("Connect External", "Transfer In", ink-primary "Buy").
- Filter chips: dashed-border pills, `Name value` (Author / Environment /
  Status) — composable, not tabs.
- Empty states sit INSIDE the list card (hairline border, rounded),
  icon-in-bordered-box + title + subtext + optional "Learn more ↗" docs
  link under the subtext.
- 404 page: giant 404, "This page doesn't exist.", `Return to Team`
  pill — background is faint repeated contributor names (charm detail).

## Project Overview
- "Production Deployment" card: header label + `Connect Git` + `⋯`.
  Empty state inside = drag-and-drop deploy zone.
- **Hint strip** under the card: hairline bar, muted text
  "To update your Production Deployment, drop your project anywhere on
  this page, connect to git, or run `vercel --prod` via the CLI." +
  Deployments link button + book icon. ← cheap, charming, teaches CLI.
- "Production Checklist 0/5" collapsible card: rows
  (icon + action label): Connect Git Repository / Add Custom Domain /
  Preview Deployment / Enable Web Analytics / upgrade row.
- "Observability 6h" card: label rows + inline chart areas, `>` expand.
- Analytics card: enable-CTA empty state.

## New Project (`/new`)
- Full-page takeover, sidebar hidden. "Let's build something new"
  headline, v0 prompt input + suggestion chips, drag-drop note,
  "file or folder" links.
- Two columns: Import Git Repository (provider buttons full-width:
  GitHub/GitLab/Bitbucket/Origin + Manage Login Connections link) and
  Build your solution (template cards + Filter + Browse All).
- Footer link: "Create an empty project" for analytics-only.

## Project Settings
- Each section = card: Title + one-line description + control(s) +
  **divided footer** with "Learn more about X ↗" left, `Save` right.
- Project ID row: mono value + copy button.

## Find palette
- Anchored dropdown under the sidebar Find button — NOT a centered
  modal. Esc chip inside input.
- Items: context-aware (project-scope "Deployments project-6it86"
  first inside a project), breadcrumb subtitles
  ("project-6it86 / Functions / Settings"), a "Navigation Assistant"
  natural-language suggestion row.

## Deployment detail — round 2 deep-dive

### Actions menu (`···` top-right)
- Instant Rollback · Promote · Redeploy · Copy URL · Logs ▸ ·
  Skew Protection Threshold ▸ · Delete (red).
- `Visit` is a split button (Visit + "Visit with" variants).
- Meta grid marks the live deployment: `Environment: Production [Current]`.

### Collapsible overview rows
Deploy Logs ✓ · Deployment Summary ✓ (Resources) · Deployment Checks ·
Assigning Custom Domains ✓ · then shortcut rows (Runtime Logs,
Observability, Speed Insights, Web Analytics) — each a `>` expander
with a status indicator.

### Deployment Settings panel (collapsed by default)
Recommendations (concurrent builds, skew protection, custom domain) +
Build Settings (machine size, prioritize prod) + Runtime Settings
(fluid compute, function CPU, Node version, deployment protection,
skew protection, cold-start prevention, function region).

### Resources tab
Deployed-artifact inventory: Path | Type | Size table
(`index.html  HTML  282 B`). "Press / to focus search".

### Source tab
Deployed-artifact file browser (tree + viewer), not the git repo.

### Open Graph tab
Live meta-tag inspector: Pathname input (+ Suggested Pathnames from
deployed assets), each tag as a card — status (Provided/Not Provided),
one-line doc hint, copyable `<meta>` snippet.

## Transfer list → Runway
Done this session:
- [x] CLI hint strip under project production card
- [x] Palette: context-aware ordering + breadcrumb subtitles
- [x] Preflight checklist card (Vercel's Production Checklist)
- [x] Persistent top bar (title + breadcrumb + New project)
- [x] `?tab=` deep links (sub-pages as addressable URLs)
- [x] 404 backdrop motif (runway strip, their contributor names)
- [x] Deployment actions ⋯ menu (Instant rollback / Copy URL / Copy ID /
      Redeploy / Refresh / Skip) — their Instant Rollback · Promote ·
      Redeploy · Copy URL · Logs ▸ · Delete
- [x] `current` chip on the deployment serving its environment
      (their `Production [Current]`); `env_state` on GET /deployments/{id}
- [x] OG inspector panel (collapsible; pathname + fetch + per-tag
      status/copy rows) — their Open Graph tab
- [x] `f` palette shortcut (their `F` on Find)
- [x] Promote to production — `POST /deployments/{id}/promote` +
      `prod_state` on the GET; ⋯ menu item with confirm banner
      (their menu order: Instant Rollback · Promote · Redeploy ·
      Copy URL · Logs ▸ · Delete); `runway promote <id>` CLI
      (`vercel promote <url>`)
- [x] Resources/artifact inventory — `GET /deployments/{id}/files`
      walks `data/static/<id>`; Files panel on the deployment page
      (Path | Type | Size + filter + totals), honest empty state for
      non-static deploys
- [x] Settings card footer — `CardFooter` primitive: divided row,
      "Learn more about X ↗" left, Save right (their settings cards:
      content · divided footer · doc link · save)
- [x] Per-step collapsible logs — `step`-stream markers at
      prepare/deploy/finalize seams (devpush has no equivalent; the
      lifecycle statuses provide the seams); log pane groups +
      collapses per phase, completed collapse, last open. Verified
      on a real deployment; also fixed SSE-history + REST-tail
      double-render it exposed
- [x] Production card resolves the live alias target — their
      "Production Deployment" card shows what the alias serves, not
      the newest deploy; project payload exposes
      `production_deployment_id`; falls back to newest concluded
      pre-alias. Verified against seeded alias state
- [x] Files panel honest empty states — distinguishes
      `static_output_missing` (artifact gone) from `no_file_payload`
      (server deploys keep artifacts in the image)

Audit deltas filed as issues (#21–#33):
- Actions menu still missing Instant Rollback + Delete (#21)
- devpushhq image namespace leaks into user-facing logs (#22)
- API key management UI (REST `/api/v1/keys` exists) (#23)
- Runtime vs build log split — theirs are separate tabs (#24)
- Project-scope sidebar swap (#25)
- Composable deployment filters (#26)
- CLI parity inventory (#27)
- a11y pass (#28), skeleton/error-boundary consistency (#29),
  card-contained empty states (#30), palette fuzzy depth (#31),
  git-provider source context (#32), notification semantics (#33)

Backlog candidates:
- Composable filter chips on Deployments (vs tabs)
- Project-scope sidebar swap (rail items become project-scoped)
- Resources for server/image deploys (needs tarball/image inspection)
