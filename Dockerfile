# syntax=docker/dockerfile:1

# ── Build stage ───────────────────────────────────────────────────────────
# BACKEND selects the embedding backend, baked in at compile time:
#   fastembed (default) — local ONNX nomic-embed-text-v1.5-Q, self-contained
#   ollama              — HTTP to an external Ollama server
# trixie (Debian 13, glibc 2.41) — NOT bookworm: the fastembed backend's `ort`
# crate downloads a prebuilt ONNX Runtime linked against glibc >= 2.38 and a
# recent libstdc++ (`__isoc23_strtol`, `__cxa_call_terminate`). bookworm's
# glibc 2.36 / GCC 12 can't satisfy those symbols at link or run time.
ARG RUST_VERSION=1
FROM rust:${RUST_VERSION}-trixie AS builder

ARG BACKEND=fastembed
WORKDIR /build

# git is needed for the [patch.crates-io] rmcp fork and the mcp-gain git dep.
RUN apt-get update \
 && apt-get install -y --no-install-recommends git \
 && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Cache the cargo registry, git deps and the target dir across builds. The
# binary is copied out of the (ephemeral) cache mount inside the same RUN.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    set -eux; \
    case "${BACKEND}" in \
      fastembed) cargo build --release --locked ;; \
      ollama)    cargo build --release --locked --no-default-features --features ollama ;; \
      *) echo "unknown BACKEND '${BACKEND}' (want: fastembed | ollama)" >&2; exit 1 ;; \
    esac; \
    cp target/release/palazzo /palazzo

# ── Runtime stage ─────────────────────────────────────────────────────────
# debian:trixie-slim (glibc 2.41) — must match the builder: the ONNX Runtime
# prebuilt needs glibc >= 2.38 at run time too. No musl/alpine (glibc-only).
FROM debian:trixie-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --home-dir /var/lib/palazzo palazzo

COPY --from=builder /palazzo /usr/local/bin/palazzo

# Container defaults — every one is overridable via compose `environment:`.
ENV PALAZZO_BIND=0.0.0.0:6334 \
    QDRANT_URL=http://qdrant:6333 \
    COLLECTION=claude-memory \
    FASTEMBED_CACHE_DIR=/var/lib/palazzo/fastembed-cache \
    PALAZZO_WAL=/var/lib/palazzo/wal.jsonl \
    PALAZZO_USAGE_LOG=/var/lib/palazzo/usage.jsonl \
    RUST_LOG=palazzo=info

USER palazzo
WORKDIR /var/lib/palazzo
EXPOSE 6334

# fastembed downloads ~110 MB of model weights on first run — generous start period.
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
  CMD curl -fsS http://127.0.0.1:6334/health || exit 1

ENTRYPOINT ["palazzo"]
CMD ["serve"]
