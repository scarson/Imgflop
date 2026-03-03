use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Router,
};

use crate::ops::scheduler::Scheduler;

pub fn app_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

pub fn app_router_with_scheduler(scheduler: Arc<Scheduler>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/admin/poll", post(trigger_manual_poll))
        .with_state(scheduler)
}

async fn trigger_manual_poll(State(scheduler): State<Arc<Scheduler>>) -> StatusCode {
    scheduler.trigger_manual().await;
    StatusCode::ACCEPTED
}
