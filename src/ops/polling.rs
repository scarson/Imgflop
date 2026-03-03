use std::{path::PathBuf, sync::Arc};

use sqlx::SqlitePool;

use crate::{
    ingest::pipeline::{PersistedPoller, PollRunSummary},
    ops::scheduler::Scheduler,
    sources::api::ImgflipApiClient,
};

#[derive(Clone)]
pub struct PollRuntime {
    poller: Arc<PersistedPoller>,
    api_client: ImgflipApiClient,
}

impl PollRuntime {
    pub fn new(
        pool: SqlitePool,
        assets_root: PathBuf,
        history_top_n: u32,
        api_endpoint: Option<String>,
    ) -> Self {
        let poller = Arc::new(PersistedPoller::new(pool, assets_root, history_top_n));
        let api_client = match api_endpoint {
            Some(endpoint) => ImgflipApiClient::new(endpoint),
            None => ImgflipApiClient::default_public(),
        };
        Self { poller, api_client }
    }

    pub fn from_parts(poller: Arc<PersistedPoller>, api_client: ImgflipApiClient) -> Self {
        Self { poller, api_client }
    }

    pub async fn run_once(&self) -> Result<PollRunSummary, String> {
        self.poller.run_api_poll(&self.api_client).await
    }
}

pub async fn trigger_and_spawn(scheduler: Arc<Scheduler>, poll_runtime: Option<Arc<PollRuntime>>) {
    if !scheduler.trigger_manual().await {
        return;
    }

    tokio::spawn(async move {
        run_poll_worker(scheduler, poll_runtime).await;
    });
}

pub async fn run_poll_worker(scheduler: Arc<Scheduler>, poll_runtime: Option<Arc<PollRuntime>>) {
    loop {
        if let Some(runtime) = poll_runtime.as_ref() {
            if let Err(err) = runtime.run_once().await {
                tracing::error!(error = %err, "poll worker run failed");
            }
        } else {
            tracing::warn!("poll requested without poll runtime configured");
        }

        if !scheduler.complete_run_and_take_repoll().await {
            break;
        }
    }
}
