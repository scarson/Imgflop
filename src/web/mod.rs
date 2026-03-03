use axum::{routing::get, Router};

pub fn app_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}
