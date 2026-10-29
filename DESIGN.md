# Runway — platform bets

Numbered, named bets. PRs name each bet they touch and the choice it
changed. Cross a red flag and argue it here first. UI bets live in
`web/DESIGN.md`; vocabulary in `CONTEXT.md`.

## Bet 1 — Single binary

API, workers, and CLI ship in `runway`. Workers are tokio tasks, not
processes. Postgres is the queue (`SELECT ... FOR UPDATE SKIP LOCKED`);
no broker, no Redis. *Flag: any second process is a new bet, not a
default.*

## Bet 2 — Port, don't redesign

`devpush` (`~/Desktop/PROG+HTML/devpush`) is the reference spec. Schema,
deployment lifecycle (`prepare → deploy → finalize → completed`), alias
semantics, and edge cases mirror it unless a documented reason exists.
The rewrite buys Rust, not invention.

## Bet 3 — Frontend-first product

When two designs are equal, pick the one that serves static/SSR frontend
deploys better. Runway is the Vercel half of the pair; Ployz owns
long-running services. Don't grow Ployz's domain.

## Bet 4 — Evidence over claims

Status reports what was observed, never what was desired. Health checks
write `unhealthy` on probe failure; a deploy reports its real phase. If
evidence is missing or stale, the UI says so rather than guessing.
*Flag: no derived "looks fine" states.*

## Bet 5 — One contract, three surfaces

CLI, REST API, and MCP serve the same operations. A new capability
lands on all three or it's unfinished. `ak_` keys are the agent surface;
the dashboard is a client, not a backdoor.

## Bet 6 — Owned edge, boring internals

Traefik routes, Cloudflare tunnels optional, ACME or sidecar TLS. The
edge adapts to the host; the core doesn't. Traefik configs write
atomically (temp + rename).

## Bet 7 — Companion to Ployz, not a fork of it

Shared vocabulary (CONTEXT.md aligns terms where domains overlap),
shared design rules, shared agent workflow (verify harness, landing
docs). Divergence needs a reason written here.
