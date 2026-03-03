# Imgflop

Local-first Imgflip clone designed for robust polling, change-only history tracking, and a browse-first web UI.

This project follows the design in:
- `docs/plans/2026-03-03-imgflop-design.md`
- `docs/plans/2026-03-03-imgflop-implementation-plan.md`

## Product Goals

- Poll Imgflip top memes on a schedule.
- Support `api_top_n = max | <int>`.
- Store image files on disk and metadata in SQLite.
- Persist history as change events only (no duplicate unchanged snapshots).
- Provide a local web UI for gallery browsing, details, admin operations, and meme creation.
- Gate admin routes with a simple secure single-account login.

## Core Design

### Runtime
- Single Rust binary for poller + scheduler + HTTP server.
- Async runtime: `tokio`.
- Web/API: `axum`.
- Database: SQLite (`sqlx`, WAL mode).

### Ingest Pipeline
`trigger -> lock -> fetch -> normalize -> assets -> rank -> diff -> commit -> publish status`

Key behavior:
- Single-flight locking prevents overlapping polls.
- Manual trigger during active run sets `pending_repoll`.
- Source failures are captured and surfaced through run/error records.

### Storage Model
- Binary image assets: content-addressed files on disk (SHA-256).
- Relational state/history: SQLite tables for runs, errors, memes, source mappings, current top state, change events, and created memes.

Important tables:
- `poll_runs`
- `poll_run_errors`
- `memes`
- `source_records`
- `image_assets`
- `top_state_current`
- `top_state_events`
- `created_memes`
- `created_meme_layers`
- `auth_events`

### History Semantics
- Emit only:
  - `entered_top`
  - `left_top`
  - `rank_changed`
  - `metadata_changed`
- No event emitted when an item is unchanged.

## Configuration (Design Target)

```toml
[polling]
schedule_cron = "0 * * * *"
api_top_n = "max"          # or integer
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

[logging]
level = "info"
format = "json"
dir = "./data/logs"
rotate = "daily"
retention_days = 14
stdout = true
```

## Security Baseline

- Single configured admin account with Argon2id password hash.
- Session-cookie auth for `/admin/*`.
- CSRF protection on state-changing admin routes.
- Login throttling and basic auth event auditing.

## Local Development

### Runtime Environment

- `ADMIN_USER` (optional fallback login)
- `ADMIN_PASSWORD_HASH` (optional fallback login, requires `ADMIN_USER`)
- `IMGFLOP_API_TOP_N` (`max` by default, or integer `>= 1`)
- `IMGFLOP_HISTORY_TOP_N` (`100` by default, integer `>= 1`)
- `IMGFLOP_POLL_INTERVAL_SECS` (`300` by default, integer `>= 1`)
- `IMGFLOP_BIND` (`127.0.0.1:8080` by default)
- `IMGFLOP_DB_URL` (`sqlite://imgflop.db?mode=rwc` by default)
- `IMGFLOP_ASSETS_DIR` (`data/images` by default)
- `IMGFLOP_API_ENDPOINT` (optional override, defaults to Imgflip public API)

`IMGFLOP_API_TOP_N` and `IMGFLOP_HISTORY_TOP_N` are independent:
- API ingest candidate count is controlled by `IMGFLOP_API_TOP_N`.
- Persisted top-state/events are controlled by `IMGFLOP_HISTORY_TOP_N`.

Admin auth behavior:
- If no admin account exists in DB, `/admin/login` shows a first-run setup form.
- Setup stores one local admin credential in SQLite using Argon2id hash.
- If `ADMIN_USER` + `ADMIN_PASSWORD_HASH` are provided, they remain usable as an emergency fallback login.

### Prerequisites
- Rust toolchain (stable)
- Docker Desktop (for `rust.testcontainers` scenarios)
- `cargo-llvm-cov` for local coverage gating

### Build
```bash
cargo build
```

### Test
```bash
cargo test
```

### Quality Gates
```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 80
```

## Test Strategy

- Unit tests for pure logic (config parsing, diff behavior, scheduler/locking behavior).
- Integration tests for ingest/store behavior and route behavior.
- End-to-end smoke tests for login, poll trigger, gallery, and create export flow.
- Use `rust.testcontainers` where tests require external services.
- Use in-memory/temp resources where no external service is needed.

## Repository Layout

- `src/` application modules (`auth`, `assets`, `config`, `diff`, `ingest`, `ops`, `sources`, `store`, `web`, `designer`)
- `migrations/` SQLite schema migrations
- `tests/` unit/integration/e2e coverage tests
- `.github/workflows/ci.yml` CI checks and coverage gate
- `docs/plans/` design and implementation plans
