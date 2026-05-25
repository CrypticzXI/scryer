use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use scryer_mediainfo::analyze_file;

#[allow(dead_code, unused_imports)]
#[path = "../src/scan.rs"]
mod scan;

fn media(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("media")
        .join(name)
}

fn bench_fixture(c: &mut Criterion, name: &str) {
    let path = media(name);
    if !Path::new(&path).is_file() {
        return;
    }

    c.bench_function(&format!("analyze {name}"), |b| {
        b.iter(|| analyze_file(std::hint::black_box(&path)).expect("fixture should analyze"))
    });
}

fn late_match_buffer(len: usize, pattern: &[u8]) -> Vec<u8> {
    let mut data = vec![0x55; len];
    let start = len.saturating_sub(pattern.len());
    data[start..].copy_from_slice(pattern);
    data
}

fn bench_start_code_late_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/start-code late match");
    let data = late_match_buffer(1024 * 1024, &[0, 0, 1, 0xB3]);
    group.throughput(Throughput::Bytes(data.len() as u64));

    group.bench_function(BenchmarkId::new("scalar", data.len()), |b| {
        b.iter(|| scan::scalar::find_mpeg_start_code_from(black_box(&data), black_box(0xB3), 0))
    });
    group.bench_function(BenchmarkId::new("dispatch", data.len()), |b| {
        b.iter(|| scan::find_mpeg_start_code(black_box(&data), black_box(0xB3)))
    });

    group.finish();
}

fn bench_audio_sync_late_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan/audio-sync late match");
    let data = late_match_buffer(1024 * 1024, &[0xFF, 0xF1, 0x50, 0x80]);
    group.throughput(Throughput::Bytes(data.len() as u64));

    group.bench_function(BenchmarkId::new("scalar byte", data.len()), |b| {
        b.iter(|| scan::scalar::find_byte_from(black_box(&data), black_box(0xFF), black_box(0)))
    });
    group.bench_function(BenchmarkId::new("dispatch byte", data.len()), |b| {
        b.iter(|| scan::find_byte_from(black_box(&data), black_box(0xFF), black_box(0)))
    });
    group.bench_function(BenchmarkId::new("scalar any-byte", data.len()), |b| {
        b.iter(|| {
            scan::scalar::find_any_byte_from(
                black_box(&data),
                black_box(&[0x7F, 0xFE, 0x1F, 0xFF]),
                black_box(0),
            )
        })
    });
    group.bench_function(BenchmarkId::new("dispatch any-byte", data.len()), |b| {
        b.iter(|| {
            scan::find_any_byte_from(
                black_box(&data),
                black_box(&[0x7F, 0xFE, 0x1F, 0xFF]),
                black_box(0),
            )
        })
    });

    group.finish();
}

fn scan_hotpaths(c: &mut Criterion) {
    bench_start_code_late_match(c);
    bench_audio_sync_late_match(c);

    for name in [
        "h264_aac.ts",
        "h264_aac.mkv",
        "hevc_hdr10plus.mkv",
        "h264_aac.mp4",
    ] {
        bench_fixture(c, name);
    }
}

criterion_group!(benches, scan_hotpaths);
criterion_main!(benches);
