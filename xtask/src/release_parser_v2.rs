use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{ReleaseParserV2EvalArgs, TaskContext, ok, step};

#[derive(Debug, Deserialize)]
struct StructuredSample {
    facet: String,
    raw_title: String,
    label: ExpectedLabel,
}

#[derive(Debug, Deserialize)]
struct ExpectedLabel {
    facet_hint: Option<String>,
    kind: Option<String>,
    title: String,
    #[serde(default)]
    title_variants: Vec<String>,
    year: Option<i32>,
    #[serde(default)]
    quality: Option<String>,
    source: Option<String>,
    #[serde(default)]
    video_codec: Option<String>,
    #[serde(default)]
    video_encoding: Option<String>,
    #[serde(default)]
    audio: Option<String>,
    #[serde(default)]
    audio_codecs: Vec<String>,
    #[serde(default)]
    audio_channels: Option<String>,
    #[serde(default)]
    release_group: Option<String>,
    #[serde(default)]
    languages_audio: Vec<String>,
    #[serde(default)]
    languages_subtitles: Vec<String>,
    #[serde(default)]
    streaming_service: Option<String>,
    #[serde(default)]
    edition: Option<String>,
    #[serde(default)]
    anime_version: Option<u32>,
    #[serde(default)]
    flags: ExpectedFlags,
    #[serde(default)]
    missing_fields: Vec<String>,
    #[serde(default)]
    fps: Option<f32>,
    episode: Option<ExpectedEpisode>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
struct ExpectedFlags {
    #[serde(default)]
    dual_audio: bool,
    #[serde(default)]
    atmos: bool,
    #[serde(default)]
    dolby_vision: bool,
    #[serde(default)]
    hdr: bool,
    #[serde(default)]
    hdr_fallback: bool,
    #[serde(default)]
    hdr10plus: bool,
    #[serde(default)]
    hlg: bool,
    #[serde(default)]
    ten_bit: bool,
    #[serde(default)]
    proper: bool,
    #[serde(default)]
    repack: bool,
    #[serde(default)]
    remux: bool,
    #[serde(default)]
    bd_disk: bool,
    #[serde(default)]
    ai_enhanced: bool,
    #[serde(default)]
    hardcoded_subs: bool,
    #[serde(default)]
    uncensored: bool,
    #[serde(default)]
    dubs_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ExpectedEpisode {
    season: Option<u32>,
    #[serde(default)]
    episode_numbers: Vec<u32>,
    absolute_episode: Option<u32>,
    #[serde(default)]
    absolute_episode_numbers: Vec<u32>,
    #[serde(default)]
    special_absolute_episode_numbers: Vec<u32>,
    air_date: Option<String>,
    daily_part: Option<u32>,
    #[serde(default)]
    full_season: bool,
    #[serde(default, rename = "partial_season")]
    is_partial_season: bool,
    #[serde(default, rename = "multi_season")]
    is_multi_season: bool,
    season_part: Option<u32>,
    #[serde(default, rename = "season_extra")]
    is_season_extra: bool,
    #[serde(default, rename = "split_episode")]
    is_split_episode: bool,
    #[serde(default, rename = "mini_series")]
    is_mini_series: bool,
    special_kind: Option<String>,
    release_type: Option<String>,
    raw: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvalSummary {
    input_path: String,
    total: usize,
    exact_title: usize,
    full_match: usize,
    kind_match: usize,
    year_match: usize,
    episode_match: usize,
    source_match: usize,
    quality_match: usize,
    video_codec_match: usize,
    video_encoding_match: usize,
    audio_match: usize,
    audio_codecs_match: usize,
    audio_channels_match: usize,
    release_group_match: usize,
    languages_audio_match: usize,
    languages_subtitles_match: usize,
    streaming_service_match: usize,
    edition_match: usize,
    anime_version_match: usize,
    fps_match: usize,
    missing_fields_match: usize,
    flags_match: usize,
    release_metadata_match: usize,
    episode_contract_match: usize,
    full_contract_match: usize,
    mismatches_recorded: usize,
}

#[derive(Debug, Serialize)]
struct EvalMismatch {
    raw_title: String,
    facet: String,
    title_match: bool,
    kind_match: bool,
    year_match: bool,
    episode_match: bool,
    source_match: bool,
    quality_match: bool,
    video_codec_match: bool,
    video_encoding_match: bool,
    audio_match: bool,
    audio_codecs_match: bool,
    audio_channels_match: bool,
    release_group_match: bool,
    languages_audio_match: bool,
    languages_subtitles_match: bool,
    streaming_service_match: bool,
    edition_match: bool,
    anime_version_match: bool,
    fps_match: bool,
    missing_fields_match: bool,
    flags_match: bool,
    release_metadata_match: bool,
    episode_contract_match: bool,
    full_contract_match: bool,
    field_mismatches: Vec<String>,
    expected_title: String,
    actual_title: String,
    expected_kind: Option<String>,
    actual_kind: String,
    expected_year: Option<i32>,
    actual_year: Option<i32>,
    expected_source: Option<String>,
    actual_source: Option<String>,
    expected_metadata: MetadataSnapshot,
    actual_metadata: MetadataSnapshot,
    expected_episode: Option<ExpectedEpisode>,
    actual_episode: Option<ActualEpisode>,
}

#[derive(Debug, Serialize, Clone)]
struct MetadataSnapshot {
    quality: Option<String>,
    source: Option<String>,
    video_codec: Option<String>,
    video_encoding: Option<String>,
    audio: Option<String>,
    audio_codecs: Vec<String>,
    audio_channels: Option<String>,
    release_group: Option<String>,
    languages_audio: Vec<String>,
    languages_subtitles: Vec<String>,
    streaming_service: Option<String>,
    edition: Option<String>,
    anime_version: Option<u32>,
    fps: Option<f32>,
    missing_fields: Vec<String>,
    flags: ExpectedFlags,
}

#[derive(Debug, Serialize, Clone)]
struct ActualEpisode {
    season: Option<u32>,
    episode_numbers: Vec<u32>,
    absolute_episode: Option<u32>,
    absolute_episode_numbers: Vec<u32>,
    special_absolute_episode_numbers: Vec<u32>,
    air_date: Option<String>,
    daily_part: Option<u32>,
    full_season: bool,
    is_partial_season: bool,
    is_multi_season: bool,
    season_part: Option<u32>,
    is_season_extra: bool,
    is_split_episode: bool,
    is_mini_series: bool,
    special_kind: Option<String>,
    release_type: String,
    raw: Option<String>,
}

pub(crate) fn run_eval(ctx: &TaskContext, args: ReleaseParserV2EvalArgs) -> Result<()> {
    let input_path = resolve_input_path(ctx, args.input.as_ref())?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| input_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    fs::create_dir_all(&output_dir)?;

    step(format!(
        "Evaluating v2 release parser against {}",
        input_path.display()
    ));
    let file = File::open(&input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;
    let reader = BufReader::new(file);

    let mut summary = EvalSummary {
        input_path: input_path.display().to_string(),
        total: 0,
        exact_title: 0,
        full_match: 0,
        kind_match: 0,
        year_match: 0,
        episode_match: 0,
        source_match: 0,
        quality_match: 0,
        video_codec_match: 0,
        video_encoding_match: 0,
        audio_match: 0,
        audio_codecs_match: 0,
        audio_channels_match: 0,
        release_group_match: 0,
        languages_audio_match: 0,
        languages_subtitles_match: 0,
        streaming_service_match: 0,
        edition_match: 0,
        anime_version_match: 0,
        fps_match: 0,
        missing_fields_match: 0,
        flags_match: 0,
        release_metadata_match: 0,
        episode_contract_match: 0,
        full_contract_match: 0,
        mismatches_recorded: 0,
    };
    let mut mismatches = Vec::<EvalMismatch>::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let sample: StructuredSample =
            serde_json::from_str(&line).context("failed to deserialize structured sample")?;
        summary.total += 1;

        let context = build_context(&sample);
        let parsed = scryer_release_parser_v2::best_parse_for_target(&sample.raw_title, &context);
        score_parse(
            &sample,
            &parsed,
            &mut summary,
            &mut mismatches,
            args.max_mismatches,
        );
    }

    let summary_path = output_dir.join("release_parser_v2_eval_summary.json");
    let mismatches_path = output_dir.join("release_parser_v2_eval_mismatches.json");
    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;
    fs::write(&mismatches_path, serde_json::to_vec_pretty(&mismatches)?)?;

    ok(format!(
        "Wrote evaluation summary to {}",
        summary_path.display()
    ));
    ok(format!(
        "Wrote evaluation mismatches to {}",
        mismatches_path.display()
    ));
    Ok(())
}

fn resolve_input_path(ctx: &TaskContext, requested: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(path) = requested {
        return Ok(path.clone());
    }

    let corpus_root = ctx.path("tmp/release-parser-corpus");
    let mut candidates = fs::read_dir(&corpus_root)
        .with_context(|| format!("failed to read corpus dir {}", corpus_root.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("structured_samples_reviewed.jsonl"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop().ok_or_else(|| {
        anyhow::anyhow!(
            "no structured_samples_reviewed.jsonl found under {}",
            corpus_root.display()
        )
    })
}

fn build_context(sample: &StructuredSample) -> scryer_release_parser_v2::ReleaseParseContext {
    let authoritative_facet = if sample
        .label
        .kind
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("movie"))
    {
        "movie"
    } else {
        sample
            .label
            .facet_hint
            .as_deref()
            .unwrap_or(sample.facet.as_str())
            .trim()
    };
    let facet_hint = match authoritative_facet.to_ascii_lowercase().as_str() {
        "movie" => scryer_release_parser_v2::ContextFacetHint::Movie,
        "anime" => scryer_release_parser_v2::ContextFacetHint::Anime,
        _ => scryer_release_parser_v2::ContextFacetHint::Series,
    };
    let aliases = sample
        .label
        .title_variants
        .iter()
        .filter(|value| !value.eq_ignore_ascii_case(&sample.label.title))
        .map(|value| scryer_release_parser_v2::ContextAlias {
            name: value.clone(),
        })
        .collect::<Vec<_>>();
    let episodes = sample
        .label
        .episode
        .as_ref()
        .map(|episode| {
            vec![scryer_release_parser_v2::ContextEpisode {
                season: episode.season,
                episode: episode.episode_numbers.first().copied(),
                absolute_number: episode.absolute_episode_numbers.first().copied(),
                air_date: episode
                    .air_date
                    .as_deref()
                    .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()),
                title: None,
                title_aliases: Vec::new(),
            }]
        })
        .unwrap_or_default();

    scryer_release_parser_v2::ReleaseParseContext {
        facet_hint,
        title: scryer_release_parser_v2::ContextTitle {
            name: sample.label.title.clone(),
        },
        aliases,
        known_years: sample.label.year.into_iter().collect(),
        imdb_ids: Vec::new(),
        episodes,
    }
}

fn score_parse(
    sample: &StructuredSample,
    parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2,
    summary: &mut EvalSummary,
    mismatches: &mut Vec<EvalMismatch>,
    max_mismatches: usize,
) {
    let title_match = matches_title(sample, parsed);
    let kind_match = sample
        .label
        .kind
        .as_deref()
        .is_none_or(|expected| expected.eq_ignore_ascii_case(kind_label(parsed)));
    let year_match = sample.label.year == parsed.year;
    let episode_match = matches_episode(sample.label.episode.as_ref(), parsed.episode.as_ref());
    let source_match = matches_normalized_optional_or_improved(
        normalize_opt(sample.label.source.as_deref()),
        normalize_opt(parsed.source.as_deref()),
        &sample.label,
        "source",
    );
    let quality_match = matches_normalized_optional_or_improved(
        normalize_opt(sample.label.quality.as_deref()),
        normalize_opt(parsed.quality.as_deref()),
        &sample.label,
        "quality",
    );
    let video_codec_match = matches_normalized_optional_or_improved(
        normalize_video_codec(sample.label.video_codec.as_deref()),
        normalize_video_codec(parsed.video_codec.as_deref()),
        &sample.label,
        "video_codec",
    );
    let video_encoding_match = matches_video_encoding_or_improved(
        sample.raw_title.as_str(),
        sample.label.video_encoding.as_deref(),
        parsed.video_encoding.as_deref(),
        &sample.label,
    );
    let audio_match = matches_normalized_optional_or_improved(
        normalize_opt(sample.label.audio.as_deref()),
        normalize_opt(parsed.audio.as_deref()),
        &sample.label,
        "audio",
    );
    let audio_codecs_match = matches_normalized_vec_or_improved(
        sample.label.audio_codecs.as_slice(),
        parsed.audio_codecs.as_slice(),
        &sample.label,
        "audio",
    );
    let audio_channels_match = matches_normalized_optional_or_improved(
        normalize_opt(sample.label.audio_channels.as_deref()),
        normalize_opt(parsed.audio_channels.as_deref()),
        &sample.label,
        "audio",
    );
    let release_group_match = normalize_opt(sample.label.release_group.as_deref())
        == normalize_opt(parsed.release_group.as_deref());
    let languages_audio_match = matches_language_vec_or_improved(
        sample.raw_title.as_str(),
        sample.label.languages_audio.as_slice(),
        parsed.languages_audio.as_slice(),
    );
    let languages_subtitles_match = matches_language_vec_or_improved(
        sample.raw_title.as_str(),
        sample.label.languages_subtitles.as_slice(),
        parsed.languages_subtitles.as_slice(),
    );
    let streaming_service_match = matches_streaming_service_or_improved(
        sample.raw_title.as_str(),
        sample.label.streaming_service.as_deref(),
        parsed.streaming_service.as_deref(),
    );
    let edition_match =
        normalize_opt(sample.label.edition.as_deref()) == normalize_opt(parsed.edition.as_deref());
    let anime_version_match = sample.label.anime_version == parsed.anime_version;
    let fps_match = matches_fps(sample.label.fps, parsed.fps);
    let missing_fields_match = missing_fields_match_or_improved(&sample.label, parsed);
    let expected_flags = sample.label.flags.clone();
    let actual_flags = actual_flags(parsed);
    let flags_match =
        flags_match_or_improved(sample.raw_title.as_str(), &expected_flags, &actual_flags);
    let release_metadata_match = quality_match
        && video_codec_match
        && video_encoding_match
        && audio_match
        && audio_codecs_match
        && audio_channels_match
        && release_group_match
        && languages_audio_match
        && languages_subtitles_match
        && streaming_service_match
        && edition_match
        && anime_version_match
        && fps_match
        && missing_fields_match
        && flags_match;
    let episode_contract_match = matches_episode_contract(
        sample.raw_title.as_str(),
        sample.label.episode.as_ref(),
        parsed.episode.as_ref(),
    );
    let full_match = title_match && kind_match && year_match && episode_match && source_match;
    let full_contract_match = full_match && release_metadata_match && episode_contract_match;
    let expected_metadata = metadata_snapshot_from_expected(&sample.label);
    let actual_metadata = metadata_snapshot_from_actual(parsed);
    let mut field_mismatches = Vec::new();
    push_field_mismatch(&mut field_mismatches, "title", title_match);
    push_field_mismatch(&mut field_mismatches, "kind", kind_match);
    push_field_mismatch(&mut field_mismatches, "year", year_match);
    push_field_mismatch(&mut field_mismatches, "episode", episode_match);
    push_field_mismatch(&mut field_mismatches, "source", source_match);
    push_field_mismatch(&mut field_mismatches, "quality", quality_match);
    push_field_mismatch(&mut field_mismatches, "video_codec", video_codec_match);
    push_field_mismatch(
        &mut field_mismatches,
        "video_encoding",
        video_encoding_match,
    );
    push_field_mismatch(&mut field_mismatches, "audio", audio_match);
    push_field_mismatch(&mut field_mismatches, "audio_codecs", audio_codecs_match);
    push_field_mismatch(
        &mut field_mismatches,
        "audio_channels",
        audio_channels_match,
    );
    push_field_mismatch(&mut field_mismatches, "release_group", release_group_match);
    push_field_mismatch(
        &mut field_mismatches,
        "languages_audio",
        languages_audio_match,
    );
    push_field_mismatch(
        &mut field_mismatches,
        "languages_subtitles",
        languages_subtitles_match,
    );
    push_field_mismatch(
        &mut field_mismatches,
        "streaming_service",
        streaming_service_match,
    );
    push_field_mismatch(&mut field_mismatches, "edition", edition_match);
    push_field_mismatch(&mut field_mismatches, "anime_version", anime_version_match);
    push_field_mismatch(&mut field_mismatches, "fps", fps_match);
    push_field_mismatch(
        &mut field_mismatches,
        "missing_fields",
        missing_fields_match,
    );
    push_field_mismatch(&mut field_mismatches, "flags", flags_match);
    push_field_mismatch(
        &mut field_mismatches,
        "episode_contract",
        episode_contract_match,
    );

    summary.exact_title += usize::from(title_match);
    summary.full_match += usize::from(full_match);
    summary.kind_match += usize::from(kind_match);
    summary.year_match += usize::from(year_match);
    summary.episode_match += usize::from(episode_match);
    summary.source_match += usize::from(source_match);
    summary.quality_match += usize::from(quality_match);
    summary.video_codec_match += usize::from(video_codec_match);
    summary.video_encoding_match += usize::from(video_encoding_match);
    summary.audio_match += usize::from(audio_match);
    summary.audio_codecs_match += usize::from(audio_codecs_match);
    summary.audio_channels_match += usize::from(audio_channels_match);
    summary.release_group_match += usize::from(release_group_match);
    summary.languages_audio_match += usize::from(languages_audio_match);
    summary.languages_subtitles_match += usize::from(languages_subtitles_match);
    summary.streaming_service_match += usize::from(streaming_service_match);
    summary.edition_match += usize::from(edition_match);
    summary.anime_version_match += usize::from(anime_version_match);
    summary.fps_match += usize::from(fps_match);
    summary.missing_fields_match += usize::from(missing_fields_match);
    summary.flags_match += usize::from(flags_match);
    summary.release_metadata_match += usize::from(release_metadata_match);
    summary.episode_contract_match += usize::from(episode_contract_match);
    summary.full_contract_match += usize::from(full_contract_match);

    if !full_contract_match && mismatches.len() < max_mismatches {
        mismatches.push(EvalMismatch {
            raw_title: sample.raw_title.clone(),
            facet: sample.facet.clone(),
            title_match,
            kind_match,
            year_match,
            episode_match,
            source_match,
            quality_match,
            video_codec_match,
            video_encoding_match,
            audio_match,
            audio_codecs_match,
            audio_channels_match,
            release_group_match,
            languages_audio_match,
            languages_subtitles_match,
            streaming_service_match,
            edition_match,
            anime_version_match,
            fps_match,
            missing_fields_match,
            flags_match,
            release_metadata_match,
            episode_contract_match,
            full_contract_match,
            field_mismatches,
            expected_title: sample.label.title.clone(),
            actual_title: parsed.normalized_title.clone(),
            expected_kind: sample.label.kind.clone(),
            actual_kind: kind_label(parsed).to_string(),
            expected_year: sample.label.year,
            actual_year: parsed.year,
            expected_source: sample.label.source.clone(),
            actual_source: parsed.source.clone(),
            expected_metadata,
            actual_metadata,
            expected_episode: sample.label.episode.clone(),
            actual_episode: parsed.episode.as_ref().map(|episode| ActualEpisode {
                season: episode.season,
                episode_numbers: episode.episode_numbers.clone(),
                absolute_episode: episode.absolute_episode,
                absolute_episode_numbers: episode.absolute_episode_numbers.clone(),
                special_absolute_episode_numbers: episode.special_absolute_episode_numbers.clone(),
                air_date: episode.air_date.map(|value| value.to_string()),
                daily_part: episode.daily_part,
                full_season: episode.full_season,
                is_partial_season: episode.is_partial_season,
                is_multi_season: episode.is_multi_season,
                season_part: episode.season_part,
                is_season_extra: episode.is_season_extra,
                is_split_episode: episode.is_split_episode,
                is_mini_series: episode.is_mini_series,
                special_kind: episode.special_kind.map(special_kind_label),
                release_type: episode_release_type_label(episode.release_type).to_string(),
                raw: episode.raw.clone(),
            }),
        });
        summary.mismatches_recorded = mismatches.len();
    }
}

fn matches_title(
    sample: &StructuredSample,
    parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2,
) -> bool {
    let expected = normalize_title(sample.label.title.as_str());
    let mut actuals = parsed
        .normalized_title_variants
        .iter()
        .map(|value| normalize_title(value))
        .collect::<Vec<_>>();
    actuals.push(normalize_title(parsed.normalized_title.as_str()));
    actuals.iter().any(|actual| actual == &expected)
}

fn matches_episode(
    expected: Option<&ExpectedEpisode>,
    actual: Option<&scryer_release_parser_v2::ParsedEpisodeMetadataV2>,
) -> bool {
    match (expected, actual) {
        (None, None) => true,
        (Some(expected), Some(actual)) => {
            if expected.season != actual.season {
                return false;
            }
            if !expected.episode_numbers.is_empty()
                && expected.episode_numbers != actual.episode_numbers
                && expected.episode_numbers != actual.absolute_episode_numbers
            {
                return false;
            }
            if !expected.absolute_episode_numbers.is_empty()
                && expected.absolute_episode_numbers != actual.absolute_episode_numbers
                && expected.absolute_episode_numbers != actual.episode_numbers
            {
                return false;
            }
            match (&expected.air_date, actual.air_date) {
                (Some(expected), Some(actual)) => expected == &actual.to_string(),
                (None, None) => true,
                (None, Some(_)) => true,
                (Some(_), None) => false,
            }
        }
        _ => false,
    }
}

fn matches_episode_contract(
    raw_title: &str,
    expected: Option<&ExpectedEpisode>,
    actual: Option<&scryer_release_parser_v2::ParsedEpisodeMetadataV2>,
) -> bool {
    match (expected, actual) {
        (None, None) => true,
        (Some(expected), Some(actual)) => {
            expected.season == actual.season
                && episode_numbers_contract_match(expected, actual)
                && expected.absolute_episode == actual.absolute_episode
                && expected.absolute_episode_numbers == actual.absolute_episode_numbers
                && expected.special_absolute_episode_numbers
                    == actual.special_absolute_episode_numbers
                && expected.air_date.as_deref()
                    == actual.air_date.map(|value| value.to_string()).as_deref()
                && expected.daily_part == actual.daily_part
                && expected.full_season == actual.full_season
                && expected.is_partial_season == actual.is_partial_season
                && expected.is_multi_season == actual.is_multi_season
                && expected.season_part == actual.season_part
                && expected.is_season_extra == actual.is_season_extra
                && split_episode_contract_match(expected, actual)
                && mini_series_contract_match(raw_title, expected, actual)
                && normalize_opt(expected.special_kind.as_deref())
                    == normalize_opt(actual.special_kind.map(special_kind_label).as_deref())
                && normalize_opt(expected.release_type.as_deref())
                    == Some(
                        episode_release_type_label(actual.release_type)
                            .to_string()
                            .to_ascii_lowercase(),
                    )
        }
        _ => false,
    }
}

fn split_episode_contract_match(
    expected: &ExpectedEpisode,
    actual: &scryer_release_parser_v2::ParsedEpisodeMetadataV2,
) -> bool {
    expected.is_split_episode == actual.is_split_episode
        || (!expected.is_split_episode
            && actual.is_split_episode
            && (actual.episode_numbers.len() > 1 || actual.absolute_episode_numbers.len() > 1))
}

fn mini_series_contract_match(
    raw_title: &str,
    expected: &ExpectedEpisode,
    actual: &scryer_release_parser_v2::ParsedEpisodeMetadataV2,
) -> bool {
    expected.is_mini_series == actual.is_mini_series
        || (expected.is_mini_series
            && !actual.is_mini_series
            && !raw_has_trusted_mini_series_evidence(raw_title))
}

fn raw_has_trusted_mini_series_evidence(raw_title: &str) -> bool {
    let tokens = raw_language_tokens(raw_title);
    tokens.windows(2).any(|window| {
        matches!(window[0].as_str(), "PART" | "PT" | "VOL" | "VOLUME")
            && window[1]
                .chars()
                .all(|character| character.is_ascii_digit())
    })
}

fn episode_numbers_contract_match(
    expected: &ExpectedEpisode,
    actual: &scryer_release_parser_v2::ParsedEpisodeMetadataV2,
) -> bool {
    expected.episode_numbers == actual.episode_numbers
        || (!expected.episode_numbers.is_empty()
            && actual.episode_numbers.is_empty()
            && expected.episode_numbers == actual.absolute_episode_numbers
            && expected.episode_numbers == expected.absolute_episode_numbers)
}

fn kind_label(parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2) -> &'static str {
    match parsed.parse_family {
        scryer_release_parser_v2::ParseFamily::Movie => "movie",
        scryer_release_parser_v2::ParseFamily::SeasonPack => "season_pack",
        scryer_release_parser_v2::ParseFamily::EpisodeRangePack => "multi_episode",
        scryer_release_parser_v2::ParseFamily::Special => "episode",
        scryer_release_parser_v2::ParseFamily::DailyEpisode => "episode",
        scryer_release_parser_v2::ParseFamily::StandardEpisode
        | scryer_release_parser_v2::ParseFamily::AnimeAbsolute => {
            if parsed.episode.as_ref().is_some_and(|episode| {
                episode.release_type
                    == scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::RangePack
                    || episode.episode_numbers.len() > 1
                    || episode.absolute_episode_numbers.len() > 1
            }) {
                "multi_episode"
            } else {
                "episode"
            }
        }
        scryer_release_parser_v2::ParseFamily::Unknown => "unknown",
    }
}

fn normalize_title(value: &str) -> String {
    value
        .chars()
        .filter_map(|ch| {
            if ch.is_alphanumeric() {
                Some(ch.to_ascii_uppercase())
            } else if ch.is_whitespace() {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_opt(value: Option<&str>) -> Option<String> {
    value.map(|value| value.trim().to_ascii_lowercase())
}

fn normalize_video_codec(value: Option<&str>) -> Option<String> {
    normalize_opt(value).map(|value| match value.as_str() {
        "hevc" | "h265" | "h.265" => "h.265".to_string(),
        "avc" | "h264" | "h.264" => "h.264".to_string(),
        other => other.to_string(),
    })
}

fn normalize_string_vec(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_language_vec(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| normalize_language_code(value.as_str()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_language_code(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "fre" => "fra".to_string(),
        "ger" => "deu".to_string(),
        "rum" => "ron".to_string(),
        "chi" => "zho".to_string(),
        "cze" => "ces".to_string(),
        other => other.to_string(),
    }
}

fn matches_language_vec_or_improved(
    raw_title: &str,
    expected: &[String],
    actual: &[String],
) -> bool {
    let expected = normalize_language_vec(expected);
    let actual = normalize_language_vec(actual);
    if expected == actual {
        return true;
    }

    let mut evidence_backed_expected = expected
        .iter()
        .filter(|language| raw_has_trusted_language_evidence(raw_title, language))
        .cloned()
        .collect::<Vec<_>>();
    for language in &actual {
        if raw_has_trusted_language_evidence(raw_title, language)
            && !evidence_backed_expected
                .iter()
                .any(|existing| existing == language)
        {
            evidence_backed_expected.push(language.clone());
        }
    }
    evidence_backed_expected.sort();
    evidence_backed_expected.dedup();

    actual == evidence_backed_expected
}

fn raw_has_trusted_language_evidence(raw_title: &str, language: &str) -> bool {
    let tokens = raw_language_tokens(raw_title);
    trusted_language_terms(language).is_some_and(|terms| {
        terms.iter().any(|term| {
            tokens.iter().any(|token| {
                token == term
                    || token.strip_suffix("DUB") == Some(*term)
                    || token.strip_suffix("DUBBED") == Some(*term)
                    || token.strip_suffix("DUBS") == Some(*term)
                    || token.strip_suffix("AUDIO") == Some(*term)
                    || token.strip_prefix("DUB") == Some(*term)
                    || token.strip_prefix("SUB") == Some(*term)
                    || token.strip_prefix("SUBS") == Some(*term)
            })
        })
    })
}

fn raw_language_tokens(raw_title: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in raw_title.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_uppercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn trusted_language_terms(language: &str) -> Option<&'static [&'static str]> {
    match language {
        "ara" => Some(&["AR", "ARA", "ARABIC"]),
        "bul" => Some(&["BG", "BUL", "BULGARIAN"]),
        "cat" => Some(&["CAT", "CATALAN"]),
        "ces" => Some(&["CES", "CZECH"]),
        "dan" => Some(&["DAN", "DANISH"]),
        "deu" => Some(&["DEU", "GER", "GERMAN"]),
        "eng" => Some(&["EN", "ENG", "ENGLISH"]),
        "fin" => Some(&["FIN", "FINNISH"]),
        "fra" => Some(&[
            "FR",
            "FRA",
            "FRE",
            "FRENCH",
            "TRUEFRENCH",
            "VF",
            "VFF",
            "VFQ",
            "VOSTFR",
        ]),
        "heb" => Some(&["HEB", "HEBREW"]),
        "hin" => Some(&["HIN", "HINDI"]),
        "hun" => Some(&["HUN", "HUNGARIAN"]),
        "isl" => Some(&["ISL", "ICELANDIC"]),
        "ita" => Some(&["ITA", "ITALIAN"]),
        "jpn" => Some(&["JP", "JPN", "JAP", "JAPANESE"]),
        "kat" => Some(&["KAT", "GEORGIAN"]),
        "kor" => Some(&["KOR", "KOREAN", "KORSUB", "KORSUBS"]),
        "lav" => Some(&["LAV", "LATVIAN"]),
        "lit" => Some(&["LIT", "LITHUANIAN"]),
        "nld" => Some(&["NLD", "DUTCH"]),
        "nor" => Some(&["NOR", "NORWEGIAN"]),
        "pol" => Some(&["POL", "POLISH"]),
        "por" => Some(&["POR", "PORTUGUESE", "PTBR"]),
        "ron" => Some(&["RON", "RUM", "ROMANIAN"]),
        "rus" => Some(&["RUS", "RUSSIAN"]),
        "spa" => Some(&["SPA", "ESP", "SPANISH"]),
        "swe" => Some(&["SWE", "SWEDISH"]),
        "tha" => Some(&["THA", "THAI"]),
        "tur" => Some(&["TR", "TUR", "TURKISH"]),
        "zho" => Some(&["ZHO", "CHINESE", "CHS", "CHT"]),
        _ => None,
    }
}

fn missing_fields_match_or_improved(
    label: &ExpectedLabel,
    parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2,
) -> bool {
    let expected_missing = normalize_string_vec(label.missing_fields.as_slice());
    let actual_missing = normalize_string_vec(parsed.missing_fields.as_slice());

    actual_missing
        .iter()
        .all(|field| expected_missing.contains(field))
}

fn matches_normalized_optional_or_improved(
    expected: Option<String>,
    actual: Option<String>,
    label: &ExpectedLabel,
    missing_field: &str,
) -> bool {
    if expected == actual {
        return true;
    }

    expected.is_none() && actual.is_some() && label_missing_field(label, missing_field)
}

fn matches_normalized_vec_or_improved(
    expected: &[String],
    actual: &[String],
    label: &ExpectedLabel,
    missing_field: &str,
) -> bool {
    let expected = normalize_string_vec(expected);
    let actual = normalize_string_vec(actual);
    if expected == actual {
        return true;
    }

    expected.is_empty() && !actual.is_empty() && label_missing_field(label, missing_field)
}

fn label_missing_field(label: &ExpectedLabel, field: &str) -> bool {
    label
        .missing_fields
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case(field))
}

fn matches_fps(expected: Option<f32>, actual: Option<f32>) -> bool {
    match (expected, actual) {
        (None, None) => true,
        (Some(expected), Some(actual)) => (expected - actual).abs() < 0.05,
        _ => false,
    }
}

fn matches_video_encoding_or_improved(
    raw_title: &str,
    expected: Option<&str>,
    actual: Option<&str>,
    label: &ExpectedLabel,
) -> bool {
    let expected = normalize_opt(expected);
    let actual = normalize_opt(actual);
    if matches_normalized_optional_or_improved(
        expected.clone(),
        actual.clone(),
        label,
        "video_encoding",
    ) {
        return true;
    }

    expected.is_none()
        && actual
            .as_deref()
            .is_some_and(|encoding| raw_has_trusted_video_encoding_evidence(raw_title, encoding))
}

fn raw_has_trusted_video_encoding_evidence(raw_title: &str, encoding: &str) -> bool {
    let tokens = raw_language_tokens(raw_title);
    match encoding {
        "x264" => has_any_token(&tokens, &["X264"]) || has_token_pair(&tokens, "X", "264"),
        "x265" => has_any_token(&tokens, &["X265"]) || has_token_pair(&tokens, "X", "265"),
        other => tokens.iter().any(|token| token.eq_ignore_ascii_case(other)),
    }
}

fn matches_streaming_service_or_improved(
    raw_title: &str,
    expected: Option<&str>,
    actual: Option<&str>,
) -> bool {
    let expected = normalize_opt(expected);
    let actual = normalize_opt(actual);
    if expected == actual {
        return true;
    }

    expected.is_none()
        && actual
            .as_deref()
            .is_some_and(|service| raw_has_trusted_service_evidence(raw_title, service))
}

fn raw_has_trusted_service_evidence(raw_title: &str, service: &str) -> bool {
    let tokens = raw_language_tokens(raw_title);
    let terms = match service {
        "amazon" => &["AMZN", "AMAZON"][..],
        "crunchyroll" => &["CR", "CRUNCHYROLL"][..],
        "disney+" => &["DSNP", "DNSP", "DISNEY"][..],
        "hbo max" => &["MAX", "HMAX", "HBO"][..],
        "hulu" => &["HULU"][..],
        "netflix" => &["NF", "NETFLIX"][..],
        "apple tv+" => &["ATVP", "APTV", "APPLE"][..],
        "paramount+" => &["PMTP", "PARAMOUNT"][..],
        "peacock" => &["PCOK", "PEACOCK"][..],
        "hidive" => &["HIDIVE", "HIDI"][..],
        other => return tokens.iter().any(|token| token.eq_ignore_ascii_case(other)),
    };
    has_any_token(&tokens, terms)
}

fn actual_flags(parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2) -> ExpectedFlags {
    ExpectedFlags {
        dual_audio: parsed.is_dual_audio,
        atmos: parsed.is_atmos,
        dolby_vision: parsed.is_dolby_vision,
        hdr: parsed.detected_hdr,
        hdr_fallback: parsed.has_hdr_fallback,
        hdr10plus: parsed.is_hdr10plus,
        hlg: parsed.is_hlg,
        ten_bit: parsed.is_10bit,
        proper: parsed.is_proper_upload,
        repack: parsed.is_repack,
        remux: parsed.is_remux,
        bd_disk: parsed.is_bd_disk,
        ai_enhanced: parsed.is_ai_enhanced,
        hardcoded_subs: parsed.is_hardcoded_subs,
        uncensored: parsed.is_uncensored,
        dubs_only: parsed.is_dubs_only,
    }
}

fn flags_match_or_improved(
    raw_title: &str,
    expected: &ExpectedFlags,
    actual: &ExpectedFlags,
) -> bool {
    flag_match_or_improved(
        raw_title,
        expected.dual_audio,
        actual.dual_audio,
        "dual_audio",
    ) && flag_match_or_improved(raw_title, expected.atmos, actual.atmos, "atmos")
        && flag_match_or_improved(
            raw_title,
            expected.dolby_vision,
            actual.dolby_vision,
            "dolby_vision",
        )
        && flag_match_or_improved(raw_title, expected.hdr, actual.hdr, "hdr")
        && flag_match_or_improved(
            raw_title,
            expected.hdr_fallback,
            actual.hdr_fallback,
            "hdr_fallback",
        )
        && flag_match_or_improved(raw_title, expected.hdr10plus, actual.hdr10plus, "hdr10plus")
        && flag_match_or_improved(raw_title, expected.hlg, actual.hlg, "hlg")
        && flag_match_or_improved(raw_title, expected.ten_bit, actual.ten_bit, "ten_bit")
        && flag_match_or_improved(raw_title, expected.proper, actual.proper, "proper")
        && flag_match_or_improved(raw_title, expected.repack, actual.repack, "repack")
        && flag_match_or_improved(raw_title, expected.remux, actual.remux, "remux")
        && flag_match_or_improved(raw_title, expected.bd_disk, actual.bd_disk, "bd_disk")
        && flag_match_or_improved(
            raw_title,
            expected.ai_enhanced,
            actual.ai_enhanced,
            "ai_enhanced",
        )
        && flag_match_or_improved(
            raw_title,
            expected.hardcoded_subs,
            actual.hardcoded_subs,
            "hardcoded_subs",
        )
        && flag_match_or_improved(
            raw_title,
            expected.uncensored,
            actual.uncensored,
            "uncensored",
        )
        && flag_match_or_improved(raw_title, expected.dubs_only, actual.dubs_only, "dubs_only")
}

fn flag_match_or_improved(raw_title: &str, expected: bool, actual: bool, flag: &str) -> bool {
    expected == actual || (!expected && actual && raw_has_trusted_flag_evidence(raw_title, flag))
}

fn raw_has_trusted_flag_evidence(raw_title: &str, flag: &str) -> bool {
    let tokens = raw_language_tokens(raw_title);
    match flag {
        "dual_audio" => has_any_token(&tokens, &["DUAL", "DUALAUDIO", "MULTIAUDIO"]),
        "atmos" => has_any_token(&tokens, &["ATMOS", "ATMOSPHERE"]),
        "dolby_vision" => {
            has_any_token(&tokens, &["DV", "DOVI"]) || has_token_pair(&tokens, "DOLBY", "VISION")
        }
        "hdr" | "hdr_fallback" => has_any_token(
            &tokens,
            &["DV", "DOVI", "HDR", "HDR10", "HDR10PLUS", "HDR10P", "HLG"],
        ),
        "hdr10plus" => has_any_token(&tokens, &["HDR10PLUS", "HDR10P", "HDR10"]),
        "hlg" => has_any_token(&tokens, &["HLG"]),
        "ten_bit" => tokens.iter().any(|token| {
            matches!(token.as_str(), "10BIT" | "10BITS" | "HI10" | "HI10P")
                || token.ends_with("10BIT")
                || token.ends_with("10BITS")
        }),
        "proper" => has_any_token(&tokens, &["PROPER"]),
        "repack" => has_any_token(&tokens, &["REPACK"]),
        "remux" => has_any_token(&tokens, &["REMUX"]),
        "bd_disk" => has_any_token(
            &tokens,
            &["BDISO", "BDMV", "BD25", "BD50", "BD66", "BD100", "BRDISK"],
        ),
        "ai_enhanced" => {
            has_any_token(&tokens, &["AIENHANCED", "RIFE"])
                || has_token_pair(&tokens, "AI", "ENHANCED")
        }
        "hardcoded_subs" => has_any_token(
            &tokens,
            &["HC", "HARDCODED", "HARDSUB", "HARDSUBBED", "KORSUB"],
        ),
        "uncensored" => has_any_token(&tokens, &["UNCENSORED", "UNCUT"]),
        "dubs_only" => {
            has_any_token(&tokens, &["DUB", "DUBBED", "DUBS"])
                || has_token_pair(&tokens, "ENGLISH", "DUB")
        }
        _ => false,
    }
}

fn has_any_token(tokens: &[String], needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| tokens.iter().any(|token| token == needle))
}

fn has_token_pair(tokens: &[String], first: &str, second: &str) -> bool {
    tokens
        .windows(2)
        .any(|window| window[0] == first && window[1] == second)
}

fn metadata_snapshot_from_expected(label: &ExpectedLabel) -> MetadataSnapshot {
    MetadataSnapshot {
        quality: label.quality.clone(),
        source: label.source.clone(),
        video_codec: label.video_codec.clone(),
        video_encoding: label.video_encoding.clone(),
        audio: label.audio.clone(),
        audio_codecs: label.audio_codecs.clone(),
        audio_channels: label.audio_channels.clone(),
        release_group: label.release_group.clone(),
        languages_audio: label.languages_audio.clone(),
        languages_subtitles: label.languages_subtitles.clone(),
        streaming_service: label.streaming_service.clone(),
        edition: label.edition.clone(),
        anime_version: label.anime_version,
        fps: label.fps,
        missing_fields: label.missing_fields.clone(),
        flags: label.flags.clone(),
    }
}

fn metadata_snapshot_from_actual(
    parsed: &scryer_release_parser_v2::ParsedReleaseMetadataV2,
) -> MetadataSnapshot {
    MetadataSnapshot {
        quality: parsed.quality.clone(),
        source: parsed.source.clone(),
        video_codec: parsed.video_codec.clone(),
        video_encoding: parsed.video_encoding.clone(),
        audio: parsed.audio.clone(),
        audio_codecs: parsed.audio_codecs.clone(),
        audio_channels: parsed.audio_channels.clone(),
        release_group: parsed.release_group.clone(),
        languages_audio: parsed.languages_audio.clone(),
        languages_subtitles: parsed.languages_subtitles.clone(),
        streaming_service: parsed.streaming_service.clone(),
        edition: parsed.edition.clone(),
        anime_version: parsed.anime_version,
        fps: parsed.fps,
        missing_fields: parsed.missing_fields.clone(),
        flags: actual_flags(parsed),
    }
}

fn push_field_mismatch(field_mismatches: &mut Vec<String>, field: &str, matched: bool) {
    if !matched {
        field_mismatches.push(field.to_string());
    }
}

fn episode_release_type_label(
    value: scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2,
) -> &'static str {
    match value {
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::SingleEpisode => "single_episode",
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::MultiEpisode => "multi_episode",
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::SeasonPack => "season_pack",
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::RangePack => "multi_episode",
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::Daily => "single_episode",
        scryer_release_parser_v2::ParsedEpisodeReleaseTypeV2::Unknown => "unknown",
    }
}

fn special_kind_label(value: scryer_release_parser_v2::ParsedSpecialKindV2) -> String {
    match value {
        scryer_release_parser_v2::ParsedSpecialKindV2::Special => "special".to_string(),
        scryer_release_parser_v2::ParsedSpecialKindV2::Ova => "ova".to_string(),
        scryer_release_parser_v2::ParsedSpecialKindV2::Oad => "oad".to_string(),
        scryer_release_parser_v2::ParsedSpecialKindV2::Ncop => "ncop".to_string(),
        scryer_release_parser_v2::ParsedSpecialKindV2::Nced => "nced".to_string(),
        scryer_release_parser_v2::ParsedSpecialKindV2::Extra => "extra".to_string(),
    }
}
