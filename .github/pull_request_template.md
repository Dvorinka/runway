## Summary

<!-- What does this change and why? Link the issue it closes. -->

## Type

- [ ] Bug fix
- [ ] Feature
- [ ] Refactor / cleanup
- [ ] Docs
- [ ] Chore (deps, CI, release)

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd web && pnpm build` (when `web/` changed)
- [ ] Migrations are numbered, idempotent, and additive
- [ ] No secrets or `.env` committed
- [ ] ROADMAP.md updated if scope or status changed
