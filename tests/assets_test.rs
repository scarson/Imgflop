use std::sync::Arc;

use imgflop::{assets::store::AssetStore, store::db};

struct TestCtx {
    assets: Arc<AssetStore>,
}

impl TestCtx {
    async fn new() -> Self {
        let pool = db::test_pool().await;
        let temp = tempfile::tempdir().expect("temp dir should create");
        let assets = AssetStore::new(pool, temp.keep());
        Self {
            assets: Arc::new(assets),
        }
    }
}

#[tokio::test]
async fn same_file_hash_dedupes_assets() {
    let ctx = TestCtx::new().await;
    let a = ctx
        .assets
        .store_bytes("image/png", b"abc")
        .await
        .expect("first write should succeed");
    let b = ctx
        .assets
        .store_bytes("image/png", b"abc")
        .await
        .expect("second write should succeed");

    assert_eq!(a.sha256, b.sha256);
    assert_eq!(a.path, b.path);
}
