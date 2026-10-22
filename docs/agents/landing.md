# Landing changes

How a change gets verified and lands. GitHub CI is the PR gate.

## Before opening the PR

Run only focused checks locally — the gates CI will run:

```sh
cargo fmt && cargo clippy --workspace -- -D warnings   # rust touched
cargo test <changed-area>                            # rust touched
cd dashboard && pnpm build                           # dashboard touched
sqlx migrate run                                     # migration touched
```

Don't run full suites speculatively; reproduce CI failures, don't
pre-empt them.

## Proving it works

- Dashboard UI, routes, loaders, API shapes the UI renders → run the
  `verify-dashboard` skill; prove it in a real browser with a
  screenshot, not a guess.
- Deploy lifecycle, Traefik writes, tunnels, Docker builds → verify
  against the devpush behavior first, then a real container build.
- Schema → `sqlx migrate run` against local Postgres; migrations are
  numbered and never edited once applied.

## The PR

- Conventional commits: `feat(api): …`, `fix(worker): …`.
- Body names each `DESIGN.md` bet the change touches and the choice it
  changed; crossing a flag means arguing it in `DESIGN.md` first.
- Terms come from `CONTEXT.md`; UI copy and structure follow
  `dashboard/DESIGN.md`.
- User-visible behavior changed → update `docs/user/` in the same PR.

## The gate

CI green, then merge to `main`. Nothing bypasses CI.
