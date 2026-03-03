use std::{path::PathBuf, sync::Arc, time::Duration};

use imgflop::{
    auth::AuthService,
    config::RuntimeConfig,
    designer::DesignerService,
    ops::{
        polling::{PollRuntime, trigger_and_spawn},
        scheduler::Scheduler,
    },
    store::db,
    web,
};

#[tokio::main]
async fn main() {
    imgflop::ops::logging::init();

    let config = RuntimeConfig::from_env()
        .unwrap_or_else(|err| panic!("runtime configuration error: {err}"));
    config
        .validate_startup()
        .unwrap_or_else(|err| panic!("runtime startup validation error: {err}"));
    let assets_root = PathBuf::from(&config.assets_dir);

    let pool = db::connect_pool(&config.database_url)
        .await
        .unwrap_or_else(|err| {
            panic!(
                "failed to initialize database at {}: {err}",
                config.database_url
            )
        });
    let poll_runtime = Arc::new(PollRuntime::new_with_api_top_n(
        pool.clone(),
        assets_root.clone(),
        config.api_top_n.clone(),
        config.history_top_n,
        config.api_endpoint.clone(),
    ));
    let designer = DesignerService::new(pool.clone(), assets_root);
    let auth = Arc::new(
        AuthService::new_with_fallback(
            config.auth.fallback_admin_user.clone(),
            config.auth.fallback_admin_password_hash.clone(),
            config.auth.session_ttl_secs,
            config.auth.secure_cookie,
        )
        .unwrap_or_else(|err| panic!("invalid auth configuration: {err}")),
    );

    let scheduler = Arc::new(Scheduler::new());
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let app = web::app_router_runtime_with_shutdown(
        Arc::clone(&scheduler),
        Arc::clone(&poll_runtime),
        auth,
        pool,
        designer,
        Some(shutdown_tx),
    );
    let scheduled_runtime = Arc::clone(&poll_runtime);
    let scheduled_scheduler = Arc::clone(&scheduler);
    let schedule_interval = Duration::from_secs(config.poll_interval_secs);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(schedule_interval);
        loop {
            ticker.tick().await;
            trigger_and_spawn(
                Arc::clone(&scheduled_scheduler),
                Some(Arc::clone(&scheduled_runtime)),
            )
            .await;
        }
    });

    let bind = config.bind.clone();

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {bind}: {err}"));

    tracing::info!(bind = %bind, "starting imgflop web server");

    let shutdown_future = async move {
        let _ = shutdown_rx.changed().await;
        tracing::info!("received shutdown request");
    };

    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_future)
        .await
    {
        panic!("server error: {err}");
    }
}
