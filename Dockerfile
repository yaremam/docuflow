# --- Chef stage: shared toolchain base for the planner and builder ---
# cargo-chef splits dependency compilation from app compilation so CI can
# cache the (slow, rarely changing) dependency layer between nightly builds
# (feature 021). Installed from crates.io rather than pulling the
# third-party lukemathwalker/cargo-chef image; the install layer itself
# caches until the Rust base image updates.
FROM rust:slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

# --- Planner stage: distill the dependency graph into a recipe ---
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Builder stage ---
FROM chef AS builder
# Compile-time query checking uses the `.sqlx/` cache checked into the repo
# instead of a live database connection, so the image can build without
# Postgres running alongside it.
ENV SQLX_OFFLINE=true
COPY --from=planner /app/recipe.json recipe.json
# Dependencies only — this layer is unchanged (and cache-hit) as long as
# Cargo.toml/Cargo.lock are, no matter how the app source changed.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release

# --- Runtime stage ---
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 tesseract-ocr tesseract-ocr-deu tesseract-ocr-nld tesseract-ocr-ukr poppler-utils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/docuflow ./docuflow
COPY static ./static

# Build revision, stamped by the nightly pipeline (feature 021). Kept as a
# runtime env var (surfaced by `GET /health`) plus OCI labels — the nightly
# workflow reads `revision` back off the published image to decide whether
# a new commit exists to build. Declared this late so the varying ARG only
# busts these final metadata layers, never the compile or apt layers.
ARG GIT_SHA=dev
ENV GIT_SHA=${GIT_SHA}
LABEL org.opencontainers.image.source="https://github.com/yaremam/docuflow" \
      org.opencontainers.image.description="DocuFlow — personal document archiving system" \
      org.opencontainers.image.revision="${GIT_SHA}"

EXPOSE 8080
CMD ["./docuflow"]
