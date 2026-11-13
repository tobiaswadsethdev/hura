# syntax=docker/dockerfile:1
#
# hura-cloud, as Coolify runs it. Built from the repository's root, since the
# crate is a workspace member and builds against its lockfile:
#
#   docker build -f crates/hura-cloud/Dockerfile -t hura-cloud .
#
# Configured by its environment; see crates/hura-cloud/src/config.rs and
# docs/cloud.md.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# hura-core compiles in the sandbox image's files and the built-in policies.
COPY images images
COPY policies policies
# Served at /privacy.
COPY docs/privacy-policy.md docs/privacy-policy.md
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p hura-cloud \
    && cp target/release/hura-cloud /hura-cloud

FROM debian:bookworm-slim
# curl for the health check, which Coolify runs inside the container.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 hura
COPY --from=build /hura-cloud /usr/local/bin/hura-cloud
USER hura
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["hura-cloud"]
