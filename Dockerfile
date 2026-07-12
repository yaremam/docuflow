# --- Build stage ---
FROM rust:slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Compile-time query checking uses the `.sqlx/` cache checked into the repo
# instead of a live database connection, so the image can build without
# Postgres running alongside it.
ENV SQLX_OFFLINE=true
RUN cargo build --release

# --- Runtime stage ---
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 tesseract-ocr tesseract-ocr-rus poppler-utils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/docuflow ./docuflow
COPY static ./static

EXPOSE 8080
CMD ["./docuflow"]
