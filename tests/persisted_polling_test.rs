use std::{net::SocketAddr, path::PathBuf};

use axum::{Router, body::Bytes, routing::get};
use imgflop::{
    ingest::pipeline::PersistedPoller,
    sources::{MemeCandidate, api::ImgflipApiClient},
    store::db,
};
use tempfile::TempDir;

#[tokio::test]
async fn persisted_poll_writes_state_assets_and_change_only_events() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let image_addr = spawn_image_server().await;

    let poller = PersistedPoller::new(pool.clone(), temp.path().to_path_buf(), 10);

    let candidates = vec![
        MemeCandidate {
            source_meme_id: "1".to_string(),
            name: "One".to_string(),
            image_url: format!("http://{image_addr}/img-1.png"),
            page_url: "https://imgflip.test/1".to_string(),
            width: 100,
            height: 100,
            rank: 1,
        },
        MemeCandidate {
            source_meme_id: "2".to_string(),
            name: "Two".to_string(),
            image_url: format!("http://{image_addr}/img-2.png"),
            page_url: "https://imgflip.test/2".to_string(),
            width: 100,
            height: 100,
            rank: 2,
        },
    ];

    let first = poller
        .run_with_candidates(candidates.clone())
        .await
        .expect("first poll should succeed");
    assert_eq!(first.events_written, 2);
    assert_eq!(first.images_downloaded, 2);

    let second = poller
        .run_with_candidates(candidates)
        .await
        .expect("second poll should succeed");
    assert_eq!(second.events_written, 0);

    let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_events")
        .fetch_one(&pool)
        .await
        .expect("events count should query");
    assert_eq!(total_events, 2);

    let current_state: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_current")
        .fetch_one(&pool)
        .await
        .expect("state count should query");
    assert_eq!(current_state, 2);

    let image_assets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_assets")
        .fetch_one(&pool)
        .await
        .expect("asset count should query");
    assert_eq!(image_assets, 2);

    assert!(has_stored_asset_file(temp.path()).await);
}

#[tokio::test]
async fn api_client_poll_persists_top_state() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let api_addr = spawn_api_server().await;
    let api_client = ImgflipApiClient::new(format!("http://{api_addr}/get_memes"));
    let poller = PersistedPoller::new(pool.clone(), temp.path().to_path_buf(), 10);

    let summary = poller
        .run_api_poll(&api_client)
        .await
        .expect("api poll should succeed");
    assert_eq!(summary.events_written, 2);

    let source_records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_records")
        .fetch_one(&pool)
        .await
        .expect("source record count should query");
    assert_eq!(source_records, 2);

    let top_state: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_current")
        .fetch_one(&pool)
        .await
        .expect("top state count should query");
    assert_eq!(top_state, 2);

    let linked_assets: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memes WHERE image_asset_id IS NOT NULL")
            .fetch_one(&pool)
            .await
            .expect("linked asset count should query");
    assert_eq!(linked_assets, 2);
}

#[tokio::test]
async fn api_top_n_and_history_top_n_are_independent() {
    let pool = db::test_pool().await;
    let temp = TempDir::new().expect("temp dir should create");
    let api_addr = spawn_three_meme_api_server().await;
    let api_client = ImgflipApiClient::new(format!("http://{api_addr}/get_memes"));
    let poller = PersistedPoller::new(pool.clone(), temp.path().to_path_buf(), 2);

    let summary = poller
        .run_api_poll_with_top_n(&api_client, Some(3))
        .await
        .expect("api poll should succeed");
    assert_eq!(summary.events_written, 2);

    let source_records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_records")
        .fetch_one(&pool)
        .await
        .expect("source record count should query");
    assert_eq!(source_records, 3);

    let top_state: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_current")
        .fetch_one(&pool)
        .await
        .expect("top state count should query");
    assert_eq!(top_state, 2);

    let top_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM top_state_events")
        .fetch_one(&pool)
        .await
        .expect("top state events count should query");
    assert_eq!(top_events, 2);
}

async fn spawn_image_server() -> SocketAddr {
    let app = Router::new()
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server bind should succeed");
    let addr = listener.local_addr().expect("local addr should resolve");

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    addr
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
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    addr
}

async fn spawn_three_meme_api_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("server bind should succeed");
    let addr = listener.local_addr().expect("local addr should resolve");

    let payload = format!(
        "{{\"success\":true,\"data\":{{\"memes\":[{{\"id\":\"11\",\"name\":\"One\",\"url\":\"http://{addr}/img-1.png\",\"width\":100,\"height\":100}},{{\"id\":\"22\",\"name\":\"Two\",\"url\":\"http://{addr}/img-2.png\",\"width\":100,\"height\":100}},{{\"id\":\"33\",\"name\":\"Three\",\"url\":\"http://{addr}/img-3.png\",\"width\":100,\"height\":100}}]}}}}"
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
        )
        .route(
            "/img-3.png",
            get(|| async {
                (
                    [("content-type", "image/png")],
                    Bytes::from_static(b"image-3"),
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
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    addr
}

async fn has_stored_asset_file(root: &std::path::Path) -> bool {
    contains_file(root.to_path_buf())
}

fn contains_file(root: PathBuf) -> bool {
    let mut stack = vec![root];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                return true;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    false
}
