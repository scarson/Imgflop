use imgflop::{designer::DesignerService, store::db};

struct TestCtx {
    pool: sqlx::SqlitePool,
    designer: DesignerService,
}

impl TestCtx {
    async fn new() -> Self {
        let pool = db::test_pool().await;
        let designer = DesignerService::new(pool.clone());
        Self { pool, designer }
    }

    async fn export_with_store(&self, store: bool) -> Result<i64, String> {
        self.designer
            .export_with_store(store)
            .await
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "export was not stored".to_string())
    }

    async fn created_exists(&self, id: i64) -> bool {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM created_memes WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("count query should succeed");
        count > 0
    }
}

#[tokio::test]
async fn stored_export_creates_created_meme_record() {
    let ctx = TestCtx::new().await;
    let id = ctx
        .export_with_store(true)
        .await
        .expect("export should succeed");
    assert!(ctx.created_exists(id).await);
}
