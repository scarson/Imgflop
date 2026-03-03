use std::sync::Arc;

use imgflop::{ops::scheduler::Scheduler, web};

#[tokio::main]
async fn main() {
    imgflop::ops::logging::init();
    let scheduler = Arc::new(Scheduler::new());
    let _app = web::app_router_with_scheduler(scheduler);
}
