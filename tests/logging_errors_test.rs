use imgflop::{
    diff::RankedState,
    ingest::pipeline::{IngestPipeline, RankedSource},
    store::db,
};

struct FailingSource;

impl RankedSource for FailingSource {
    fn fetch_ranked(&self) -> Result<Vec<RankedState>, String> {
        Err("forced source failure".to_string())
    }
}

struct TestCtx {
    pool: sqlx::SqlitePool,
    pipeline: IngestPipeline<FailingSource>,
}

impl TestCtx {
    async fn with_failing_source() -> Self {
        let pool = db::test_pool().await;
        let pipeline = IngestPipeline::new(FailingSource);
        Self { pool, pipeline }
    }

    async fn run_poll(&self) -> Result<(), String> {
        self.pipeline.run_poll_recording_errors(&self.pool).await
    }

    async fn poll_run_errors_count(&self) -> i64 {
        imgflop::store::poll_run_errors_count(&self.pool)
            .await
            .expect("error count query should succeed")
    }
}

#[tokio::test]
async fn failed_run_inserts_poll_run_error() {
    let ctx = TestCtx::with_failing_source().await;
    let _ = ctx.run_poll().await;
    assert!(ctx.poll_run_errors_count().await > 0);
}
