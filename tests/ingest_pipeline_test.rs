use imgflop::{
    diff::RankedState,
    ingest::pipeline::{InMemorySource, IngestPipeline},
};

struct TestCtx {
    pipeline: IngestPipeline<InMemorySource>,
}

impl TestCtx {
    fn with_fake_source(snapshots: Vec<Vec<RankedState>>) -> Self {
        let source = InMemorySource::new(snapshots);
        let pipeline = IngestPipeline::new(source);
        Self { pipeline }
    }

    async fn run_poll(&self) -> Result<(), String> {
        self.pipeline.run_poll().await
    }

    async fn count_events(&self) -> usize {
        self.pipeline.event_count().await
    }
}

#[tokio::test]
async fn unchanged_second_run_writes_zero_events() {
    let ctx = TestCtx::with_fake_source(vec![fixture_items()]);
    ctx.run_poll().await.expect("first poll should succeed");
    let first = ctx.count_events().await;

    ctx.run_poll().await.expect("second poll should succeed");
    let second = ctx.count_events().await;

    assert_eq!(first, second);
}

fn fixture_items() -> Vec<RankedState> {
    vec![
        RankedState {
            meme_id: "m1".to_string(),
            rank: 1,
            metadata_hash: None,
        },
        RankedState {
            meme_id: "m2".to_string(),
            rank: 2,
            metadata_hash: None,
        },
    ]
}
