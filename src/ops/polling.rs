use std::{path::PathBuf, sync::Arc};

use sqlx::SqlitePool;

use crate::{
    config::ApiTopN,
    ingest::pipeline::{PersistedPoller, PollRunSummary},
    ops::scheduler::Scheduler,
    sources::api::ImgflipApiClient,
};

#[derive(Clone)]
pub struct PollRuntime {
    poller: Arc<PersistedPoller>,
    api_client: ImgflipApiClient,
    api_top_n: ApiTopN,
}

impl PollRuntime {
    pub fn new(
        pool: SqlitePool,
        assets_root: PathBuf,
        history_top_n: u32,
        api_endpoint: Option<String>,
    ) -> Self {
        Self::new_with_api_top_n(pool, assets_root, ApiTopN::Max, history_top_n, api_endpoint)
    }

    pub fn new_with_api_top_n(
        pool: SqlitePool,
        assets_root: PathBuf,
        api_top_n: ApiTopN,
        history_top_n: u32,
        api_endpoint: Option<String>,
    ) -> Self {
        let poller = Arc::new(PersistedPoller::new(pool, assets_root, history_top_n));
        let api_client = match api_endpoint {
            Some(endpoint) => ImgflipApiClient::new(endpoint),
            None => ImgflipApiClient::default_public(),
        };
        Self {
            poller,
            api_client,
            api_top_n,
        }
    }

    pub fn from_parts(poller: Arc<PersistedPoller>, api_client: ImgflipApiClient) -> Self {
        Self {
            poller,
            api_client,
            api_top_n: ApiTopN::Max,
        }
    }

    pub async fn run_once(&self) -> Result<PollRunSummary, String> {
        self.poller
            .run_api_poll_with_top_n(&self.api_client, self.api_top_n_limit())
            .await
    }

    fn api_top_n_limit(&self) -> Option<u32> {
        match self.api_top_n {
            ApiTopN::Max => None,
            ApiTopN::Int(value) => Some(value),
        }
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
