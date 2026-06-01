use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use xtask_support::{TaskContext, command_available, ok, require_command, step, warn};

#[derive(Args)]
pub(crate) struct MediaFixturesArgs {
    #[command(subcommand)]
    pub(crate) command: MediaFixturesCommand,
}

#[derive(Subcommand)]
pub(crate) enum MediaFixturesCommand {
    Generate(MediaFixturesGenerateArgs),
}

#[derive(Args)]
pub(crate) struct MediaFixturesGenerateArgs {
    #[arg(
        long,
        help = "Only rewrite media-fixtures.toml; do not invoke ffmpeg or touch media files"
    )]
    pub(crate) manifest_only: bool,
}

#[derive(Clone)]
struct FixtureCase {
    name: String,
    container: &'static str,
    video_codec: &'static str,
    width: i32,
    height: i32,
    fps: i32,
    audio_codecs: Vec<String>,
    source_audio_codecs: Vec<&'static str>,
    audio_channels: Vec<i32>,
    audio_languages: Vec<String>,
    source_audio_languages: Vec<&'static str>,
    subtitle_stream_count: usize,
    generated: bool,
    duration_seconds: Option<f64>,
    min_duration_seconds: i32,
    valid_video: bool,
    derive_ts_layout: Option<DerivedTsLayout>,
}

#[derive(Clone)]
struct DerivedTsLayout {
    source: &'static str,
    raw_packet_size: usize,
    sync_offset: usize,
}

const WIDTHS: [i32; 4] = [64, 80, 96, 112];
const HEIGHTS: [i32; 4] = [36, 44, 54, 64];
const RATES: [i32; 5] = [12, 15, 24, 25, 30];
const CHANNELS: [i32; 3] = [1, 2, 6];
const LANGS: [&str; 4] = ["eng", "jpn", "spa", "und"];

pub(crate) fn generate(ctx: &TaskContext, args: &MediaFixturesGenerateArgs) -> Result<()> {
    step("Generating scryer-mediainfo media fixtures");

    let tests_dir = ctx.path("crates/scryer-mediainfo/tests");
    let media_dir = tests_dir.join("media");
    let manifest_path = tests_dir.join("media-fixtures.toml");
    fs::create_dir_all(&media_dir)
        .with_context(|| format!("failed to create {}", media_dir.display()))?;

    let ffprobe_available = if args.manifest_only {
        false
    } else {
        require_command("ffmpeg").context("ffmpeg is required to generate media fixtures")?;
        let available = command_available("ffprobe").unwrap_or(false);
        if !available {
            warn("ffprobe was not found; generated fixtures will not get ffprobe validation");
        }
        available
    };

    let mut cases = build_matrix();
    cases.extend(build_dense_simd_cases());
    write_manifest(&manifest_path, &cases)?;

    if args.manifest_only {
        ok(format!("wrote {}", manifest_path.display()));
        return Ok(());
    }

    for (index, case) in cases.iter().enumerate() {
        generate_case(ctx, &media_dir, case, ffprobe_available)
            .with_context(|| format!("failed to generate {} at case {}", case.name, index + 1))?;
    }

    write_simd_scan_fixture(&media_dir)?;
    ok(format!(
        "generated {} media fixtures in {}",
        cases.len(),
        media_dir.display()
    ));
    Ok(())
}

fn dims(index: usize) -> (i32, i32) {
    (WIDTHS[index % WIDTHS.len()], HEIGHTS[index % HEIGHTS.len()])
}

fn rate(index: usize) -> i32 {
    RATES[index % RATES.len()]
}

fn channel_layout(channels: i32) -> &'static str {
    match channels {
        1 => "mono",
        2 => "stereo",
        6 => "5.1",
        _ => "stereo",
    }
}

fn normalize_audio(codec: &'static str) -> &'static str {
    match codec {
        "opus" => "opus",
        "vorbis" => "vorbis",
        "mp2" => "mp2",
        "dts" => "dts",
        other => other,
    }
}

fn supported_channels(codec: &str, requested: i32) -> i32 {
    if codec == "vorbis" {
        return 2;
    }
    if matches!(codec, "mp3" | "mp2" | "vorbis") {
        return requested.min(2);
    }
    requested
}

struct FixtureFlags {
    subtitle: bool,
    dual: bool,
}

fn build_case(
    prefix: &str,
    index: usize,
    ext: &'static str,
    container: &'static str,
    video: &'static str,
    audio: &[&'static str],
    flags: FixtureFlags,
) -> FixtureCase {
    let (width, height) = dims(index);
    let mut audios = audio.to_vec();
    if flags.dual && audios.len() == 1 {
        let extra = if ext == "webm" {
            if audios[0] != "vorbis" {
                "vorbis"
            } else {
                "opus"
            }
        } else if ext == "mov" {
            "aac"
        } else if matches!(ext, "mp4" | "m4v" | "mov") {
            if audios[0] == "aac" { "ac3" } else { "aac" }
        } else if audios[0] != "aac" {
            "aac"
        } else {
            "mp3"
        };
        audios.push(extra);
    }

    let mut channels = audios
        .iter()
        .enumerate()
        .map(|(stream, codec)| {
            supported_channels(codec, CHANNELS[(index + stream) % CHANNELS.len()])
        })
        .collect::<Vec<_>>();
    if ext == "webm" {
        channels.fill(2);
    }

    let source_audio_languages = (0..audios.len())
        .map(|stream| LANGS[(index + stream) % LANGS.len()])
        .collect::<Vec<_>>();
    let audio_languages = source_audio_languages
        .iter()
        .copied()
        .filter(|language| *language != "und" && container != "avi")
        .map(str::to_string)
        .collect::<Vec<_>>();

    FixtureCase {
        name: format!("matrix_{prefix}_{index:03}.{ext}"),
        container,
        video_codec: video,
        width,
        height,
        fps: rate(index),
        audio_codecs: audios
            .iter()
            .copied()
            .map(normalize_audio)
            .map(str::to_string)
            .collect(),
        source_audio_codecs: audios,
        audio_channels: channels,
        audio_languages,
        source_audio_languages,
        subtitle_stream_count: usize::from(flags.subtitle),
        generated: true,
        duration_seconds: None,
        min_duration_seconds: 1,
        valid_video: true,
        derive_ts_layout: None,
    }
}

fn build_matrix() -> Vec<FixtureCase> {
    let mut cases = Vec::new();

    let mkv_pairs = product(
        &["h264", "mpeg4", "mjpeg", "mpeg2video", "vp9"],
        &["aac", "mp3", "flac", "ac3", "eac3", "opus", "vorbis"],
    );
    for i in 0..43 {
        let (video, audio) = mkv_pairs[i % mkv_pairs.len()];
        cases.push(build_case(
            "mkv",
            i + 1,
            "mkv",
            "matroska",
            video,
            &[audio],
            FixtureFlags {
                subtitle: i % 9 == 0,
                dual: i % 7 == 0,
            },
        ));
    }

    let webm_pairs = product(&["vp8", "vp9"], &["opus", "vorbis"]);
    for i in 0..12 {
        let (video, audio) = webm_pairs[i % webm_pairs.len()];
        cases.push(build_case(
            "webm",
            i + 1,
            "webm",
            "webm",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 5 == 0,
            },
        ));
    }

    let mp4_exts = ["mp4", "m4v", "mov"];
    let mp4_pairs = product(&["h264", "mpeg4", "vp9"], &["aac", "ac3", "eac3", "opus"]);
    for i in 0..60 {
        let ext = mp4_exts[i % mp4_exts.len()];
        let (mut video, mut audio) = mp4_pairs[i % mp4_pairs.len()];
        if ext == "mov" {
            video = "h264";
            if audio != "aac" {
                audio = "aac";
            }
        }
        let container = if ext == "mov" { "mov" } else { "mp4" };
        cases.push(build_case(
            "mp4",
            i + 1,
            ext,
            container,
            video,
            &[audio],
            FixtureFlags {
                subtitle: i % 10 == 0,
                dual: i % 8 == 0,
            },
        ));
    }

    let avi_pairs = product(
        &["mpeg4", "mjpeg", "mpeg2video", "h264"],
        &["mp3", "pcm_s16le", "ac3", "aac"],
    );
    for i in 0..40 {
        let (video, audio) = avi_pairs[i % avi_pairs.len()];
        cases.push(build_case(
            "avi",
            i + 1,
            "avi",
            "avi",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 6 == 0,
            },
        ));
    }

    let ts_exts = ["ts", "m2ts"];
    let ts_pairs = product(
        &["h264", "mpeg2video"],
        &["aac", "mp2", "ac3", "eac3", "dts"],
    );
    for i in 0..45 {
        let ext = ts_exts[i % ts_exts.len()];
        let (video, audio) = ts_pairs[i % ts_pairs.len()];
        cases.push(build_case(
            "ts",
            i + 1,
            ext,
            "mpegts",
            video,
            &[audio],
            FixtureFlags {
                subtitle: false,
                dual: i % 9 == 0,
            },
        ));
    }

    assert_eq!(cases.len(), 200);
    cases
}

fn product(
    left: &'static [&'static str],
    right: &'static [&'static str],
) -> Vec<(&'static str, &'static str)> {
    let mut pairs = Vec::with_capacity(left.len() * right.len());
    for lhs in left {
        for rhs in right {
            pairs.push((*lhs, *rhs));
        }
    }
    pairs
}

fn build_dense_idx1_case() -> FixtureCase {
    FixtureCase {
        name: "avi_idx1_dense_mpeg4.avi".to_string(),
        container: "avi",
        video_codec: "mpeg4",
        width: 32,
        height: 18,
        fps: 120,
        audio_codecs: Vec::new(),
        source_audio_codecs: Vec::new(),
        audio_channels: Vec::new(),
        audio_languages: Vec::new(),
        source_audio_languages: Vec::new(),
        subtitle_stream_count: 0,
        generated: true,
        duration_seconds: Some(8.0),
        min_duration_seconds: 8,
        valid_video: true,
        derive_ts_layout: None,
    }
}

fn build_dense_simd_case(
    name: &'static str,
    container: &'static str,
    video: &'static str,
    audio: &'static str,
) -> FixtureCase {
    FixtureCase {
        name: name.to_string(),
        container,
        video_codec: video,
        width: 32,
        height: 18,
        fps: 120,
        audio_codecs: vec![normalize_audio(audio).to_string()],
        source_audio_codecs: vec![audio],
        audio_channels: vec![2],
        audio_languages: vec!["eng".to_string()],
        source_audio_languages: vec!["eng"],
        subtitle_stream_count: 0,
        generated: true,
        duration_seconds: Some(8.0),
        min_duration_seconds: 7,
        valid_video: true,
        derive_ts_layout: None,
    }
}

fn build_derived_ts_layout_case(
    name: &'static str,
    raw_packet_size: usize,
    sync_offset: usize,
) -> FixtureCase {
    let mut case = build_dense_simd_case(name, "mpegts", "h264", "aac");
    case.derive_ts_layout = Some(DerivedTsLayout {
        source: "simd_dense_h264_aac.ts",
        raw_packet_size,
        sync_offset,
    });
    case
}

fn build_dense_simd_cases() -> Vec<FixtureCase> {
    vec![
        build_dense_idx1_case(),
        build_dense_simd_case("simd_dense_h264_aac.ts", "mpegts", "h264", "aac"),
        build_derived_ts_layout_case("simd_dense_h264_aac_192.m2ts", 192, 4),
        build_derived_ts_layout_case("simd_dense_h264_aac_204.ts", 204, 0),
        build_dense_simd_case("simd_dense_mpeg2_mp2.ts", "mpegts", "mpeg2video", "mp2"),
        build_dense_simd_case("simd_dense_h264_ac3.ts", "mpegts", "h264", "ac3"),
        build_dense_simd_case("simd_dense_h264_dts.ts", "mpegts", "h264", "dts"),
        build_dense_simd_case("simd_dense_vp9_opus.webm", "webm", "vp9", "opus"),
        build_dense_simd_case("simd_dense_h264_aac.mp4", "mp4", "h264", "aac"),
    ]
}

fn write_manifest(manifest_path: &Path, cases: &[FixtureCase]) -> Result<()> {
    fs::write(manifest_path, manifest_contents(cases))
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn manifest_contents(cases: &[FixtureCase]) -> String {
    let mut manifest = String::from(
        "# Generated by cargo xtask media-fixtures generate.\n\
         # This manifest is the source of truth for the committed matrix fixtures.\n\n",
    );

    for (index, case) in cases.iter().enumerate() {
        if index > 0 {
            manifest.push('\n');
        }
        manifest.push_str("[[fixtures]]\n");
        push_toml_str(&mut manifest, "name", &case.name);
        push_toml_bool(&mut manifest, "generated", case.generated);
        push_toml_str(&mut manifest, "container", case.container);
        push_toml_str(&mut manifest, "video_codec", case.video_codec);
        push_toml_i32(&mut manifest, "width", case.width);
        push_toml_i32(&mut manifest, "height", case.height);
        push_toml_i32(&mut manifest, "fps", case.fps);
        push_toml_str_array(&mut manifest, "audio_codecs", &case.audio_codecs);
        push_toml_i32_array(&mut manifest, "audio_channels", &case.audio_channels);
        push_toml_str_array(&mut manifest, "audio_languages", &case.audio_languages);
        push_toml_usize(
            &mut manifest,
            "subtitle_stream_count",
            case.subtitle_stream_count,
        );
        push_toml_i32(
            &mut manifest,
            "min_duration_seconds",
            case.min_duration_seconds,
        );
        push_toml_bool(&mut manifest, "valid_video", case.valid_video);
    }

    manifest
}

fn push_toml_str(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push_str(" = \"");
    output.push_str(&toml_escape(value));
    output.push_str("\"\n");
}

fn push_toml_bool(output: &mut String, key: &str, value: bool) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(if value { "true" } else { "false" });
    output.push('\n');
}

fn push_toml_i32(output: &mut String, key: &str, value: i32) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_string());
    output.push('\n');
}

fn push_toml_usize(output: &mut String, key: &str, value: usize) {
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_string());
    output.push('\n');
}

fn push_toml_str_array(output: &mut String, key: &str, values: &[String]) {
    output.push_str(key);
    output.push_str(" = [");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push('"');
        output.push_str(&toml_escape(value));
        output.push('"');
    }
    output.push_str("]\n");
}

fn push_toml_i32_array(output: &mut String, key: &str, values: &[i32]) {
    output.push_str(key);
    output.push_str(" = [");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&value.to_string());
    }
    output.push_str("]\n");
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn derive_ts_layout_case(
    media_dir: &Path,
    case: &FixtureCase,
    spec: &DerivedTsLayout,
) -> Result<()> {
    let source = media_dir.join(spec.source);
    let out = media_dir.join(&case.name);
    if spec.sync_offset + 188 > spec.raw_packet_size {
        bail!("invalid TS layout for {}", case.name);
    }

    let source_bytes =
        fs::read(&source).with_context(|| format!("failed to read {}", source.display()))?;
    let mut out_bytes = Vec::new();
    let usable_len = source_bytes.len() - (source_bytes.len() % 188);
    let mut packet_count = 0usize;
    for offset in (0..usable_len).step_by(188) {
        let packet = &source_bytes[offset..offset + 188];
        if packet[0] != 0x47 {
            bail!(
                "{} lost TS sync at packet {}",
                source.display(),
                packet_count
            );
        }

        let mut raw_packet = vec![0_u8; spec.raw_packet_size];
        raw_packet[spec.sync_offset..spec.sync_offset + 188].copy_from_slice(packet);
        out_bytes.extend(raw_packet);
        packet_count += 1;
    }

    if packet_count == 0 {
        bail!("{} did not contain TS packets", source.display());
    }

    fs::write(&out, out_bytes).with_context(|| format!("failed to write {}", out.display()))
}

fn generate_case(
    ctx: &TaskContext,
    media_dir: &Path,
    case: &FixtureCase,
    ffprobe_available: bool,
) -> Result<()> {
    let out = media_dir.join(&case.name);
    if let Some(spec) = &case.derive_ts_layout {
        derive_ts_layout_case(media_dir, case, spec)?;
        if ffprobe_available {
            verify_with_ffprobe(ctx, &out)?;
        }
        return Ok(());
    }

    let duration = case.duration_seconds.unwrap_or(1.4).to_string();
    let mut command = ctx.command("ffmpeg");
    command
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "testsrc2=size={}x{}:rate={}:duration={duration}",
            case.width, case.height, case.fps
        ));

    for channels in &case.audio_channels {
        command
            .arg("-f")
            .arg("lavfi")
            .arg("-t")
            .arg(&duration)
            .arg("-i")
            .arg(format!("anullsrc=r=48000:cl={}", channel_layout(*channels)));
    }

    let subtitle_path = if case.subtitle_stream_count > 0 {
        let path = media_dir.join(format!(".{}.srt", output_stem(&case.name)?));
        fs::write(&path, "1\n00:00:00,000 --> 00:00:01,000\nfixture\n")
            .with_context(|| format!("failed to write {}", path.display()))?;
        command.arg("-f").arg("srt").arg("-i").arg(&path);
        Some(path)
    } else {
        None
    };

    command.arg("-map").arg("0:v:0");
    for stream in 0..case.audio_channels.len() {
        command.arg("-map").arg(format!("{}:a:0", stream + 1));
    }
    if subtitle_path.is_some() {
        command
            .arg("-map")
            .arg(format!("{}:s:0", case.audio_channels.len() + 1));
    }

    append_video_encoder(&mut command, case.video_codec)?;
    for (stream, codec) in case.source_audio_codecs.iter().enumerate() {
        append_audio_encoder(&mut command, codec, stream)?;
        command
            .arg(format!("-ac:a:{stream}"))
            .arg(case.audio_channels[stream].to_string());
        command
            .arg(format!("-metadata:s:a:{stream}"))
            .arg(format!("language={}", case.source_audio_languages[stream]));
    }

    if subtitle_path.is_some() {
        if matches!(output_ext(&case.name)?, "mp4" | "m4v" | "mov") {
            command
                .arg("-c:s")
                .arg("mov_text")
                .arg("-metadata:s:s:0")
                .arg("language=eng");
        } else {
            command
                .arg("-c:s")
                .arg("srt")
                .arg("-metadata:s:s:0")
                .arg("language=eng");
        }
    }

    append_muxer_format(&mut command, output_ext(&case.name)?)?;
    command.arg("-shortest").arg(&out);

    let generate_result = crate::run_checked(&mut command);
    if let Some(path) = subtitle_path {
        let _ = fs::remove_file(path);
    }
    generate_result?;

    if ffprobe_available {
        verify_with_ffprobe(ctx, &out)?;
    }
    Ok(())
}

fn append_video_encoder(command: &mut Command, codec: &str) -> Result<()> {
    match codec {
        "h264" => {
            command
                .arg("-c:v")
                .arg("libx264")
                .arg("-preset")
                .arg("ultrafast")
                .arg("-tune")
                .arg("zerolatency")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "mpeg4" => {
            command
                .arg("-c:v")
                .arg("mpeg4")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "mjpeg" => {
            command
                .arg("-c:v")
                .arg("mjpeg")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuvj420p");
        }
        "mpeg2video" => {
            command
                .arg("-c:v")
                .arg("mpeg2video")
                .arg("-q:v")
                .arg("8")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "vp8" => {
            command
                .arg("-c:v")
                .arg("libvpx")
                .arg("-deadline")
                .arg("realtime")
                .arg("-cpu-used")
                .arg("8")
                .arg("-b:v")
                .arg("80k")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        "vp9" => {
            command
                .arg("-c:v")
                .arg("libvpx-vp9")
                .arg("-deadline")
                .arg("realtime")
                .arg("-cpu-used")
                .arg("8")
                .arg("-b:v")
                .arg("80k")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        _ => bail!("unsupported video codec {codec}"),
    }
    Ok(())
}

fn append_audio_encoder(command: &mut Command, codec: &str, stream: usize) -> Result<()> {
    let stream_spec = format!(":a:{stream}");
    match codec {
        "aac" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("aac")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "mp3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("libmp3lame")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "pcm_s16le" => {
            command.arg(format!("-c{stream_spec}")).arg("pcm_s16le");
        }
        "ac3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("ac3")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "eac3" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("eac3")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "flac" => {
            command.arg(format!("-c{stream_spec}")).arg("flac");
        }
        "opus" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("libopus")
                .arg(format!("-b{stream_spec}"))
                .arg("64k");
        }
        "vorbis" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("vorbis")
                .arg(format!("-strict{stream_spec}"))
                .arg("-2");
        }
        "mp2" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("mp2")
                .arg(format!("-b{stream_spec}"))
                .arg("96k");
        }
        "dts" => {
            command
                .arg(format!("-c{stream_spec}"))
                .arg("dca")
                .arg(format!("-strict{stream_spec}"))
                .arg("-2");
        }
        _ => bail!("unsupported audio codec {codec}"),
    }
    Ok(())
}

fn append_muxer_format(command: &mut Command, extension: &str) -> Result<()> {
    match extension {
        "webm" => command.arg("-f").arg("webm"),
        "mp4" | "m4v" => command.arg("-f").arg("mp4"),
        "mov" => command.arg("-f").arg("mov"),
        "ts" | "m2ts" => command.arg("-f").arg("mpegts"),
        "avi" => command.arg("-f").arg("avi"),
        _ => bail!("unsupported fixture extension {extension}"),
    };
    Ok(())
}

fn verify_with_ffprobe(ctx: &TaskContext, path: &Path) -> Result<()> {
    let mut command = ctx.command("ffprobe");
    command
        .arg("-v")
        .arg("error")
        .arg("-show_streams")
        .arg("-show_format")
        .arg(path)
        .stdout(Stdio::null());
    crate::run_checked(&mut command).with_context(|| format!("ffprobe rejected {}", path.display()))
}

fn output_ext(name: &str) -> Result<&str> {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| anyhow::anyhow!("fixture name has no UTF-8 extension: {name}"))
}

fn output_stem(name: &str) -> Result<&str> {
    Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow::anyhow!("fixture name has no UTF-8 stem: {name}"))
}

fn write_simd_scan_fixture(media_dir: &Path) -> Result<()> {
    let mut data = Vec::new();

    late(&mut data, b"\xA7", 1024);
    late(&mut data, b"\x47", 512);
    late(&mut data, b"\x00\x00\x01\xB3", 1024);
    late(&mut data, b"\x00\x00\x00\x01\x67", 1024);
    late(&mut data, b"\xFF\xF1\x50\x80", 512);
    late(&mut data, b"\x56\xE0\x00\x00", 512);
    late(&mut data, b"\xFF\xE2\x00\x00", 512);
    late(&mut data, b"\x0B\x77\x00\x00", 512);
    late(&mut data, b"\x7F\xFE\x80\x01", 512);
    late(&mut data, b"\x1F\x43\xB6\x75", 1024);
    late(&mut data, b"\xA3", 512);
    late(&mut data, b"\x75\xA1", 512);
    late(&mut data, b"\xE7", 512);
    late(&mut data, b"\x4E\x01\x50\x00", 1024);
    late(&mut data, b"\xB5\x00\x3C\x00\x01\x04", 1024);
    late(&mut data, b"\x00\x00\x03\x01", 512);
    late(&mut data, b"\x00\x00\x00\x0Cmoov\x00\x00\x00\x00", 1024);

    let trailing_padding = (16 - (data.len() % 16)) % 16;
    if trailing_padding != 0 {
        pad(&mut data, trailing_padding, 0x55);
    }
    data.extend_from_slice(b"02wb");
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&128_u32.to_le_bytes());
    data.extend_from_slice(&64_u32.to_le_bytes());

    let out = media_dir.join("simd_scan_dense.bin");
    fs::write(&out, data).with_context(|| format!("failed to write {}", out.display()))
}

fn late(data: &mut Vec<u8>, payload: &[u8], pad_len: usize) {
    pad(data, pad_len, 0x55);
    data.extend_from_slice(payload);
}

fn pad(data: &mut Vec<u8>, length: usize, byte: u8) {
    data.extend(std::iter::repeat_n(byte, length));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_manifest_matches_committed_fixture_manifest() {
        let mut cases = build_matrix();
        cases.extend(build_dense_simd_cases());
        assert_eq!(cases.len(), 209);

        let manifest_path =
            xtask_support::repo_root().join("crates/scryer-mediainfo/tests/media-fixtures.toml");
        let committed = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));

        assert_eq!(manifest_contents(&cases), committed);
    }
}
