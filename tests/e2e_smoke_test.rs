use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn e2e_poll_then_gallery_then_create_export() {
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
        .expect("login should return session cookie")
        .to_string();

    let poll_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/poll")
                .header(header::COOKIE, session_cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("poll request should complete");
    assert_eq!(poll_response.status(), StatusCode::ACCEPTED);

    let gallery_response = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("gallery request should complete");
    assert_eq!(gallery_response.status(), StatusCode::OK);

    let create_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/create/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("create export request should complete");
    assert_eq!(create_response.status(), StatusCode::ACCEPTED);
}
