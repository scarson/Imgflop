pub mod db;

use sqlx::{query_scalar, SqlitePool};

pub async fn created_meme_exists(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = query_scalar("SELECT COUNT(*) FROM created_memes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}
