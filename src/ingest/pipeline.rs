use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::header::CONTENT_TYPE;
use sqlx::SqlitePool;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    assets::store::AssetStore,
    diff::{self, DiffEvent, RankedState},
    sources::{MemeCandidate, api::ImgflipApiClient},
    store,
};

pub trait RankedSource: Send + Sync {
    fn fetch_ranked(&self) -> Result<Vec<RankedState>, String>;
}

pub struct InMemorySource {
    state: Mutex<InMemorySourceState>,
}

struct InMemorySourceState {
    snapshots: Vec<Vec<RankedState>>,
    index: usize,
}

impl InMemorySource {
    pub fn new(snapshots: Vec<Vec<RankedState>>) -> Self {
        Self {
            state: Mutex::new(InMemorySourceState {
                snapshots,
                index: 0,
            }),
        }
    }
}

impl RankedSource for InMemorySource {
    fn fetch_ranked(&self) -> Result<Vec<RankedState>, String> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| "source lock poisoned".to_string())?;

        if guard.snapshots.is_empty() {
            return Ok(Vec::new());
        }

        let idx = if guard.index < guard.snapshots.len() {
            guard.index
        } else {
            guard.snapshots.len() - 1
        };
        guard.index += 1;

        Ok(guard.snapshots[idx].clone())
    }
}

pub struct IngestPipeline<S: RankedSource> {
    source: S,
    state: AsyncMutex<PipelineState>,
}

struct PipelineState {
    current: Vec<RankedState>,
    events: Vec<DiffEvent>,
}

pub struct PollRunSummary {
    pub run_id: i64,
    pub events_written: usize,
    pub images_downloaded: usize,
}

pub struct PersistedPoller {
    pool: SqlitePool,
    http_client: reqwest::Client,
    asset_store: AssetStore,
    history_top_n: u32,
    scope: &'static str,
    source_name: &'static str,
}

impl PersistedPoller {
    pub fn new(pool: SqlitePool, assets_root: PathBuf, history_top_n: u32) -> Self {
        Self {
            asset_store: AssetStore::new(pool.clone(), assets_root),
            pool,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("reqwest client should build"),
            history_top_n: history_top_n.max(1),
            scope: "api",
            source_name: "imgflip_api",
        }
    }

    pub async fn run_api_poll(&self, client: &ImgflipApiClient) -> Result<PollRunSummary, String> {
        self.run_api_poll_with_top_n(client, None).await
    }

    pub async fn run_api_poll_with_top_n(
        &self,
        client: &ImgflipApiClient,
        api_top_n: Option<u32>,
    ) -> Result<PollRunSummary, String> {
        let candidates = client.fetch_memes_with_top_n(api_top_n).await?;
        self.run_with_candidates(candidates).await
    }

    pub async fn run_with_candidates(
        &self,
        candidates: Vec<MemeCandidate>,
    ) -> Result<PollRunSummary, String> {
        match self.run_with_candidates_inner(candidates).await {
            Ok(summary) => Ok(summary),
            Err(err) => {
                let _ = store::record_poll_run_error(&self.pool, "poll_persist_error", &err).await;
                Err(err)
            }
        }
    }

    async fn run_with_candidates_inner(
        &self,
        candidates: Vec<MemeCandidate>,
    ) -> Result<PollRunSummary, String> {
        let mut images_downloaded = 0usize;
        let mut downloaded_assets: HashMap<String, i64> = HashMap::new();
        for candidate in &candidates {
            match self.try_download_asset(candidate).await {
                Ok(Some(asset_id)) => {
                    images_downloaded += 1;
                    downloaded_assets.insert(candidate.source_meme_id.clone(), asset_id);
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(source_meme_id = %candidate.source_meme_id, error = %err, "image download skipped");
                }
            }
        }

        let now = now_epoch_seconds().to_string();
        let mut tx = self.pool.begin().await.map_err(|err| err.to_string())?;

        let run_insert = sqlx::query(
            "INSERT INTO poll_runs (status, started_at_utc, completed_at_utc, run_key) VALUES (?, ?, NULL, NULL)",
        )
        .bind("running")
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        let run_id = run_insert.last_insert_rowid();

        let mut next_rows: Vec<(i64, u32)> = Vec::new();
        for candidate in &candidates {
            let meme_id = self
                .upsert_meme_record(
                    &mut tx,
                    candidate,
                    &now,
                    downloaded_assets.get(&candidate.source_meme_id).copied(),
                )
                .await
                .map_err(|err| err.to_string())?;
            next_rows.push((meme_id, candidate.rank));
        }

        next_rows.sort_by_key(|(_, rank)| *rank);
        next_rows.truncate(self.history_top_n as usize);

        let previous_rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT meme_id, rank FROM top_state_current WHERE scope = ? ORDER BY rank",
        )
        .bind(self.scope)
        .fetch_all(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

        let previous_state: Vec<RankedState> = previous_rows
            .into_iter()
            .map(|(meme_id, rank)| RankedState {
                meme_id: meme_id.to_string(),
                rank: rank as u32,
                metadata_hash: None,
            })
            .collect();

        let next_state: Vec<RankedState> = next_rows
            .iter()
            .map(|(meme_id, rank)| RankedState {
                meme_id: meme_id.to_string(),
                rank: *rank,
                metadata_hash: None,
            })
            .collect();

        let events = diff::compute(&previous_state, &next_state);
        for event in &events {
            let (event_type, meme_id, old_rank, new_rank) = event_parts(event)?;
            sqlx::query(
                "INSERT INTO top_state_events (run_id, meme_id, event_type, old_rank, new_rank, at_utc) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(run_id)
            .bind(meme_id)
            .bind(event_type)
            .bind(old_rank.map(i64::from))
            .bind(new_rank.map(i64::from))
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        }

        sqlx::query("DELETE FROM top_state_current WHERE scope = ?")
            .bind(self.scope)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;

        for (meme_id, rank) in &next_rows {
            sqlx::query(
                "INSERT INTO top_state_current (scope, meme_id, rank, last_seen_run_id) VALUES (?, ?, ?, ?)",
            )
            .bind(self.scope)
            .bind(meme_id)
            .bind(i64::from(*rank))
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;
        }

        sqlx::query("UPDATE poll_runs SET status = ?, completed_at_utc = ? WHERE id = ?")
            .bind("success")
            .bind(&now)
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| err.to_string())?;

        tx.commit().await.map_err(|err| err.to_string())?;

        Ok(PollRunSummary {
            run_id,
            events_written: events.len(),
            images_downloaded,
        })
    }

    async fn upsert_meme_record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        candidate: &MemeCandidate,
        now: &str,
        image_asset_id: Option<i64>,
    ) -> Result<i64, sqlx::Error> {
        let existing_meme_id: Option<i64> = sqlx::query_scalar(
            "SELECT meme_id FROM source_records WHERE source = ? AND source_meme_id = ?",
        )
        .bind(self.source_name)
        .bind(&candidate.source_meme_id)
        .fetch_optional(&mut **tx)
        .await?;

        match existing_meme_id {
            Some(meme_id) => {
                sqlx::query(
                    "UPDATE memes SET title = ?, page_url = ?, last_seen_at_utc = ?, image_asset_id = COALESCE(?, image_asset_id) WHERE id = ?",
                )
                .bind(&candidate.name)
                .bind(&candidate.page_url)
                .bind(now)
                .bind(image_asset_id)
                .bind(meme_id)
                .execute(&mut **tx)
                .await?;
                Ok(meme_id)
            }
            None => {
                let inserted = sqlx::query(
                    "INSERT INTO memes (title, page_url, first_seen_at_utc, last_seen_at_utc, image_asset_id) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&candidate.name)
                .bind(&candidate.page_url)
                .bind(now)
                .bind(now)
                .bind(image_asset_id)
                .execute(&mut **tx)
                .await?;
                let meme_id = inserted.last_insert_rowid();

                sqlx::query(
                    "INSERT INTO source_records (source, source_meme_id, meme_id, raw_payload) VALUES (?, ?, ?, NULL)",
                )
                .bind(self.source_name)
                .bind(&candidate.source_meme_id)
                .bind(meme_id)
                .execute(&mut **tx)
                .await?;

                Ok(meme_id)
            }
        }
    }

    async fn try_download_asset(&self, candidate: &MemeCandidate) -> Result<Option<i64>, String> {
        let response = self
            .http_client
            .get(&candidate.image_url)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if !response.status().is_success() {
            return Ok(None);
        }

        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| err.to_string())?;

        let stored_asset = self
            .asset_store
            .store_bytes(&mime, &bytes)
            .await
            .map_err(|err| format!("{err:?}"))?;
        let asset_id: i64 = sqlx::query_scalar("SELECT id FROM image_assets WHERE sha256 = ?")
            .bind(&stored_asset.sha256)
            .fetch_one(&self.pool)
            .await
            .map_err(|err| err.to_string())?;

        Ok(Some(asset_id))
    }
}

impl<S: RankedSource> IngestPipeline<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            state: AsyncMutex::new(PipelineState {
                current: Vec::new(),
                events: Vec::new(),
            }),
        }
    }

    pub async fn run_poll(&self) -> Result<(), String> {
        let next = self.source.fetch_ranked()?;
        let mut state = self.state.lock().await;

        let events = diff::compute(&state.current, &next);
        state.events.extend(events);
        state.current = next;

        Ok(())
    }

    pub async fn run_poll_recording_errors(&self, pool: &SqlitePool) -> Result<(), String> {
        match self.run_poll().await {
            Ok(()) => Ok(()),
            Err(err) => {
                let _ = store::record_poll_run_error(pool, "source_fetch_error", &err).await;
                tracing::error!(error_kind = "source_fetch_error", message = %err, "poll failed");
                Err(err)
            }
        }
    }

    pub async fn event_count(&self) -> usize {
        self.state.lock().await.events.len()
    }
}

type EventParts = (&'static str, i64, Option<u32>, Option<u32>);

fn event_parts(event: &DiffEvent) -> Result<EventParts, String> {
    match event {
        DiffEvent::EnteredTop { meme_id, new_rank } => Ok((
            "entered_top",
            meme_id.parse::<i64>().map_err(|err| err.to_string())?,
            None,
            Some(*new_rank),
        )),
        DiffEvent::LeftTop { meme_id, old_rank } => Ok((
            "left_top",
            meme_id.parse::<i64>().map_err(|err| err.to_string())?,
            Some(*old_rank),
            None,
        )),
        DiffEvent::RankChanged {
            meme_id,
            old_rank,
            new_rank,
        } => Ok((
            "rank_changed",
            meme_id.parse::<i64>().map_err(|err| err.to_string())?,
            Some(*old_rank),
            Some(*new_rank),
        )),
        DiffEvent::MetadataChanged { meme_id } => Ok((
            "metadata_changed",
            meme_id.parse::<i64>().map_err(|err| err.to_string())?,
            None,
            None,
        )),
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
