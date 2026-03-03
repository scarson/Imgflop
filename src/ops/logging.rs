use std::sync::OnceLock;

static LOGGING_INIT: OnceLock<()> = OnceLock::new();

pub fn init() {
    LOGGING_INIT.get_or_init(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("info")
            .json()
            .try_init();
    });
}
