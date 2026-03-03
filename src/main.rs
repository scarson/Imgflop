use std::sync::Arc;

use imgflop::{ops::scheduler::Scheduler, web};

#[tokio::main]
async fn main() {
    imgflop::ops::logging::init();

    let scheduler = Arc::new(Scheduler::new());
    let app = web::app_router_with_scheduler(scheduler);
    let bind = std::env::var("IMGFLOP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {bind}: {err}"));

    tracing::info!(bind = %bind, "starting imgflop web server");

    if let Err(err) = axum::serve(listener, app).await {
        panic!("server error: {err}");
    }
}
