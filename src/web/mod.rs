use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json,
    Router,
};
use serde::Deserialize;

use crate::{
    auth::{session::extract_session_token, AuthService},
    ops::scheduler::Scheduler,
};

pub mod routes;

#[derive(Clone)]
struct AppState {
    scheduler: Arc<Scheduler>,
    auth: Arc<AuthService>,
}

pub fn app_router() -> Router {
    let scheduler = Arc::new(Scheduler::new());
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState { scheduler, auth })
}

pub fn app_router_with_scheduler(scheduler: Arc<Scheduler>) -> Router {
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState { scheduler, auth })
}

fn app_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/", get(gallery_page))
        .route("/create", get(create_page))
        .route("/create/export", post(create_export))
        .route("/health", get(|| async { "ok" }))
        .route("/static/app.css", get(stylesheet))
        .route("/admin", get(admin_home))
        .route("/admin/login", post(admin_login))
        .route("/admin/logout", post(admin_logout))
        .route("/admin/poll", post(trigger_manual_poll))
        .with_state(state)
}

async fn gallery_page() -> Html<&'static str> {
    Html(routes::gallery::render())
}

async fn create_page() -> Html<&'static str> {
    Html(routes::create::render())
}

async fn create_export() -> StatusCode {
    StatusCode::OK
}

async fn stylesheet() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "text/css; charset=utf-8")], include_str!("static/app.css"))
}

async fn admin_home(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if state.auth.is_authenticated_headers(&headers) {
        Html(routes::admin::render()).into_response()
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn admin_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    match state.auth.login(&payload.username, &payload.password) {
        Ok(token) => {
            let cookie = format!("imgflop_session={token}; HttpOnly; SameSite=Lax; Path=/");
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).expect("cookie should be valid"),
            );
            response
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = extract_session_token(&headers) {
        state.auth.logout_token(&token);
    }

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("imgflop_session=; Max-Age=0; Path=/"),
    );
    response
}

async fn trigger_manual_poll(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    if !state.auth.is_authenticated_headers(&headers) {
        return StatusCode::UNAUTHORIZED;
    }

    state.scheduler.trigger_manual().await;
    StatusCode::ACCEPTED
}
