use std::{fs, path::PathBuf, sync::Arc, time::Duration};

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
    let assets_root = PathBuf::from(&config.assets_dir);
    fs::create_dir_all(&assets_root)
        .unwrap_or_else(|err| panic!("failed to create assets dir {}: {err}", config.assets_dir));

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
        AuthService::new(
            config.auth.admin_user.clone(),
            config.auth.admin_password_hash.clone(),
            config.auth.session_ttl_secs,
            config.auth.secure_cookie,
        )
        .unwrap_or_else(|err| panic!("invalid auth configuration: {err}")),
    );

    let scheduler = Arc::new(Scheduler::new());
    let app = web::app_router_runtime(
        Arc::clone(&scheduler),
        Arc::clone(&poll_runtime),
        auth,
        pool,
        designer,
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

    if let Err(err) = axum::serve(listener, app).await {
        panic!("server error: {err}");
    }
}
