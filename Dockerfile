# melon-server — production image.
#
# ⚠️ BUILD CONTEXT: the **parent** directory that contains BOTH `melon/` and
# `felica-rs/`. The workspace pins felica-rs through a `[patch]` to `../felica-rs`
# (the `usb` feature change is not upstream yet), which lives outside this repo,
# so a repo-root context cannot see it:
#
#     docker build -f melon/Dockerfile -t melon-server:latest .   # run from the parent dir
#
# `deploy/compose.yaml` already sets `context: ../..` for this reason. Once the
# felica-rs change is upstream, drop the `[patch]`, delete the felica-rs COPY, and
# the context can shrink back to the repo root.

# ---------- builder ----------
# Any rustup-based image works: rust-toolchain.toml pins the exact toolchain and
# cargo installs it on first use.
FROM rust:1-bookworm AS builder

# aws-lc-rs (rustls, via sqlx) needs cmake + a C toolchain to build.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake clang pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY felica-rs/ felica-rs/
COPY melon/ melon/

WORKDIR /src/melon
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

COPY --from=builder /src/melon/target/release/melon-server /usr/local/bin/melon-server

# The Web UIs (admin/merchant SPAs) and the SQL migrations are compiled into the
# binary (`include_str!` / `sqlx::migrate!`), so there is nothing else to ship.
USER melon:melon
EXPOSE 8080

# Listen on all interfaces *inside* the container; the reverse proxy is the only
# thing that reaches it (the port is never published to the host).
ENV MELON_BIND=0.0.0.0:8080 \
    RUST_LOG=info

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/melon-server"]
