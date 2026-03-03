use imgflop::{ops::locking::LockingService, store::db};

struct TestCtx {
    locking: LockingService,
}

impl TestCtx {
    async fn new() -> Self {
        let pool = db::test_pool().await;
        let locking = LockingService::new(pool).await.expect("locking should initialize");
        Self { locking }
    }
}

#[tokio::test]
async fn second_lock_attempt_fails_while_first_active() {
    let ctx = TestCtx::new().await;
    let a = ctx.locking.acquire("poll").await.expect("first lock should acquire");
    let b = ctx.locking.acquire("poll").await;

    assert!(b.is_err());
    drop(a);
}
