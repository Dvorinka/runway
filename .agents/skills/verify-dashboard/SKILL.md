---
name: verify-dashboard
description: Run a seeded, sign-in-able Runway dashboard for this checkout and prove a UI change in a real browser. Use after changing dashboard/ UI, routes, or API responses the dashboard renders, when asked to verify, screenshot or click through Runway, or when a fresh worktree needs the dashboard running.
---

# verify-dashboard

## Steps

1. `scripts/verify/up.sh` (add `--fresh` for a clean db). Waits healthy,
   bootstraps `dev@runway.local` / `runway-dev-pass`, seeds project
   `demo-app`. Prints the URL and credentials.
2. Open `http://localhost:8000`, sign in, reach the changed route.
3. Prove the change in a real browser: click the path the change alters
   and capture a screenshot. State what the evidence shows — never
   "looks right" without a screenshot or observed behavior.
4. For deploy-flow changes: `runway link` inside a sample repo, then
   `runway deploy --follow`, and verify the row/status the change affects
   on the dashboard and via `runway deployments` — UI and CLI should tell
   one story (DESIGN.md bet 5).

## Rules

- A dashboard change isn't verified until the browser shows it.
- `data-run/` is throwaway; `data/` is not. Never point seeds at `data/`.
- Down with `docker compose -f compose/development.yml down` (`-v` wipes).
