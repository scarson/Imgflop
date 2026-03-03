use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
async fn gallery_search_filters_top_memes() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let one = insert_meme(&pool, "Drake Hotline").await;
    let two = insert_meme(&pool, "Distracted Boyfriend").await;
    insert_top_state(&pool, one, run_id, 1).await;
    insert_top_state(&pool, two, run_id, 2).await;
    let local_asset = insert_image_asset(&pool, temp.path(), b"local-meme").await;
    insert_created_meme_with_template(&pool, local_asset, two).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/?q=drake")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Drake Hotline"));
    assert!(!html.contains("Distracted Boyfriend"));
}

#[tokio::test]
async fn gallery_renders_local_memes_section() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let output_asset = insert_image_asset(&pool, temp.path(), b"created-meme").await;
    insert_created_meme(&pool, output_asset).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Local Memes"));
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
    assert!(html.contains(&format!("/create/{meme_id}")));
}

#[tokio::test]
async fn create_designer_page_renders_large_template_editor() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let asset_id = insert_image_asset(&pool, temp.path(), b"img-template").await;
    let meme_id = insert_meme_with_asset(&pool, "This Is Fine", asset_id).await;
    insert_top_state(&pool, meme_id, run_id, 1).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/create/{meme_id}"))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Designer"));
    assert!(html.contains("preview-stage"));
    assert!(html.contains(&format!("/media/image/{asset_id}")));
}

#[tokio::test]
async fn admin_upload_adds_template_to_create_gallery() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
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
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let session_cookie = login_response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("session cookie should be set")
        .to_string();

    let boundary = "X-BOUNDARY-IMGFLOP";
    let payload = multipart_body(
        boundary,
        "Knight Rider",
        "template.png",
        "image/png",
        b"fake-png-template",
    );
    let upload = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/templates/upload")
                .header(header::COOKIE, session_cookie)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(payload))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(upload.status(), StatusCode::SEE_OTHER);

    let create = app
        .oneshot(
            Request::builder()
                .uri("/create")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let html = create.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&html).contains("Knight Rider"));
}

#[tokio::test]
async fn admin_shutdown_signals_graceful_shutdown() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let scheduler = Arc::new(Scheduler::new());
    let poll_runtime = Arc::new(PollRuntime::new(
        pool.clone(),
        temp.path().to_path_buf(),
        10,
        Some("http://127.0.0.1:9/get_memes".to_string()),
    ));
    let auth = Arc::new(AuthService::dev_default());
    let designer = DesignerService::new(pool.clone(), temp.path().to_path_buf());
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let app = web::app_router_runtime_with_shutdown(
        scheduler,
        poll_runtime,
        auth,
        pool,
        designer,
        Some(shutdown_tx),
    );

    let login_payload = json!({ "username": "admin", "password": "admin" }).to_string();
    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(login_payload))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let session_cookie = login_response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("session cookie should be set")
        .to_string();

    let shutdown_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/shutdown")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(shutdown_response.status(), StatusCode::OK);

    tokio::time::timeout(Duration::from_millis(100), shutdown_rx.changed())
        .await
        .expect("shutdown signal should fire")
        .expect("watch channel should remain open");
    assert!(*shutdown_rx.borrow());
}

#[tokio::test]
async fn create_export_downloads_png_and_stores_when_requested() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let run_id = insert_success_run(&pool).await;
    let template_png = render::render_png_bytes(&[]).expect("png should render");
    let asset_id = insert_image_asset(&pool, temp.path(), &template_png).await;
    let meme_id = insert_meme_with_asset(&pool, "Doge", asset_id).await;
    insert_top_state(&pool, meme_id, run_id, 1).await;

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/create/export")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "store": true,
                        "download": true,
                        "base_meme_id": meme_id,
                        "layers": [{"text":"wow","x":20,"y":24,"scale":4,"color_hex":"#FFFFFF"}]
                    })
                    .to_string(),
                ))
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
    assert!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .contains("attachment")
    );

    let created_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM created_memes")
        .fetch_one(&pool)
        .await
        .expect("created count should query");
    assert!(created_count >= 1);
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

#[tokio::test]
async fn admin_first_run_setup_creates_db_credential_and_allows_login() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let app = runtime_app_with_auth(
        pool.clone(),
        temp.path().to_path_buf(),
        Arc::new(
            AuthService::new_with_fallback(None, None, 3600, false)
                .expect("no-fallback auth should build"),
        ),
    )
    .await;

    let login_page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/login")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(login_page.status(), StatusCode::OK);
    let page = login_page
        .into_body()
        .collect()
        .await
        .expect("body should read")
        .to_bytes();
    let page_text = String::from_utf8_lossy(&page);
    assert!(page_text.contains("Create Admin Account"));

    let setup_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "mode=setup&username=owner&password=secret123&confirm_password=secret123",
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(setup_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        setup_response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin")
    );

    let stored_user: Option<String> =
        sqlx::query_scalar("SELECT username FROM admin_credentials WHERE id = 1")
            .fetch_optional(&pool)
            .await
            .expect("admin credentials should query");
    assert_eq!(stored_user.as_deref(), Some("owner"));

    let login = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"owner","password":"secret123"}).to_string(),
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn admin_login_accepts_fallback_even_when_db_admin_exists() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let db_hash = imgflop::auth::hash_password("ownerpass").expect("hash should build");
    sqlx::query(
        "INSERT INTO admin_credentials (id, username, password_hash, created_at_utc, updated_at_utc) VALUES (1, ?, ?, ?, ?)",
    )
    .bind("owner")
    .bind(&db_hash)
    .bind(now_epoch_seconds().to_string())
    .bind(now_epoch_seconds().to_string())
    .execute(&pool)
    .await
    .expect("admin credential should insert");

    let app = runtime_app(pool.clone(), temp.path().to_path_buf()).await;

    let fallback_login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"admin"}).to_string(),
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(fallback_login.status(), StatusCode::NO_CONTENT);

    let db_login = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"owner","password":"ownerpass"}).to_string(),
                ))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(db_login.status(), StatusCode::NO_CONTENT);
}

async fn runtime_app(pool: sqlx::SqlitePool, assets_dir: std::path::PathBuf) -> axum::Router {
    let auth = Arc::new(AuthService::dev_default());
    runtime_app_with_auth(pool, assets_dir, auth).await
}

async fn runtime_app_with_auth(
    pool: sqlx::SqlitePool,
    assets_dir: std::path::PathBuf,
    auth: Arc<AuthService>,
) -> axum::Router {
    let scheduler = Arc::new(Scheduler::new());
    let poll_runtime = Arc::new(PollRuntime::new(
        pool.clone(),
        assets_dir.clone(),
        10,
        Some("http://127.0.0.1:9/get_memes".to_string()),
    ));
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

async fn insert_created_meme(pool: &sqlx::SqlitePool, output_asset_id: i64) -> i64 {
    sqlx::query(
        "INSERT INTO created_memes (base_meme_id, output_asset_id, stored, created_at_utc) VALUES (NULL, ?, 1, ?)",
    )
    .bind(output_asset_id)
    .bind(now_epoch_seconds().to_string())
    .execute(pool)
    .await
    .expect("created meme should insert")
    .last_insert_rowid()
}

async fn insert_created_meme_with_template(
    pool: &sqlx::SqlitePool,
    output_asset_id: i64,
    base_meme_id: i64,
) -> i64 {
    sqlx::query(
        "INSERT INTO created_memes (base_meme_id, output_asset_id, stored, created_at_utc) VALUES (?, ?, 1, ?)",
    )
    .bind(base_meme_id)
    .bind(output_asset_id)
    .bind(now_epoch_seconds().to_string())
    .execute(pool)
    .await
    .expect("created meme should insert")
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

fn multipart_body(
    boundary: &str,
    title: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\n{title}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}
