#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResolvedAnalysisReleaseLabels {
    pub quality: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub is_atmos: bool,
}

pub(crate) fn resolve_release_labels_from_analysis(
    video_height: Option<i32>,
    video_codec: Option<&crate::release_parser::VideoCodec>,
    primary_audio_codec: Option<&str>,
    primary_audio_profile: Option<&str>,
    primary_audio_channels: Option<i32>,
    audio_streams: &[crate::AudioStreamDetail],
) -> ResolvedAnalysisReleaseLabels {
    let quality = quality_from_video_height(video_height).map(str::to_string);
    let video_codec = video_codec
        .and_then(normalize_video_codec_for_release)
        .map(str::to_string);

    let mut best_audio_label =
        normalize_audio_codec_for_release(primary_audio_codec, primary_audio_profile);

    for stream in audio_streams {
        let candidate =
            normalize_audio_codec_for_release(stream.codec.as_deref(), stream.profile.as_deref());
        if candidate.as_deref().is_some_and(|candidate| {
            audio_codec_rank_for_release_label(candidate)
                > best_audio_label
                    .as_deref()
                    .map(audio_codec_rank_for_release_label)
                    .unwrap_or(0)
        }) {
            best_audio_label = candidate;
        }
    }

    let max_channels = audio_streams
        .iter()
        .filter_map(|stream| stream.channels)
        .max()
        .or(primary_audio_channels);
    let audio_channels = max_channels.map(format_audio_channels_for_release);

    let is_atmos = audio_streams.iter().any(|stream| {
        normalize_audio_codec_for_release(stream.codec.as_deref(), stream.profile.as_deref())
            .as_deref()
            .is_some_and(|label| label.contains("Atmos") || label == "DTS:X")
    }) || best_audio_label
        .as_deref()
        .is_some_and(|label| label.contains("Atmos") || label == "DTS:X");

    ResolvedAnalysisReleaseLabels {
        quality,
        video_codec,
        audio_codec: best_audio_label,
        audio_channels,
        is_atmos,
    }
}

pub(crate) fn quality_from_video_height(height: Option<i32>) -> Option<&'static str> {
    match height? {
        h if h >= 2100 => Some("2160p"),
        h if h >= 1000 => Some("1080p"),
        h if h >= 700 => Some("720p"),
        h if h >= 480 => Some("480p"),
        _ => None,
    }
}

pub(crate) fn normalize_video_codec_for_release(
    codec: &crate::release_parser::VideoCodec,
) -> Option<&'static str> {
    match codec {
        crate::release_parser::VideoCodec::H265 => Some("H.265"),
        crate::release_parser::VideoCodec::H264 => Some("H.264"),
        crate::release_parser::VideoCodec::Av1 => Some("AV1"),
        crate::release_parser::VideoCodec::Vp9 => Some("VP9"),
        crate::release_parser::VideoCodec::Mpeg4
        | crate::release_parser::VideoCodec::Xvid
        | crate::release_parser::VideoCodec::Divx => Some("MPEG-4"),
        _ => None,
    }
}

pub(crate) fn normalize_audio_codec_for_release(
    codec: Option<&str>,
    profile: Option<&str>,
) -> Option<String> {
    let profile_lower = profile.unwrap_or_default().to_ascii_lowercase();
    let codec_lower = codec.unwrap_or_default().to_ascii_lowercase();

    if profile_lower.contains("dolby truehd") && profile_lower.contains("atmos") {
        return Some("TrueHD Atmos".into());
    }
    if profile_lower.contains("dolby digital plus") && profile_lower.contains("atmos") {
        return Some("EAC3 Atmos".into());
    }
    if profile_lower.contains("dts:x") {
        return Some("DTS:X".into());
    }
    if profile_lower.contains("dts-hd ma") {
        return Some("DTS-HD MA".into());
    }
    if profile_lower.contains("dts-hd hra") {
        return Some("DTS-HD".into());
    }
    if profile_lower.contains("dts") {
        return Some("DTS".into());
    }

    if codec_lower.contains("truehd") {
        return Some("TrueHD".into());
    }
    if codec_lower.contains("e-ac-3") || codec_lower.contains("eac3") || codec_lower.contains("dd+")
    {
        return Some("EAC3".into());
    }
    if codec_lower.contains("ac-3") || codec_lower.contains("ac3") {
        return Some("AC3".into());
    }
    if codec_lower.contains("dts-hd ma") || codec_lower.contains("dts-hd master") {
        return Some("DTS-HD MA".into());
    }
    if codec_lower.contains("dts-hd") {
        return Some("DTS-HD".into());
    }
    if codec_lower.contains("dts") {
        return Some("DTS".into());
    }
    if codec_lower.contains("flac") {
        return Some("FLAC".into());
    }
    if codec_lower.contains("aac") {
        return Some("AAC".into());
    }
    if codec_lower.contains("mp3") || codec_lower.contains("mpeg audio") {
        return Some("MP3".into());
    }
    if codec_lower.contains("opus") {
        return Some("Opus".into());
    }
    if codec_lower.contains("vorbis") {
        return Some("Vorbis".into());
    }
    if codec_lower.contains("pcm") || codec_lower.contains("lpcm") {
        return Some("PCM".into());
    }

    None
}

pub(crate) fn audio_codec_rank_for_release_label(label: &str) -> i32 {
    match label {
        "TrueHD Atmos" => 100,
        "DTS:X" => 95,
        "TrueHD" => 90,
        "DTS-HD MA" => 85,
        "FLAC" => 80,
        "EAC3 Atmos" => 75,
        "EAC3" => 70,
        "DTS-HD" => 65,
        "DTS" => 60,
        "AC3" => 50,
        "AAC" | "Opus" => 40,
        "MP3" | "Vorbis" => 30,
        "PCM" => 20,
        _ => 10,
    }
}

pub(crate) fn format_audio_channels_for_release(channels: i32) -> String {
    match channels {
        8 => "7.1".to_string(),
        7 => "6.1".to_string(),
        6 => "5.1".to_string(),
        2 => "2.0".to_string(),
        1 => "1.0".to_string(),
        n => format!("{n}.0"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_profile_driven_audio_labels() {
        assert_eq!(
            normalize_audio_codec_for_release(Some("dts"), Some("DTS-HD MA + DTS:X IMAX"))
                .as_deref(),
            Some("DTS:X")
        );
        assert_eq!(
            normalize_audio_codec_for_release(Some("truehd"), Some("Dolby TrueHD + Dolby Atmos"))
                .as_deref(),
            Some("TrueHD Atmos")
        );
        assert_eq!(
            normalize_audio_codec_for_release(
                Some("eac3"),
                Some("Dolby Digital Plus + Dolby Atmos")
            )
            .as_deref(),
            Some("EAC3 Atmos")
        );
    }

    #[test]
    fn formats_audio_channels_like_release_names() {
        assert_eq!(format_audio_channels_for_release(8), "7.1");
        assert_eq!(format_audio_channels_for_release(6), "5.1");
        assert_eq!(format_audio_channels_for_release(2), "2.0");
    }
}
