# syntax=docker/dockerfile:1
#
# melon-server — production image.
#
# It depends on the PRIVATE git crate `felica-rs`, fetched over SSH at build time.
# Build with BuildKit and forward your SSH agent:
#
#   DOCKER_BUILDKIT=1 docker build --ssh default -t melon-server .
#   (or `docker compose build`, which forwards the agent via deploy/compose.yaml)
#
# `deploy/compose.yaml` sets `context: ..` (the repo root, since deploy/ is inside
# it). The build host must have an ssh-agent with a key authorized for the repo.

# ---------- builder ----------
FROM rust:1-bookworm AS builder

# cmake/clang: aws-lc-rs (rustls, via sqlx). git/openssh-client: fetch felica-rs.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake clang pkg-config git openssh-client ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p -m 0700 /root/.ssh \
    && ssh-keyscan github.com >> /root/.ssh/known_hosts

# Fetch git deps with the system git client so the forwarded SSH agent is used
# (libgit2's SSH auth is limited). The felica-rs URL is already `ssh://…`, so no
# https→ssh rewrite is needed.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

WORKDIR /src
COPY . .

# Build ONLY the server: a workspace-wide build would union felica-rs's `usb`
# feature (enabled by melon-terminal) and link rusb/libusb into the server.
# `--locked` makes the build reproducible from Cargo.lock. `--mount=type=ssh`
# exposes the forwarded agent to the `cargo build` (and thus to git).
RUN --mount=type=ssh \
    cargo build --release --locked -p melon-server \
    && strip target/release/melon-server

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

# ca-certificates: outbound TLS. curl: the container HEALTHCHECK.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin melon

COPY --from=builder /src/target/release/melon-server /usr/local/bin/melon-server

# The Web UIs (admin/merchant SPAs) and the SQL migrations are compiled into the
# binary (`include_str!` / `sqlx::migrate!`), so there is nothing else to ship.
USER melon:melon
EXPOSE 8080

# Listen on all interfaces *inside* the container; cloudflared is the only thing
# that reaches it (the port is never published to the host).
ENV MELON_BIND=0.0.0.0:8080 \
    RUST_LOG=info

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/melon-server"]
