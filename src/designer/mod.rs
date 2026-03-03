use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use sqlx::{query, query_scalar, SqlitePool};

pub mod render;

#[derive(Clone)]
pub struct DesignerService {
    pool: SqlitePool,
}

impl DesignerService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn export_with_store(&self, store: bool) -> Result<Option<i64>, sqlx::Error> {
        let bytes = render::render_png_bytes();
        if !store {
            return Ok(None);
        }

        let hash = format!("{:x}", Sha256::digest(&bytes));
        query(
            "INSERT OR IGNORE INTO image_assets (sha256, disk_path, bytes, mime) VALUES (?, ?, ?, ?)",
        )
        .bind(&hash)
        .bind(format!("created/{hash}.png"))
        .bind(bytes.len() as i64)
        .bind("image/png")
        .execute(&self.pool)
        .await?;

        let asset_id: i64 = query_scalar("SELECT id FROM image_assets WHERE sha256 = ?")
            .bind(&hash)
            .fetch_one(&self.pool)
            .await?;

        let created_at_utc = now_epoch_seconds().to_string();
        let insert = query(
            "INSERT INTO created_memes (base_meme_id, output_asset_id, stored, created_at_utc) VALUES (NULL, ?, 1, ?)",
        )
        .bind(asset_id)
        .bind(&created_at_utc)
        .execute(&self.pool)
        .await?;
        let created_id = insert.last_insert_rowid();

        query(
            "INSERT INTO created_meme_layers (created_meme_id, layer_index, layer_text, x, y, style_json) VALUES (?, 0, ?, 0.5, 0.5, ?)",
        )
        .bind(created_id)
        .bind("placeholder")
        .bind("{\"font\":\"Impact\",\"size\":48}")
        .execute(&self.pool)
        .await?;

        Ok(Some(created_id))
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
