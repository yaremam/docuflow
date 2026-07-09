# DocuFlow

An asynchronous, highly concurrent multi-tenant Document Management System written in Rust, featuring native OpenTelemetry tracing, relational metadata indexing, and background OCR worker pipelines.

## Repository Blueprint
- `/src`: Main application entrypoints and async web loops.
- `/tests`: Core integration test harnesses (TDD engine).
- `/docs/backlog`: Dynamic product user stories and acceptance criteria.
- `/docs/tdr`: Technical Design Records evaluating structural engineering decisions.

## Local Infrastructure Stack
- **Database:** PostgreSQL (Port `5432`)
- **Telemetry Ingestion:** OpenTelemetry Collector + Jaeger UI (`http://localhost:16686`)

