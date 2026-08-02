//! Process-wide wasmtime engine for the archive host.
//!
//! One lazily-initialised `Engine` is shared for the whole process. Its `Config`
//! turns on epoch interruption (for wall-clock cancellation) and pins the
//! safety-relevant knobs (wasm stack bound, linear-memory guard page, native
//! unwind info) so a future wasmtime bump cannot silently weaken them. Only the
//! default-on SIMD / relaxed-SIMD proposals are exposed to guests; threads and
//! exceptions are deliberately left off (see `archive_engine_config`). A single
//! background thread increments the engine epoch on a fixed tick so
//! per-invocation deadlines actually fire without a timer thread per call.

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

/// Async command-model engine. WASI `poll_oneoff` and friends yield through
/// Wasmtime async support, so adapter-level timeouts can cancel sleeps without
/// parking Tokio blocking threads.
static SHARED_ASYNC_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let engine = Engine::new(&archive_engine_config())
        .expect("async wasmtime engine config for the command host must be valid");
    spawn_epoch_ticker(engine.clone());
    engine
});

/// Borrow the process-wide archive engine, initialising it (and its epoch
/// ticker) on first call.
pub(crate) fn shared_engine() -> &'static Engine {
    &SHARED_ENGINE
}

/// Borrow the async command-model engine, initialising it (and its epoch ticker)
/// on first use.
pub(crate) fn shared_async_engine() -> &'static Engine {
    &SHARED_ASYNC_ENGINE
}

/// Build the archive host `Config`.
///
/// This engine is process-wide and shared by every untrusted plugin module, so
/// its feature surface is kept to the minimum today's guests actually consume.
/// SIMD / relaxed SIMD are on by default in wasmtime and set explicitly for
/// intent.
///
/// `wasm_threads` and `wasm_exceptions` are deliberately NOT enabled. No
/// shipping guest (the archive + subtitle-command artifacts, or the legacy
/// Extism-compat reactors) uses either, and turning them on process-wide only
/// adds attack surface: threads bring a `memory.atomic.wait` blocking primitive
/// that epoch interruption cannot preempt (a worker-thread DoS), and exceptions
/// bring needless Cranelift EH codegen. They are dropped pending a real WP6
/// consumer. When one lands, re-enable them behind a PER-ARTIFACT declared
/// feature gate on a purpose-built engine — never by flipping them back on for
/// this shared, untrusted engine.
fn archive_engine_config() -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_simd(true);
    config.wasm_relaxed_simd(true);
    config.wasm_threads(false);
    // Pin the safety-relevant posture to wasmtime 46's current defaults so a
    // future bump cannot silently weaken it. This is a sync engine (no async
    // path), so there is no async-stack coupling to worry about.
    config.max_wasm_stack(512 * 1024); // 512 KiB wasm stack bound (wasmtime default).
    config.guard_before_linear_memory(true); // OOB guard page before linear memory.
    config.native_unwind_info(true); // Keep native unwind info for trap/backtrace fidelity.
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
        // Proves the pinned, minimized feature surface still yields a valid
        // Engine on the resolved wasmtime (46.0.1).
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

        let async_a = shared_async_engine();
        let async_b = shared_async_engine();
        assert!(std::ptr::eq(async_a, async_b));
        assert!(!std::ptr::eq(a, async_a));
    }
}
