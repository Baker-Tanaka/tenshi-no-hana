use defmt::{info, warn};
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use micro_xrce_dds_rs::Session;

const SYNC_PERIOD_SECS: u64 = 10;

#[task]
pub(crate) async fn run(session: Session) -> ! {
    loop {
        synchronize(session).await;
        Timer::after(Duration::from_secs(SYNC_PERIOD_SECS)).await;
    }
}

pub(crate) async fn synchronize(session: Session) {
    match session.sync_clock().await {
        Ok(offset) => {
            info!("[microros] Agent clock synchronized offset_ns={}", offset);
        }
        Err(error) => warn!("[microros] Agent clock synchronization failed: {}", error),
    }
}
