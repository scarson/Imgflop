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

    pub async fn trigger_manual(&self) -> bool {
        let mut state = self.state.lock().await;
        if state.running {
            state.pending_repoll = true;
            false
        } else {
            state.running = true;
            true
        }
    }

    pub async fn complete_run_and_take_repoll(&self) -> bool {
        let mut state = self.state.lock().await;
        if state.pending_repoll {
            state.pending_repoll = false;
            true
        } else {
            state.running = false;
            false
        }
    }

    pub async fn pending_repoll(&self) -> bool {
        self.state.lock().await.pending_repoll
    }
}
