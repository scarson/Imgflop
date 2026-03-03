use std::sync::Arc;

use imgflop::ops::scheduler::Scheduler;

struct TestCtx {
    scheduler: Arc<Scheduler>,
}

impl TestCtx {
    async fn new() -> Self {
        Self {
            scheduler: Arc::new(Scheduler::new()),
        }
    }

    async fn start_long_poll(&self) {
        self.scheduler.mark_poll_running().await;
    }

    async fn manual_trigger(&self) {
        self.scheduler.trigger_manual().await;
    }

    async fn pending_repoll(&self) -> bool {
        self.scheduler.pending_repoll().await
    }
}

#[tokio::test]
async fn manual_trigger_while_running_sets_pending_repoll() {
    let ctx = TestCtx::new().await;
    ctx.start_long_poll().await;
    ctx.manual_trigger().await;
    assert!(ctx.pending_repoll().await);
}
