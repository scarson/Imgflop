use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    auth::{AuthService, session::extract_session_token},
    ops::{polling::PollRuntime, scheduler::Scheduler},
};

pub mod routes;

#[derive(Clone)]
struct AppState {
    scheduler: Arc<Scheduler>,
    poll_runtime: Option<Arc<PollRuntime>>,
    auth: Arc<AuthService>,
}

pub fn app_router() -> Router {
    let scheduler = Arc::new(Scheduler::new());
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState {
        scheduler,
        poll_runtime: None,
        auth,
    })
}

pub fn app_router_with_scheduler(scheduler: Arc<Scheduler>) -> Router {
    app_router_with_scheduler_and_poll_runtime(scheduler, None)
}

pub fn app_router_with_scheduler_and_poll_runtime(
    scheduler: Arc<Scheduler>,
    poll_runtime: Option<Arc<PollRuntime>>,
) -> Router {
    let auth = Arc::new(AuthService::dev_default());
    app_router_with_state(AppState {
        scheduler,
        poll_runtime,
        auth,
    })
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
    StatusCode::ACCEPTED
}

async fn stylesheet() -> ([(&'static str, &'static str); 1], &'static str) {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("static/app.css"),
    )
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

    let should_start_worker = state.scheduler.trigger_manual().await;
    if should_start_worker {
        let scheduler = Arc::clone(&state.scheduler);
        let poll_runtime = state.poll_runtime.clone();
        tokio::spawn(async move {
            run_manual_poll_worker(scheduler, poll_runtime).await;
        });
    }

    StatusCode::ACCEPTED
}

async fn run_manual_poll_worker(scheduler: Arc<Scheduler>, poll_runtime: Option<Arc<PollRuntime>>) {
    loop {
        if let Some(runtime) = poll_runtime.as_ref() {
            if let Err(err) = runtime.run_once().await {
                tracing::error!(error = %err, "manual poll failed");
            }
        } else {
            tracing::warn!("manual poll requested without poll runtime configured");
        }

        if !scheduler.complete_run_and_take_repoll().await {
            break;
        }
    }
}
