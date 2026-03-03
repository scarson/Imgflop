use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use imgflop::{
    auth::AuthService,
    designer::DesignerService,
    designer::render,
    ops::{polling::PollRuntime, scheduler::Scheduler},
    store::db,
    web,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn gallery_page_renders_db_backed_ranked_rows() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let meme_id = insert_meme(&pool, "Drake Hotline").await;
    insert_top_state(&pool, meme_id, run_id, 1).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Drake Hotline"));
    assert!(html.contains(&format!("/memes/{meme_id}")));
}

#[tokio::test]
async fn meme_detail_page_renders_event_timeline_from_db() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let meme_id = insert_meme(&pool, "Distracted Boyfriend").await;
    insert_event(&pool, run_id, meme_id, "rank_changed").await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/memes/{meme_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Distracted Boyfriend"));
    assert!(html.contains("rank_changed"));
}

#[tokio::test]
async fn admin_page_renders_run_and_error_tables_from_db() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    sqlx::query(
        "INSERT INTO poll_run_errors (run_id, at_utc, severity, error_kind, message, context_json) VALUES (?, ?, ?, ?, ?, NULL)",
    )
    .bind(run_id)
    .bind(now_epoch_seconds().to_string())
    .bind("error")
    .bind("fetch_failed")
    .bind("upstream timeout")
    .execute(&pool)
    .await
    .expect("error row should insert");

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let login_payload = json!({ "username": "admin", "password": "admin" }).to_string();
    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(login_payload))
                .unwrap(),
        )
        .await
        .expect("login request should complete");
    assert_eq!(login_response.status(), StatusCode::NO_CONTENT);

    let session_cookie = login_response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("session cookie should be set")
        .to_string();

    let admin_response = app
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("admin request should complete");
    assert_eq!(admin_response.status(), StatusCode::OK);

    let body = admin_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Recent Poll Runs"));
    assert!(html.contains("fetch_failed"));
    assert!(html.contains("upstream timeout"));
}

#[tokio::test]
async fn create_page_renders_template_options_from_db() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let asset_id = insert_image_asset(&pool, temp.path(), b"img-template").await;
    let meme_id = insert_meme_with_asset(&pool, "Change My Mind", asset_id).await;
    insert_top_state(&pool, meme_id, run_id, 1).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/create")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Select Template"));
    assert!(html.contains("Change My Mind"));
    assert!(html.contains(&format!("/media/image/{asset_id}")));
}

#[tokio::test]
async fn media_route_serves_local_asset_bytes() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let png = render::render_png_bytes(&[]).expect("png should render");
    let asset_id = insert_image_asset(&pool, temp.path(), &png).await;

    let app = runtime_app(pool, temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/media/image/{asset_id}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/png")
    );
}

async fn runtime_app(pool: sqlx::SqlitePool, assets_dir: std::path::PathBuf) -> axum::Router {
    let scheduler = Arc::new(Scheduler::new());
    let poll_runtime = Arc::new(PollRuntime::new(
        pool.clone(),
        assets_dir.clone(),
        10,
        Some("http://127.0.0.1:9/get_memes".to_string()),
    ));
    let auth = Arc::new(AuthService::dev_default());
    let designer = DesignerService::new(pool.clone(), assets_dir);
    web::app_router_runtime(scheduler, poll_runtime, auth, pool, designer)
}

async fn insert_success_run(pool: &sqlx::SqlitePool) -> i64 {
    let now = now_epoch_seconds().to_string();
    sqlx::query(
        "INSERT INTO poll_runs (status, started_at_utc, completed_at_utc, run_key) VALUES (?, ?, ?, NULL)",
    )
    .bind("success")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("run should insert")
    .last_insert_rowid()
}

async fn insert_meme(pool: &sqlx::SqlitePool, title: &str) -> i64 {
    let now = now_epoch_seconds().to_string();
    sqlx::query(
        "INSERT INTO memes (title, page_url, first_seen_at_utc, last_seen_at_utc) VALUES (?, ?, ?, ?)",
    )
    .bind(title)
    .bind("https://imgflip.com")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("meme should insert")
    .last_insert_rowid()
}

async fn insert_meme_with_asset(pool: &sqlx::SqlitePool, title: &str, asset_id: i64) -> i64 {
    let now = now_epoch_seconds().to_string();
    sqlx::query(
        "INSERT INTO memes (title, page_url, first_seen_at_utc, last_seen_at_utc, image_asset_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(title)
    .bind("https://imgflip.com")
    .bind(&now)
    .bind(&now)
    .bind(asset_id)
    .execute(pool)
    .await
    .expect("meme should insert")
    .last_insert_rowid()
}

async fn insert_image_asset(pool: &sqlx::SqlitePool, root: &Path, bytes: &[u8]) -> i64 {
    let sha = format!("{:x}", Sha256::digest(bytes));
    let path = root.join(format!("asset-{sha}.png"));
    std::fs::write(&path, bytes).expect("asset file should write");
    sqlx::query("INSERT INTO image_assets (sha256, disk_path, bytes, mime) VALUES (?, ?, ?, ?)")
        .bind(&sha)
        .bind(path.to_string_lossy().to_string())
        .bind(bytes.len() as i64)
        .bind("image/png")
        .execute(pool)
        .await
        .expect("image asset should insert")
        .last_insert_rowid()
}

async fn insert_top_state(pool: &sqlx::SqlitePool, meme_id: i64, run_id: i64, rank: i64) {
    sqlx::query(
        "INSERT INTO top_state_current (scope, meme_id, rank, last_seen_run_id) VALUES ('api', ?, ?, ?)",
    )
    .bind(meme_id)
    .bind(rank)
    .bind(run_id)
    .execute(pool)
    .await
    .expect("top state should insert");
}

async fn insert_event(pool: &sqlx::SqlitePool, run_id: i64, meme_id: i64, event_type: &str) {
    sqlx::query(
        "INSERT INTO top_state_events (run_id, meme_id, event_type, old_rank, new_rank, at_utc) VALUES (?, ?, ?, 1, 2, ?)",
    )
    .bind(run_id)
    .bind(meme_id)
    .bind(event_type)
    .bind(now_epoch_seconds().to_string())
    .execute(pool)
    .await
    .expect("event should insert");
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
