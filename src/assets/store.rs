use std::{fs, path::PathBuf};

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Debug)]
pub enum AssetError {
    Io(std::io::Error),
    Db(sqlx::Error),
}

impl From<std::io::Error> for AssetError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<sqlx::Error> for AssetError {
    fn from(value: sqlx::Error) -> Self {
        Self::Db(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAsset {
    pub sha256: String,
    pub path: String,
}

#[derive(Clone)]
pub struct AssetStore {
    pool: SqlitePool,
    root_dir: PathBuf,
}

impl AssetStore {
    pub fn new(pool: SqlitePool, root_dir: PathBuf) -> Self {
        Self { pool, root_dir }
    }

    pub async fn store_bytes(&self, mime: &str, bytes: &[u8]) -> Result<StoredAsset, AssetError> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let ext = extension_for_mime(mime);
        let dir = self.root_dir.join(&hash[0..2]);
        fs::create_dir_all(&dir)?;

        let path = dir.join(format!("{hash}.{ext}"));
        if !path.exists() {
            fs::write(&path, bytes)?;
        }

        let path_string = path.to_string_lossy().into_owned();
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO image_assets (sha256, disk_path, bytes, mime)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&hash)
        .bind(&path_string)
        .bind(bytes.len() as i64)
        .bind(mime)
        .execute(&self.pool)
        .await?;

        Ok(StoredAsset {
            sha256: hash,
            path: path_string,
        })
    }
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}
