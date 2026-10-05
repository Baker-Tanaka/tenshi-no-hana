use core::cell::RefCell;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time::Instant;

pub(crate) const CMD_VEL_TIMEOUT_MS: u64 = 500;

struct Motion {
    linear: f32,
    angular: f32,
    updated_at: Option<Instant>,
}

static MOTION: Mutex<CriticalSectionRawMutex, RefCell<Motion>> = Mutex::new(RefCell::new(Motion {
    linear: 0.0,
    angular: 0.0,
    updated_at: None,
}));

pub(crate) fn publish_motion_state(linear: f32, angular: f32) {
    MOTION.lock(|state| {
        *state.borrow_mut() = Motion {
            linear,
            angular,
            updated_at: Some(Instant::now()),
        };
    });
}

pub(crate) fn read_motion_state_with_timeout(now: Instant) -> (f32, f32) {
    MOTION.lock(|state| {
        let state = state.borrow();
        match state.updated_at {
            Some(last)
                if now.as_micros().saturating_sub(last.as_micros())
                    <= CMD_VEL_TIMEOUT_MS * 1_000 =>
            {
                (state.linear, state.angular)
            }
            _ => (0.0, 0.0),
        }
    })
}
