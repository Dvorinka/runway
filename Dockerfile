# Runway — single binary: API + workers + CLI.
# Build:  docker build -t runway .
# Run:    see compose/development.yml for a full stack.

FROM node:22-bookworm-slim AS dashboard
WORKDIR /src/dashboard
RUN corepack enable
COPY dashboard/package.json dashboard/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY dashboard/ ./
RUN pnpm build

FROM rust:1-bookworm AS build
WORKDIR /src

# Cache deps before copying sources. rust-toolchain.toml stays out of
# the image on purpose: it asks rustup for `stable` (a ~500MB download
# per uncached build) while rust:1-bookworm already ships a pinned
# toolchain that satisfies rust-version = "1.85". Local dev keeps it
# for the rustfmt/clippy components.
COPY Cargo.toml Cargo.lock ./
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
COPY crates/api/Cargo.toml crates/api/Cargo.toml
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/worker/Cargo.toml crates/worker/Cargo.toml
RUN mkdir -p crates/cli/src crates/api/src crates/core/src crates/worker/src \
    && echo 'fn main() {}' > crates/cli/src/main.rs \
    && echo '' > crates/api/src/lib.rs \
    && echo '' > crates/core/src/lib.rs \
    && echo '' > crates/worker/src/lib.rs \
    && cargo build --release -p runway 2>/dev/null || true

COPY crates crates
COPY migrations migrations
# Embedded by include_str! in the API (node enrollment installer).
COPY scripts/node-install.sh scripts/node-install.sh
# COPY preserves checkout mtimes (often older than the stub build above),
# and `cargo clean -p` no longer invalidates the workspace units (verified:
# clean reports "Removed 0 files" and the next build is a silent no-op that
# keeps the stub binary). `touch` the real sources so mtime comparison
# forces a rebuild; third-party deps stay cached. The size + --version
# gates fail the build instead of shipping a stub that exits 0 silently.
RUN find crates migrations -type f -exec touch {} + \
    && cargo build --release -p runway \
    && strip target/release/runway \
    && test "$(stat -c%s target/release/runway)" -gt 5000000 \
    && ./target/release/runway --version

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1000 runway \
    && mkdir -p /home/runway/data \
    && chown runway:runway /home/runway/data
USER runway
WORKDIR /home/runway
COPY --from=build /src/target/release/runway /usr/local/bin/runway
COPY --from=dashboard /src/dashboard/dist /home/runway/dashboard/dist
ENV WEB_DIR=/home/runway/dashboard/dist

EXPOSE 8000
VOLUME ["/home/runway/data"]
ENTRYPOINT ["runway"]
CMD ["serve"]
