# syntax=docker/dockerfile:1
#
# melon-server — production image.
#
# `deploy/compose.yaml` sets `context: ..` (the repo root, since deploy/ is inside
# it).

# ---------- builder ----------
FROM rust:1-bookworm AS builder

# cmake/clang: aws-lc-rs (rustls, via sqlx). git: fetch git dependencies.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake clang pkg-config git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Fetch git deps with the system git client, matching `.cargo/config.toml`.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

WORKDIR /src
COPY . .

# Build ONLY the server: a workspace-wide build would union felica-rs's `usb`
# feature (enabled by melon-terminal) and link rusb/libusb into the server.
# `--locked` makes the build reproducible from Cargo.lock.
RUN cargo build --release --locked -p melon-server \
    && strip target/release/melon-server

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

# ca-certificates: outbound TLS. curl: the container HEALTHCHECK.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin melon

COPY --from=builder /src/target/release/melon-server /usr/local/bin/melon-server

# The SQL migrations are compiled into the binary (`sqlx::migrate!`), so there is
# nothing else to ship. The front-end is a separate app (see `web/`); this image
# is a pure JSON API.
USER melon:melon
EXPOSE 8080

# Listen on all interfaces *inside* the container; cloudflared is the only thing
# that reaches it (the port is never published to the host).
ENV MELON_BIND=0.0.0.0:8080 \
    RUST_LOG=info

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/melon-server"]
