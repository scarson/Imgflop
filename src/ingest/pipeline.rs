use std::sync::Mutex;

use sqlx::SqlitePool;
use tokio::sync::Mutex as AsyncMutex;

use crate::diff::{self, DiffEvent, RankedState};
use crate::store;

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
