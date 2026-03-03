use tokio::sync::Mutex;

#[derive(Debug, Default)]
struct SchedulerState {
    running: bool,
    pending_repoll: bool,
}

#[derive(Debug, Default)]
pub struct Scheduler {
    state: Mutex<SchedulerState>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn mark_poll_running(&self) {
        let mut state = self.state.lock().await;
        state.running = true;
    }

    pub async fn mark_poll_complete(&self) {
        let mut state = self.state.lock().await;
        state.running = false;
    }

    pub async fn trigger_manual(&self) {
        let mut state = self.state.lock().await;
        if state.running {
            state.pending_repoll = true;
        } else {
            state.running = true;
        }
    }

    pub async fn pending_repoll(&self) -> bool {
        self.state.lock().await.pending_repoll
    }
}
