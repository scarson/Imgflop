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
        self.export_from_template(None, store, layers).await
    }

    pub async fn export_from_template(
        &self,
        base_meme_id: Option<i64>,
        store: bool,
        layers: &[TextLayer],
    ) -> Result<Option<i64>, sqlx::Error> {
        let effective_layers: Vec<TextLayer> = if layers.is_empty() {
            vec![TextLayer::default()]
        } else {
            layers.to_vec()
        };
        let bytes = self
            .render_png_from_template(base_meme_id, &effective_layers)
            .await?;
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
            "INSERT INTO created_memes (base_meme_id, output_asset_id, stored, created_at_utc) VALUES (?, ?, 1, ?)",
        )
        .bind(base_meme_id)
        .bind(asset_id)
        .bind(&created_at_utc)
        .execute(&self.pool)
        .await?;
        let created_id = insert.last_insert_rowid();

        for (index, layer) in effective_layers.iter().enumerate() {
            query(
                "INSERT INTO created_meme_layers (created_meme_id, layer_index, layer_text, x, y, style_json) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(created_id)
            .bind(index as i64)
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

    pub async fn render_png_from_template(
        &self,
        base_meme_id: Option<i64>,
        layers: &[TextLayer],
    ) -> Result<Vec<u8>, sqlx::Error> {
        let base_image = self.load_template_image_bytes(base_meme_id).await?;
        let effective_layers: Vec<TextLayer> = if layers.is_empty() {
            vec![TextLayer::default()]
        } else {
            layers.to_vec()
        };
        render::render_png_bytes_with_base(base_image.as_deref(), &effective_layers)
            .map_err(to_sqlx_protocol_error)
    }

    pub async fn upload_template(
        &self,
        title: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<i64, sqlx::Error> {
        if !mime.starts_with("image/") {
            return Err(to_sqlx_protocol_error(
                "template upload must be an image MIME type",
            ));
        }
        if title.trim().is_empty() {
            return Err(to_sqlx_protocol_error("template title must not be empty"));
        }

        let stored_asset = self
            .asset_store
            .store_bytes(mime, bytes)
            .await
            .map_err(|err| to_sqlx_protocol_error(format!("{err:?}")))?;
        let asset_id: i64 = sqlx::query_scalar("SELECT id FROM image_assets WHERE sha256 = ?")
            .bind(&stored_asset.sha256)
            .fetch_one(&self.pool)
            .await?;

        let now = now_epoch_seconds().to_string();
        let insert = query(
            "INSERT INTO memes (title, page_url, first_seen_at_utc, last_seen_at_utc, image_asset_id) VALUES (?, NULL, ?, ?, ?)",
        )
        .bind(title.trim())
        .bind(&now)
        .bind(&now)
        .bind(asset_id)
        .execute(&self.pool)
        .await?;
        let meme_id = insert.last_insert_rowid();

        query(
            "INSERT INTO source_records (source, source_meme_id, meme_id, raw_payload) VALUES ('admin_upload', ?, ?, NULL)",
        )
        .bind(meme_id.to_string())
        .bind(meme_id)
        .execute(&self.pool)
        .await?;

        Ok(meme_id)
    }

    async fn load_template_image_bytes(
        &self,
        base_meme_id: Option<i64>,
    ) -> Result<Option<Vec<u8>>, sqlx::Error> {
        let Some(meme_id) = base_meme_id else {
            return Ok(None);
        };

        let row = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT a.disk_path
            FROM memes m
            JOIN image_assets a ON a.id = m.image_asset_id
            WHERE m.id = ?
            "#,
        )
        .bind(meme_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((disk_path,)) = row else {
            return Err(sqlx::Error::RowNotFound);
        };

        let bytes = std::fs::read(&disk_path).map_err(to_sqlx_protocol_error)?;
        Ok(Some(bytes))
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
