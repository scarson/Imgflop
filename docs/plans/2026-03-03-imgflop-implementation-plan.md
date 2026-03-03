# Imgflop Local Clone Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a robust local Rust app that polls Imgflip top memes, stores change-only history and assets locally, serves a browse-first UI, and includes a basic meme designer plus secure admin gate.

**Architecture:** A single Rust binary (`axum` + `tokio`) runs scheduler, ingest pipeline, and web UI. SQLite (WAL) stores metadata/state/events; image files are content-addressed on disk. Polling uses single-flight DB locking, change-only diffing, structured logging, and resilient error handling.

**Tech Stack:** Rust, axum, tokio, sqlx (SQLite), reqwest, scraper, askama, htmx/alpine, tracing, argon2, cargo-llvm-cov.

---

### Task 1: Project Bootstrap and Health Endpoint

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `src/web/mod.rs`
- Create: `tests/health_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn health_returns_ok() {
    let app = imgflop::web::app_router();
    let response = axum_test::TestServer::new(app).get("/health").await;
    response.assert_status_ok();
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test health_returns_ok -q`
Expected: FAIL with missing crate/module/router errors.

**Step 3: Write minimal implementation**

```rust
use axum::{routing::get, Router};

pub fn app_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test health_returns_ok -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml src/main.rs src/lib.rs src/web/mod.rs tests/health_test.rs
git commit -m "chore: bootstrap rust app with health route"
```

### Task 2: Config Loading and Validation

**Files:**
- Create: `src/config.rs`
- Create: `tests/config_test.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn parses_api_max_and_history_top_n() {
    let cfg = imgflop::config::from_toml(r#"
[polling]
api_top_n = "max"
history_top_n = 2000
"#).unwrap();
    assert!(cfg.polling.api_top_n.is_max());
    assert_eq!(cfg.polling.history_top_n, 2000);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test parses_api_max_and_history_top_n -q`
Expected: FAIL with missing config parser.

**Step 3: Write minimal implementation**

```rust
#[derive(Deserialize)]
pub enum ApiTopN { Max, Int(u32) }
```

Implement parser + validation (`history_top_n >= 1`).

**Step 4: Run test to verify it passes**

Run: `cargo test config_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/config.rs src/lib.rs src/main.rs tests/config_test.rs
git commit -m "feat: add app config parsing and validation"
```

### Task 3: SQLite Setup and Migrations

**Files:**
- Create: `migrations/0001_init.sql`
- Create: `src/store/mod.rs`
- Create: `src/store/db.rs`
- Create: `tests/migrations_test.rs`
- Modify: `Cargo.toml`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn migrations_create_core_tables() {
    let pool = imgflop::store::db::test_pool().await;
    let tables = imgflop::store::db::table_names(&pool).await.unwrap();
    assert!(tables.contains(&"poll_runs".to_string()));
    assert!(tables.contains(&"top_state_events".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test migrations_create_core_tables -q`
Expected: FAIL with missing migration setup/tables.

**Step 3: Write minimal implementation**

Add migration for:
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

**Step 4: Run test to verify it passes**

Run: `cargo test migrations_create_core_tables -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add migrations src/store Cargo.toml tests/migrations_test.rs
git commit -m "feat: add sqlite migrations and store bootstrap"
```

### Task 4: Poll Locking and Run Bookkeeping

**Files:**
- Create: `src/ops/locking.rs`
- Create: `src/ops/runs.rs`
- Create: `tests/locking_test.rs`
- Modify: `src/lib.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn second_lock_attempt_fails_while_first_active() {
    let ctx = TestCtx::new().await;
    let a = ctx.locking.acquire("poll").await.unwrap();
    let b = ctx.locking.acquire("poll").await;
    assert!(b.is_err());
    drop(a);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test second_lock_attempt_fails_while_first_active -q`
Expected: FAIL with missing lock implementation.

**Step 3: Write minimal implementation**

Implement DB lease lock table + acquire/release + stale timeout recovery.

**Step 4: Run test to verify it passes**

Run: `cargo test locking_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/ops src/lib.rs tests/locking_test.rs
git commit -m "feat: add single-flight poll locking and run bookkeeping"
```

### Task 5: Imgflip API Adapter

**Files:**
- Create: `src/sources/mod.rs`
- Create: `src/sources/api.rs`
- Create: `tests/api_source_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn parses_imgflip_api_memes() {
    let body = include_str!("fixtures/imgflip_get_memes.json");
    let list = imgflop::sources::api::parse_memes(body).unwrap();
    assert!(!list.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test parses_imgflip_api_memes -q`
Expected: FAIL with missing parser/model.

**Step 3: Write minimal implementation**

Implement API model + parser + mapper into canonical `MemeCandidate`.

**Step 4: Run test to verify it passes**

Run: `cargo test api_source_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/sources tests/api_source_test.rs
git commit -m "feat: add imgflip api source adapter"
```

### Task 6: Asset Download and Hash Storage

**Files:**
- Create: `src/assets/mod.rs`
- Create: `src/assets/store.rs`
- Create: `tests/assets_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn same_file_hash_dedupes_assets() {
    let ctx = TestCtx::new().await;
    let a = ctx.assets.store_bytes("image/png", b"abc").await.unwrap();
    let b = ctx.assets.store_bytes("image/png", b"abc").await.unwrap();
    assert_eq!(a.sha256, b.sha256);
    assert_eq!(a.path, b.path);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test same_file_hash_dedupes_assets -q`
Expected: FAIL with missing asset store.

**Step 3: Write minimal implementation**

Implement temp write -> hash -> atomic rename -> DB upsert by hash.

**Step 4: Run test to verify it passes**

Run: `cargo test assets_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/assets tests/assets_test.rs
git commit -m "feat: add content-addressed asset storage"
```

### Task 7: Diff Engine (Change-Only History)

**Files:**
- Create: `src/diff.rs`
- Create: `tests/diff_test.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn unchanged_rank_emits_no_events() {
    let prev = vec![state("m1", 1)];
    let next = vec![state("m1", 1)];
    let events = imgflop::diff::compute(&prev, &next);
    assert!(events.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test unchanged_rank_emits_no_events -q`
Expected: FAIL with missing diff function.

**Step 3: Write minimal implementation**

Implement event generation for `entered_top`, `left_top`, `rank_changed`, `metadata_changed`.

**Step 4: Run test to verify it passes**

Run: `cargo test diff_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/diff.rs tests/diff_test.rs
git commit -m "feat: add change-only ranking diff engine"
```

### Task 8: Ingest Orchestrator and Transactional Commit

**Files:**
- Create: `src/ingest/mod.rs`
- Create: `src/ingest/pipeline.rs`
- Create: `tests/ingest_pipeline_test.rs`
- Modify: `src/store/mod.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn unchanged_second_run_writes_zero_events() {
    let ctx = TestCtx::with_fake_source(vec![fixture_items()]);
    ctx.run_poll().await.unwrap();
    let first = ctx.count_events().await;
    ctx.run_poll().await.unwrap();
    let second = ctx.count_events().await;
    assert_eq!(first, second);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test unchanged_second_run_writes_zero_events -q`
Expected: FAIL with missing ingest pipeline.

**Step 3: Write minimal implementation**

Implement orchestrator and one transaction that writes run + events + current state.

**Step 4: Run test to verify it passes**

Run: `cargo test ingest_pipeline_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/ingest src/store tests/ingest_pipeline_test.rs
git commit -m "feat: add transactional ingest pipeline"
```

### Task 9: Scheduler and Manual Poll Trigger

**Files:**
- Create: `src/ops/scheduler.rs`
- Modify: `src/main.rs`
- Modify: `src/web/mod.rs`
- Create: `tests/scheduler_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn manual_trigger_while_running_sets_pending_repoll() {
    let ctx = TestCtx::new().await;
    ctx.start_long_poll().await;
    ctx.manual_trigger().await;
    assert!(ctx.pending_repoll().await);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test manual_trigger_while_running_sets_pending_repoll -q`
Expected: FAIL with missing scheduler state handling.

**Step 3: Write minimal implementation**

Implement scheduler loop + manual trigger endpoint + pending repoll flag.

**Step 4: Run test to verify it passes**

Run: `cargo test scheduler_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/ops/scheduler.rs src/main.rs src/web/mod.rs tests/scheduler_test.rs
git commit -m "feat: add scheduler and manual poll trigger flow"
```

### Task 10: Admin Auth Gate (Single Account)

**Files:**
- Create: `src/auth/mod.rs`
- Create: `src/auth/session.rs`
- Modify: `src/web/mod.rs`
- Create: `tests/auth_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn admin_route_requires_login() {
    let app = test_app().await;
    let res = app.get("/admin").await;
    res.assert_status_unauthorized();
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test admin_route_requires_login -q`
Expected: FAIL with unprotected admin route.

**Step 3: Write minimal implementation**

Add Argon2 verification, login/logout endpoints, session cookie middleware, CSRF checks.

**Step 4: Run test to verify it passes**

Run: `cargo test auth_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/auth src/web/mod.rs tests/auth_test.rs
git commit -m "feat: secure admin routes with single-account auth"
```

### Task 11: Browse UI and Admin Pages

**Files:**
- Create: `src/web/routes/gallery.rs`
- Create: `src/web/routes/admin.rs`
- Create: `src/web/templates/layout.html`
- Create: `src/web/templates/gallery.html`
- Create: `src/web/templates/meme_detail.html`
- Create: `src/web/templates/admin.html`
- Create: `src/web/static/app.css`
- Create: `tests/web_pages_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn gallery_page_renders_ranked_memes() {
    let app = seeded_app().await;
    let res = app.get("/").await;
    res.assert_status_ok();
    res.assert_text_contains("Top Memes");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test gallery_page_renders_ranked_memes -q`
Expected: FAIL with missing templates/routes.

**Step 3: Write minimal implementation**

Render gallery/detail/admin pages with pagination, filters, and run status panel.

**Step 4: Run test to verify it passes**

Run: `cargo test web_pages_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/web tests/web_pages_test.rs
git commit -m "feat: add gallery, detail, and admin web pages"
```

### Task 12: Meme Designer and Export Modes

**Files:**
- Create: `src/designer/mod.rs`
- Create: `src/designer/render.rs`
- Create: `src/web/routes/create.rs`
- Create: `src/web/templates/create.html`
- Create: `tests/designer_test.rs`
- Modify: `src/store/mod.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn stored_export_creates_created_meme_record() {
    let ctx = TestCtx::new().await;
    let id = ctx.export_with_store(true).await.unwrap();
    assert!(ctx.created_exists(id).await);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test stored_export_creates_created_meme_record -q`
Expected: FAIL with missing designer/export code.

**Step 3: Write minimal implementation**

Implement multi-layer text model, server-side PNG render, and per-export `store` toggle.

**Step 4: Run test to verify it passes**

Run: `cargo test designer_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/designer src/web/routes/create.rs src/web/templates/create.html src/store/mod.rs tests/designer_test.rs
git commit -m "feat: add meme designer with store-or-export mode"
```

### Task 13: Structured Logging and Error Surfacing

**Files:**
- Create: `src/ops/logging.rs`
- Modify: `src/main.rs`
- Modify: `src/ingest/pipeline.rs`
- Modify: `src/store/mod.rs`
- Create: `tests/logging_errors_test.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn failed_run_inserts_poll_run_error() {
    let ctx = TestCtx::with_failing_source().await;
    let _ = ctx.run_poll().await;
    assert!(ctx.poll_run_errors_count().await > 0);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test failed_run_inserts_poll_run_error -q`
Expected: FAIL with missing error persistence/logging wiring.

**Step 3: Write minimal implementation**

Add JSON structured logging init and persist summarized errors to `poll_run_errors`.

**Step 4: Run test to verify it passes**

Run: `cargo test logging_errors_test -q`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/ops/logging.rs src/main.rs src/ingest/pipeline.rs src/store/mod.rs tests/logging_errors_test.rs
git commit -m "feat: add structured logging and run error surfacing"
```

### Task 14: End-to-End Verification and Documentation

**Files:**
- Create: `tests/e2e_smoke_test.rs`
- Create: `.github/workflows/ci.yml`
- Modify: `README.md`
- Modify: `docs/plans/2026-03-03-imgflop-design.md`

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn e2e_poll_then_gallery_then_create_export() {
    let app = boot_test_app().await;
    app.post("/admin/poll").await.assert_status_accepted();
    app.get("/").await.assert_status_ok();
    app.post("/create/export").await.assert_status_ok();
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test e2e_poll_then_gallery_then_create_export -q`
Expected: FAIL until pipeline and routes are wired.

**Step 3: Write minimal implementation**

Complete remaining glue, seed/test harness helpers, and doc updates for setup/run/config.
Add CI workflow gates for:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test -q`
- `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 80`

**Step 4: Run test to verify it passes**

Run:
- `cargo test -q`
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 80`

Expected: all PASS, with line coverage >= 80%.

**Step 5: Commit**

```bash
git add tests/e2e_smoke_test.rs .github/workflows/ci.yml README.md docs/plans/2026-03-03-imgflop-design.md
git commit -m "chore: finalize e2e verification and docs"
```

## Skills to Apply During Execution

- `@test-driven-development` before each implementation change.
- `@systematic-debugging` on any failure that is not immediately obvious.
- `@verification-before-completion` before claiming task/feature completion.
- `@requesting-code-review` after major milestones.

## Execution Notes

- Keep commits small and task-scoped.
- Do not begin Task N+1 until Task N tests are green.
- Prefer deterministic fixtures for API/scraper tests.
- Use `rust.testcontainers` wherever tests require external services (and prefer local in-memory/temp resources when no external service is involved).
- Preserve local-first defaults (`127.0.0.1`, local files, no CDN).
- Treat line coverage below 80% as a hard failure until fixed.
