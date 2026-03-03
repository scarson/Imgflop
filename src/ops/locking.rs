use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::SqlitePool;

#[derive(Debug)]
pub enum AcquireError {
    Busy,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AcquireError {
    fn from(value: sqlx::Error) -> Self {
        Self::Db(value)
    }
}

#[derive(Clone)]
pub struct LockingService {
    pool: SqlitePool,
    lease_seconds: i64,
}

impl LockingService {
    pub async fn new(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS process_locks (
                lock_name TEXT PRIMARY KEY,
                leased_until INTEGER NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self {
            pool,
            lease_seconds: 60,
        })
    }

    pub async fn acquire(&self, name: &str) -> Result<LockLease, AcquireError> {
        let now = now_epoch_seconds();
        let leased_until = now + self.lease_seconds;

        let inserted = sqlx::query(
            r#"
            INSERT OR IGNORE INTO process_locks (lock_name, leased_until)
            VALUES (?, ?)
            "#,
        )
        .bind(name)
        .bind(leased_until)
        .execute(&self.pool)
        .await?;

        if inserted.rows_affected() == 0 {
            let stolen = sqlx::query(
                r#"
                UPDATE process_locks
                SET leased_until = ?
                WHERE lock_name = ? AND leased_until < ?
                "#,
            )
            .bind(leased_until)
            .bind(name)
            .bind(now)
            .execute(&self.pool)
            .await?;

            if stolen.rows_affected() == 0 {
                return Err(AcquireError::Busy);
            }
        }

        Ok(LockLease {
            pool: self.pool.clone(),
            lock_name: Arc::<str>::from(name),
            released: false,
        })
    }
}

pub struct LockLease {
    pool: SqlitePool,
    lock_name: Arc<str>,
    released: bool,
}

impl LockLease {
    pub async fn release(mut self) -> Result<(), sqlx::Error> {
        self.released = true;
        sqlx::query("DELETE FROM process_locks WHERE lock_name = ?")
            .bind(&*self.lock_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl Drop for LockLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }

        let pool = self.pool.clone();
        let lock_name = self.lock_name.clone();

        tokio::spawn(async move {
            let _ = sqlx::query("DELETE FROM process_locks WHERE lock_name = ?")
                .bind(&*lock_name)
                .execute(&pool)
                .await;
        });
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
