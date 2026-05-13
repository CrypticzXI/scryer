use criterion::{Criterion, criterion_group, criterion_main};
use std::collections::{BTreeMap, HashSet};
use std::hint::black_box;

pub use scryer_application::{AppError, AppResult, ParsedReleaseMetadata, parse_release_metadata};

mod subtitles_impl {
    pub mod language {
        pub use scryer_application::subtitles::language::*;
    }

    #[allow(dead_code, unused_imports)]
    pub mod scoring {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/subtitles/scoring.rs"
        ));
    }

    #[allow(dead_code, unused_imports)]
    pub mod provider {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/subtitles/provider.rs"
        ));
    }
}

use subtitles_impl::provider::{
    PreparedQueryTitles, SubtitleMediaKind, SubtitleQuery, collect_title_candidates,
    release_group_matches, title_matches_query,
};
use subtitles_impl::scoring::{
    MOVIE_WEIGHTS, SERIES_WEIGHTS, SubtitleScoreKind, compute_verified_score,
};

fn subtitle_query() -> SubtitleQuery {
    SubtitleQuery {
        media_kind: SubtitleMediaKind::Episode,
        facet: Some("anime".into()),
        file_hash: None,
        imdb_id: None,
        series_imdb_id: None,
        title: "Starfall: Iron Eclipse".into(),
        title_aliases: vec!["Starfall Iron Eclipse".into()],
        title_candidates: vec!["Starfall - Iron Eclipse".into()],
        year: Some(2022),
        season: Some(1),
        episode: Some(14),
        absolute_episode: Some(14),
        external_ids: BTreeMap::new(),
        languages: vec!["eng".into()],
        release_group: Some("Studio Nova".into()),
        source: Some("BD".into()),
        video_codec: Some("HEVC".into()),
        audio_codec: Some("AAC".into()),
        resolution: Some("1080p".into()),
        hearing_impaired: Some(false),
        include_ai_translated: false,
        include_machine_translated: false,
    }
}

fn verified_score_bench(c: &mut Criterion) {
    let episode_weights = SERIES_WEIGHTS.weights();
    let movie_weights = MOVIE_WEIGHTS.weights();

    let episode_matches: HashSet<String> = [
        "series",
        "season",
        "episode",
        "source",
        "release_group",
        "video_codec",
        "audio_codec",
        "resolution",
        "hearing_impaired",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let movie_matches: HashSet<String> = [
        "hash",
        "title",
        "year",
        "source",
        "video_codec",
        "release_group",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    c.bench_function("subtitles/verified_score_episode", |b| {
        b.iter(|| {
            compute_verified_score(
                black_box(&episode_weights),
                black_box(SubtitleScoreKind::Episode),
                black_box(&episode_matches),
                black_box(false),
            )
        })
    });
    c.bench_function("subtitles/verified_score_movie", |b| {
        b.iter(|| {
            compute_verified_score(
                black_box(&movie_weights),
                black_box(SubtitleScoreKind::Movie),
                black_box(&movie_matches),
                black_box(false),
            )
        })
    });
}

fn title_matching_bench(c: &mut Criterion) {
    let query = subtitle_query();
    let candidates = collect_title_candidates(&query);
    let prepared = PreparedQueryTitles::from_candidates(&candidates);
    let candidate = Some("Starfall Iron Eclipse");

    c.bench_function("subtitles/title_match_prepared_query", |b| {
        b.iter(|| title_matches_query(black_box(candidate), black_box(&prepared)))
    });
}

fn release_group_matching_bench(c: &mut Criterion) {
    let left = Some("FRAMESTOR");
    let right = Some("W4NK3R");

    c.bench_function("subtitles/release_group_equivalence", |b| {
        b.iter(|| release_group_matches(black_box(left), black_box(right)))
    });
}

criterion_group!(
    benches,
    verified_score_bench,
    title_matching_bench,
    release_group_matching_bench
);
criterion_main!(benches);
