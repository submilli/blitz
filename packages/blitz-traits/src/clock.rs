//! The clock a document reads "now" from.

/// A monotonic millisecond timeline. Only differences between readings are
/// meaningful: the origin is up to the embedder.
///
/// Documents read time through this trait rather than the system clock, so
/// an embedder can drive them from virtual time, or run them on targets with
/// no clock (such as `wasm32-unknown-unknown` without JavaScript).
pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> f64;
}

/// A clock that never advances. Time-dependent behaviour (double clicks,
/// smooth scrolling, scrollbar fading) sees every event as simultaneous.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrozenClock;

impl Clock for FrozenClock {
    fn now_ms(&self) -> f64 {
        0.0
    }
}
