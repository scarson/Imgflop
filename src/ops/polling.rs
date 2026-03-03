use std::{path::PathBuf, sync::Arc};

use sqlx::SqlitePool;

use crate::{
    ingest::pipeline::{PersistedPoller, PollRunSummary},
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
