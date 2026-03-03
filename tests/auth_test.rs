use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn admin_route_requires_login() {
    let app = imgflop::web::app_router();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin/login")
    );
}

#[tokio::test]
async fn browser_login_form_returns_session_cookie() {
    let app = imgflop::web::app_router();
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
    assert!(String::from_utf8_lossy(&page).contains("Admin Login"));

    let login = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=admin"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        login
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin")
    );
    assert!(login.headers().get(header::SET_COOKIE).is_some());
}

#[tokio::test]
async fn browser_logout_redirects_home_and_invalidates_session() {
    let app = imgflop::web::app_router();
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=admin"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("session cookie should exist")
        .to_string();

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/logout")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        logout
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/")
    );

    let admin = app
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(admin.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        admin
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin/login")
    );
}
