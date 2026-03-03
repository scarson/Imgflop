# Imgflop Design

Date: 2026-03-03
Status: Approved design draft

## 1. Scope

Build a robust local Imgflip clone as a single Rust binary that:
- Polls Imgflip on a configurable schedule.
- Ingests top memes from API (`N` or `max` supported by API response).
- Optionally enriches with scraper data.
- Stores image files on disk and metadata/history in SQLite.
- Records only ranking/metadata changes (no duplicate unchanged snapshots).
- Provides a browse-first web UI plus a basic meme designer.
- Protects admin routes with a secure single-account login gate.

## 2. Product Decisions

- Runtime model: single Rust process (poller + web/API + scheduler).
- Database: SQLite in WAL mode.
- Storage: content-addressed image files on disk; DB tracks metadata.
- Top ingestion control: `api_top_n = max | <int>`.
- History control: `history_top_n = <int>`, independent from ingest count.
- History write policy: change-only events, never full duplicate snapshots.
- Source strategy: API authoritative for rank list length; scraper optional enrichment/fallback.

## 3. Non-Goals (V1)

- Multi-user auth/RBAC/SSO.
- Distributed deployment and horizontal scaling.
- Full Photoshop-like editor (stickers, freehand drawing, complex undo graphs).

## 4. Configuration Model

Example TOML:

```toml
[polling]
schedule_cron = "0 * * * *"   # hourly default
api_top_n = "max"             # or integer
history_top_n = 2000
scraper_enabled = true

[storage]
data_dir = "./data"
images_dir = "./data/images"
temp_dir = "./data/tmp"

[web]
bind = "127.0.0.1:8080"
base_url = "http://localhost:8080"

[auth]
admin_user = "admin"
admin_password_hash = "$argon2id$..."
session_idle_minutes = 30
session_absolute_hours = 12

[logging]
level = "info"
format = "json"      # json|pretty
dir = "./data/logs"
rotate = "daily"
retention_days = 14
stdout = true
```

Validation rules:
- `history_top_n >= 1`.
- `api_top_n` is either `"max"` or integer `>=1`.
- Clamp `history_top_n` to effective ingest count per run with warning metric.
- Validate writable directories at boot.

## 5. Architecture

Crates/modules (single workspace acceptable):

- `config`: load/validate config (env overrides supported).
- `ops::logging`: initialize `tracing` subscribers, JSON output, rotation.
- `ops::scheduler`: scheduled triggers + overdue-on-boot behavior.
- `ops::locking`: DB-backed lease lock with heartbeat and stale recovery.
- `sources::api`: Imgflip API adapter.
- `sources::scraper`: optional scraper adapter.
- `ingest`: fetch/normalize/upsert/download orchestration.
- `diff`: computes `entered`, `left`, `rank_changed`, `metadata_changed`.
- `store`: SQLx repositories and migrations.
- `web`: axum routes, Askama templates, HTMX partial endpoints.
- `designer`: template composition and PNG rendering pipeline.

Pipeline is strict and ordered:
`trigger -> lock -> fetch -> normalize -> assets -> rank -> diff -> commit -> publish status`.

## 6. Data Model

Core tables:
- `poll_runs`
  - run metadata, status (`success|degraded|failed|aborted_stale`), timings, counts.
- `poll_run_errors`
  - structured UI-visible run errors (`severity`, `error_kind`, `message`, `context_json`).
- `memes`
  - canonical meme metadata and timestamps.
- `source_records`
  - source identity mappings (`source`, external id, raw payload, `meme_id`).
- `image_assets`
  - `sha256`, path, bytes, mime, dimensions, uniqueness.
- `top_state_current`
  - current top set state for scope.
- `top_state_events`
  - append-only change events only.
- `created_memes`
  - rendered user outputs metadata.
- `created_meme_layers`
  - layer model for generated memes.
- `auth_events`
  - login/logout/session expiration audit events.

Recommended indexes:
- `top_state_current(scope, rank)`.
- `top_state_events(meme_id, at_utc desc)`.
- `poll_runs(started_at desc)`.
- `poll_run_errors(run_id, at_utc desc)`.
- `source_records(source, source_meme_id)` unique.
- `image_assets(sha256)` unique.

## 7. Change-Only History Semantics

For each run:
1. Build current ranked set for chosen scope.
2. Trim/clamp to `history_top_n`.
3. Compare with `top_state_current`.
4. Emit events only when changed:
   - not previously present -> `entered_top`
   - no longer present -> `left_top`
   - present with different rank -> `rank_changed`
   - metadata changed -> `metadata_changed`
5. Transactionally write events and replace current state.

If no item changes, write zero events for that run.

Determinism:
- Apply stable tie-breakers (source rank then canonical meme ID) to avoid false churn.

## 8. Polling, Idempotency, and Recovery

- Single-flight via DB lease lock; manual and scheduler triggers cannot overlap.
- If poll requested during active run, set `pending_repoll=true` for one follow-up run.
- Assign deterministic `run_key` to avoid duplicate commits from near-simultaneous triggers.
- On startup:
  - detect stale lock and stale `running` runs;
  - mark stale runs `aborted_stale`;
  - recover and continue.

Retry policy:
- Network fetch/download: bounded retries with jitter and timeout.
- Parse/schema errors: non-retryable for that source/run.
- Scraper failure: run can be `degraded` if API path succeeded.

## 9. Asset Storage Strategy

- Download to temp file, compute SHA-256, validate size/content-type, then atomic rename.
- Disk path pattern: `images/sha256-prefix/fullhash.ext`.
- Deduplicate by hash.
- Optional background GC job:
  - remove temp orphan files;
  - optionally prune unreferenced assets (policy-gated).

## 10. Logging and Observability

Structured logging:
- `tracing` events in JSON files (and optional stdout).
- Include `run_id`, `request_id`, `source`, `meme_id`, and operation context.
- Error logs use `error_kind` classification and redact sensitive values.

Data store vs logs:
- DB stores summarized operational state (`poll_runs`, `poll_run_errors`).
- Full verbose diagnostics remain in rotated log files.

## 11. Security Model

Admin authentication:
- Single configured admin (`admin_user` + Argon2id hash).
- Constant-time verification and generic failure messages.
- Session cookie signed/encrypted, `HttpOnly`, `SameSite=Lax`, `Secure` under HTTPS.
- Session idle timeout + absolute TTL.
- CSRF protection for state-changing admin endpoints.
- Login rate limiting + temporary lockout backoff.

Threat mitigations:
- No plaintext secrets in DB/logs.
- Validate and sanitize all user inputs (designer text length, colors, coordinates).
- Keep local-only default bind (`127.0.0.1`), explicit opt-in for LAN exposure.

## 12. UI Design

Pages:
- `/` gallery (browse-first):
  - filters, search, rank controls, source tabs (`Templates|Created|All`),
  - quick scopes (`Top 100|500|2000|All`),
  - paginated cards with lazy thumbnails.
- `/memes/:id` detail:
  - full image, metadata, rank timeline, event history.
- `/create` meme designer:
  - multi-layer text boxes, drag/drop positioning, style controls,
  - server-rendered deterministic PNG export,
  - export toggle: store-and-index vs export-only.
- `/admin`:
  - login-gated status/config/run tables/errors/manual poll actions.

Frontend delivery:
- Askama SSR + HTMX partial updates + minimal Alpine state.
- No external CDN dependency by default.

## 13. Testing and Verification

Unit tests:
- Diff correctness and deterministic ordering.
- Config validation and clamping logic.
- Auth/session/CSRF primitives.

Integration tests:
- End-to-end poll pipeline with SQLite + temp filesystem.
- Verify no-event writes on unchanged runs.
- Locking behavior, stale recovery, pending repoll semantics.
- Asset atomic write and dedupe behavior.

UI/E2E tests:
- Gallery filtering/pagination.
- Admin auth gate + config changes.
- Designer layer editing and export modes.

CI gates:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

## 14. Risks and Mitigations

- Imgflip API/scraper shape changes:
  - isolate adapters, track parse errors, degrade gracefully, avoid corrupting state.
- Large `N` memory or UI latency:
  - paginate server-side, stream processing where practical, enforce caps per request.
- Duplicate/false change events:
  - deterministic ranking + transactional publish + explicit diff tests.
- Auth misconfiguration:
  - fail fast at boot if hash/user missing for admin-enabled mode.

## 15. Implementation Readiness

Design is feasible in Rust and aligned with local-first constraints.
Next step: produce a detailed implementation plan (`writing-plans` workflow) from this document.
