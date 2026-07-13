---
name: verify
description: Build/launch/drive recipe for manually verifying a DocuFlow change end-to-end through the real HTTP server, not just cargo test.
---

Build and run the real containerized app, then drive it with `curl` (no
browser automation available in this sandbox — see
`reference_docuflow_environment` memory).

1. **Rebuild only the `app` service** after a source change:
   `docker compose build app && docker compose up -d app`. The other
   services (postgres/minio/mailpit/jaeger) stay running.
2. **Port**: the app listens on `8081` inside `docker-compose.yml`
   (`PORT: 8081`, `ports: ["8081:8081"]`) — hit `http://localhost:8081`
   from the host, not `8080` (that's just `EXPOSE`d in the Dockerfile,
   unused by compose). Confirm with `curl localhost:8081/health`.
3. **No `curl` inside the runtime container** (debian-slim, only
   `tesseract-ocr`/`poppler-utils`/certs installed) — always curl from
   the host, not via `docker compose exec app curl ...`.
4. **Drive real flows with a cookie jar**:
   ```bash
   JAR=/tmp/cookies.txt
   curl -s -c $JAR -b $JAR -X POST http://localhost:8081/signup \
     -d "email=you@example.com&password=documentspassword"
   curl -s -c $JAR -b $JAR -X POST http://localhost:8081/documents \
     -F "file=@tests/fixtures/german_sample.png;type=image/png"
   # response Location header has the new document's id
   curl -s -c $JAR -b $JAR http://localhost:8081/documents/<id>
   ```
   OCR runs as a detached background task — poll the show page (or
   `ocr_status` via a DB query) rather than assuming it's done
   immediately; it's normally sub-second for a small fixture image.
5. **Verifying tesseract language packs actually installed**:
   `docker compose exec -T app tesseract --list-langs`.
6. Data created this way lands in the real dev `doc_manager_db`/
   `docuflow-uploads` (not the test DB/bucket) — harmless dev rows, no
   need to clean up unless asked.
