pub mod db;

use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{SqlitePool, query, query_scalar};

pub async fn created_meme_exists(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = query_scalar("SELECT COUNT(*) FROM created_memes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

pub async fn record_poll_run_error(
    pool: &SqlitePool,
    error_kind: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    let now = now_epoch_seconds().to_string();
    let run_insert = query(
        "INSERT INTO poll_runs (status, started_at_utc, completed_at_utc, run_key) VALUES (?, ?, ?, NULL)",
    )
    .bind("failed")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    let run_id = run_insert.last_insert_rowid();
    query(
        "INSERT INTO poll_run_errors (run_id, at_utc, severity, error_kind, message, context_json) VALUES (?, ?, ?, ?, ?, NULL)",
    )
    .bind(run_id)
    .bind(&now)
    .bind("error")
    .bind(error_kind)
    .bind(message)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn poll_run_errors_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    query_scalar("SELECT COUNT(*) FROM poll_run_errors")
        .fetch_one(pool)
        .await
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
