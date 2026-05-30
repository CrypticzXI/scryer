#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEDIA_DIR="${SCRIPT_DIR}/media"
MANIFEST="${SCRIPT_DIR}/media-fixtures.toml"

python3 - "$SCRIPT_DIR" "$MEDIA_DIR" "$MANIFEST" <<'PY'
from __future__ import annotations

import itertools
import os
import shutil
import subprocess
import sys
from pathlib import Path

script_dir = Path(sys.argv[1])
media_dir = Path(sys.argv[2])
manifest_path = Path(sys.argv[3])

ffmpeg = shutil.which("ffmpeg")
ffprobe = shutil.which("ffprobe")
if not ffmpeg:
    raise SystemExit("ffmpeg is required to generate media fixtures")

media_dir.mkdir(parents=True, exist_ok=True)

WIDTHS = [64, 80, 96, 112]
HEIGHTS = [36, 44, 54, 64]
RATES = [12, 15, 24, 25, 30]
CHANNELS = [1, 2, 6]
LANGS = ["eng", "jpn", "spa", "und"]


def dims(i: int) -> tuple[int, int]:
    return WIDTHS[i % len(WIDTHS)], HEIGHTS[i % len(HEIGHTS)]


def rate(i: int) -> int:
    return RATES[i % len(RATES)]


def channel_layout(channels: int) -> str:
    return {1: "mono", 2: "stereo", 6: "5.1"}.get(channels, "stereo")


def video_encoder(codec: str) -> list[str]:
    if codec == "h264":
        return ["-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-pix_fmt", "yuv420p"]
    if codec == "mpeg4":
        return ["-c:v", "mpeg4", "-q:v", "8", "-pix_fmt", "yuv420p"]
    if codec == "mjpeg":
        return ["-c:v", "mjpeg", "-q:v", "8", "-pix_fmt", "yuvj420p"]
    if codec == "mpeg2video":
        return ["-c:v", "mpeg2video", "-q:v", "8", "-pix_fmt", "yuv420p"]
    if codec == "vp8":
        return ["-c:v", "libvpx", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "80k", "-pix_fmt", "yuv420p"]
    if codec == "vp9":
        return ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "80k", "-pix_fmt", "yuv420p"]
    raise ValueError(codec)


def audio_encoder(codec: str, stream: int) -> list[str]:
    prefix = f":a:{stream}"
    if codec == "aac":
        return [f"-c{prefix}", "aac", f"-b{prefix}", "64k"]
    if codec == "mp3":
        return [f"-c{prefix}", "libmp3lame", f"-b{prefix}", "64k"]
    if codec == "pcm_s16le":
        return [f"-c{prefix}", "pcm_s16le"]
    if codec == "ac3":
        return [f"-c{prefix}", "ac3", f"-b{prefix}", "96k"]
    if codec == "eac3":
        return [f"-c{prefix}", "eac3", f"-b{prefix}", "96k"]
    if codec == "flac":
        return [f"-c{prefix}", "flac"]
    if codec == "opus":
        return [f"-c{prefix}", "libopus", f"-b{prefix}", "64k"]
    if codec == "vorbis":
        return [f"-c{prefix}", "vorbis", f"-strict{prefix}", "-2"]
    if codec == "mp2":
        return [f"-c{prefix}", "mp2", f"-b{prefix}", "96k"]
    if codec == "dts":
        return [f"-c{prefix}", "dca", f"-strict{prefix}", "-2"]
    raise ValueError(codec)


def normalize_audio(codec: str) -> str:
    return {
        "opus": "opus",
        "vorbis": "vorbis",
        "mp2": "mp2",
        "dts": "dts",
    }.get(codec, codec)


def supported_channels(codec: str, requested: int) -> int:
    if codec == "vorbis":
        return 2
    if codec in {"mp3", "mp2", "vorbis"}:
        return min(requested, 2)
    return requested


def build_case(prefix: str, index: int, ext: str, container: str, video: str, audio: list[str], *,
               subtitle: bool = False, dual: bool = False) -> dict:
    width, height = dims(index)
    audios = audio[:]
    if dual and len(audios) == 1:
        if ext == "webm":
            audios.append("vorbis" if audios[0] != "vorbis" else "opus")
        elif ext == "mov":
            audios.append("aac")
        elif ext in {"mp4", "m4v", "mov"}:
            audios.append("ac3" if audios[0] == "aac" else "aac")
        else:
            audios.append("aac" if audios[0] != "aac" else "mp3")
    channels = [
        supported_channels(codec, CHANNELS[(index + n) % len(CHANNELS)])
        for n, codec in enumerate(audios)
    ]
    if ext == "webm":
        channels = [2 for _ in audios]
    langs = [LANGS[(index + n) % len(LANGS)] for n in range(len(audios))]
    return {
        "name": f"matrix_{prefix}_{index:03d}.{ext}",
        "container": container,
        "video_codec": video,
        "width": width,
        "height": height,
        "fps": rate(index),
        "audio_codecs": [normalize_audio(a) for a in audios],
        "source_audio_codecs": audios,
        "audio_channels": channels,
        "audio_languages": [lang for lang in langs if lang != "und" and container not in {"avi"}],
        "source_audio_languages": langs,
        "subtitle_stream_count": 1 if subtitle else 0,
        "generated": True,
        "min_duration_seconds": 1,
        "valid_video": True,
    }


def build_matrix() -> list[dict]:
    cases: list[dict] = []

    mkv_pairs = list(itertools.product(
        ["h264", "mpeg4", "mjpeg", "mpeg2video", "vp9"],
        ["aac", "mp3", "flac", "ac3", "eac3", "opus", "vorbis"],
    ))
    for i in range(43):
        v, a = mkv_pairs[i % len(mkv_pairs)]
        cases.append(build_case("mkv", i + 1, "mkv", "matroska", v, [a],
                                subtitle=(i % 9 == 0), dual=(i % 7 == 0)))

    webm_pairs = list(itertools.product(["vp8", "vp9"], ["opus", "vorbis"]))
    for i in range(12):
        v, a = webm_pairs[i % len(webm_pairs)]
        cases.append(build_case("webm", i + 1, "webm", "webm", v, [a], dual=(i % 5 == 0)))

    mp4_exts = ["mp4", "m4v", "mov"]
    mp4_pairs = list(itertools.product(["h264", "mpeg4", "vp9"], ["aac", "ac3", "eac3", "opus"]))
    for i in range(60):
        ext = mp4_exts[i % len(mp4_exts)]
        v, a = mp4_pairs[i % len(mp4_pairs)]
        if ext == "mov":
            v = "h264"
            if a != "aac":
                a = "aac"
        container = "mov" if ext == "mov" else "mp4"
        cases.append(build_case("mp4", i + 1, ext, container, v, [a],
                                subtitle=(i % 10 == 0), dual=(i % 8 == 0)))

    avi_pairs = list(itertools.product(["mpeg4", "mjpeg", "mpeg2video", "h264"], ["mp3", "pcm_s16le", "ac3", "aac"]))
    for i in range(40):
        v, a = avi_pairs[i % len(avi_pairs)]
        cases.append(build_case("avi", i + 1, "avi", "avi", v, [a], dual=(i % 6 == 0)))

    ts_exts = ["ts", "m2ts"]
    ts_pairs = list(itertools.product(["h264", "mpeg2video"], ["aac", "mp2", "ac3", "eac3", "dts"]))
    for i in range(45):
        ext = ts_exts[i % len(ts_exts)]
        v, a = ts_pairs[i % len(ts_pairs)]
        cases.append(build_case("ts", i + 1, ext, "mpegts", v, [a], dual=(i % 9 == 0)))

    assert len(cases) == 200, len(cases)
    return cases


def build_dense_idx1_case() -> dict:
    return {
        "name": "avi_idx1_dense_mpeg4.avi",
        "container": "avi",
        "video_codec": "mpeg4",
        "width": 32,
        "height": 18,
        "fps": 120,
        "audio_codecs": [],
        "source_audio_codecs": [],
        "audio_channels": [],
        "audio_languages": [],
        "source_audio_languages": [],
        "subtitle_stream_count": 0,
        "generated": True,
        "duration_seconds": 8.0,
        "min_duration_seconds": 8,
        "valid_video": True,
    }


def build_dense_simd_case(name: str, ext: str, container: str, video: str, audio: str) -> dict:
    return {
        "name": name,
        "container": container,
        "video_codec": video,
        "width": 32,
        "height": 18,
        "fps": 120,
        "audio_codecs": [normalize_audio(audio)],
        "source_audio_codecs": [audio],
        "audio_channels": [2],
        "audio_languages": ["eng"],
        "source_audio_languages": ["eng"],
        "subtitle_stream_count": 0,
        "generated": True,
        "duration_seconds": 8.0,
        "min_duration_seconds": 7,
        "valid_video": True,
    }


def build_derived_ts_layout_case(name: str, raw_packet_size: int, sync_offset: int) -> dict:
    case = build_dense_simd_case(name, "ts", "mpegts", "h264", "aac")
    case["derive_ts_layout"] = {
        "source": "simd_dense_h264_aac.ts",
        "raw_packet_size": raw_packet_size,
        "sync_offset": sync_offset,
    }
    return case


def build_dense_simd_cases() -> list[dict]:
    return [
        build_dense_idx1_case(),
        build_dense_simd_case("simd_dense_h264_aac.ts", "ts", "mpegts", "h264", "aac"),
        build_derived_ts_layout_case("simd_dense_h264_aac_192.m2ts", 192, 4),
        build_derived_ts_layout_case("simd_dense_h264_aac_204.ts", 204, 0),
        build_dense_simd_case("simd_dense_mpeg2_mp2.ts", "ts", "mpegts", "mpeg2video", "mp2"),
        build_dense_simd_case("simd_dense_h264_ac3.ts", "ts", "mpegts", "h264", "ac3"),
        build_dense_simd_case("simd_dense_h264_dts.ts", "ts", "mpegts", "h264", "dts"),
        build_dense_simd_case("simd_dense_vp9_opus.webm", "webm", "webm", "vp9", "opus"),
        build_dense_simd_case("simd_dense_h264_aac.mp4", "mp4", "mp4", "h264", "aac"),
    ]


def write_simd_scan_fixture() -> None:
    data = bytearray()

    def pad(length: int, byte: int = 0x55) -> None:
        data.extend(bytes([byte]) * length)

    def late(payload: bytes, pad_len: int = 512) -> None:
        pad(pad_len)
        data.extend(payload)

    late(b"\xA7", 1024)
    late(b"\x47", 512)
    late(b"\x00\x00\x01\xB3", 1024)
    late(b"\x00\x00\x00\x01\x67", 1024)
    late(b"\xFF\xF1\x50\x80", 512)
    late(b"\x56\xE0\x00\x00", 512)
    late(b"\xFF\xE2\x00\x00", 512)
    late(b"\x0B\x77\x00\x00", 512)
    late(b"\x7F\xFE\x80\x01", 512)
    late(b"\x1F\x43\xB6\x75", 1024)
    late(b"\xA3", 512)
    late(b"\x75\xA1", 512)
    late(b"\xE7", 512)
    late(b"\x4E\x01\x50\x00", 1024)
    late(b"\xB5\x00\x3C\x00\x01\x04", 1024)
    late(b"\x00\x00\x03\x01", 512)
    late(b"\x00\x00\x00\x0Cmoov\x00\x00\x00\x00", 1024)

    if len(data) % 16:
        pad(16 - (len(data) % 16))
    data.extend(b"02wb")
    data.extend((0).to_bytes(4, "little"))
    data.extend((128).to_bytes(4, "little"))
    data.extend((64).to_bytes(4, "little"))

    out = media_dir / "simd_scan_dense.bin"
    out.write_bytes(data)


def toml_bool(value: bool) -> str:
    return "true" if value else "false"


def toml_array(values) -> str:
    rendered = []
    for value in values:
        if isinstance(value, str):
            rendered.append('"' + value.replace('"', '\\"') + '"')
        elif isinstance(value, bool):
            rendered.append(toml_bool(value))
        else:
            rendered.append(str(value))
    return "[" + ", ".join(rendered) + "]"


def write_manifest(cases: list[dict]) -> None:
    lines = [
        "# Generated by tests/generate_media_fixtures.sh.",
        "# This manifest is the source of truth for the committed matrix fixtures.",
        "",
    ]
    for case in cases:
        lines.append("[[fixtures]]")
        for key in [
            "name",
            "generated",
            "container",
            "video_codec",
            "width",
            "height",
            "fps",
            "audio_codecs",
            "audio_channels",
            "audio_languages",
            "subtitle_stream_count",
            "min_duration_seconds",
            "valid_video",
        ]:
            value = case[key]
            if isinstance(value, str):
                lines.append(f'{key} = "{value}"')
            elif isinstance(value, bool):
                lines.append(f"{key} = {toml_bool(value)}")
            elif isinstance(value, list):
                lines.append(f"{key} = {toml_array(value)}")
            else:
                lines.append(f"{key} = {value}")
        lines.append("")
    manifest_path.write_text("\n".join(lines), encoding="utf-8")


def derive_ts_layout_case(case: dict) -> None:
    spec = case["derive_ts_layout"]
    source = media_dir / spec["source"]
    out = media_dir / case["name"]
    raw_packet_size = spec["raw_packet_size"]
    sync_offset = spec["sync_offset"]
    if sync_offset + 188 > raw_packet_size:
        raise ValueError(f"invalid TS layout for {case['name']}")

    source_bytes = source.read_bytes()
    out_bytes = bytearray()
    packet_count = 0
    for offset in range(0, len(source_bytes) - (len(source_bytes) % 188), 188):
        packet = source_bytes[offset:offset + 188]
        if packet[0] != 0x47:
            raise ValueError(f"{source.name} lost TS sync at packet {packet_count}")
        raw_packet = bytearray(raw_packet_size)
        raw_packet[sync_offset:sync_offset + 188] = packet
        out_bytes.extend(raw_packet)
        packet_count += 1
    if packet_count == 0:
        raise ValueError(f"{source.name} did not contain TS packets")
    out.write_bytes(out_bytes)


def generate_case(case: dict) -> None:
    out = media_dir / case["name"]
    if "derive_ts_layout" in case:
        derive_ts_layout_case(case)
        if ffprobe:
            subprocess.run([ffprobe, "-v", "error", "-show_streams", "-show_format", str(out)],
                           check=True, stdout=subprocess.DEVNULL)
        return

    duration = str(case.get("duration_seconds", 1.4))
    inputs = [
        ffmpeg,
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        f"testsrc2=size={case['width']}x{case['height']}:rate={case['fps']}:duration={duration}",
    ]
    for stream, channels in enumerate(case["audio_channels"]):
        inputs += [
            "-f",
            "lavfi",
            "-t",
            duration,
            "-i",
            f"anullsrc=r=48000:cl={channel_layout(channels)}",
        ]

    subtitle_path = None
    if case["subtitle_stream_count"]:
        subtitle_path = media_dir / f".{out.stem}.srt"
        subtitle_path.write_text("1\n00:00:00,000 --> 00:00:01,000\nfixture\n", encoding="utf-8")
        inputs += ["-f", "srt", "-i", str(subtitle_path)]

    cmd = inputs + ["-map", "0:v:0"]
    for stream in range(len(case["audio_channels"])):
        cmd += ["-map", f"{stream + 1}:a:0"]
    if subtitle_path is not None:
        cmd += ["-map", f"{len(case['audio_channels']) + 1}:s:0"]

    cmd += video_encoder(case["video_codec"])
    for stream, codec in enumerate(case["source_audio_codecs"]):
        cmd += audio_encoder(codec, stream)
        cmd += [f"-ac:a:{stream}", str(case["audio_channels"][stream])]
        lang = case["source_audio_languages"][stream]
        cmd += [f"-metadata:s:a:{stream}", f"language={lang}"]

    if subtitle_path is not None:
        if out.suffix in {".mp4", ".m4v", ".mov"}:
            cmd += ["-c:s", "mov_text", "-metadata:s:s:0", "language=eng"]
        else:
            cmd += ["-c:s", "srt", "-metadata:s:s:0", "language=eng"]

    if out.suffix == ".webm":
        cmd += ["-f", "webm"]
    elif out.suffix in {".mp4", ".m4v"}:
        cmd += ["-f", "mp4"]
    elif out.suffix == ".mov":
        cmd += ["-f", "mov"]
    elif out.suffix in {".ts", ".m2ts"}:
        cmd += ["-f", "mpegts"]
    elif out.suffix == ".avi":
        cmd += ["-f", "avi"]

    cmd += ["-shortest", str(out)]
    subprocess.run(cmd, check=True)
    if subtitle_path is not None:
        subtitle_path.unlink(missing_ok=True)

    if ffprobe:
        subprocess.run([ffprobe, "-v", "error", "-show_streams", "-show_format", str(out)],
                       check=True, stdout=subprocess.DEVNULL)


cases = build_matrix()
cases.extend(build_dense_simd_cases())
write_manifest(cases)
for index, case in enumerate(cases, 1):
    try:
        generate_case(case)
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"failed to generate {case['name']} at case {index}: {error}") from error

write_simd_scan_fixture()
print(f"generated {len(cases)} media fixtures in {media_dir}")
PY
