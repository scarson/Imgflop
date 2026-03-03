use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{Request, StatusCode, header},
    routing::get,
};
use imgflop::{
    ingest::pipeline::PersistedPoller,
    ops::{polling::PollRuntime, scheduler::Scheduler},
    sources::api::ImgflipApiClient,
    store::db,
    web,
};
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn admin_poll_endpoint_runs_persisted_poller_when_configured() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let api_addr = spawn_api_server().await;

    let poller = Arc::new(PersistedPoller::new(
        pool.clone(),
        temp.path().to_path_buf(),
        10,
    ));
    let api_client = ImgflipApiClient::new(format!("http://{api_addr}/get_memes"));
    let poll_runtime = Arc::new(PollRuntime::from_parts(poller, api_client));
    let scheduler = Arc::new(Scheduler::new());
    let app = web::app_router_with_scheduler_and_poll_runtime(scheduler, Some(poll_runtime));

    let login_payload = json!({ "username": "admin", "password": "admin" }).to_string();
    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(login_payload))
                .expect("login request should build"),
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

    let poll_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/poll")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .expect("poll request should build"),
        )
        .await
        .expect("poll request should complete");
    assert_eq!(poll_response.status(), StatusCode::ACCEPTED);

    let run_persisted = wait_for(
        Duration::from_secs(3),
        Duration::from_millis(50),
        || async {
            let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM poll_runs")
                .fetch_one(&pool)
                .await
                .expect("poll run count should query");
            runs > 0
        },
    )
    .await;
    assert!(
        run_persisted,
        "manual poll should persist at least one poll run"
    );

    let current_state: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_current")
        .fetch_one(&pool)
        .await
        .expect("top_state_current count should query");
    assert_eq!(current_state, 2);

    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_events")
        .fetch_one(&pool)
        .await
        .expect("top_state_events count should query");
    assert_eq!(events, 2);

    let assets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_assets")
        .fetch_one(&pool)
        .await
        .expect("image_assets count should query");
    assert_eq!(assets, 2);
}

async fn wait_for<F, Fut>(timeout: Duration, interval: Duration, mut probe: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = tokio::time::Instant::now();
    while start.elapsed() < timeout {
        if probe().await {
            return true;
        }
        tokio::time::sleep(interval).await;
    }
    false
}

async fn spawn_api_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server bind should succeed");
    let addr = listener.local_addr().expect("local addr should resolve");

    let payload = format!(
        "{{\"success\":true,\"data\":{{\"memes\":[{{\"id\":\"11\",\"name\":\"One\",\"url\":\"http://{addr}/img-1.png\",\"width\":100,\"height\":100}},{{\"id\":\"22\",\"name\":\"Two\",\"url\":\"http://{addr}/img-2.png\",\"width\":100,\"height\":100}}]}}}}"
    );

    let app = Router::new()
        .route(
            "/get_memes",
            get(move || {
                let body = payload.clone();
                async move { ([("content-type", "application/json")], body) }
            }),
        )
        .route(
            "/img-1.png",
            get(|| async {
                (
                    [("content-type", "image/png")],
                    Bytes::from_static(b"image-1"),
                )
            }),
        )
        .route(
            "/img-2.png",
            get(|| async {
                (
                    [("content-type", "image/png")],
                    Bytes::from_static(b"image-2"),
                )
            }),
        );

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    addr
}
