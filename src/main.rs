use std::{path::PathBuf, sync::Arc};

use imgflop::{
    ops::{polling::PollRuntime, scheduler::Scheduler},
    store::db,
    web,
};

#[tokio::main]
async fn main() {
    imgflop::ops::logging::init();

    let database_url = std::env::var("IMGFLOP_DB_URL")
        .unwrap_or_else(|_| "sqlite://imgflop.db?mode=rwc".to_string());
    let assets_root = std::env::var("IMGFLOP_ASSETS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/images"));
    let history_top_n = std::env::var("IMGFLOP_HISTORY_TOP_N")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value >= 1)
        .unwrap_or(100);
    let api_endpoint = std::env::var("IMGFLOP_API_ENDPOINT").ok();

    let pool = db::connect_pool(&database_url)
        .await
        .unwrap_or_else(|err| panic!("failed to initialize database at {database_url}: {err}"));
    let poll_runtime = Arc::new(PollRuntime::new(
        pool,
        assets_root,
        history_top_n,
        api_endpoint,
    ));

    let scheduler = Arc::new(Scheduler::new());
    let app = web::app_router_with_scheduler_and_poll_runtime(scheduler, Some(poll_runtime));
    let bind = std::env::var("IMGFLOP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {bind}: {err}"));

    tracing::info!(bind = %bind, "starting imgflop web server");

    if let Err(err) = axum::serve(listener, app).await {
        panic!("server error: {err}");
    }
}
