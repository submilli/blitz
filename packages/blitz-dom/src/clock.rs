//! Document clocks. See [`Clock`].

use std::sync::Arc;

pub use blitz_traits::clock::{Clock, FrozenClock};

/// The system's monotonic clock, in milliseconds since the first reading in
/// this process.
#[cfg(feature = "system-clock")]
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

#[cfg(feature = "system-clock")]
impl Clock for SystemClock {
    fn now_ms(&self) -> f64 {
        use std::sync::OnceLock;
        use web_time::Instant;
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
    }
}

pub(crate) fn default_clock() -> Arc<dyn Clock> {
    #[cfg(feature = "system-clock")]
    return Arc::new(SystemClock);
    #[cfg(not(feature = "system-clock"))]
    return Arc::new(FrozenClock);
}
