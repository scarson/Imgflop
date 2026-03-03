use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    thread,
    time::Duration,
};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use imgflop::{
    config::{self, ApiTopN},
    diff::{self, DiffEvent, RankedState},
};
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

#[test]
fn config_parses_integer_top_n() {
    let cfg = config::from_toml(
        r#"
[polling]
api_top_n = 5
history_top_n = 2
"#,
    )
    .expect("config should parse");

    match cfg.polling.api_top_n {
        ApiTopN::Int(value) => assert_eq!(value, 5),
        ApiTopN::Max => panic!("expected integer top_n"),
    }
}

#[test]
fn config_rejects_invalid_values() {
    let bad_history = config::from_toml(
        r#"
[polling]
api_top_n = "max"
history_top_n = 0
"#,
    );
    assert!(bad_history.is_err());

    let bad_top_n = config::from_toml(
        r#"
[polling]
api_top_n = "bad"
history_top_n = 1
"#,
    );
    assert!(bad_top_n.is_err());
}

#[test]
fn diff_emits_expected_change_events() {
    let prev = vec![
        RankedState {
            meme_id: "m1".to_string(),
            rank: 1,
            metadata_hash: Some("a".to_string()),
        },
        RankedState {
            meme_id: "m2".to_string(),
            rank: 2,
            metadata_hash: None,
        },
    ];
    let next = vec![
        RankedState {
            meme_id: "m1".to_string(),
            rank: 2,
            metadata_hash: Some("b".to_string()),
        },
        RankedState {
            meme_id: "m3".to_string(),
            rank: 1,
            metadata_hash: None,
        },
    ];

    let events = diff::compute(&prev, &next);
    assert!(events.iter().any(|event| matches!(
        event,
        DiffEvent::RankChanged {
            meme_id,
            old_rank: 1,
            new_rank: 2
        } if meme_id == "m1"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DiffEvent::MetadataChanged { meme_id } if meme_id == "m1"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DiffEvent::LeftTop {
            meme_id,
            old_rank: 2
        } if meme_id == "m2"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DiffEvent::EnteredTop {
            meme_id,
            new_rank: 1
        } if meme_id == "m3"
    )));
}

#[test]
fn logging_init_is_idempotent() {
    imgflop::ops::logging::init();
    imgflop::ops::logging::init();
}

#[test]
fn route_templates_render_content() {
    assert!(imgflop::web::routes::admin::render().contains("Admin"));
    assert!(imgflop::web::routes::create::render().contains("Create Meme"));
}

#[tokio::test]
async fn login_allows_admin_page_access() {
    let app = imgflop::web::app_router();
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
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, session_cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("admin request should complete");
    assert_eq!(admin_response.status(), StatusCode::OK);
    let admin_body = admin_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(String::from_utf8_lossy(&admin_body).contains("Admin"));

    let logout_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/logout")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("logout request should complete");
    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn create_routes_return_expected_statuses() {
    let app = imgflop::web::app_router();

    let create_page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/create")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("create page request should complete");
    assert_eq!(create_page.status(), StatusCode::OK);

    let create_export = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/create/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("create export request should complete");
    assert_eq!(create_export.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn unauthenticated_admin_poll_is_rejected() {
    let app = imgflop::web::app_router();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/poll")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("poll request should complete");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn binary_serves_health_endpoint() {
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("ephemeral port should bind")
        .local_addr()
        .expect("local addr should resolve")
        .port();
    let temp = TempDir::new().expect("temp dir should create");
    let db_path = temp.path().join("imgflop-test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
    let salt = SaltString::encode_b64(b"fixedsaltfixed12").expect("test salt should encode");
    let password_hash = Argon2::default()
        .hash_password(b"admin", &salt)
        .expect("password hash should build")
        .to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_imgflop"))
        .env("IMGFLOP_BIND", format!("127.0.0.1:{port}"))
        .env("IMGFLOP_DB_URL", db_url)
        .env("IMGFLOP_ASSETS_DIR", temp.path().join("images"))
        .env("ADMIN_USER", "admin")
        .env("ADMIN_PASSWORD_HASH", password_hash)
        .spawn()
        .expect("binary should start");

    let mut response = String::new();
    let mut success = false;

    for _ in 0..40 {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream
                    .write_all(
                        b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    )
                    .expect("request should write");
                stream
                    .read_to_string(&mut response)
                    .expect("response should read");
                success = true;
                break;
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(success, "server never became reachable");
    assert!(response.contains("200 OK"));
    assert!(response.contains("ok"));
}
