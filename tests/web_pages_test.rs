use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn gallery_page_renders_ranked_memes() {
    let app = imgflop::web::app_router();
    let res = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("request should complete");

    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let page = String::from_utf8_lossy(&body);
    assert!(page.contains("Imgflop - Top Memes"));
    assert!(page.contains(r#"<a href="/" class="brand-link">"#));
}

#[tokio::test]
async fn static_logo_route_serves_png() {
    let app = imgflop::web::app_router();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/static/logo.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/png")
    );
}
