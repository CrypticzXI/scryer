//! Process-wide wasmtime engine for the archive host (RFC 123 §7.2.2).
//!
//! One lazily-initialised `Engine` is shared for the whole process. Its `Config`
//! turns on epoch interruption (for wall-clock cancellation) plus the full wasm
//! feature surface Scryer's PDK targets (owner decision §2.5): SIMD, relaxed
//! SIMD, threads, and exceptions. A single background thread increments the
//! engine epoch on a fixed tick so per-invocation deadlines actually fire
//! without a timer thread per call.

use std::sync::LazyLock;
use std::time::Duration;

use wasmtime::{Config, Engine};

/// Epoch tick interval. Per-invocation deadlines are expressed as a whole
/// number of ticks, so this bounds the timeout granularity (~100 ms).
pub(crate) const EPOCH_TICK: Duration = Duration::from_millis(100);

/// The shared engine. Constructed on first use; the ticker thread is spawned
/// alongside it and lives for the remainder of the process.
static SHARED_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let engine = Engine::new(&archive_engine_config())
        .expect("wasmtime engine config for the archive host must be valid");
    spawn_epoch_ticker(engine.clone());
    engine
});

/// Borrow the process-wide archive engine, initialising it (and its epoch
/// ticker) on first call.
pub(crate) fn shared_engine() -> &'static Engine {
    &SHARED_ENGINE
}

/// Build the archive host `Config`.
///
/// Feature flags mirror the surface the PDK guests may be built against so
/// artifact selection can keep modeling flavors (§7.2.2). SIMD / relaxed SIMD
/// are on by default in wasmtime but are set explicitly for intent. Enabling
/// threads and exceptions is harmless for single-threaded, non-EH modules — the
/// archive artifact today is exactly that (RFC §11 risk row: plain wasip1
/// modules are unaffected).
fn archive_engine_config() -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_simd(true);
    config.wasm_relaxed_simd(true);
    config.wasm_threads(true);
    // Exceptions proposal: available in the resolved wasmtime (46.x, behind the
    // default-on `gc` feature). Enabled per RFC §7.2.2 / owner decision §2.5. No
    // current guest emits EH, so this is a forward-enable — non-EH modules are
    // unaffected. (Threaded-artifact *linkage* via wasmtime-wasi-threads is
    // deferred to WP6; only the engine capability is turned on here.)
    config.wasm_exceptions(true);
    config
}

/// Translate a wall-clock timeout into an epoch-tick deadline for
/// `Store::set_epoch_deadline`. Always at least one tick so a zero/short budget
/// still terminates a wedged guest.
pub(crate) fn deadline_ticks(timeout: Duration) -> u64 {
    let tick = EPOCH_TICK.as_millis().max(1);
    let budget = timeout.as_millis();
    budget.div_ceil(tick).max(1) as u64
}

/// Spawn the single background epoch ticker for `engine`. Detached daemon
/// thread: it holds only a cheap `Engine` clone (an `Arc`) and loops for the
/// life of the process.
fn spawn_epoch_ticker(engine: Engine) {
    std::thread::Builder::new()
        .name("scryer-archive-epoch".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        })
        .expect("spawn archive epoch ticker thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_config_is_accepted_by_wasmtime() {
        // Proves the full feature surface (incl. wasm_exceptions) yields a valid
        // Engine on the resolved wasmtime — the RFC §7.2.2 / §11 spike check.
        Engine::new(&archive_engine_config()).expect("archive engine config must build");
    }

    #[test]
    fn deadline_ticks_rounds_up_and_has_floor() {
        assert_eq!(deadline_ticks(Duration::from_millis(0)), 1);
        assert_eq!(deadline_ticks(Duration::from_millis(1)), 1);
        assert_eq!(deadline_ticks(EPOCH_TICK), 1);
        assert_eq!(deadline_ticks(EPOCH_TICK * 2), 2);
        // 1-hour archive budget at a 100ms tick.
        assert_eq!(deadline_ticks(Duration::from_secs(3600)), 36_000);
    }

    #[test]
    fn shared_engine_is_stable() {
        let a = shared_engine();
        let b = shared_engine();
        assert!(std::ptr::eq(a, b));
    }
}
