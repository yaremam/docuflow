# TDR 005: User Profile & Blob Storage

## 1. Context & Architectural Requirements
The `users` table (feature 003) has only `id`, `tenant_id`, `email`,
`password_hash`, `created_at` — no profile data at all. This feature adds
seven free-text profile fields directly to `users` (1:1 with a user, no need
for a separate table) plus a profile picture, which needs real file storage.
`aws-sdk-s3`/`aws-config` have been declared dependencies since the initial
scaffold but never used anywhere in `src/` — this is the first feature that
actually implements blob storage, which CLAUDE.md's architecture blueprint
calls for ("Blob Storage Manager... Files must be securely chunked and
pushed to the storage layers using streaming wrappers to prevent high memory
spikes") but which nothing before this exercised.

## 2. Alternatives Evaluated

### Alternative A: A newtype per profile field (`FirstName`, `LastName`, `StreetAddress`, ...)
- **Pros:** Maximal type-driven precision — matches `Password`/`EmailAddress`'s pattern exactly, one type per distinct concept.
- **Cons:** None of these seven fields has any behavior that differs from the others (no hashing, no format grammar, no distinct validation rule beyond a length cap) — seven near-identical newtypes would be pure ceremony with no payoff, unlike `Password` (needs hashing) or `EmailAddress` (needs the `@`/length check tied specifically to email semantics).

### Alternative B: Raw `String` fields, no validation at all
- **Pros:** Simplest possible code.
- **Cons:** No length bound at all — an absurdly long paste could reach a `text` column unconstrained (Postgres `text` has no inherent length cap), and it abandons CLAUDE.md's type-driven-constraints rule entirely for this data.

### Alternative C: One shared `ProfileField` newtype, reused for all seven columns (chosen)
- **Pros:** One `TryFrom<String>` (trims, caps at 200 chars) shared by every free-text profile field — the pragmatic middle ground between A's ceremony and B's lack of any constraint. Still satisfies CLAUDE.md's type-driven-constraints rule (validation lives in the type, not scattered across handler code).
- **Cons:** Doesn't distinguish "this string is a phone number" from "this string is a city" at the type level — acceptable, since nothing in this feature treats those differently yet.

**Chosen: Alternative C.**

---

### Alternative D: Store the picture on local disk (e.g. `static/uploads/`)
- **Pros:** Zero new infrastructure, fastest to build.
- **Cons:** Directly contradicts CLAUDE.md's blob-storage architecture; would need migrating to S3 later anyway once real document uploads (the product's actual core feature) are built, at which point two different storage mechanisms would need reconciling.

### Alternative E: Wire up LocalStack + real streaming S3 multipart upload (chosen)
- **Pros:** Matches CLAUDE.md's stated architecture from day one; puts the already-declared `aws-sdk-s3`/`aws-config` dependencies to their first real use; the streaming multipart implementation (`src/blob.rs`) is written as a general-purpose primitive, not a profile-picture-only shortcut, so future document-upload features build on it directly.
- **Cons:** More upfront infrastructure (a new `docker-compose.yml` service, new client/streaming code) than Alternative D for a feature whose immediate need is "store one small image per user."

**Chosen: Alternative E**, per the user's explicit direction this feature round.

## 3. Structural Decision
`migrations/*_add_user_profile_fields.sql` adds seven nullable `text`
columns plus `profile_picture_key` (an S3 object key, not a URL — the URL is
generated at render time via a short-lived presigned GET, so a bucket
rename or CDN migration never requires a data migration).

`src/blob.rs` (new module) provides `BlobStore`: `ensure_bucket()`
(idempotent head-then-create, called at boot alongside `sqlx::migrate!()`),
`stream_upload()` (S3 multipart upload in fixed 5MB parts read directly off
the incoming `axum::extract::Multipart` field — memory use stays bounded to
one part regardless of file size), and `presigned_get_url()`. `AppState`
gains a `blob: BlobStore` field alongside `pool`.

Three concurrency/compatibility issues surfaced during implementation, all
fixed in the shared infrastructure rather than worked around per-call-site:

1. **Cross-process bucket-creation race.** Many integration test binaries
   (separate OS processes) call `ensure_bucket()` around the same time on a
   fresh environment; without coordination, several can race to
   `head_bucket`/`create_bucket` the same not-yet-existing bucket
   simultaneously. `state::migrate()` now wraps the bucket-ensure step in a
   Postgres advisory lock (`pg_advisory_lock`/`pg_advisory_unlock` on a fixed
   key) — the same class of cross-process coordination `sqlx::migrate!()`
   already relies on internally for its own migration-locking safety, reused
   here for the same reason.
2. **LocalStack checksum mismatch on multipart uploads.** The AWS SDK for
   Rust defaults `request_checksum_calculation` to `WhenSupported`, which
   silently attaches a CRC32 checksum to `UploadPart` requests. LocalStack's
   multipart-upload handling doesn't reconcile that with
   `CompleteMultipartUpload`, failing every multipart upload with "Checksum
   Type mismatch occurred, expected checksum Type: null, actual checksum
   Type: crc32". `client_from_env()` pins `RequestChecksumCalculation::WhenRequired`
   instead (this feature doesn't rely on that integrity check), which is a
   clean fix rather than a per-upload workaround, and applies equally to
   real S3 (still opt-in there, just not silently defaulted on).
3. **Presigned URLs aren't browser-reachable from inside Docker Compose.**
   The dockerized app reaches LocalStack via the internal service hostname
   (`AWS_ENDPOINT_URL=http://localstack:4566`), but a presigned URL embedded
   in a rendered page is followed by the *user's own browser*, which can't
   resolve that hostname — only the host-mapped `localhost:4566` works
   there. `BlobStore` now holds a second client (`presign_client`, built by
   `blob::presign_client_from_env()`) used only for
   `presigned_get_url`, pointed at `BLOB_PUBLIC_ENDPOINT_URL` when set;
   unset (real S3, or host-side `cargo run`), it falls back to the same
   client used for everything else, since there's only one true endpoint in
   those cases.

`src/web/handlers/profile.rs` provides `show`/`update`/`upload_picture`, all
gated by `TenantContext` on the router's existing `protected` sub-router
(`/profile` GET+POST, `/profile/picture` POST with an explicit 8MB
`DefaultBodyLimit` layer as defense-in-depth alongside `stream_upload`'s own
mid-stream size check).

## 4. OpenTelemetry Implications
`show`, `update`, and `upload_picture` all carry `#[tracing::instrument(skip(...))]`
with the state/tenancy/form/multipart parameters skipped — no profile field
value (name, address, phone) or raw file bytes are ever captured as a span
attribute, matching the existing PII-redaction idiom from `Password`/session
handling. `BlobStore::stream_upload` is itself instrumented
(`skip(self, field)`) so the streaming operation appears as its own span
without exposing the file contents. `TenantContext`'s existing span-attribute
tagging (`tenant.id`/`user.id`, see TDR 003/004) applies unchanged to these
routes via the same `route_layer` middleware.
