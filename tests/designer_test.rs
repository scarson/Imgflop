use std::path::Path;

use imgflop::{
    designer::{DesignerService, render::TextLayer},
    store::db,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct TestCtx {
    pool: sqlx::SqlitePool,
    designer: DesignerService,
    _temp: TempDir,
}

impl TestCtx {
    async fn new() -> Self {
        let pool = db::test_pool().await;
        let temp = TempDir::new().expect("temp dir should create");
        let designer = DesignerService::new(pool.clone(), temp.path().to_path_buf());
        Self {
            pool,
            designer,
            _temp: temp,
        }
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

#[tokio::test]
async fn stored_export_with_template_sets_base_meme_id() {
    let ctx = TestCtx::new().await;
    let template_id = insert_template_meme(&ctx.pool, ctx._temp.path()).await;
    let layer = TextLayer {
        text: "Top text".to_string(),
        x: 18,
        y: 20,
        scale: 3,
        color: [255, 255, 255, 255],
    };

    let created_id = ctx
        .designer
        .export_from_template(Some(template_id), true, &[layer])
        .await
        .expect("template export should succeed")
        .expect("stored export should return id");

    let base_meme_id: Option<i64> =
        sqlx::query_scalar("SELECT base_meme_id FROM created_memes WHERE id = ?")
            .bind(created_id)
            .fetch_one(&ctx.pool)
            .await
            .expect("created meme row should exist");
    assert_eq!(base_meme_id, Some(template_id));
}

async fn insert_template_meme(pool: &sqlx::SqlitePool, root: &Path) -> i64 {
    let png = imgflop::designer::render::render_png_bytes(&[]).expect("png should render");
    let sha = format!("{:x}", Sha256::digest(&png));
    let path = root.join(format!("template-{sha}.png"));
    std::fs::write(&path, &png).expect("template image should write");

    let asset_id = sqlx::query(
        "INSERT INTO image_assets (sha256, disk_path, bytes, mime) VALUES (?, ?, ?, ?)",
    )
    .bind(&sha)
    .bind(path.to_string_lossy().to_string())
    .bind(png.len() as i64)
    .bind("image/png")
    .execute(pool)
    .await
    .expect("asset row should insert")
    .last_insert_rowid();

    let now = "1700000000";
    sqlx::query(
        "INSERT INTO memes (title, page_url, first_seen_at_utc, last_seen_at_utc, image_asset_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind("Template Meme")
    .bind("https://imgflip.com")
    .bind(now)
    .bind(now)
    .bind(asset_id)
    .execute(pool)
    .await
    .expect("meme row should insert")
    .last_insert_rowid()
}
