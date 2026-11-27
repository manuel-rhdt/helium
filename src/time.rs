//! `Instant` that also exists on `wasm32-unknown-unknown`, where `std::time::Instant::now()` panics
//! because the target has no clock. It only feeds the `TIMED:` log lines, so there it reports zero.

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Debug, Clone, Copy)]
pub struct Instant;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Instant {
    pub fn now() -> Instant {
        Instant
    }

    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
