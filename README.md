# DocuFlow

An asynchronous, highly concurrent multi-tenant Document Management System written in Rust, featuring native OpenTelemetry tracing, relational metadata indexing, and background OCR worker pipelines.

## Repository Blueprint
- `/src`: Main application entrypoints and async web loops.
- `/tests`: Core integration test harnesses (TDD engine).
- `/docs/ARCHITECTURE.md`: Living system architecture — components, interfaces, DB schema, and a decision-log index. Start here.
- `/docs/backlog`: Dynamic product user stories and acceptance criteria.
- `/docs/tdr`: Technical Design Records evaluating structural engineering decisions.

## Local Infrastructure Stack
- **App:** the DocuFlow server itself, built from `Dockerfile` (Port `8080`)
- **Database:** PostgreSQL (Port `5432`)
- **Telemetry Ingestion:** OpenTelemetry Collector + Jaeger UI (`http://localhost:16686`)

## Prerequisites
- Docker + Docker Compose

That's it for just running the app — the container image ships a prebuilt
binary, so no local Rust toolchain is required. You only need Rust installed
if you're editing the code (see [Developing without Docker](#developing-without-docker)).

## Build & Run

```sh
docker compose up -d --build
```

This single command builds the app image (multi-stage `Dockerfile`, compiled
in release mode), starts Postgres, Jaeger, and the app together, and the app
container runs migrations automatically on boot. Nothing else to install —
`docker compose up -d --build` is the entire setup.

Verify it's up:
```sh
curl http://localhost:8080/health
```
Then visit `http://localhost:8080` in a browser for the landing page, or
`http://localhost:16686` for the Jaeger trace UI.

If port `8080` is already taken on your machine, override the host-side port
mapping for the `app` service in `docker-compose.yml` (e.g. `"8081:8080"`)
before running `docker compose up -d --build`.

To rebuild after changing the code:
```sh
docker compose up -d --build app
```

## Developing without Docker

If you're editing the Rust code and want a faster local iteration loop than
rebuilding the Docker image every time:

### Prerequisites
- Rust (latest stable) — `rustup update stable`
- Docker + Docker Compose (for Postgres/Jaeger only)
- [`sqlx-cli`](https://github.com/launchbadge/sqlx) — only needed if you change a
  query and must regenerate the offline cache: `cargo install sqlx-cli --no-default-features --features rustls,postgres`

### Steps

1. **Start Postgres and Jaeger** (skip the `app` service — you'll run it via `cargo` instead):
   ```sh
   docker compose up -d postgres jaeger
   ```
2. **Configure the database URL**:
   ```sh
   cp .env.example .env
   ```
   The default in `.env.example` matches the `docker-compose.yml` Postgres service, so no edits are needed for local dev.
3. **Build**:
   ```sh
   cargo build
   ```
   Compiling doesn't require a live database connection — query correctness is checked at compile time against the `.sqlx/` offline cache checked into the repo, not a live connection.
4. **Run**:
   ```sh
   cargo run
   ```
   On startup the server reads `DATABASE_URL` from `.env`, runs any pending database migrations automatically, and starts listening on `http://localhost:8080`.

   If port `8080` is already in use on your machine, override it with the
   `PORT` env var (either export it, or set it in `.env`):
   ```sh
   PORT=8081 cargo run
   ```
5. **Verify it's up**:
   ```sh
   curl http://localhost:8080/health
   ```
   (substitute your chosen port if you overrode it with `PORT`).

### Running the tests

Integration tests exercise a real database, so Postgres must be running first:
```sh
docker compose up -d postgres
cargo test
```

Tests run against their own `doc_manager_db_test` database — created
automatically on first run, on the same Postgres server as dev but never
the `doc_manager_db` your `docker compose up` app container actually uses.
This means `cargo test` is safe to run at any time, even with real
signed-up accounts sitting in your dev database — it can't truncate them.
Override the test database with `TEST_DATABASE_URL` if you need tests to
target a different Postgres server (e.g. in CI).

### Changing a database query

If you add or modify a `sqlx::query!`/`sqlx::query_as!` call, regenerate the offline
query cache so `cargo build`/`cargo check` (and the Docker image build) keep
working without a live database:
```sh
cargo sqlx prepare --workspace -- --tests
```
Commit the resulting changes under `.sqlx/`.

## Database persistence

Postgres's data directory is mounted to a named Docker volume (`pgdata` in
`docker-compose.yml`), not stored inside the container itself. That means your
data survives `docker compose stop` / `up -d`, `docker compose restart`, and
even `docker compose down` (without `-v`) — starting a new container just
reattaches to the same volume, it doesn't start empty. Migrations are also
tracked in a `_sqlx_migrations` bookkeeping table, so re-running the app
against existing data just skips migrations that already applied.

The data is only wiped if you explicitly remove the volume:
```sh
docker compose down -v          # removes this project's volumes
# or
docker volume rm docuflow_pgdata
```

