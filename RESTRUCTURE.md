# Runway × Ployz — restructure plan

Runway positioned as the frontend-deployment half of the pair: Ployz runs
your services (Railway-shaped), Runway ships your frontends (Vercel-shaped).
Same conventions, same workflow, same UI discipline — different domain.

## What Ployz does that Runway should adopt

| Ployz convention | Purpose | Runway status |
|---|---|---|
| `core/DESIGN.md` numbered **bets** | Every non-obvious architectural choice named once; PRs reference the bet they touch | ✅ added — `DESIGN.md` |
| `CONTEXT.md` **glossary** | Canonical terms; no synonyms across code, docs, UI | ✅ added — `CONTEXT.md` |
| `dashboard/DESIGN.md` **design system** | North Star, token table, named rules ("Visible Intent", "Semantic Honesty"), per-component contracts | ✅ added — `dashboard/DESIGN.md` |
| `docs/user/` shipped-behavior docs | Docs describe what *shipped*, not intent; updated in the same PR | 📋 phase 2 |
| `scripts/verify/up.sh` **seeded harness** | One command → running, signed-in, seeded dashboard for browser verification | ✅ added — `scripts/verify/up.sh` |
| `.agents/skills/` verify + landing skills | Agents prove changes on the real artifact | ✅ added — `.agents/skills/verify-dashboard/` |
| `docs/agents/landing.md` PR workflow | How PRs get verified and landed | 📋 phase 2 |
| Two-tier naming (`core/` ↔ `dashboard/`) | Product halves are legible from the root | ✅ done — `web/` → `dashboard/` |

Already matching Ployz discipline: `AGENTS.md` principles-as-rules,
conventional commits, CLI/API/MCP as one contract (`ak_` keys, OpenAPI,
`/api/mcp`), health checks reporting *observed* state (Ployz's "evidence
over claims" bet, already shipped), Postgres as the only queue.

## UI direction — "same UI as Ployz, positioned like Vercel"

Decision recorded in `dashboard/DESIGN.md`; the summary:

- **Adopt Ployz's rulebook wholesale** — Neutral Action, Visible Intent,
  Semantic Honesty, Structured Flatness, Product Scale, Evidence Type.
- **Keep Runway's register**: dark-only product UI (like Vercel's
  dashboard), `#ff4d00` brand orange taking the role Ployz's pink takes —
  the single chromatic signal for staged intent and focus. Logo untouched.
- **Adopt Vercel's shell**: left sidebar with grouped nav + section
  chevrons, team-switcher chip top-left, `⌘K`/`F` command palette entry in
  the rail, breadcrumb bar under the header, centered empty states
  (icon-in-rounded-box + heading + muted subtext + action rows).
  Reference: `vercel-screenshots/01-dashboard.png`.
- **Adopt Ployz's vocabulary** where domains overlap: Deployment states
  are evidence words; config diffs are the same "staged" treatment
  Runway's env editor already gestures at.

## Phases

**Phase 1 — conventions (this commit).** `DESIGN.md`, `CONTEXT.md`,
`dashboard/DESIGN.md`, `scripts/verify/up.sh`, `.agents/skills/verify-dashboard/`.
Zero code movement.

✅ **Phase 2 — layout (done).** `web/` → `dashboard/`; `docs/user/` seeded from
README behavior sections; `docs/agents/landing.md` written; Dockerfile,
`compose/*.yml`, `install.sh`, CI, and `AGENTS.md` path pointers updated
in one commit. ~6 touchpoints, mechanical.

**Phase 3 — shell + tokens.** Port `index.css` to the `dashboard/DESIGN.md`
token table; rebuild `Layout.tsx` as the Vercel sidebar shell; empty-state
component (icon box + heading + subtext + action list) used on
Projects/Deployments/Notifications.

**Phase 4 — workflow parity.** Deployment detail as the Ployz
inspector-equivalent: status word from evidence + staged diff treatment
for env/config changes; `runway:` keyboard layer already present
(⌘K palette) — extend to `G D`/`G P` navigation.

Not doing: renaming crates or the REST surface (parity with devpush is a
bet, churn for aesthetics isn't), light mode (product is dark; marketing
site is a separate surface), touching the logo.

## Proposed — project-scope rail swap (not yet implemented)

Vercel's rail is scope-aware: inside a project, the global items swap for
project-scoped ones (Overview, Deployments, Logs, …, Settings) and the
top-left chip becomes the project. Runway's rail is static today; the
project tab bar (`?tab=` deep links) carries the scope instead. This is
the last big shell difference from the research (`FINDINGS.md` sidebar).

**Proposal.** `RailNav` reads `useLocation()`; on `/projects/:id` it
renders a "‹ Projects" back-link + the project name (fetch once, like
`TopBar` does) followed by the TABS as rail items linking to
`/projects/:id?tab=X`. Active state reads the `tab` param — same deep
links, no route changes. Find, the wordmark, and the account row stay
global at all times.

- **In-page tab bar**: hide on `lg+` where the rail is the tab bar
  (Vercel: no duplicate); keep below `lg` — the drawer is one tap away
  but the strip is the faster path on touch.
- **Deployment pages** (`/deployments/:id`): Vercel keeps the rail
  project-scoped there too. Runway's route is global; adopting the
  deployment's project needs a `deployment → project` fetch in the rail
  (TopBar already does this lookup for its title). Phase 2 of this
  change — the basic swap lands first, deployment scoping follows.
- **What this buys**: the project becomes the working context (its ten
  tabs visible at all times, one click apart) instead of a page-level
  strip below the fold; matches the palette's existing scope-aware
  ordering.

**Bets touched.**

- `dashboard/DESIGN.md` Shell section — "grouped nav" gains the
  scope-swap rule: inside `/projects/:id` the rail IS the tab bar.
- Bet 7 (companion to Ployz) — Ployz's rail is static; this diverges
  toward Vercel's dynamic rail on the evidence that project work is the
  dominant mode. Argued here before implementation per the rule.
- Untouched: bet 5 (no API surface change — `?tab=` links are the whole
  mechanism), bet 4 (no status semantics change).

**Open questions.**

- Does the rail keep section chevrons for the ten tabs, or a flat list
  (Vercel uses flat)? Lean flat — ten items fit without grouping.
- Notifications badge during project scope — global item gone from the
  rail; the badge could ride the account row or Find instead. Lean:
  keep a slim global cluster (Find + Notifications + back to
  "All Projects") under the scope switcher.
