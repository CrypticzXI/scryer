#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_UNSET: u8 = 0;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_AUTO: u8 = 1;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const ACCEL_OFF: u8 = 2;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
static ACCEL_MODE: AtomicU8 = AtomicU8::new(ACCEL_UNSET);

#[allow(dead_code)]
pub(crate) fn find_byte_from(data: &[u8], needle: u8, start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            if let Some(offset) = unsafe { x86_64::find_byte_avx2(data, needle) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            if let Some(offset) = unsafe { aarch64::find_byte_neon(data, needle) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    scalar::find_byte_from(data, needle, 0).map(|offset| start + offset)
}

#[allow(dead_code)]
pub(crate) fn find_any_byte_from(data: &[u8], needles: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;
    if needles.is_empty() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            if let Some(offset) = unsafe { x86_64::find_any_byte_avx2(data, needles) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            if let Some(offset) = unsafe { aarch64::find_any_byte_neon(data, needles) } {
                return Some(start + offset);
            }
            return None;
        }
    }

    scalar::find_any_byte_from(data, needles, 0).map(|offset| start + offset)
}

pub(crate) fn find_mpeg_start_code(data: &[u8], code: u8) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_mpeg_start_code_avx2(data, code) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_mpeg_start_code_neon(data, code) };
        }
    }

    scalar::find_mpeg_start_code_from(data, code, 0)
}

pub(crate) fn find_annexb_start_code(data: &[u8], start: usize) -> Option<(usize, usize)> {
    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_annexb_start_code_avx2(data, start) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_annexb_start_code_neon(data, start) };
        }
    }

    scalar::find_annexb_start_code_from(data, start)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AudioSyncKind {
    Adts,
    Latm,
    MpegAudio,
    Ac3,
    Dts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AudioSyncCandidate {
    pub(crate) offset: usize,
    pub(crate) kind: AudioSyncKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EbmlCandidate {
    pub(crate) offset: usize,
    pub(crate) id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LengthPrefixedNalCandidate {
    pub(crate) offset: usize,
    pub(crate) nal_type: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mp4BoxNameCandidate {
    pub(crate) offset: usize,
    pub(crate) name: [u8; 4],
}

#[allow(dead_code)]
pub(crate) fn find_audio_sync_candidate(data: &[u8], start: usize) -> Option<AudioSyncCandidate> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_audio_sync_candidate_avx2(data) }.map(|candidate| {
                AudioSyncCandidate {
                    offset: start + candidate.offset,
                    kind: candidate.kind,
                }
            });
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_audio_sync_candidate_neon(data) }.map(|candidate| {
                AudioSyncCandidate {
                    offset: start + candidate.offset,
                    kind: candidate.kind,
                }
            });
        }
    }

    scalar::find_audio_sync_candidate_from(data, 0).map(|candidate| AudioSyncCandidate {
        offset: start + candidate.offset,
        kind: candidate.kind,
    })
}

pub(crate) fn find_ebml_candidate(data: &[u8], start: usize, ids: &[u32]) -> Option<EbmlCandidate> {
    let data = data.get(start..)?;
    if ids.is_empty() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_ebml_candidate_avx2(data, ids) }.map(|candidate| {
                EbmlCandidate {
                    offset: start + candidate.offset,
                    id: candidate.id,
                }
            });
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_ebml_candidate_neon(data, ids) }.map(|candidate| {
                EbmlCandidate {
                    offset: start + candidate.offset,
                    id: candidate.id,
                }
            });
        }
    }

    scalar::find_ebml_candidate_from(data, 0, ids).map(|candidate| EbmlCandidate {
        offset: start + candidate.offset,
        id: candidate.id,
    })
}

pub(crate) fn find_hevc_sei_nal_header_candidate(
    data: &[u8],
    start: usize,
) -> Option<LengthPrefixedNalCandidate> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_hevc_sei_nal_header_candidate_avx2(data) }.map(
                |candidate| LengthPrefixedNalCandidate {
                    offset: start + candidate.offset,
                    nal_type: candidate.nal_type,
                },
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_hevc_sei_nal_header_candidate_neon(data) }.map(
                |candidate| LengthPrefixedNalCandidate {
                    offset: start + candidate.offset,
                    nal_type: candidate.nal_type,
                },
            );
        }
    }

    scalar::find_hevc_sei_nal_header_candidate_from(data, 0).map(|candidate| {
        LengthPrefixedNalCandidate {
            offset: start + candidate.offset,
            nal_type: candidate.nal_type,
        }
    })
}

pub(crate) fn find_hdr10plus_itu_t35_candidate(data: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_hdr10plus_itu_t35_candidate_avx2(data) }
                .map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_hdr10plus_itu_t35_candidate_neon(data) }
                .map(|offset| start + offset);
        }
    }

    scalar::find_hdr10plus_itu_t35_candidate_from(data, 0).map(|offset| start + offset)
}

pub(crate) fn find_mp4_box_name_candidate(
    data: &[u8],
    start: usize,
    names: &[[u8; 4]],
) -> Option<Mp4BoxNameCandidate> {
    if names.is_empty() || data.len() < 8 {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_mp4_box_name_candidate_avx2(data, start, names) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_mp4_box_name_candidate_neon(data, start, names) };
        }
    }

    scalar::find_mp4_box_name_candidate_from(data, start, names)
}

pub(crate) fn find_avi_idx1_stream_prefix(
    data: &[u8],
    start: usize,
    stream_number: usize,
) -> Option<usize> {
    let prefix = avi_idx1_stream_prefix(stream_number)?;
    let mut cursor = start;
    while let Some(offset) = find_pair_from(data, cursor, prefix[0], 0xFF, prefix[1]) {
        if offset % 16 == 0 {
            return Some(offset);
        }
        cursor = offset + 1;
    }
    None
}

fn audio_sync_kind_at(data: &[u8], offset: usize) -> Option<AudioSyncKind> {
    if offset + 2 <= data.len() {
        let first = data[offset];
        let second = data[offset + 1];
        if first == 0xFF && second & 0xF0 == 0xF0 {
            return Some(AudioSyncKind::Adts);
        }
        if first == 0x56 && second & 0xE0 == 0xE0 {
            return Some(AudioSyncKind::Latm);
        }
        if first == 0xFF && second & 0xE0 == 0xE0 {
            return Some(AudioSyncKind::MpegAudio);
        }
        if first == 0x0B && second == 0x77 {
            return Some(AudioSyncKind::Ac3);
        }
    }

    if offset + 4 <= data.len() {
        let marker = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?);
        if matches!(
            marker,
            0x7FFE_8001 | 0xFE7F_0180 | 0x1FFF_E800 | 0xFF1F_00E8
        ) {
            return Some(AudioSyncKind::Dts);
        }
    }

    None
}

fn ebml_id_bytes(id: u32) -> ([u8; 4], usize) {
    let bytes = id.to_be_bytes();
    let first = bytes.iter().position(|&byte| byte != 0).unwrap_or(3);
    let mut out = [0_u8; 4];
    let len = 4 - first;
    out[..len].copy_from_slice(&bytes[first..]);
    (out, len)
}

fn ebml_candidate_at(data: &[u8], offset: usize, ids: &[u32]) -> Option<EbmlCandidate> {
    for &id in ids {
        let (bytes, len) = ebml_id_bytes(id);
        if len > 0 && offset + len <= data.len() && data[offset..offset + len] == bytes[..len] {
            return Some(EbmlCandidate { offset, id });
        }
    }
    None
}

fn hevc_sei_nal_candidate_at(data: &[u8], offset: usize) -> Option<LengthPrefixedNalCandidate> {
    let byte = *data.get(offset)?;
    let nal_type = (byte >> 1) & 0x3F;
    matches!(nal_type, 39 | 40).then_some(LengthPrefixedNalCandidate { offset, nal_type })
}

fn audio_sync_candidate_at(data: &[u8], offset: usize) -> Option<AudioSyncCandidate> {
    Some(AudioSyncCandidate {
        offset,
        kind: audio_sync_kind_at(data, offset)?,
    })
}

fn mp4_box_name_candidate_at(
    data: &[u8],
    name_offset: usize,
    names: &[[u8; 4]],
) -> Option<Mp4BoxNameCandidate> {
    if name_offset < 4 || name_offset + 4 > data.len() {
        return None;
    }
    let name: [u8; 4] = data[name_offset..name_offset + 4].try_into().ok()?;
    names.contains(&name).then_some(Mp4BoxNameCandidate {
        offset: name_offset - 4,
        name,
    })
}

fn avi_idx1_stream_prefix(stream_number: usize) -> Option<[u8; 2]> {
    (stream_number < 100).then_some([
        b'0' + (stream_number / 10) as u8,
        b'0' + (stream_number % 10) as u8,
    ])
}

#[allow(dead_code)]
pub(crate) fn find_adts_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0xFF, 0xF0, 0xF0)
}

#[allow(dead_code)]
pub(crate) fn find_latm_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0x56, 0xE0, 0xE0)
}

#[allow(dead_code)]
pub(crate) fn find_mpeg_audio_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0xFF, 0xE0, 0xE0)
}

#[allow(dead_code)]
pub(crate) fn find_ac3_sync(data: &[u8], start: usize) -> Option<usize> {
    find_pair_from(data, start, 0x0B, 0xFF, 0x77)
}

#[allow(dead_code)]
pub(crate) fn find_dts_sync(data: &[u8], start: usize) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_dts_sync_avx2(data) }.map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_dts_sync_neon(data) }.map(|offset| start + offset);
        }
    }

    scalar::find_dts_sync_from(data, 0).map(|offset| start + offset)
}

#[allow(dead_code)]
fn find_pair_from(
    data: &[u8],
    start: usize,
    first: u8,
    second_mask: u8,
    second_value: u8,
) -> Option<usize> {
    let data = data.get(start..)?;

    #[cfg(target_arch = "x86_64")]
    {
        if accel_runtime_enabled() && std::arch::is_x86_feature_detected!("avx2") {
            return unsafe { x86_64::find_pair_avx2(data, first, second_mask, second_value) }
                .map(|offset| start + offset);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        if accel_runtime_enabled() && aarch64_neon_available() {
            return unsafe { aarch64::find_pair_neon(data, first, second_mask, second_value) }
                .map(|offset| start + offset);
        }
    }

    scalar::find_pair_from(data, 0, first, second_mask, second_value).map(|offset| start + offset)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn accel_runtime_enabled() -> bool {
    match ACCEL_MODE.load(Ordering::Relaxed) {
        ACCEL_AUTO => true,
        ACCEL_OFF => false,
        _ => {
            let disabled = std::env::var_os("SCRYER_MEDIAINFO_ACCEL")
                .map(|value| {
                    let value = value.to_string_lossy();
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "off" | "0" | "false" | "no"
                    )
                })
                .unwrap_or(false);
            let mode = if disabled { ACCEL_OFF } else { ACCEL_AUTO };
            ACCEL_MODE.store(mode, Ordering::Relaxed);
            !disabled
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn aarch64_neon_available() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

pub(crate) mod scalar {
    use super::{
        AudioSyncCandidate, EbmlCandidate, LengthPrefixedNalCandidate, Mp4BoxNameCandidate,
    };

    pub(crate) fn find_byte_from(data: &[u8], needle: u8, start: usize) -> Option<usize> {
        data.get(start..)?
            .iter()
            .position(|&byte| byte == needle)
            .map(|offset| start + offset)
    }

    pub(crate) fn find_any_byte_from(data: &[u8], needles: &[u8], start: usize) -> Option<usize> {
        if needles.is_empty() {
            return None;
        }
        data.get(start..)?
            .iter()
            .position(|byte| needles.contains(byte))
            .map(|offset| start + offset)
    }

    #[allow(dead_code)]
    pub(crate) fn find_pair_from(
        data: &[u8],
        start: usize,
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let mut pos = start;
        while pos + 2 <= data.len() {
            let i = find_byte_from(data, first, pos)?;
            if i + 2 > data.len() {
                return None;
            }
            if data[i + 1] & second_mask == second_value {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_mpeg_start_code_from(data: &[u8], code: u8, start: usize) -> Option<usize> {
        find_mpeg_start_code_with(data, code, start, find_byte_from)
    }

    pub(crate) fn find_annexb_start_code_from(data: &[u8], start: usize) -> Option<(usize, usize)> {
        find_annexb_start_code_with(data, start, find_byte_from)
    }

    #[allow(dead_code)]
    pub(crate) fn find_dts_sync_from(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;
        while pos + 4 <= data.len() {
            let i = find_any_byte_from(data, &[0x7F, 0xFE, 0x1F, 0xFF], pos)?;
            if i + 4 > data.len() {
                return None;
            }
            let marker = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
            if matches!(
                marker,
                0x7FFE_8001 | 0xFE7F_0180 | 0x1FFF_E800 | 0xFF1F_00E8
            ) {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_audio_sync_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<AudioSyncCandidate> {
        let mut pos = start;
        while pos + 2 <= data.len() {
            let i = find_any_byte_from(data, &[0xFF, 0x56, 0x0B, 0x7F, 0xFE, 0x1F], pos)?;
            if let Some(candidate) = super::audio_sync_candidate_at(data, i) {
                return Some(candidate);
            }

            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_ebml_candidate_from(
        data: &[u8],
        start: usize,
        ids: &[u32],
    ) -> Option<EbmlCandidate> {
        if ids.is_empty() {
            return None;
        }
        let mut first_bytes = [0_u8; 8];
        let mut first_count = 0usize;
        for &id in ids {
            let (bytes, len) = super::ebml_id_bytes(id);
            if len == 0 || first_bytes[..first_count].contains(&bytes[0]) {
                continue;
            }
            if first_count < first_bytes.len() {
                first_bytes[first_count] = bytes[0];
                first_count += 1;
            }
        }

        let mut pos = start;
        while pos < data.len() {
            let i = find_any_byte_from(data, &first_bytes[..first_count], pos)?;
            if let Some(candidate) = super::ebml_candidate_at(data, i, ids) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_hevc_sei_nal_header_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<LengthPrefixedNalCandidate> {
        let mut pos = start;
        while pos < data.len() {
            let i = find_any_byte_from(data, &[0x4E, 0x4F, 0x50, 0x51], pos)?;
            if let Some(candidate) = super::hevc_sei_nal_candidate_at(data, i) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_hdr10plus_itu_t35_candidate_from(
        data: &[u8],
        start: usize,
    ) -> Option<usize> {
        let pattern = [0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04];
        let mut pos = start;
        while pos + pattern.len() <= data.len() {
            let i = find_byte_from(data, pattern[0], pos)?;
            if i + pattern.len() > data.len() {
                return None;
            }
            if data[i..i + pattern.len()] == pattern {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    pub(crate) fn find_mp4_box_name_candidate_from(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<Mp4BoxNameCandidate> {
        if names.is_empty() || data.len() < 8 {
            return None;
        }
        let mut first_bytes = [0_u8; 16];
        let mut first_count = 0usize;
        for name in names {
            if !first_bytes[..first_count].contains(&name[0]) {
                if first_count == first_bytes.len() {
                    return find_mp4_box_name_candidate_by_name(data, start, names);
                }
                first_bytes[first_count] = name[0];
                first_count += 1;
            }
        }

        let mut pos = start.saturating_add(4);
        while pos + 4 <= data.len() {
            let i = find_any_byte_from(data, &first_bytes[..first_count], pos)?;
            if i + 4 > data.len() {
                return None;
            }
            if let Some(candidate) = super::mp4_box_name_candidate_at(data, i, names) {
                return Some(candidate);
            }
            pos = i + 1;
        }
        None
    }

    fn find_mp4_box_name_candidate_by_name(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<Mp4BoxNameCandidate> {
        let mut best: Option<Mp4BoxNameCandidate> = None;
        for name in names {
            let mut pos = start.saturating_add(4);
            while pos + 4 <= data.len() {
                let i = find_byte_from(data, name[0], pos)?;
                if i + 4 > data.len() {
                    break;
                }
                if data[i..i + 4] == *name
                    && let Some(candidate) = super::mp4_box_name_candidate_at(data, i, names)
                    && best.is_none_or(|existing| candidate.offset < existing.offset)
                {
                    best = Some(candidate);
                    break;
                }
                pos = i + 1;
            }
        }
        best
    }

    fn find_mpeg_start_code_with(
        data: &[u8],
        code: u8,
        start: usize,
        mut find_zero: impl FnMut(&[u8], u8, usize) -> Option<usize>,
    ) -> Option<usize> {
        let mut pos = start;
        while pos + 4 <= data.len() {
            let i = find_zero(data, 0, pos)?;
            if i + 4 > data.len() {
                return None;
            }
            if data[i + 1] == 0 && data[i + 2] == 1 && data[i + 3] == code {
                return Some(i);
            }
            pos = i + 1;
        }
        None
    }

    fn find_annexb_start_code_with(
        data: &[u8],
        start: usize,
        mut find_zero: impl FnMut(&[u8], u8, usize) -> Option<usize>,
    ) -> Option<(usize, usize)> {
        let mut pos = start;
        while pos + 3 <= data.len() {
            let i = find_zero(data, 0, pos)?;
            if i + 3 <= data.len() && data[i + 1] == 0 && data[i + 2] == 1 {
                return Some((i, 3));
            }
            if i + 4 <= data.len() && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some((i, 4));
            }
            pos = i + 1;
        }
        None
    }
}

#[cfg(target_arch = "x86_64")]
mod x86_64 {
    use super::AudioSyncCandidate;
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_byte_avx2(data: &[u8], needle: u8) -> Option<usize> {
        let needle_vec = _mm256_set1_epi8(needle as i8);
        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, needle_vec)) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_byte_from(data, needle, offset)
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_any_byte_avx2(data: &[u8], needles: &[u8]) -> Option<usize> {
        if needles.len() == 1 {
            return unsafe { find_byte_avx2(data, needles[0]) };
        }

        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mut mask = 0_u32;
            for &needle in needles {
                let needle = _mm256_set1_epi8(needle as i8);
                mask |= _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, needle)) as u32;
            }
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_any_byte_from(data, needles, offset)
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_pair_avx2(
        data: &[u8],
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let first_vec = _mm256_set1_epi8(first as i8);
        let second_mask_vec = _mm256_set1_epi8(second_mask as i8);
        let second_value_vec = _mm256_set1_epi8(second_value as i8);
        let mut offset = 0;
        while offset + 33 <= data.len() {
            let first_chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let second_chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(first_chunk, first_vec),
                _mm256_cmpeq_epi8(
                    _mm256_and_si256(second_chunk, second_mask_vec),
                    second_value_vec,
                ),
            )) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_pair_from(data, offset, first, second_mask, second_value)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_mpeg_start_code_avx2(data: &[u8], code: u8) -> Option<usize> {
        let zero = _mm256_setzero_si256();
        let one = _mm256_set1_epi8(1);
        let code_vec = _mm256_set1_epi8(code as i8);
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mask = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_and_si256(_mm256_cmpeq_epi8(b0, zero), _mm256_cmpeq_epi8(b1, zero)),
                _mm256_and_si256(_mm256_cmpeq_epi8(b2, one), _mm256_cmpeq_epi8(b3, code_vec)),
            )) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_mpeg_start_code_from(data, code, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_annexb_start_code_avx2(
        data: &[u8],
        start: usize,
    ) -> Option<(usize, usize)> {
        let data = data.get(start..)?;
        let zero = _mm256_setzero_si256();
        let one = _mm256_set1_epi8(1);
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let first_two =
                _mm256_and_si256(_mm256_cmpeq_epi8(b0, zero), _mm256_cmpeq_epi8(b1, zero));
            let three = _mm256_and_si256(first_two, _mm256_cmpeq_epi8(b2, one));
            let four = _mm256_and_si256(
                first_two,
                _mm256_and_si256(_mm256_cmpeq_epi8(b2, zero), _mm256_cmpeq_epi8(b3, one)),
            );
            let mask = _mm256_movemask_epi8(_mm256_or_si256(three, four)) as u32;
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                let len = if data[found + 2] == 1 { 3 } else { 4 };
                return Some((start + found, len));
            }
            offset += 32;
        }
        super::scalar::find_annexb_start_code_from(data, offset)
            .map(|(found, len)| (start + found, len))
    }

    #[target_feature(enable = "avx2")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_dts_sync_avx2(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mask = unsafe { dts_word_mask(b0, b1, b2, b3) };
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }
        super::scalar::find_dts_sync_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_audio_sync_candidate_avx2(data: &[u8]) -> Option<AudioSyncCandidate> {
        let ff = _mm256_set1_epi8(0xFF_u8 as i8);
        let f0_mask = _mm256_set1_epi8(0xF0_u8 as i8);
        let f0 = _mm256_set1_epi8(0xF0_u8 as i8);
        let e0_mask = _mm256_set1_epi8(0xE0_u8 as i8);
        let e0 = _mm256_set1_epi8(0xE0_u8 as i8);
        let latm_first = _mm256_set1_epi8(0x56);
        let ac3_first = _mm256_set1_epi8(0x0B);
        let ac3_second = _mm256_set1_epi8(0x77);

        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };

            let first_ff = _mm256_cmpeq_epi8(b0, ff);
            let adts = _mm256_and_si256(
                first_ff,
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, f0_mask), f0),
            );
            let mpeg_audio = _mm256_and_si256(
                first_ff,
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, e0_mask), e0),
            );
            let latm = _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, latm_first),
                _mm256_cmpeq_epi8(_mm256_and_si256(b1, e0_mask), e0),
            );
            let ac3 = _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, ac3_first),
                _mm256_cmpeq_epi8(b1, ac3_second),
            );
            let dts = unsafe { dts_word_mask(b0, b1, b2, b3) };

            let pair_mask = _mm256_movemask_epi8(_mm256_or_si256(
                _mm256_or_si256(adts, mpeg_audio),
                _mm256_or_si256(latm, ac3),
            )) as u32;
            let mask = pair_mask | dts;
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::audio_sync_candidate_at(data, found);
            }
            offset += 32;
        }

        super::scalar::find_audio_sync_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_ebml_candidate_avx2(
        data: &[u8],
        ids: &[u32],
    ) -> Option<super::EbmlCandidate> {
        if ids.len() > 8 {
            return super::scalar::find_ebml_candidate_from(data, 0, ids);
        }
        let mut specs = [([0_u8; 4], 0usize, 0_u32); 8];
        let mut spec_count = 0usize;
        for &id in ids.iter().take(specs.len()) {
            let (bytes, len) = super::ebml_id_bytes(id);
            specs[spec_count] = (bytes, len, id);
            spec_count += 1;
        }

        let mut offset = 0;
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mut mask = 0_u32;
            for &(bytes, len, _) in &specs[..spec_count] {
                mask |= unsafe { byte_sequence_mask(b0, b1, b2, b3, bytes, len) };
            }
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::ebml_candidate_at(data, found, ids);
            }
            offset += 32;
        }

        super::scalar::find_ebml_candidate_from(data, offset, ids)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_hevc_sei_nal_header_candidate_avx2(
        data: &[u8],
    ) -> Option<super::LengthPrefixedNalCandidate> {
        let candidates = [0x4E_u8, 0x4F, 0x50, 0x51];
        let mut offset = 0;
        while offset + 32 <= data.len() {
            let chunk = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let mut mask = 0_u32;
            for &candidate in &candidates {
                mask |= _mm256_movemask_epi8(_mm256_cmpeq_epi8(
                    chunk,
                    _mm256_set1_epi8(candidate as i8),
                )) as u32;
            }
            if mask != 0 {
                let found = offset + mask.trailing_zeros() as usize;
                return super::hevc_sei_nal_candidate_at(data, found);
            }
            offset += 32;
        }

        super::scalar::find_hevc_sei_nal_header_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_hdr10plus_itu_t35_candidate_avx2(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 37 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let b4 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 4).cast()) };
            let b5 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 5).cast()) };
            let first_four = unsafe { word_mask(b0, b1, b2, b3, [0xB5, 0x00, 0x3C, 0x00]) };
            let last_two = _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(b4, _mm256_set1_epi8(0x01)),
                _mm256_cmpeq_epi8(b5, _mm256_set1_epi8(0x04)),
            )) as u32;
            let mask = first_four & last_two;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
            offset += 32;
        }

        super::scalar::find_hdr10plus_itu_t35_candidate_from(data, offset)
    }

    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn find_mp4_box_name_candidate_avx2(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<super::Mp4BoxNameCandidate> {
        let mut offset = start.saturating_add(4);
        while offset + 35 <= data.len() {
            let b0 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 1).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 2).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(data.as_ptr().add(offset + 3).cast()) };
            let mut mask = 0_u32;
            for &name in names {
                mask |= unsafe { word_mask(b0, b1, b2, b3, name) };
            }
            if mask != 0 {
                let name_offset = offset + mask.trailing_zeros() as usize;
                return super::mp4_box_name_candidate_at(data, name_offset, names);
            }
            offset += 32;
        }

        super::scalar::find_mp4_box_name_candidate_from(data, offset.saturating_sub(4), names)
    }

    #[target_feature(enable = "avx2")]
    unsafe fn dts_word_mask(b0: __m256i, b1: __m256i, b2: __m256i, b3: __m256i) -> u32 {
        let mut mask = unsafe { word_mask(b0, b1, b2, b3, [0x7F, 0xFE, 0x80, 0x01]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0xFE, 0x7F, 0x01, 0x80]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0x1F, 0xFF, 0xE8, 0x00]) };
        mask |= unsafe { word_mask(b0, b1, b2, b3, [0xFF, 0x1F, 0x00, 0xE8]) };
        mask
    }

    #[target_feature(enable = "avx2")]
    unsafe fn byte_sequence_mask(
        b0: __m256i,
        b1: __m256i,
        b2: __m256i,
        b3: __m256i,
        bytes: [u8; 4],
        len: usize,
    ) -> u32 {
        match len {
            1 => {
                _mm256_movemask_epi8(_mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8))) as u32
            }
            2 => _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8)),
                _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(bytes[1] as i8)),
            )) as u32,
            3 => _mm256_movemask_epi8(_mm256_and_si256(
                _mm256_and_si256(
                    _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(bytes[0] as i8)),
                    _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(bytes[1] as i8)),
                ),
                _mm256_cmpeq_epi8(b2, _mm256_set1_epi8(bytes[2] as i8)),
            )) as u32,
            4 => unsafe { word_mask(b0, b1, b2, b3, bytes) },
            _ => 0,
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn word_mask(b0: __m256i, b1: __m256i, b2: __m256i, b3: __m256i, word: [u8; 4]) -> u32 {
        let cmp = _mm256_and_si256(
            _mm256_and_si256(
                _mm256_cmpeq_epi8(b0, _mm256_set1_epi8(word[0] as i8)),
                _mm256_cmpeq_epi8(b1, _mm256_set1_epi8(word[1] as i8)),
            ),
            _mm256_and_si256(
                _mm256_cmpeq_epi8(b2, _mm256_set1_epi8(word[2] as i8)),
                _mm256_cmpeq_epi8(b3, _mm256_set1_epi8(word[3] as i8)),
            ),
        );
        _mm256_movemask_epi8(cmp) as u32
    }
}

#[cfg(target_arch = "aarch64")]
mod aarch64 {
    use super::AudioSyncCandidate;
    use std::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_byte_neon(data: &[u8], needle: u8) -> Option<usize> {
        let needle_vec = vdupq_n_u8(needle);
        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let cmp = vceqq_u8(chunk, needle_vec);
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_byte_from(data, needle, offset)
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_any_byte_neon(data: &[u8], needles: &[u8]) -> Option<usize> {
        if needles.len() == 1 {
            return unsafe { find_byte_neon(data, needles[0]) };
        }

        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let mut cmp = vdupq_n_u8(0);
            for &needle in needles {
                cmp = vorrq_u8(cmp, vceqq_u8(chunk, vdupq_n_u8(needle)));
            }
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_any_byte_from(data, needles, offset)
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_pair_neon(
        data: &[u8],
        first: u8,
        second_mask: u8,
        second_value: u8,
    ) -> Option<usize> {
        let first_vec = vdupq_n_u8(first);
        let second_mask_vec = vdupq_n_u8(second_mask);
        let second_value_vec = vdupq_n_u8(second_value);
        let mut offset = 0;
        while offset + 17 <= data.len() {
            let first_chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let second_chunk = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let cmp = vandq_u8(
                vceqq_u8(first_chunk, first_vec),
                vceqq_u8(vandq_u8(second_chunk, second_mask_vec), second_value_vec),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_pair_from(data, offset, first, second_mask, second_value)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_mpeg_start_code_neon(data: &[u8], code: u8) -> Option<usize> {
        let zero = vdupq_n_u8(0);
        let one = vdupq_n_u8(1);
        let code_vec = vdupq_n_u8(code);
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let cmp = vandq_u8(
                vandq_u8(vceqq_u8(b0, zero), vceqq_u8(b1, zero)),
                vandq_u8(vceqq_u8(b2, one), vceqq_u8(b3, code_vec)),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_mpeg_start_code_from(data, code, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_annexb_start_code_neon(
        data: &[u8],
        start: usize,
    ) -> Option<(usize, usize)> {
        let data = data.get(start..)?;
        let zero = vdupq_n_u8(0);
        let one = vdupq_n_u8(1);
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let first_two = vandq_u8(vceqq_u8(b0, zero), vceqq_u8(b1, zero));
            let three = vandq_u8(first_two, vceqq_u8(b2, one));
            let four = vandq_u8(first_two, vandq_u8(vceqq_u8(b2, zero), vceqq_u8(b3, one)));
            let cmp = vorrq_u8(three, four);
            if let Some(index) = first_neon_match(cmp) {
                let found = offset + index;
                let len = if data[found + 2] == 1 { 3 } else { 4 };
                return Some((start + found, len));
            }
            offset += 16;
        }
        super::scalar::find_annexb_start_code_from(data, offset)
            .map(|(found, len)| (start + found, len))
    }

    #[target_feature(enable = "neon")]
    #[allow(dead_code)]
    pub(crate) unsafe fn find_dts_sync_neon(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let cmp = unsafe { dts_word_match(b0, b1, b2, b3) };
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }
        super::scalar::find_dts_sync_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_audio_sync_candidate_neon(data: &[u8]) -> Option<AudioSyncCandidate> {
        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };

            let first_ff = vceqq_u8(b0, vdupq_n_u8(0xFF));
            let adts = vandq_u8(
                first_ff,
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xF0)), vdupq_n_u8(0xF0)),
            );
            let mpeg_audio = vandq_u8(
                first_ff,
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xE0)), vdupq_n_u8(0xE0)),
            );
            let latm = vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(0x56)),
                vceqq_u8(vandq_u8(b1, vdupq_n_u8(0xE0)), vdupq_n_u8(0xE0)),
            );
            let ac3 = vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(0x0B)),
                vceqq_u8(b1, vdupq_n_u8(0x77)),
            );
            let dts = unsafe { dts_word_match(b0, b1, b2, b3) };
            let cmp = vorrq_u8(
                vorrq_u8(vorrq_u8(adts, mpeg_audio), vorrq_u8(latm, ac3)),
                dts,
            );

            if let Some(index) = first_neon_match(cmp) {
                return super::audio_sync_candidate_at(data, offset + index);
            }
            offset += 16;
        }

        super::scalar::find_audio_sync_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_ebml_candidate_neon(
        data: &[u8],
        ids: &[u32],
    ) -> Option<super::EbmlCandidate> {
        if ids.len() > 8 {
            return super::scalar::find_ebml_candidate_from(data, 0, ids);
        }
        let mut specs = [([0_u8; 4], 0usize, 0_u32); 8];
        let mut spec_count = 0usize;
        for &id in ids.iter().take(specs.len()) {
            let (bytes, len) = super::ebml_id_bytes(id);
            specs[spec_count] = (bytes, len, id);
            spec_count += 1;
        }

        let mut offset = 0;
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let mut cmp = vdupq_n_u8(0);
            for &(bytes, len, _) in &specs[..spec_count] {
                cmp = vorrq_u8(cmp, unsafe {
                    byte_sequence_match(b0, b1, b2, b3, bytes, len)
                });
            }
            if let Some(index) = first_neon_match(cmp) {
                return super::ebml_candidate_at(data, offset + index, ids);
            }
            offset += 16;
        }

        super::scalar::find_ebml_candidate_from(data, offset, ids)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_hevc_sei_nal_header_candidate_neon(
        data: &[u8],
    ) -> Option<super::LengthPrefixedNalCandidate> {
        let mut offset = 0;
        while offset + 16 <= data.len() {
            let chunk = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let cmp = vorrq_u8(
                vorrq_u8(
                    vceqq_u8(chunk, vdupq_n_u8(0x4E)),
                    vceqq_u8(chunk, vdupq_n_u8(0x4F)),
                ),
                vorrq_u8(
                    vceqq_u8(chunk, vdupq_n_u8(0x50)),
                    vceqq_u8(chunk, vdupq_n_u8(0x51)),
                ),
            );
            if let Some(index) = first_neon_match(cmp) {
                return super::hevc_sei_nal_candidate_at(data, offset + index);
            }
            offset += 16;
        }

        super::scalar::find_hevc_sei_nal_header_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_hdr10plus_itu_t35_candidate_neon(data: &[u8]) -> Option<usize> {
        let mut offset = 0;
        while offset + 21 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let b4 = unsafe { vld1q_u8(data.as_ptr().add(offset + 4)) };
            let b5 = unsafe { vld1q_u8(data.as_ptr().add(offset + 5)) };
            let cmp = vandq_u8(
                unsafe { word_match(b0, b1, b2, b3, [0xB5, 0x00, 0x3C, 0x00]) },
                vandq_u8(
                    vceqq_u8(b4, vdupq_n_u8(0x01)),
                    vceqq_u8(b5, vdupq_n_u8(0x04)),
                ),
            );
            if let Some(index) = first_neon_match(cmp) {
                return Some(offset + index);
            }
            offset += 16;
        }

        super::scalar::find_hdr10plus_itu_t35_candidate_from(data, offset)
    }

    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn find_mp4_box_name_candidate_neon(
        data: &[u8],
        start: usize,
        names: &[[u8; 4]],
    ) -> Option<super::Mp4BoxNameCandidate> {
        let mut offset = start.saturating_add(4);
        while offset + 19 <= data.len() {
            let b0 = unsafe { vld1q_u8(data.as_ptr().add(offset)) };
            let b1 = unsafe { vld1q_u8(data.as_ptr().add(offset + 1)) };
            let b2 = unsafe { vld1q_u8(data.as_ptr().add(offset + 2)) };
            let b3 = unsafe { vld1q_u8(data.as_ptr().add(offset + 3)) };
            let mut cmp = vdupq_n_u8(0);
            for &name in names {
                cmp = vorrq_u8(cmp, unsafe { word_match(b0, b1, b2, b3, name) });
            }
            if let Some(index) = first_neon_match(cmp) {
                return super::mp4_box_name_candidate_at(data, offset + index, names);
            }
            offset += 16;
        }

        super::scalar::find_mp4_box_name_candidate_from(data, offset.saturating_sub(4), names)
    }

    #[target_feature(enable = "neon")]
    unsafe fn dts_word_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
    ) -> uint8x16_t {
        let mut cmp = unsafe { word_match(b0, b1, b2, b3, [0x7F, 0xFE, 0x80, 0x01]) };
        cmp = vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0xFE, 0x7F, 0x01, 0x80])
        });
        cmp = vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0x1F, 0xFF, 0xE8, 0x00])
        });
        vorrq_u8(cmp, unsafe {
            word_match(b0, b1, b2, b3, [0xFF, 0x1F, 0x00, 0xE8])
        })
    }

    #[target_feature(enable = "neon")]
    unsafe fn byte_sequence_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
        bytes: [u8; 4],
        len: usize,
    ) -> uint8x16_t {
        match len {
            1 => vceqq_u8(b0, vdupq_n_u8(bytes[0])),
            2 => vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(bytes[0])),
                vceqq_u8(b1, vdupq_n_u8(bytes[1])),
            ),
            3 => vandq_u8(
                vandq_u8(
                    vceqq_u8(b0, vdupq_n_u8(bytes[0])),
                    vceqq_u8(b1, vdupq_n_u8(bytes[1])),
                ),
                vceqq_u8(b2, vdupq_n_u8(bytes[2])),
            ),
            4 => unsafe { word_match(b0, b1, b2, b3, bytes) },
            _ => vdupq_n_u8(0),
        }
    }

    #[target_feature(enable = "neon")]
    unsafe fn word_match(
        b0: uint8x16_t,
        b1: uint8x16_t,
        b2: uint8x16_t,
        b3: uint8x16_t,
        word: [u8; 4],
    ) -> uint8x16_t {
        vandq_u8(
            vandq_u8(
                vceqq_u8(b0, vdupq_n_u8(word[0])),
                vceqq_u8(b1, vdupq_n_u8(word[1])),
            ),
            vandq_u8(
                vceqq_u8(b2, vdupq_n_u8(word[2])),
                vceqq_u8(b3, vdupq_n_u8(word[3])),
            ),
        )
    }

    #[inline]
    fn first_neon_match(mask: uint8x16_t) -> Option<usize> {
        if unsafe { vmaxvq_u8(mask) } == 0 {
            return None;
        }
        let mut lanes = [0_u8; 16];
        unsafe { vst1q_u8(lanes.as_mut_ptr(), mask) };
        lanes.iter().position(|&lane| lane != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_byte_matches_scalar() {
        for len in 0..96 {
            let mut data = vec![0x55; len];
            for needle_at in 0..=len {
                if needle_at < len {
                    data[needle_at] = 0xA7;
                }
                assert_eq!(
                    find_byte_from(&data, 0xA7, 0),
                    scalar::find_byte_from(&data, 0xA7, 0)
                );
                assert_eq!(
                    find_byte_from(&data, 0xA7, len.min(needle_at.saturating_add(1))),
                    scalar::find_byte_from(&data, 0xA7, len.min(needle_at.saturating_add(1)))
                );
                if needle_at < len {
                    data[needle_at] = 0x55;
                }
            }
        }
    }

    #[test]
    fn find_any_byte_matches_scalar() {
        let needles = [0x00, 0x47, 0xFF, 0x0B];
        for len in 0..128 {
            let mut data = vec![0x22; len];
            for needle_at in 0..=len {
                if needle_at < len {
                    data[needle_at] = needles[needle_at % needles.len()];
                }
                assert_eq!(
                    find_any_byte_from(&data, &needles, 0),
                    scalar::find_any_byte_from(&data, &needles, 0)
                );
                if needle_at < len {
                    data[needle_at] = 0x22;
                }
            }
        }
    }

    #[test]
    fn finds_mpeg_start_code_boundaries() {
        assert_eq!(find_mpeg_start_code(&[], 0xB3), None);
        assert_eq!(find_mpeg_start_code(&[0, 0, 1, 0xB3], 0xB3), Some(0));
        assert_eq!(find_mpeg_start_code(&[9, 0, 0, 1, 0xB3, 1], 0xB3), Some(1));
        assert_eq!(find_mpeg_start_code(&[0, 0, 1, 0xB8], 0xB3), None);
        assert_eq!(find_mpeg_start_code(&[0, 0, 1], 0xB3), None);
    }

    #[test]
    fn finds_annexb_start_code_boundaries() {
        assert_eq!(find_annexb_start_code(&[], 0), None);
        assert_eq!(find_annexb_start_code(&[0, 0, 1, 7], 0), Some((0, 3)));
        assert_eq!(find_annexb_start_code(&[9, 0, 0, 0, 1, 7], 0), Some((1, 4)));
        assert_eq!(find_annexb_start_code(&[0, 0, 2, 1], 0), None);
        assert_eq!(find_annexb_start_code(&[0, 0], 0), None);
    }

    #[test]
    fn finds_audio_sync_boundaries() {
        assert_eq!(find_adts_sync(&[0, 0xFF, 0xF1, 0], 0), Some(1));
        assert_eq!(find_latm_sync(&[0, 0x56, 0xE0, 0], 0), Some(1));
        assert_eq!(find_mpeg_audio_sync(&[0, 0xFF, 0xE2, 0], 0), Some(1));
        assert_eq!(find_ac3_sync(&[0, 0x0B, 0x77, 0], 0), Some(1));
        assert_eq!(find_dts_sync(&[0, 0x7F, 0xFE, 0x80, 0x01, 0], 0), Some(1));
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x56, 0xE0, 0xFF, 0xF1], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Latm,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0xFF, 0xF1, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Adts,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0xFF, 0xE2, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::MpegAudio,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x0B, 0x77, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Ac3,
            })
        );
        assert_eq!(
            find_audio_sync_candidate(&[0, 0x7F, 0xFE, 0x80, 0x01, 0], 0),
            Some(AudioSyncCandidate {
                offset: 1,
                kind: AudioSyncKind::Dts,
            })
        );
        assert_eq!(find_adts_sync(&[0xFF, 0xE1], 0), None);
        assert_eq!(find_dts_sync(&[0x7F, 0xFE, 0x80], 0), None);
    }

    #[test]
    fn finds_ebml_and_payload_candidates() {
        let ids = [0x1F43_B675, 0xA3, 0xA0, 0xA1, 0x75A1, 0xA6, 0xA5, 0xE7];
        let data = [0x55, 0x75, 0x00, 0xAA, 0x1F, 0x43, 0xB6, 0x75, 0xE7];
        assert_eq!(
            find_ebml_candidate(&data, 0, &ids),
            Some(EbmlCandidate {
                offset: 4,
                id: 0x1F43_B675,
            })
        );
        assert_eq!(
            find_ebml_candidate(&data, 5, &ids),
            Some(EbmlCandidate {
                offset: 8,
                id: 0xE7,
            })
        );

        let nal_data = [0x00, 0x4D, 0x4E, 0x01, 0x50];
        assert_eq!(
            find_hevc_sei_nal_header_candidate(&nal_data, 0),
            Some(LengthPrefixedNalCandidate {
                offset: 2,
                nal_type: 39,
            })
        );
        assert_eq!(
            find_hdr10plus_itu_t35_candidate(&[0, 0xB5, 0, 0x3C, 0, 1, 4], 0),
            Some(1)
        );

        let mp4 = [0, 0, 0, 12, b'm', b'o', b'o', b'v', 0, 0, 0, 0];
        assert_eq!(
            find_mp4_box_name_candidate(&mp4, 0, &[*b"moov", *b"trak"]),
            Some(Mp4BoxNameCandidate {
                offset: 0,
                name: *b"moov",
            })
        );
        assert_eq!(
            find_avi_idx1_stream_prefix(&[b'0', b'2', b'w', b'b', 0, 0, 0, 0], 0, 2),
            Some(0)
        );
    }

    #[test]
    fn random_buffers_match_scalar() {
        let mut state = 0x1234_5678_9ABC_DEF0_u64;
        for len in [0, 1, 2, 3, 4, 7, 15, 16, 31, 32, 33, 63, 64, 127, 255] {
            let mut data = vec![0_u8; len];
            for byte in &mut data {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }

            for start in 0..=len {
                assert_eq!(
                    find_byte_from(&data, 0x00, start),
                    scalar::find_byte_from(&data, 0x00, start)
                );
                assert_eq!(
                    find_any_byte_from(&data, &[0, 1, 0x47, 0xFF], start),
                    scalar::find_any_byte_from(&data, &[0, 1, 0x47, 0xFF], start)
                );
                assert_eq!(
                    find_adts_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0xFF, 0xF0, 0xF0)
                );
                assert_eq!(
                    find_latm_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0x56, 0xE0, 0xE0)
                );
                assert_eq!(
                    find_mpeg_audio_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0xFF, 0xE0, 0xE0)
                );
                assert_eq!(
                    find_ac3_sync(&data, start),
                    scalar::find_pair_from(&data, start, 0x0B, 0xFF, 0x77)
                );
                assert_eq!(
                    find_dts_sync(&data, start),
                    scalar::find_dts_sync_from(&data, start)
                );
                assert_eq!(
                    find_audio_sync_candidate(&data, start),
                    scalar::find_audio_sync_candidate_from(&data, start)
                );
                assert_eq!(
                    find_ebml_candidate(&data, start, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]),
                    scalar::find_ebml_candidate_from(
                        &data,
                        start,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7]
                    )
                );
                assert_eq!(
                    find_hevc_sei_nal_header_candidate(&data, start),
                    scalar::find_hevc_sei_nal_header_candidate_from(&data, start)
                );
                assert_eq!(
                    find_hdr10plus_itu_t35_candidate(&data, start),
                    scalar::find_hdr10plus_itu_t35_candidate_from(&data, start)
                );
                assert_eq!(
                    find_mp4_box_name_candidate(&data, start, &[*b"moov", *b"trak", *b"stsd"]),
                    scalar::find_mp4_box_name_candidate_from(
                        &data,
                        start,
                        &[*b"moov", *b"trak", *b"stsd"]
                    )
                );
                assert_eq!(find_avi_idx1_stream_prefix(&data, start, 2), {
                    let mut cursor = start;
                    let mut found = None;
                    while let Some(offset) = scalar::find_pair_from(&data, cursor, b'0', 0xFF, b'2')
                    {
                        if offset % 16 == 0 {
                            found = Some(offset);
                            break;
                        }
                        cursor = offset + 1;
                    }
                    found
                });
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_backend_matches_scalar_when_supported() {
        let mut data = vec![0x31; 257];
        data[211] = 0xA7;
        data[233] = 0x47;
        data[241..245].copy_from_slice(&[0x7F, 0xFE, 0x80, 0x01]);
        let needles = [0x00, 0x47, 0xFF];

        if std::arch::is_x86_feature_detected!("avx2") {
            assert_eq!(
                unsafe { super::x86_64::find_byte_avx2(&data, 0xA7) },
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_any_byte_avx2(&data, &needles) },
                scalar::find_any_byte_from(&data, &needles, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_pair_avx2(&data, 0x7F, 0xFF, 0xFE) },
                scalar::find_pair_from(&data, 0, 0x7F, 0xFF, 0xFE)
            );
            assert_eq!(
                unsafe { super::x86_64::find_dts_sync_avx2(&data) },
                scalar::find_dts_sync_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_audio_sync_candidate_avx2(&data) },
                scalar::find_audio_sync_candidate_from(&data, 0)
            );
            data[17..21].copy_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
            data[41] = 0x4E;
            data[73..79].copy_from_slice(&[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
            assert_eq!(
                unsafe {
                    super::x86_64::find_ebml_candidate_avx2(
                        &data,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(&data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::x86_64::find_hevc_sei_nal_header_candidate_avx2(&data) },
                scalar::find_hevc_sei_nal_header_candidate_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::x86_64::find_hdr10plus_itu_t35_candidate_avx2(&data) },
                scalar::find_hdr10plus_itu_t35_candidate_from(&data, 0)
            );
            data[105..109].copy_from_slice(b"moov");
            assert_eq!(
                unsafe {
                    super::x86_64::find_mp4_box_name_candidate_avx2(&data, 0, &[*b"moov", *b"trak"])
                },
                scalar::find_mp4_box_name_candidate_from(&data, 0, &[*b"moov", *b"trak"])
            );
        } else {
            assert_eq!(
                find_byte_from(&data, 0xA7, 0),
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                find_any_byte_from(&data, &needles, 0),
                scalar::find_any_byte_from(&data, &needles, 0)
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn neon_backend_matches_scalar_when_supported() {
        let mut data = vec![0x31; 257];
        data[211] = 0xA7;
        data[233] = 0x47;
        data[241..245].copy_from_slice(&[0x7F, 0xFE, 0x80, 0x01]);
        let needles = [0x00, 0x47, 0xFF];

        if super::aarch64_neon_available() {
            assert_eq!(
                unsafe { super::aarch64::find_byte_neon(&data, 0xA7) },
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_any_byte_neon(&data, &needles) },
                scalar::find_any_byte_from(&data, &needles, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_pair_neon(&data, 0x7F, 0xFF, 0xFE) },
                scalar::find_pair_from(&data, 0, 0x7F, 0xFF, 0xFE)
            );
            assert_eq!(
                unsafe { super::aarch64::find_dts_sync_neon(&data) },
                scalar::find_dts_sync_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_audio_sync_candidate_neon(&data) },
                scalar::find_audio_sync_candidate_from(&data, 0)
            );
            data[17..21].copy_from_slice(&[0x1F, 0x43, 0xB6, 0x75]);
            data[41] = 0x4E;
            data[73..79].copy_from_slice(&[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
            assert_eq!(
                unsafe {
                    super::aarch64::find_ebml_candidate_neon(
                        &data,
                        &[0x1F43_B675, 0xA3, 0x75A1, 0xE7],
                    )
                },
                scalar::find_ebml_candidate_from(&data, 0, &[0x1F43_B675, 0xA3, 0x75A1, 0xE7])
            );
            assert_eq!(
                unsafe { super::aarch64::find_hevc_sei_nal_header_candidate_neon(&data) },
                scalar::find_hevc_sei_nal_header_candidate_from(&data, 0)
            );
            assert_eq!(
                unsafe { super::aarch64::find_hdr10plus_itu_t35_candidate_neon(&data) },
                scalar::find_hdr10plus_itu_t35_candidate_from(&data, 0)
            );
            data[105..109].copy_from_slice(b"moov");
            assert_eq!(
                unsafe {
                    super::aarch64::find_mp4_box_name_candidate_neon(
                        &data,
                        0,
                        &[*b"moov", *b"trak"],
                    )
                },
                scalar::find_mp4_box_name_candidate_from(&data, 0, &[*b"moov", *b"trak"])
            );
        } else {
            assert_eq!(
                find_byte_from(&data, 0xA7, 0),
                scalar::find_byte_from(&data, 0xA7, 0)
            );
            assert_eq!(
                find_any_byte_from(&data, &needles, 0),
                scalar::find_any_byte_from(&data, &needles, 0)
            );
        }
    }
}
