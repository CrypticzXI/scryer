//! Shared descriptor ABI for the host PAR2 damaged-member reconstruct function
//! (RFC 123 WP2.5).
//!
//! A plain `wasm32-wasip1` archive/repair guest cannot run the parallel
//! GF(2^16) Reed-Solomon *reconstruct* (no rayon worker pool; the decode-matrix
//! construction overflows the ~1 MiB guest stack). Instead the guest lays a
//! small problem spec plus the bulk slice regions into its own linear memory and
//! calls the host import `scryer_par2_reconstruct`, which builds the repair
//! matrix on a native stack and runs the GF matmul across host threads
//! zero-copy over the guest's memory (mirroring the frozen crypto host ABI,
//! RFC §5).
//!
//! This module is the ONE shared definition of that byte layout (RFC §2.6 — the
//! SDK owns the wire types). Both the native host and the external repair plugin
//! consume it. It is dependency-free and `#![no_std]`-friendly in spirit (only
//! `alloc`/`std` `Vec` for the array readers), so it compiles unchanged for the
//! wasm guest and for the native host.
//!
//! # Wire format (all integers little-endian)
//!
//! Every `*_ptr` is a byte offset into the guest's exported `"memory"`. The
//! host validates every `[ptr, ptr+len)` against the real memory size with
//! overflow-checked arithmetic before touching it.
//!
//! ```text
//! header (80 bytes) @ desc_ptr:
//!    0  u32 magic          = 0x50415232 ("PAR2")
//!    4  u32 version        = 2
//!    8  u32 total_inputs   # input slices in the recovery set
//!   12  u32 n_out          # missing slices == # recovery exponents == matrix rows
//!   16  u32 n_avail        # available input slices
//!   20  u32 word_count     # u16 words per slice
//!   24  u32 slice_bytes    = word_count * 2
//!   28  u32 flags          reserved (host currently ignores; write 0)
//!   32  u64 missing_idx_ptr -> n_out   u32 global indices of the missing slices
//!   40  u64 avail_idx_ptr   -> n_avail u32 global indices of the available slices
//!   48  u64 exponent_ptr    -> n_out   u32 recovery exponents
//!   56  u64 src_table_ptr   -> n_src = (n_avail + n_out) entries of (u64 ptr, u64 len):
//!                              first n_avail = available slice data (avail_idx order),
//!                              then  n_out   = recovery block data (exponent order)
//!   64  u64 out_table_ptr   -> n_out entries of (u64 ptr, u64 len): the missing outputs
//!   72  u64 reserved
//! ```
//!
//! Host op: `input_factors = build_repair_matrix(avail_idx, missing_idx,
//! exponents, constants)`; then for each `j in 0..n_out`,
//! `out[j][*] = XOR_s gfmul(input_factors[j][s], src[s][*])`.

/// Import module string the host function lives under. Cosmetic legacy shared
/// with the crypto ABI — the guest declares
/// `#[link(wasm_import_module = "extism:host/user")]` with no extism dependency;
/// both sides must simply agree (RFC §5).
pub const PAR2_RECONSTRUCT_NAMESPACE: &str = "extism:host/user";

/// Import name of the host reconstruct function:
/// `scryer_par2_reconstruct(desc_ptr: i64, desc_len: i64) -> i64`.
pub const PAR2_RECONSTRUCT_IMPORT: &str = "scryer_par2_reconstruct";

/// Descriptor magic: ASCII "PAR2" as a little-endian u32.
pub const DESC_MAGIC: u32 = 0x5041_5232;

/// Descriptor wire version. Any change to the byte layout bumps this.
pub const DESC_VERSION: u32 = 2;

/// Fixed size of the descriptor header in bytes.
pub const DESC_HEADER_LEN: usize = 80;

/// Size of one `(u64 ptr, u64 len)` region-table entry in bytes.
pub const TABLE_ENTRY_LEN: usize = 16;

// Byte offsets of every header field (keeps the reader/writer in lockstep and
// self-documenting; changing one here changes both sides).
const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_TOTAL_INPUTS: usize = 8;
const OFF_N_OUT: usize = 12;
const OFF_N_AVAIL: usize = 16;
const OFF_WORD_COUNT: usize = 20;
const OFF_SLICE_BYTES: usize = 24;
const OFF_FLAGS: usize = 28;
const OFF_MISSING_IDX_PTR: usize = 32;
const OFF_AVAIL_IDX_PTR: usize = 40;
const OFF_EXPONENT_PTR: usize = 48;
const OFF_SRC_TABLE_PTR: usize = 56;
const OFF_OUT_TABLE_PTR: usize = 64;
const OFF_RESERVED: usize = 72;

/// Return codes of `scryer_par2_reconstruct`.
///
/// `0` is success; every negative code is a fatal contract violation. The guest
/// maps any negative return to an in-band [`crate::ArchivePluginStatus::RepairFailed`]
/// (the operation failed but the process is intact); the host separately
/// attributes [`Self::DeadlineExceeded`] to a wall-clock timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum Par2ReconstructStatus {
    /// The reconstruct completed; every missing output region is written.
    Ok = 0,
    /// The guest exports no `"memory"`, so no region can be addressed.
    NoMemory = -1,
    /// The header is malformed: bad magic, bad version, or out of bounds.
    BadDescriptor = -2,
    /// The declared dimensions are inconsistent or impossible
    /// (`n_out == 0`, `word_count == 0`, `slice_bytes != word_count*2`, or an
    /// index outside `0..total_inputs`).
    BadDimensions = -3,
    /// A source or output region lies outside the guest's linear memory, or a
    /// region length does not equal `slice_bytes`.
    BadRegion = -4,
    /// Output regions overlap each other or a source region — the host refuses
    /// to run parallel writes that would be a data race (validated before any
    /// thread is spawned).
    Alias = -5,
    /// The selected recovery exponents produced a singular repair matrix.
    Singular = -6,
    /// The host-side wall-clock deadline was exceeded mid-solve.
    DeadlineExceeded = -7,
    /// A declared dimension exceeded the host's configured ceiling.
    DimensionCap = -8,
}

impl Par2ReconstructStatus {
    /// The raw `i64` return code for this status.
    #[inline]
    pub const fn code(self) -> i64 {
        self as i64
    }

    /// Recover the status from a raw return code, if it is a defined one.
    #[inline]
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Ok,
            -1 => Self::NoMemory,
            -2 => Self::BadDescriptor,
            -3 => Self::BadDimensions,
            -4 => Self::BadRegion,
            -5 => Self::Alias,
            -6 => Self::Singular,
            -7 => Self::DeadlineExceeded,
            -8 => Self::DimensionCap,
            _ => return None,
        })
    }

    /// A short, stable operator-facing description of a raw return code.
    pub fn describe(code: i64) -> &'static str {
        match Self::from_code(code) {
            Some(Self::Ok) => "ok",
            Some(Self::NoMemory) => "no guest memory export",
            Some(Self::BadDescriptor) => "malformed descriptor header",
            Some(Self::BadDimensions) => "inconsistent reconstruct dimensions",
            Some(Self::BadRegion) => "region outside guest memory",
            Some(Self::Alias) => "overlapping output/source regions",
            Some(Self::Singular) => "singular repair matrix",
            Some(Self::DeadlineExceeded) => "reconstruct deadline exceeded",
            Some(Self::DimensionCap) => "reconstruct dimension cap exceeded",
            None => "unknown reconstruct error",
        }
    }

    /// True for any negative (fatal) return code.
    #[inline]
    pub const fn is_error(code: i64) -> bool {
        code < 0
    }
}

/// The parsed descriptor header. All pointers are byte offsets into the guest's
/// exported `"memory"`; all counts are element counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Par2ReconstructHeader {
    pub total_inputs: usize,
    pub n_out: usize,
    pub n_avail: usize,
    pub word_count: usize,
    pub slice_bytes: usize,
    pub flags: u32,
    pub missing_idx_ptr: usize,
    pub avail_idx_ptr: usize,
    pub exponent_ptr: usize,
    pub src_table_ptr: usize,
    pub out_table_ptr: usize,
}

impl Par2ReconstructHeader {
    /// Number of source regions: available slices followed by recovery blocks.
    #[inline]
    pub const fn n_src(&self) -> usize {
        self.n_avail + self.n_out
    }
}

#[inline]
fn rd_u32(mem: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([mem[off], mem[off + 1], mem[off + 2], mem[off + 3]])
}

#[inline]
fn rd_u64(mem: &[u8], off: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&mem[off..off + 8]);
    u64::from_le_bytes(bytes)
}

/// Parse and structurally validate the header at `mem[desc_ptr..]`.
///
/// Checks the 80-byte window fits in `mem`, and that the magic and version
/// match. Returns `None` (host: [`Par2ReconstructStatus::BadDescriptor`]) on any
/// failure. Dimension policy (index bounds, `slice_bytes == word_count*2`,
/// caps) is the host's responsibility, not the parser's.
pub fn parse_header(mem: &[u8], desc_ptr: usize) -> Option<Par2ReconstructHeader> {
    let end = desc_ptr.checked_add(DESC_HEADER_LEN)?;
    if end > mem.len() {
        return None;
    }
    let base = &mem[desc_ptr..end];
    if rd_u32(base, OFF_MAGIC) != DESC_MAGIC || rd_u32(base, OFF_VERSION) != DESC_VERSION {
        return None;
    }
    debug_assert_eq!(rd_u64(base, OFF_RESERVED), rd_u64(base, OFF_RESERVED));
    Some(Par2ReconstructHeader {
        total_inputs: rd_u32(base, OFF_TOTAL_INPUTS) as usize,
        n_out: rd_u32(base, OFF_N_OUT) as usize,
        n_avail: rd_u32(base, OFF_N_AVAIL) as usize,
        word_count: rd_u32(base, OFF_WORD_COUNT) as usize,
        slice_bytes: rd_u32(base, OFF_SLICE_BYTES) as usize,
        flags: rd_u32(base, OFF_FLAGS),
        missing_idx_ptr: rd_u64(base, OFF_MISSING_IDX_PTR) as usize,
        avail_idx_ptr: rd_u64(base, OFF_AVAIL_IDX_PTR) as usize,
        exponent_ptr: rd_u64(base, OFF_EXPONENT_PTR) as usize,
        src_table_ptr: rd_u64(base, OFF_SRC_TABLE_PTR) as usize,
        out_table_ptr: rd_u64(base, OFF_OUT_TABLE_PTR) as usize,
    })
}

/// Read `count` little-endian `u32` values from a guest region at `ptr`,
/// bounds-checked against `mem`. Returns `None` if the region does not fit.
pub fn read_u32_array(mem: &[u8], ptr: usize, count: usize) -> Option<Vec<u32>> {
    let span = count.checked_mul(4)?;
    let end = ptr.checked_add(span)?;
    if end > mem.len() {
        return None;
    }
    Some((0..count).map(|i| rd_u32(mem, ptr + i * 4)).collect())
}

/// Read the `(ptr, len)` entry at index `index` from a region table whose base
/// offset is `table_ptr`, bounds-checked against `mem`. Returns the entry's
/// `(ptr, len)` reinterpreted as `usize`s, or `None` if the entry itself is out
/// of bounds. (The referenced region is validated separately by the caller.)
pub fn read_table_entry(mem: &[u8], table_ptr: usize, index: usize) -> Option<(usize, usize)> {
    let entry_off = index.checked_mul(TABLE_ENTRY_LEN)?;
    let base = table_ptr.checked_add(entry_off)?;
    let end = base.checked_add(TABLE_ENTRY_LEN)?;
    if end > mem.len() {
        return None;
    }
    Some((rd_u64(mem, base) as usize, rd_u64(mem, base + 8) as usize))
}

/// The scalar header fields a guest supplies when serializing a descriptor.
/// `slice_bytes` is derived (`word_count * 2`); `flags` is reserved (write 0).
#[derive(Debug, Clone, Copy, Default)]
pub struct Par2ReconstructHeaderFields {
    pub total_inputs: u32,
    pub n_out: u32,
    pub n_avail: u32,
    pub word_count: u32,
    pub flags: u32,
    pub missing_idx_ptr: u64,
    pub avail_idx_ptr: u64,
    pub exponent_ptr: u64,
    pub src_table_ptr: u64,
    pub out_table_ptr: u64,
}

#[inline]
fn wr_u32(out: &mut [u8], off: usize, value: u32) {
    out[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

#[inline]
fn wr_u64(out: &mut [u8], off: usize, value: u64) {
    out[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

/// Guest-side: serialize the 80-byte header into `out[0..DESC_HEADER_LEN]`.
/// Returns `None` if `out` is shorter than the header. `slice_bytes` is set to
/// `word_count * 2` and `reserved` to 0.
pub fn write_header(out: &mut [u8], fields: &Par2ReconstructHeaderFields) -> Option<()> {
    if out.len() < DESC_HEADER_LEN {
        return None;
    }
    wr_u32(out, OFF_MAGIC, DESC_MAGIC);
    wr_u32(out, OFF_VERSION, DESC_VERSION);
    wr_u32(out, OFF_TOTAL_INPUTS, fields.total_inputs);
    wr_u32(out, OFF_N_OUT, fields.n_out);
    wr_u32(out, OFF_N_AVAIL, fields.n_avail);
    wr_u32(out, OFF_WORD_COUNT, fields.word_count);
    wr_u32(out, OFF_SLICE_BYTES, fields.word_count.wrapping_mul(2));
    wr_u32(out, OFF_FLAGS, fields.flags);
    wr_u64(out, OFF_MISSING_IDX_PTR, fields.missing_idx_ptr);
    wr_u64(out, OFF_AVAIL_IDX_PTR, fields.avail_idx_ptr);
    wr_u64(out, OFF_EXPONENT_PTR, fields.exponent_ptr);
    wr_u64(out, OFF_SRC_TABLE_PTR, fields.src_table_ptr);
    wr_u64(out, OFF_OUT_TABLE_PTR, fields.out_table_ptr);
    wr_u64(out, OFF_RESERVED, 0);
    Some(())
}

/// Guest-side: write the `index`-th `u32` of an array whose base offset in
/// `out` is `array_ptr`. Returns `None` if the slot is out of bounds.
pub fn write_u32_array_entry(
    out: &mut [u8],
    array_ptr: usize,
    index: usize,
    value: u32,
) -> Option<()> {
    let off = array_ptr.checked_add(index.checked_mul(4)?)?;
    if off.checked_add(4)? > out.len() {
        return None;
    }
    wr_u32(out, off, value);
    Some(())
}

/// Guest-side: write the `(ptr, len)` entry at index `index` into a region
/// table whose base offset in `out` is `table_ptr`. Returns `None` if the entry
/// is out of bounds.
pub fn write_table_entry(
    out: &mut [u8],
    table_ptr: usize,
    index: usize,
    ptr: u64,
    len: u64,
) -> Option<()> {
    let base = table_ptr.checked_add(index.checked_mul(TABLE_ENTRY_LEN)?)?;
    if base.checked_add(TABLE_ENTRY_LEN)? > out.len() {
        return None;
    }
    wr_u64(out, base, ptr);
    wr_u64(out, base + 8, len);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_round_trips() {
        for status in [
            Par2ReconstructStatus::Ok,
            Par2ReconstructStatus::NoMemory,
            Par2ReconstructStatus::BadDescriptor,
            Par2ReconstructStatus::BadDimensions,
            Par2ReconstructStatus::BadRegion,
            Par2ReconstructStatus::Alias,
            Par2ReconstructStatus::Singular,
            Par2ReconstructStatus::DeadlineExceeded,
            Par2ReconstructStatus::DimensionCap,
        ] {
            assert_eq!(
                Par2ReconstructStatus::from_code(status.code()),
                Some(status)
            );
            assert_eq!(
                Par2ReconstructStatus::is_error(status.code()),
                status != Par2ReconstructStatus::Ok
            );
        }
        assert_eq!(Par2ReconstructStatus::from_code(-9), None);
        assert_eq!(Par2ReconstructStatus::from_code(1), None);
        assert_eq!(Par2ReconstructStatus::DeadlineExceeded.code(), -7);
        assert_eq!(Par2ReconstructStatus::DimensionCap.code(), -8);
    }

    #[test]
    fn header_round_trips_through_the_wire() {
        let fields = Par2ReconstructHeaderFields {
            total_inputs: 256,
            n_out: 64,
            n_avail: 192,
            word_count: 4096,
            flags: 0,
            missing_idx_ptr: 0x100,
            avail_idx_ptr: 0x200,
            exponent_ptr: 0x300,
            src_table_ptr: 0x400,
            out_table_ptr: 0x800,
        };
        let mut buf = vec![0u8; DESC_HEADER_LEN];
        write_header(&mut buf, &fields).unwrap();

        let header = parse_header(&buf, 0).expect("valid header parses");
        assert_eq!(header.total_inputs, 256);
        assert_eq!(header.n_out, 64);
        assert_eq!(header.n_avail, 192);
        assert_eq!(header.word_count, 4096);
        assert_eq!(header.slice_bytes, 4096 * 2);
        assert_eq!(header.flags, 0);
        assert_eq!(header.n_src(), 192 + 64);
        assert_eq!(header.missing_idx_ptr, 0x100);
        assert_eq!(header.out_table_ptr, 0x800);
    }

    #[test]
    fn parse_header_rejects_bad_magic_version_and_bounds() {
        let good = Par2ReconstructHeaderFields {
            total_inputs: 1,
            n_out: 1,
            n_avail: 1,
            word_count: 1,
            ..Par2ReconstructHeaderFields::default()
        };
        let mut buf = vec![0u8; DESC_HEADER_LEN];
        write_header(&mut buf, &good).unwrap();
        assert!(parse_header(&buf, 0).is_some());

        // Truncated window.
        assert!(parse_header(&buf[..DESC_HEADER_LEN - 1], 0).is_none());
        // Offset that runs off the end.
        assert!(parse_header(&buf, 1).is_none());

        // Bad magic.
        let mut bad_magic = buf.clone();
        wr_u32(&mut bad_magic, OFF_MAGIC, 0xDEAD_BEEF);
        assert!(parse_header(&bad_magic, 0).is_none());

        // Bad version.
        let mut bad_version = buf.clone();
        wr_u32(&mut bad_version, OFF_VERSION, DESC_VERSION + 1);
        assert!(parse_header(&bad_version, 0).is_none());
    }

    #[test]
    fn array_and_table_readers_are_bounds_checked() {
        let mut buf = vec![0u8; 64];
        write_u32_array_entry(&mut buf, 8, 0, 0xAABB_CCDD).unwrap();
        write_u32_array_entry(&mut buf, 8, 1, 0x1122_3344).unwrap();
        let values = read_u32_array(&buf, 8, 2).unwrap();
        assert_eq!(values, vec![0xAABB_CCDD, 0x1122_3344]);
        // One element past the end is rejected.
        assert!(read_u32_array(&buf, 8, 100).is_none());

        write_table_entry(&mut buf, 24, 0, 0x10, 0x20).unwrap();
        assert_eq!(read_table_entry(&buf, 24, 0), Some((0x10, 0x20)));
        assert!(read_table_entry(&buf, 24, 100).is_none());
        // Overflow-prone offsets are rejected, not wrapped.
        assert!(read_table_entry(&buf, usize::MAX, 1).is_none());
        assert!(read_u32_array(&buf, usize::MAX, 1).is_none());
    }
}
