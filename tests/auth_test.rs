use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[tokio::test]
async fn admin_route_requires_login() {
    let app = imgflop::web::app_router();
    let res = app
        .oneshot(Request::builder().uri("/admin").body(Body::empty()).unwrap())
        .await
        .expect("request should complete");

    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
