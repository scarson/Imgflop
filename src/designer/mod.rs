use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{SqlitePool, query};

use crate::assets::store::AssetStore;

pub mod render;

use render::TextLayer;

#[derive(Clone)]
pub struct DesignerService {
    pool: SqlitePool,
    asset_store: AssetStore,
}

impl DesignerService {
    pub fn new(pool: SqlitePool, assets_root: PathBuf) -> Self {
        Self {
            asset_store: AssetStore::new(pool.clone(), assets_root),
            pool,
        }
    }

    pub async fn export_with_store(&self, store: bool) -> Result<Option<i64>, sqlx::Error> {
        self.export_with_layers(store, &[]).await
    }

    pub async fn export_with_layers(
        &self,
        store: bool,
        layers: &[TextLayer],
    ) -> Result<Option<i64>, sqlx::Error> {
        let bytes = render::render_png_bytes(layers).map_err(to_sqlx_protocol_error)?;
        if !store {
            return Ok(None);
        }

        let stored_asset = self
            .asset_store
            .store_bytes("image/png", &bytes)
            .await
            .map_err(|err| to_sqlx_protocol_error(format!("{err:?}")))?;

        let asset_id: i64 = sqlx::query_scalar("SELECT id FROM image_assets WHERE sha256 = ?")
            .bind(&stored_asset.sha256)
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
        .bind("IMGFLOP")
        .bind("{\"font\":\"font8x8\",\"scale\":4}")
        .execute(&self.pool)
        .await?;

        for (index, layer) in layers.iter().enumerate() {
            query(
                "INSERT INTO created_meme_layers (created_meme_id, layer_index, layer_text, x, y, style_json) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(created_id)
            .bind(index as i64 + 1)
            .bind(&layer.text)
            .bind(layer.x as f64)
            .bind(layer.y as f64)
            .bind(format!(
                "{{\"scale\":{},\"color\":\"#{:02X}{:02X}{:02X}\"}}",
                layer.scale, layer.color[0], layer.color[1], layer.color[2]
            ))
            .execute(&self.pool)
            .await?;
        }

        Ok(Some(created_id))
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn to_sqlx_protocol_error<T: ToString>(value: T) -> sqlx::Error {
    sqlx::Error::Protocol(value.to_string())
}
