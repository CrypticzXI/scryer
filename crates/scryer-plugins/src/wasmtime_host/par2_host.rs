//! Host-thread PAR2 damaged-member reconstruct (RFC 123 WP2.5, §13.6 item 6).
//!
//! A plain single-threaded `wasm32-wasip1` guest cannot run the parallel
//! GF(2^16) Reed-Solomon *reconstruct*: it has no rayon worker pool, and the
//! decode-matrix construction overflows its ~1 MiB stack (weaver's own
//! `wpar2` example runs repair on a 256 MiB native stack). The owner decided
//! (§13.6) to dispatch the whole solve to the HOST via one import,
//! `scryer_par2_reconstruct`, which extends the zero-copy crypto host pattern
//! ([`super::crypto_host`]): it slices the guest's exported linear memory
//! directly and runs the solve across host threads over that memory.
//!
//! The guest lays a small problem spec plus the bulk slice regions into its own
//! memory (the [`scryer_plugin_sdk::par2_reconstruct`] wire format) and calls
//! the import. The host then:
//!
//! 1. parses + bounds-checks the descriptor against the real memory size;
//! 2. rejects out-of-range dimensions, over-cap dimensions, and — critically —
//!    any overlap among output regions or between an output and a source (the
//!    soundness gate for parallel writes), BEFORE spawning any thread;
//! 3. builds the `n_out × n_src` repair coefficient matrix on a 256 MiB native
//!    stack (the "decode-matrix construction" that overflows the guest stack);
//! 4. runs the GF(2^16) multiply-accumulate matmul across host threads, writing
//!    the disjoint guest output regions in place.
//!
//! The whole solve is the real [`weaver_reed_solomon`] crate, so the host repair
//! is byte-identical to weaver's own repair path: the coefficient matrix comes
//! from [`weaver_reed_solomon::matrix::build_repair_matrix`] (the public
//! host-agnostic Gauss-Jordan decode-matrix build, cross-checked byte-for-byte
//! against `weaver-par2`), the field is `weaver_reed_solomon::gf`, and the
//! mul-accumulate kernel is `weaver_reed_solomon::gf_simd::mul_acc_region`.
//!
//! ## Epoch caveat (the proven PoC gap)
//!
//! The engine epoch bounds *guest* code at instruction boundaries, but a
//! synchronous host call runs to completion on the blocking OS thread — the
//! epoch never interrupts it. So this function enforces its OWN wall-clock
//! deadline: it gates entry to the solve, bounds the (uninterruptible, black-box)
//! matrix build with a timed wait on the worker thread, and checks the budget per
//! output row in the matmul. On overrun it returns
//! [`Par2ReconstructStatus::DeadlineExceeded`] (`-7`), recording it in a shared
//! flag so the invocation caller can map it to a timeout `AppError`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use scryer_plugin_sdk::par2_reconstruct::{
    self as abi, PAR2_RECONSTRUCT_IMPORT, PAR2_RECONSTRUCT_NAMESPACE, Par2ReconstructHeader,
    Par2ReconstructStatus,
};
use wasmtime::{Caller, Extern, Linker};
use weaver_reed_solomon::{gf, gf_simd, matrix};

// Return codes (RFC §13.6 frozen descriptor). Sourced from the shared SDK enum
// so the host and the guest agree on one definition.
const OK: i64 = Par2ReconstructStatus::Ok.code();
const E_NO_MEMORY: i64 = Par2ReconstructStatus::NoMemory.code();
const E_DESC: i64 = Par2ReconstructStatus::BadDescriptor.code();
const E_DIMS: i64 = Par2ReconstructStatus::BadDimensions.code();
const E_REGION: i64 = Par2ReconstructStatus::BadRegion.code();
const E_ALIAS: i64 = Par2ReconstructStatus::Alias.code();
const E_SINGULAR: i64 = Par2ReconstructStatus::Singular.code();
const E_DEADLINE: i64 = Par2ReconstructStatus::DeadlineExceeded.code();
const E_DIM_CAP: i64 = Par2ReconstructStatus::DimensionCap.code();

/// Native stack for the elimination worker. Mirrors weaver's `wpar2` example
/// (256 MiB): the many-slice single-file shape overflows both the wasm guest
/// (~1 MiB) and the native default (Linux 8 MiB / Windows 1 MiB) stacks.
const ELIM_STACK_BYTES: usize = 256 << 20;

/// Hard ceiling on solver worker threads regardless of `available_parallelism`.
const MAX_WORKERS: usize = 256;

/// PAR2 caps the input slice count at 32768; it is also exactly `phi(65535)`,
/// the number of valid GF(2^16) input-slice constants, so it is the natural
/// ceiling for `total_inputs` (and thus `n_out`/`n_avail`).
pub(crate) const DEFAULT_MAX_TOTAL_INPUTS: usize = 32_768;
pub(crate) const DEFAULT_MAX_N_OUT: usize = 32_768;
/// 64 MiB per-slice ceiling (`word_count <= 32 Mi`). Well above any real PAR2
/// slice size; guards a lying descriptor from claiming an absurd single slice.
pub(crate) const DEFAULT_MAX_SLICE_BYTES: usize = 64 * 1024 * 1024;
/// `n_out * n_src` repair-matrix elements (u16), so the coefficient matrix is at
/// most 128 MiB. Bounds the elimination working set independently of the slice
/// data, which is the allocation a memory-only cap leaves open.
pub(crate) const DEFAULT_MAX_REPAIR_MATRIX_ELEMENTS: usize = 64 * 1024 * 1024;
/// Aggregate source + output region bytes ceiling (4 GiB). Primarily an
/// overflow guard; the guest memory cap already bounds the true working set.
pub(crate) const DEFAULT_MAX_AGGREGATE_REGION_BYTES: usize = 4 * 1024 * 1024 * 1024;

/// Operator-tunable dimension ceilings, checked before any allocation or thread
/// spawn. A descriptor exceeding any of these returns [`E_DIM_CAP`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Par2HostCaps {
    pub(crate) max_total_inputs: usize,
    pub(crate) max_n_out: usize,
    pub(crate) max_slice_bytes: usize,
    pub(crate) max_repair_matrix_elements: usize,
    pub(crate) max_aggregate_region_bytes: usize,
}

impl Default for Par2HostCaps {
    fn default() -> Self {
        Self {
            max_total_inputs: DEFAULT_MAX_TOTAL_INPUTS,
            max_n_out: DEFAULT_MAX_N_OUT,
            max_slice_bytes: DEFAULT_MAX_SLICE_BYTES,
            max_repair_matrix_elements: DEFAULT_MAX_REPAIR_MATRIX_ELEMENTS,
            max_aggregate_region_bytes: DEFAULT_MAX_AGGREGATE_REGION_BYTES,
        }
    }
}

/// Per-invocation configuration for the reconstruct host function.
///
/// Built once per archive invocation and captured in the registered closure.
/// The `deadline` is an absolute wall-clock instant (derived from the
/// invocation budget); `deadline_exceeded` is shared with the invocation caller
/// so a host-side timeout surfaces as a timeout `AppError` even though the guest
/// itself exits cleanly with an in-band repair failure.
#[derive(Clone)]
pub(crate) struct Par2HostConfig {
    pub(crate) deadline: Instant,
    pub(crate) caps: Par2HostCaps,
    pub(crate) workers: usize,
    pub(crate) deadline_exceeded: Arc<AtomicBool>,
}

impl Par2HostConfig {
    /// Configure one invocation from an absolute `deadline` and a shared
    /// overflow flag. Worker count is `available_parallelism`, capped.
    pub(crate) fn for_invocation(deadline: Instant, deadline_exceeded: Arc<AtomicBool>) -> Self {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, MAX_WORKERS);
        Self {
            deadline,
            caps: Par2HostCaps::default(),
            workers,
            deadline_exceeded,
        }
    }
}

/// Register `scryer_par2_reconstruct` under `extism:host/user` on `linker`.
///
/// Generic over the store data `T`: the function touches only the guest's
/// exported memory and the captured [`Par2HostConfig`], never the store
/// context, so it composes with any store (the archive host's `HostCtx`, or a
/// bare `()` store in tests) — exactly like [`super::crypto_host`]. It must be
/// registered ONLY on the archive linker path, never on a networked/fleet
/// backing.
pub(crate) fn add_to_linker<T: 'static>(
    linker: &mut Linker<T>,
    config: Par2HostConfig,
) -> wasmtime::Result<()> {
    linker.func_wrap(
        PAR2_RECONSTRUCT_NAMESPACE,
        PAR2_RECONSTRUCT_IMPORT,
        move |caller: Caller<'_, T>, desc_ptr: i64, desc_len: i64| -> i64 {
            host_reconstruct(caller, desc_ptr, desc_len, &config)
        },
    )?;
    Ok(())
}

/// `scryer_par2_reconstruct(desc_ptr, desc_len) -> i64`.
///
/// `desc_len` is currently advisory (reserved); all bounds are derived from the
/// header pointers validated against the real memory size. Returns `0` on
/// success or a negative [`Par2ReconstructStatus`] code.
fn host_reconstruct<T: 'static>(
    mut caller: Caller<'_, T>,
    desc_ptr: i64,
    _desc_len: i64,
    config: &Par2HostConfig,
) -> i64 {
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => return E_NO_MEMORY,
    };
    let data: &mut [u8] = memory.data_mut(&mut caller);
    let mem_len = data.len();
    let desc_ptr = desc_ptr as u64 as usize;

    // -- Parse + structurally validate the header (magic/version/bounds). --
    let Some(header) = abi::parse_header(data, desc_ptr) else {
        return E_DESC;
    };

    // -- Dimension validity (shape must be self-consistent). --
    if header.n_out == 0
        || header.word_count == 0
        || header.word_count.checked_mul(2) != Some(header.slice_bytes)
    {
        return E_DIMS;
    }

    // -- Dimension caps BEFORE any allocation / constants build / thread spawn. --
    if !within_caps(&header, &config.caps) {
        return E_DIM_CAP;
    }

    // -- Small problem-spec arrays (bounds-checked reads over `data`). --
    let (Some(missing), Some(avail), Some(exponents)) = (
        abi::read_u32_array(data, header.missing_idx_ptr, header.n_out),
        abi::read_u32_array(data, header.avail_idx_ptr, header.n_avail),
        abi::read_u32_array(data, header.exponent_ptr, header.n_out),
    ) else {
        return E_DIMS;
    };
    if missing.iter().any(|&i| i as usize >= header.total_inputs)
        || avail.iter().any(|&i| i as usize >= header.total_inputs)
    {
        return E_DIMS;
    }

    let n_src = header.n_src();
    let slice_bytes = header.slice_bytes;

    // -- Read + bounds-check every source region (available data, then recovery
    //    block data — the frozen ABI order). --
    let mut src_ranges: Vec<(usize, usize)> = Vec::with_capacity(n_src);
    for s in 0..n_src {
        match abi::read_table_entry(data, header.src_table_ptr, s) {
            Some((ptr, len)) if len == slice_bytes && checked_in_bounds(ptr, len, mem_len) => {
                src_ranges.push((ptr, len));
            }
            _ => return E_REGION,
        }
    }
    // -- Read + bounds-check every output (missing-slice) region. --
    let mut out_ranges: Vec<(usize, usize)> = Vec::with_capacity(header.n_out);
    for j in 0..header.n_out {
        match abi::read_table_entry(data, header.out_table_ptr, j) {
            Some((ptr, len)) if len == slice_bytes && checked_in_bounds(ptr, len, mem_len) => {
                out_ranges.push((ptr, len));
            }
            _ => return E_REGION,
        }
    }

    // -- DISJOINTNESS: the soundness gate for parallel host writes. Output
    //    regions must be pairwise disjoint AND disjoint from every source. A
    //    lying guest (overlapping outputs) is rejected here, BEFORE any thread
    //    is spawned, so it can never induce a host-side data race. --
    if !pairwise_disjoint(&out_ranges) || intersects_any(&out_ranges, &src_ranges) {
        return E_ALIAS;
    }

    // -- Deadline gate + remaining budget for the solve (the epoch does not bound
    //    host work, so an already-past budget must terminate here). --
    let Some(solve_budget) = config.deadline.checked_duration_since(Instant::now()) else {
        config.deadline_exceeded.store(true, Ordering::Relaxed);
        return E_DEADLINE;
    };
    let deadline = config.deadline;

    // -- Build the repair coefficient matrix HOST-SIDE via weaver's public,
    //    host-agnostic decode-matrix build (byte-identical to weaver-par2's own
    //    repair). It runs on a 256 MiB native worker stack — the construction that
    //    overflows the ~1 MiB guest stack — and is a black box we cannot cancel
    //    mid-run, so we bound it with a timed wait: on overrun we return -7 and
    //    let the worker detach (it only touches its OWN heap, never guest memory,
    //    so this is sound). `constants` is recomputed host-side from
    //    `total_inputs`; the descriptor need not carry it. --
    let constants = gf::input_slice_constants(header.total_inputs);
    let avail_u: Vec<usize> = avail.iter().map(|&x| x as usize).collect();
    let missing_u: Vec<usize> = missing.iter().map(|&x| x as usize).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("scryer-par2-elim".into())
        .stack_size(ELIM_STACK_BYTES)
        .spawn(move || {
            let _ = tx.send(matrix::build_repair_matrix(
                &avail_u, &missing_u, &exponents, &constants,
            ));
        })
        .expect("spawn PAR2 elimination worker");
    let coeffs = match rx.recv_timeout(solve_budget) {
        Ok(Ok(coeffs)) => coeffs,
        Ok(Err(_singular)) => return E_SINGULAR,
        Err(RecvTimeoutError::Timeout) => {
            // The elimination overran the budget; leave the worker to finish and
            // drop its result on the now-dead channel.
            config.deadline_exceeded.store(true, Ordering::Relaxed);
            return E_DEADLINE;
        }
        // The worker panicked (not expected: indices are validated and `constants`
        // covers every index). Fail the solve rather than the host.
        Err(RecvTimeoutError::Disconnected) => return E_SINGULAR,
    };
    let coeffs = Arc::new(coeffs);

    // -- Parallel zero-copy GF matmul into the guest's output regions. --
    //
    // Take the guest memory base as a raw address. The `&mut [u8]` borrow of
    // `data` ends here (its last use); the raw address stays valid for the
    // duration of this SYNCHRONOUS call because (a) the guest is suspended — the
    // single OS thread that entered this host fn is blocked right here, and NO
    // guest thread exists (wasi-threads is not enabled), so the host worker
    // threads below are the only accessors of the linear memory; and (b) this fn
    // never calls `memory.grow`, so the mapping is neither moved nor resized.
    let base = data.as_mut_ptr() as usize;
    let n_out = header.n_out;
    let assignments = partition_rows(n_out, config.workers);
    let cancelled = Arc::new(AtomicBool::new(false));

    std::thread::scope(|scope| {
        for rows in assignments {
            let coeffs = coeffs.clone();
            let cancelled = cancelled.clone();
            let src_ranges = &src_ranges;
            let out_ranges = &out_ranges;
            scope.spawn(move || {
                for j in rows {
                    // Cooperative cancel: the epoch cannot interrupt this host
                    // solve, so each worker checks the wall-clock deadline per
                    // output row and bails, flagging the overrun.
                    if deadline_reached(deadline) {
                        cancelled.store(true, Ordering::Relaxed);
                        return;
                    }
                    let (out_ptr, out_len) = out_ranges[j];
                    // SAFETY: `out_ptr..out_ptr+out_len` was bounds-checked
                    // (`<= mem_len`); output ranges are pairwise disjoint and
                    // disjoint from all sources (validated above); this worker is
                    // the ONLY writer of row `j` (`partition_rows` gives every row
                    // to exactly one worker); the guest is suspended for the whole
                    // call. => this `&mut` slice uniquely aliases its bytes for the
                    // thread's lifetime.
                    let out = unsafe {
                        std::slice::from_raw_parts_mut((base as *mut u8).add(out_ptr), out_len)
                    };
                    out.fill(0);
                    for (s, &(src_ptr, src_len)) in src_ranges.iter().enumerate() {
                        // SAFETY: `src_ptr..src_ptr+src_len` bounds-checked;
                        // sources are read-only and disjoint from every output, so
                        // no `&` here aliases a concurrent `&mut` output write.
                        let src = unsafe {
                            std::slice::from_raw_parts((base as *const u8).add(src_ptr), src_len)
                        };
                        gf_simd::mul_acc_region(coeffs.get(j, s), src, out);
                    }
                }
            });
        }
    });

    if cancelled.load(Ordering::Relaxed) {
        config.deadline_exceeded.store(true, Ordering::Relaxed);
        return E_DEADLINE;
    }
    OK
}

/// True once the wall-clock `deadline` has been reached.
#[inline]
fn deadline_reached(deadline: Instant) -> bool {
    Instant::now() >= deadline
}

/// Check every dimension ceiling with overflow-safe arithmetic.
fn within_caps(header: &Par2ReconstructHeader, caps: &Par2HostCaps) -> bool {
    if header.total_inputs > caps.max_total_inputs
        || header.n_avail > caps.max_total_inputs
        || header.n_out > caps.max_n_out
        || header.slice_bytes > caps.max_slice_bytes
    {
        return false;
    }
    let Some(matrix_elems) = header.n_out.checked_mul(header.n_src()) else {
        return false;
    };
    if matrix_elems > caps.max_repair_matrix_elements {
        return false;
    }
    let aggregate = header
        .n_src()
        .checked_mul(header.slice_bytes)
        .zip(header.n_out.checked_mul(header.slice_bytes))
        .and_then(|(src, out)| src.checked_add(out));
    matches!(aggregate, Some(bytes) if bytes <= caps.max_aggregate_region_bytes)
}

/// Return the checked byte range end iff `[ptr, ptr+len)` fits in `mem_len`
/// (overflow-checked).
fn checked_in_bounds(ptr: usize, len: usize, mem_len: usize) -> bool {
    matches!(ptr.checked_add(len), Some(end) if end <= mem_len)
}

/// True iff no two ranges overlap. O(k log k) sort-and-sweep.
fn pairwise_disjoint(ranges: &[(usize, usize)]) -> bool {
    let mut sorted: Vec<(usize, usize)> = ranges.to_vec();
    sorted.sort_by_key(|&(ptr, _)| ptr);
    sorted.windows(2).all(|w| {
        let (p0, l0) = w[0];
        let (p1, _) = w[1];
        p0 + l0 <= p1
    })
}

/// True iff any range in `a` intersects any range in `b`.
fn intersects_any(a: &[(usize, usize)], b: &[(usize, usize)]) -> bool {
    a.iter()
        .any(|&(pa, la)| b.iter().any(|&(pb, lb)| pa < pb + lb && pb < pa + la))
}

/// Contiguous partition of row indices `0..n_out` across at most `workers`
/// groups (never more groups than rows).
fn partition_rows(n_out: usize, workers: usize) -> Vec<Vec<usize>> {
    let workers = workers.clamp(1, n_out.max(1));
    let base = n_out / workers;
    let extra = n_out % workers;
    let mut groups = Vec::with_capacity(workers);
    let mut start = 0usize;
    for w in 0..workers {
        let take = base + usize::from(w < extra);
        groups.push((start..start + take).collect());
        start += take;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wasmtime::{Engine, Linker, Module, Store};

    // ── A guest that imports the host fn, exports memory, and forwards. ──────
    const RECONSTRUCT_GUEST_WAT: &str = r#"
        (module
          (import "extism:host/user" "scryer_par2_reconstruct"
            (func $rec (param i64 i64) (result i64)))
          (memory (export "memory") 4)
          (func (export "call_reconstruct") (param i64 i64) (result i64)
            (call $rec (local.get 0) (local.get 1))))
    "#;

    /// A guest that imports the host fn but exports NO memory.
    const NO_MEMORY_GUEST_WAT: &str = r#"
        (module
          (import "extism:host/user" "scryer_par2_reconstruct"
            (func $rec (param i64 i64) (result i64)))
          (func (export "call_reconstruct") (param i64 i64) (result i64)
            (call $rec (local.get 0) (local.get 1))))
    "#;

    /// Deterministic source bytes keyed by `(seed, index)` — LE u16 words.
    fn gen_slice(seed: u64, index: usize, word_count: usize) -> Vec<u8> {
        let mut state = seed ^ (0x5011_CE50_0000_0000 + index as u64);
        let mut bytes = vec![0u8; word_count * 2];
        for word in bytes.chunks_exact_mut(2) {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let value = ((z ^ (z >> 31)) & 0xFFFF) as u16;
            word.copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// One PAR2 recovery block for exponent `e` over all original slices, using
    /// weaver's real field + kernel (the encode operation weaver performs).
    fn encode_recovery(
        originals: &[Vec<u8>],
        constants: &[u16],
        exp: u32,
        slice_bytes: usize,
    ) -> Vec<u8> {
        let mut block = vec![0u8; slice_bytes];
        for (i, orig) in originals.iter().enumerate() {
            gf_simd::mul_acc_region(gf::pow(constants[i], exp), orig, &mut block);
        }
        block
    }

    /// A laid-out reconstruct problem in a single guest-memory image.
    struct Problem {
        image: Vec<u8>,
        desc_ptr: usize,
        /// Expected recovered bytes per output row (== original of each missing).
        expected: Vec<Vec<u8>>,
        out_offsets: Vec<usize>,
        slice_bytes: usize,
    }

    /// Build a valid reconstruct problem. `alias_outputs` overlaps two output
    /// regions to exercise the disjointness gate.
    fn build_problem(
        total: usize,
        word_count: usize,
        missing: &[usize],
        alias_outputs: bool,
    ) -> Problem {
        let slice_bytes = word_count * 2;
        let n_out = missing.len();
        let constants = gf::input_slice_constants(total);
        let originals: Vec<Vec<u8>> = (0..total)
            .map(|i| gen_slice(0xABCD, i, word_count))
            .collect();
        let exponents: Vec<u32> = (0..n_out as u32).collect();
        let avail: Vec<usize> = (0..total).filter(|i| !missing.contains(i)).collect();
        let n_avail = avail.len();
        let n_src = n_avail + n_out;

        // Sources: available data (avail order), then recovery blocks (exp order).
        let mut sources: Vec<Vec<u8>> = avail.iter().map(|&i| originals[i].clone()).collect();
        for &exp in &exponents {
            sources.push(encode_recovery(&originals, &constants, exp, slice_bytes));
        }

        // Lay everything out in one image with 8-byte-aligned offsets.
        let mut image = vec![0u8; 4 * 65_536];
        let align = |off: usize| off.next_multiple_of(8);
        let mut cursor = 16usize;

        let mut src_offsets = Vec::with_capacity(n_src);
        for src in &sources {
            cursor = align(cursor);
            image[cursor..cursor + slice_bytes].copy_from_slice(src);
            src_offsets.push(cursor);
            cursor += slice_bytes;
        }
        let mut out_offsets = Vec::with_capacity(n_out);
        for _ in 0..n_out {
            cursor = align(cursor);
            out_offsets.push(cursor);
            cursor += slice_bytes;
        }
        if alias_outputs && n_out >= 2 {
            // Point the second output at the first — an overlapping write.
            out_offsets[1] = out_offsets[0];
        }

        cursor = align(cursor);
        let missing_ptr = cursor;
        for (i, &m) in missing.iter().enumerate() {
            abi::write_u32_array_entry(&mut image, missing_ptr, i, m as u32).unwrap();
        }
        cursor += n_out * 4;

        cursor = align(cursor);
        let avail_ptr = cursor;
        for (i, &a) in avail.iter().enumerate() {
            abi::write_u32_array_entry(&mut image, avail_ptr, i, a as u32).unwrap();
        }
        cursor += n_avail * 4;

        cursor = align(cursor);
        let exponent_ptr = cursor;
        for (i, &e) in exponents.iter().enumerate() {
            abi::write_u32_array_entry(&mut image, exponent_ptr, i, e).unwrap();
        }
        cursor += n_out * 4;

        cursor = align(cursor);
        let src_table_ptr = cursor;
        for (i, &off) in src_offsets.iter().enumerate() {
            abi::write_table_entry(&mut image, src_table_ptr, i, off as u64, slice_bytes as u64)
                .unwrap();
        }
        cursor += n_src * abi::TABLE_ENTRY_LEN;

        cursor = align(cursor);
        let out_table_ptr = cursor;
        for (i, &off) in out_offsets.iter().enumerate() {
            abi::write_table_entry(&mut image, out_table_ptr, i, off as u64, slice_bytes as u64)
                .unwrap();
        }
        cursor += n_out * abi::TABLE_ENTRY_LEN;

        cursor = align(cursor);
        let desc_ptr = cursor;
        let fields = abi::Par2ReconstructHeaderFields {
            total_inputs: total as u32,
            n_out: n_out as u32,
            n_avail: n_avail as u32,
            word_count: word_count as u32,
            flags: 0,
            missing_idx_ptr: missing_ptr as u64,
            avail_idx_ptr: avail_ptr as u64,
            exponent_ptr: exponent_ptr as u64,
            src_table_ptr: src_table_ptr as u64,
            out_table_ptr: out_table_ptr as u64,
        };
        abi::write_header(&mut image[desc_ptr..], &fields).unwrap();

        let expected = missing.iter().map(|&m| originals[m].clone()).collect();
        Problem {
            image,
            desc_ptr,
            expected,
            out_offsets,
            slice_bytes,
        }
    }

    fn far_future() -> Instant {
        Instant::now() + Duration::from_secs(3600)
    }

    fn config_with_deadline(deadline: Instant) -> (Par2HostConfig, Arc<AtomicBool>) {
        let flag = Arc::new(AtomicBool::new(false));
        (Par2HostConfig::for_invocation(deadline, flag.clone()), flag)
    }

    /// Instantiate the reconstruct guest with a given config and return the
    /// store, instance, and memory.
    fn instantiate(config: Par2HostConfig) -> (Store<()>, wasmtime::Instance, wasmtime::Memory) {
        let engine = Engine::default();
        let module = Module::new(&engine, RECONSTRUCT_GUEST_WAT).expect("compile guest");
        let mut linker: Linker<()> = Linker::new(&engine);
        add_to_linker(&mut linker, config).expect("register reconstruct host fn");
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("instantiate guest");
        let memory = instance
            .get_memory(&mut store, "memory")
            .expect("guest exports memory");
        (store, instance, memory)
    }

    /// Write an image, call the host fn through the guest, and return the code.
    fn run(
        store: &mut Store<()>,
        instance: &wasmtime::Instance,
        memory: &wasmtime::Memory,
        image: &[u8],
        desc_ptr: usize,
    ) -> i64 {
        memory.write(&mut *store, 0, image).expect("write image");
        let call = instance
            .get_typed_func::<(i64, i64), i64>(&mut *store, "call_reconstruct")
            .unwrap();
        call.call(&mut *store, (desc_ptr as i64, image.len() as i64))
            .expect("host call did not trap")
    }

    #[test]
    fn reconstructs_original_bytes_through_guest_memory() {
        // A real RS reconstruct: encode recovery with weaver's field, drop
        // slices {2,5,7,10}, and recover them host-side through weaver's kernel.
        let problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);

        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, OK, "reconstruct should succeed");

        for (row, expected) in problem.expected.iter().enumerate() {
            let mut got = vec![0u8; problem.slice_bytes];
            memory
                .read(&store, problem.out_offsets[row], &mut got)
                .unwrap();
            assert_eq!(
                &got, expected,
                "missing row {row} must be recovered exactly"
            );
        }
    }

    #[test]
    fn reconstructs_with_all_inputs_missing() {
        // n_avail == 0: reconstruct purely from recovery blocks.
        let problem = build_problem(4, 32, &[0, 1, 2, 3], false);
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, OK);
        for (row, expected) in problem.expected.iter().enumerate() {
            let mut got = vec![0u8; problem.slice_bytes];
            memory
                .read(&store, problem.out_offsets[row], &mut got)
                .unwrap();
            assert_eq!(&got, expected);
        }
    }

    #[test]
    fn rejects_aliased_output_regions() {
        let problem = build_problem(12, 64, &[2, 5, 7, 10], true);
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(
            code, E_ALIAS,
            "overlapping outputs must be rejected pre-spawn"
        );
    }

    #[test]
    fn rejects_out_of_bounds_region() {
        let mut problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        // Point the first output region far past the end of memory.
        let header = abi::parse_header(&problem.image, problem.desc_ptr).unwrap();
        let past_end = problem.image.len() as u64 + 1_000_000;
        abi::write_table_entry(
            &mut problem.image,
            header.out_table_ptr,
            0,
            past_end,
            problem.slice_bytes as u64,
        )
        .unwrap();
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_REGION);
    }

    #[test]
    fn rejects_wrong_region_length() {
        let mut problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        let header = abi::parse_header(&problem.image, problem.desc_ptr).unwrap();
        // A source region whose length != slice_bytes.
        abi::write_table_entry(
            &mut problem.image,
            header.src_table_ptr,
            0,
            16,
            (problem.slice_bytes as u64) - 2,
        )
        .unwrap();
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_REGION);
    }

    #[test]
    fn rejects_out_of_range_index() {
        let mut problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        let header = abi::parse_header(&problem.image, problem.desc_ptr).unwrap();
        // A missing index >= total_inputs.
        abi::write_u32_array_entry(&mut problem.image, header.missing_idx_ptr, 0, 99).unwrap();
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_DIMS);
    }

    #[test]
    fn rejects_bad_descriptor_magic() {
        let mut problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        // Corrupt the magic at the header.
        problem.image[problem.desc_ptr] ^= 0xFF;
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_DESC);
    }

    #[test]
    fn rejects_dimension_cap_overflow() {
        // A well-formed header claiming more inputs than the cap allows. Nothing
        // is read past the header because the cap trips first.
        let mut image = vec![0u8; 256];
        let desc_ptr = 0usize;
        let fields = abi::Par2ReconstructHeaderFields {
            total_inputs: (DEFAULT_MAX_TOTAL_INPUTS as u32) + 1,
            n_out: 1,
            n_avail: 0,
            word_count: 1,
            flags: 0,
            missing_idx_ptr: 96,
            avail_idx_ptr: 96,
            exponent_ptr: 100,
            src_table_ptr: 104,
            out_table_ptr: 120,
        };
        abi::write_header(&mut image, &fields).unwrap();
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(&mut store, &instance, &memory, &image, desc_ptr);
        assert_eq!(code, E_DIM_CAP);
    }

    #[test]
    fn enforces_host_side_deadline() {
        // An already-elapsed deadline: the host must not run the solve and must
        // report the overrun both in-band (-7) and via the shared flag.
        let problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        let past = Instant::now() - Duration::from_millis(1);
        let (config, flag) = config_with_deadline(past);
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_DEADLINE);
        assert!(
            flag.load(Ordering::Relaxed),
            "deadline flag must be set for the caller"
        );
    }

    #[test]
    fn reports_missing_memory_export() {
        let engine = Engine::default();
        let module = Module::new(&engine, NO_MEMORY_GUEST_WAT).expect("compile no-memory guest");
        let mut linker: Linker<()> = Linker::new(&engine);
        let (config, _flag) = config_with_deadline(far_future());
        add_to_linker(&mut linker, config).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let call = instance
            .get_typed_func::<(i64, i64), i64>(&mut store, "call_reconstruct")
            .unwrap();
        let code = call.call(&mut store, (0, 80)).unwrap();
        assert_eq!(code, E_NO_MEMORY);
    }

    #[test]
    fn weaver_build_repair_matrix_recovers_original() {
        // Exercise weaver's public matrix build directly (the exact API the host
        // uses) plus weaver's kernel: reconstruct the missing slices and confirm
        // they equal the ground-truth originals. Column order is
        // [available | recovery]; weaver already locks byte-identity to
        // weaver-par2, so this is the integration smoke test.
        let total = 8usize;
        let word_count = 16usize;
        let slice_bytes = word_count * 2;
        let constants = gf::input_slice_constants(total);
        let originals: Vec<Vec<u8>> = (0..total).map(|i| gen_slice(7, i, word_count)).collect();
        let missing = [1usize, 4, 6];
        let exponents: Vec<u32> = (0..missing.len() as u32).collect();
        let avail: Vec<usize> = (0..total).filter(|i| !missing.contains(i)).collect();

        let coeffs = matrix::build_repair_matrix(&avail, &missing, &exponents, &constants).unwrap();
        assert_eq!(coeffs.rows, missing.len());
        assert_eq!(coeffs.cols, avail.len() + missing.len());

        // sources = [available data..., recovery data...]
        let mut sources: Vec<Vec<u8>> = avail.iter().map(|&i| originals[i].clone()).collect();
        for &exp in &exponents {
            sources.push(encode_recovery(&originals, &constants, exp, slice_bytes));
        }

        for (row, &m) in missing.iter().enumerate() {
            let mut out = vec![0u8; slice_bytes];
            for (s, src) in sources.iter().enumerate() {
                gf_simd::mul_acc_region(coeffs.get(row, s), src, &mut out);
            }
            assert_eq!(
                out, originals[m],
                "row {row} (slice {m}) mismatch vs weaver ground truth"
            );
        }
    }

    #[test]
    fn rejects_singular_recovery_set() {
        // Overwrite the exponent array so two rows share exponent 0; weaver's
        // build_repair_matrix reports a singular submatrix, which the host
        // surfaces end-to-end as -6.
        let mut problem = build_problem(12, 64, &[2, 5, 7, 10], false);
        let header = abi::parse_header(&problem.image, problem.desc_ptr).unwrap();
        abi::write_u32_array_entry(&mut problem.image, header.exponent_ptr, 1, 0).unwrap();
        let (config, _flag) = config_with_deadline(far_future());
        let (mut store, instance, memory) = instantiate(config);
        let code = run(
            &mut store,
            &instance,
            &memory,
            &problem.image,
            problem.desc_ptr,
        );
        assert_eq!(code, E_SINGULAR);
    }
}
