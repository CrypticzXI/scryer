use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use quick_xml::de::from_str as from_xml_str;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{ReleaseParserCorpusArgs, TaskContext, ok, step, warn};
use scryer_release_parser::{
    ContextFacetHint, ContextTitle, ParsedEpisodeMetadata, ParsedEpisodeReleaseType,
    ParsedReleaseMetadata, ParsedSpecialKind, ReleaseParseContext, best_parse_for_target,
};

const ANIMETOSHO_BASE_URL: &str = "https://feed.animetosho.org";
const ANIMETOSHO_PAGE_SIZE: usize = 75;
const ANIMETOSHO_QUERY_PAGE_LIMIT: usize = 4;
const NZBGEEK_BASE_URL: &str = "https://api.nzbgeek.info";
const NZBGEEK_PAGE_SIZE: usize = 100;
const RELEASE_PARSER_SYSTEM_PROMPT: &str = "You are a release-title parser for Scryer. Parse the user-supplied release title and return JSON only with these keys: facet_hint, kind, title, title_variants, year, quality, source, video_codec, video_encoding, audio, audio_codecs, audio_channels, release_group, languages_audio, languages_subtitles, streaming_service, edition, anime_version, episode, flags. Use null for unknown scalar values, [] for unknown arrays, and do not add extra keys.";
const ANIMETOSHO_ANIME_DIVISOR: usize = 3;
const ANIMETOSHO_ANIME_MAX: usize = 150;
const ANIMETOSHO_ANIME_MIN: usize = 100;
const FACET_SEASON_PACK_DIVISOR: usize = 14;
const FACET_SEASON_PACK_MAX: usize = 32;
const FACET_SEASON_PACK_MIN: usize = 18;
const SERIES_DAILY_DIVISOR: usize = 12;
const SERIES_DAILY_MAX: usize = 36;
const SERIES_DAILY_MIN: usize = 18;
const ABSOLUTE_EPISODE_QUERY_TERMS: &[&str] = &[
    "1", "12", "24", "48", "96", "144", "192", "240", "1-12", "13-26", "27-52",
];
const SEASON_EPISODE_QUERY_TERMS: &[&str] = &[
    "S01E01",
    "S01E06",
    "S01E12",
    "S01E24",
    "S02E01",
    "S02E06",
    "S02E12",
    "S01E01-S01E06",
    "S01E01-S01E12",
    "S01E13-S01E24",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum CorpusFacet {
    Movie,
    Series,
    Anime,
}

impl CorpusFacet {
    fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum CorpusSource {
    NzbGeek,
    AnimeTosho,
}

impl CorpusSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::NzbGeek => "nzbgeek",
            Self::AnimeTosho => "animetosho",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum ComplexityBucket {
    Simple,
    Standard,
    Complex,
}

impl ComplexityBucket {
    fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Standard => "standard",
            Self::Complex => "complex",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReleaseKind {
    Movie,
    Episode,
    MultiEpisode,
    SeasonPack,
    Special,
}

#[derive(Debug, Clone)]
struct HarvestedRelease {
    source: CorpusSource,
    facet: CorpusFacet,
    raw_title: String,
    published_at: Option<String>,
    published_ts: Option<i64>,
    size_bytes: Option<i64>,
    link: Option<String>,
    download_url: Option<String>,
    category_hint: Option<String>,
}

#[derive(Debug, Clone)]
struct EvaluatedCandidate {
    source: CorpusSource,
    facet: CorpusFacet,
    raw_title: String,
    raw_key: String,
    title_key: String,
    published_at: Option<String>,
    published_ts: Option<i64>,
    size_bytes: Option<i64>,
    link: Option<String>,
    download_url: Option<String>,
    category_hint: Option<String>,
    daily_series: bool,
    complexity: ComplexityBucket,
    field_density: usize,
    parse_confidence_score: i32,
    parser_snapshot: ParserSnapshot,
    label: TrainingLabel,
    review_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ParserSnapshot {
    normalized_title: String,
    normalized_title_variants: Vec<String>,
    release_group: Option<String>,
    languages_audio: Vec<String>,
    languages_subtitles: Vec<String>,
    imdb_id: Option<String>,
    tmdb_id: Option<String>,
    year: Option<i32>,
    quality: Option<String>,
    source: Option<String>,
    video_codec: Option<String>,
    video_encoding: Option<String>,
    audio: Option<String>,
    audio_codecs: Vec<String>,
    audio_channels: Option<String>,
    is_dual_audio: bool,
    is_atmos: bool,
    is_dolby_vision: bool,
    detected_hdr: bool,
    has_hdr_fallback: bool,
    is_hdr10plus: bool,
    is_hlg: bool,
    is_10bit: bool,
    is_proper_upload: bool,
    is_repack: bool,
    is_remux: bool,
    is_bd_disk: bool,
    is_ai_enhanced: bool,
    is_hardcoded_subs: bool,
    is_uncensored: bool,
    is_dubs_only: bool,
    streaming_service: Option<String>,
    edition: Option<String>,
    anime_version: Option<u32>,
    episode: Option<EpisodeLabel>,
    parser_version: &'static str,
    parse_confidence: f32,
    missing_fields: Vec<String>,
    parse_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TrainingLabel {
    facet_hint: CorpusFacet,
    kind: ReleaseKind,
    title: String,
    title_variants: Vec<String>,
    year: Option<i32>,
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
    episode: Option<EpisodeLabel>,
    flags: FlagsLabel,
    fps: Option<f32>,
    missing_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FlagsLabel {
    dual_audio: bool,
    atmos: bool,
    dolby_vision: bool,
    hdr: bool,
    hdr_fallback: bool,
    hdr10plus: bool,
    hlg: bool,
    ten_bit: bool,
    proper: bool,
    repack: bool,
    remux: bool,
    bd_disk: bool,
    ai_enhanced: bool,
    hardcoded_subs: bool,
    uncensored: bool,
    dubs_only: bool,
}

#[derive(Debug, Clone, Serialize)]
struct EpisodeLabel {
    season: Option<u32>,
    episode_numbers: Vec<u32>,
    absolute_episode: Option<u32>,
    air_date: Option<String>,
    daily_part: Option<u32>,
    absolute_episode_numbers: Vec<u32>,
    special_absolute_episode_numbers: Vec<u32>,
    full_season: bool,
    partial_season: bool,
    multi_season: bool,
    season_part: Option<u32>,
    season_extra: bool,
    split_episode: bool,
    mini_series: bool,
    special_kind: Option<String>,
    release_type: String,
    raw: Option<String>,
}

#[derive(Debug, Serialize)]
struct CorpusSample {
    source: CorpusSource,
    facet: CorpusFacet,
    complexity: ComplexityBucket,
    raw_title: String,
    title_key: String,
    published_at: Option<String>,
    size_bytes: Option<i64>,
    link: Option<String>,
    download_url: Option<String>,
    category_hint: Option<String>,
    daily_series: bool,
    parser_first_pass: ParserSnapshot,
    label: TrainingLabel,
    review_notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReviewSample {
    source: CorpusSource,
    facet: CorpusFacet,
    complexity: ComplexityBucket,
    raw_title: String,
    title_key: String,
    review_notes: Vec<String>,
    parser_first_pass: ParserSnapshot,
    label: TrainingLabel,
}

#[derive(Debug, Serialize)]
struct OumiRow {
    messages: Vec<OumiMessage>,
}

#[derive(Debug, Serialize)]
struct OumiMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct CorpusSummary {
    generated_at: String,
    requested_total: usize,
    actual_total: usize,
    daily_series_target: usize,
    daily_series_actual: usize,
    pack_series_target: usize,
    pack_series_actual: usize,
    pack_anime_target: usize,
    pack_anime_actual: usize,
    counts_by_facet: BTreeMap<String, usize>,
    counts_by_source: BTreeMap<String, usize>,
    counts_by_complexity: BTreeMap<String, usize>,
    unique_titles_by_facet: BTreeMap<String, usize>,
    review_row_count: usize,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct NewznabFeed {
    channel: NewznabChannel,
}

#[derive(Debug, Deserialize)]
struct NewznabChannel {
    #[serde(rename = "item", default)]
    items: Vec<NewznabItem>,
}

#[derive(Debug, Deserialize)]
struct NewznabItem {
    title: Option<String>,
    link: Option<String>,
    comments: Option<String>,
    #[serde(rename = "pubDate")]
    pub_date: Option<String>,
    category: Option<String>,
    description: Option<String>,
    enclosure: Option<NewznabEnclosure>,
    #[serde(rename = "newznab:attr", default)]
    attrs: Vec<NewznabAttr>,
}

#[derive(Debug, Deserialize)]
struct NewznabEnclosure {
    #[serde(rename = "@url")]
    url: Option<String>,
    #[serde(rename = "@length")]
    length: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NewznabAttr {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@value")]
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AnimeToshoItem {
    title: Option<String>,
    link: Option<String>,
    timestamp: Option<i64>,
    torrent_url: Option<String>,
    nzb_url: Option<String>,
    total_size: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AniDbQueryStyle {
    Absolute,
    SeasonEpisode,
}

struct AniDbSeed {
    title: &'static str,
    anidb_aid: i64,
    style: AniDbQueryStyle,
}

const ANIMETOSHO_AID_SEEDS: &[AniDbSeed] = &[
    AniDbSeed {
        title: "Bleach",
        anidb_aid: 2369,
        style: AniDbQueryStyle::Absolute,
    },
    AniDbSeed {
        title: "Frieren",
        anidb_aid: 18886,
        style: AniDbQueryStyle::SeasonEpisode,
    },
];

pub(crate) fn run_release_parser(ctx: &TaskContext, args: ReleaseParserCorpusArgs) -> Result<()> {
    if args.total < 12 {
        bail!("release-parser corpus total must be at least 12 samples");
    }

    load_local_env()?;
    let output_dir = resolve_output_dir(ctx, args.output_dir.as_deref());
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let api_key = env::var("NZBGEEK_API_KEY")
        .context("NZBGEEK_API_KEY is required; source ~/.env or export it first")?;
    let client = Client::builder()
        .user_agent("scryer-xtask/release-parser-corpus")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client")?;

    step("Harvesting NZBGeek movie recents");
    let movie_candidates = fetch_nzbgeek_recent(
        &client,
        &api_key,
        "2000",
        CorpusFacet::Movie,
        args.nzbgeek_movie_pages,
    )?;
    ok(format!(
        "harvested {} raw NZBGeek movie candidates",
        movie_candidates.len()
    ));

    step("Harvesting NZBGeek series recents");
    let series_candidates = fetch_nzbgeek_recent(
        &client,
        &api_key,
        "5000",
        CorpusFacet::Series,
        args.nzbgeek_series_pages,
    )?;
    ok(format!(
        "harvested {} raw NZBGeek series candidates",
        series_candidates.len()
    ));

    step("Harvesting AnimeTosho anime recents");
    let animetosho_anime_candidates = fetch_animetosho_recent(&client, args.animetosho_pages)?;
    ok(format!(
        "harvested {} raw AnimeTosho anime candidates",
        animetosho_anime_candidates.len()
    ));

    step("Harvesting AnimeTosho AniDB seed searches");
    let animetosho_seeded_candidates = fetch_animetosho_seeded_anime(&client)?;
    ok(format!(
        "harvested {} raw AnimeTosho AniDB-seeded candidates",
        animetosho_seeded_candidates.len()
    ));

    step("Harvesting NZBGeek anime recents");
    let nzbgeek_anime_candidates = fetch_nzbgeek_recent(
        &client,
        &api_key,
        "5070",
        CorpusFacet::Anime,
        args.nzbgeek_anime_pages,
    )?;
    ok(format!(
        "harvested {} raw NZBGeek anime candidates",
        nzbgeek_anime_candidates.len()
    ));

    step("Parsing and ranking release titles");
    let all_candidates = movie_candidates
        .into_iter()
        .chain(series_candidates)
        .chain(animetosho_anime_candidates)
        .chain(animetosho_seeded_candidates)
        .chain(nzbgeek_anime_candidates)
        .collect::<Vec<_>>();
    let evaluated = evaluate_candidates(all_candidates);
    ok(format!(
        "retained {} viable parsed candidates",
        evaluated.len()
    ));

    step("Selecting a balanced 1000-sample corpus");
    let selected = select_corpus(&evaluated, args.total, args.max_per_title);
    if selected.len() != args.total {
        warn(format!(
            "requested {} samples but only selected {}",
            args.total,
            selected.len()
        ));
    }
    ok(format!(
        "selected {} releases across movies, series, anime, and daily shows",
        selected.len()
    ));

    step("Writing corpus artifacts");
    let files = write_corpus_files(&output_dir, &selected)?;
    let summary = build_summary(&selected, &files, args.total);
    let summary_path = output_dir.join("summary.json");
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(&summary).context("failed to serialize summary")?,
    )
    .with_context(|| format!("failed to write {}", summary_path.display()))?;
    ok(format!(
        "wrote corpus artifacts under {}",
        output_dir.display()
    ));

    Ok(())
}

fn load_local_env() -> Result<()> {
    let Some(home) = env::var_os("HOME") else {
        return Ok(());
    };
    let path = PathBuf::from(home).join(".env");
    if !path.is_file() {
        return Ok(());
    }

    dotenvy::from_path_override(&path)
        .with_context(|| format!("failed to load {}", path.display()))?;
    Ok(())
}

fn resolve_output_dir(ctx: &TaskContext, output_dir: Option<&Path>) -> PathBuf {
    match output_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => ctx.repo_root.join(path),
        None => ctx
            .repo_root
            .join("tmp")
            .join("release-parser-corpus")
            .join(Utc::now().format("%Y%m%d-%H%M%S").to_string()),
    }
}

fn fetch_nzbgeek_recent(
    client: &Client,
    api_key: &str,
    category: &str,
    facet: CorpusFacet,
    pages: usize,
) -> Result<Vec<HarvestedRelease>> {
    let mut harvested = Vec::new();

    for page in 0..pages {
        let offset = page * NZBGEEK_PAGE_SIZE;
        let endpoint = format!(
            "{NZBGEEK_BASE_URL}/api?t=search&o=xml&cat={category}&extended=1&limit={NZBGEEK_PAGE_SIZE}&offset={offset}&apikey={api_key}"
        );
        let response = client
            .get(&endpoint)
            .send()
            .with_context(|| {
                format!(
                    "NZBGeek request failed for {} page {}",
                    facet.as_str(),
                    page + 1
                )
            })?
            .error_for_status()
            .with_context(|| {
                format!(
                    "NZBGeek returned an error for {} page {}",
                    facet.as_str(),
                    page + 1
                )
            })?
            .text()
            .context("failed to read NZBGeek response body")?;

        let feed: NewznabFeed = from_xml_str(&response).with_context(|| {
            format!(
                "failed to decode NZBGeek XML for {} page {}",
                facet.as_str(),
                page + 1
            )
        })?;

        if feed.channel.items.is_empty() {
            break;
        }

        let page_items = feed
            .channel
            .items
            .into_iter()
            .filter_map(|item| newznab_item_to_candidate(item, facet))
            .collect::<Vec<_>>();

        if page_items.is_empty() {
            break;
        }

        harvested.extend(page_items);
    }

    Ok(harvested)
}

fn fetch_animetosho_recent(client: &Client, pages: usize) -> Result<Vec<HarvestedRelease>> {
    let mut harvested = Vec::new();

    for page in 0..pages {
        let start = page * ANIMETOSHO_PAGE_SIZE;
        let endpoint = format!("{ANIMETOSHO_BASE_URL}/json?start={start}");
        let items = client
            .get(&endpoint)
            .send()
            .with_context(|| format!("AnimeTosho request failed for page {}", page + 1))?
            .error_for_status()
            .with_context(|| format!("AnimeTosho returned an error for page {}", page + 1))?
            .json::<Vec<AnimeToshoItem>>()
            .context("failed to decode AnimeTosho JSON")?;

        if items.is_empty() {
            break;
        }

        harvested.extend(
            items
                .into_iter()
                .filter_map(animetosho_item_to_candidate)
                .collect::<Vec<_>>(),
        );
    }

    Ok(harvested)
}

fn fetch_animetosho_seeded_anime(client: &Client) -> Result<Vec<HarvestedRelease>> {
    let mut harvested = Vec::new();

    for seed in ANIMETOSHO_AID_SEEDS {
        let mut seed_harvested = 0;
        for query in query_terms_for_style(seed.style) {
            let params = format!("aid={}&q={}", seed.anidb_aid, url_encode(query));
            let items = match fetch_animetosho_query(client, &params) {
                Ok(items) => items,
                Err(error) => {
                    warn(format!(
                        "AnimeTosho AniDB request failed for '{}' ({}) query '{}': {error:#}",
                        seed.title, seed.anidb_aid, query
                    ));
                    continue;
                }
            };

            seed_harvested += items.len();
            harvested.extend(items.into_iter().filter_map(animetosho_item_to_candidate));
        }

        if seed_harvested == 0 {
            warn(format!(
                "AnimeTosho AniDB seed '{}' ({}) returned no results",
                seed.title, seed.anidb_aid
            ));
        }
    }

    Ok(harvested)
}

fn fetch_animetosho_query(client: &Client, params: &str) -> Result<Vec<AnimeToshoItem>> {
    let mut harvested = Vec::new();

    for page in 1..=ANIMETOSHO_QUERY_PAGE_LIMIT {
        let endpoint = format!("{ANIMETOSHO_BASE_URL}/json?{params}&page={page}");
        let items = client
            .get(&endpoint)
            .send()
            .with_context(|| {
                format!("AnimeTosho request failed for params '{params}' page {page}")
            })?
            .error_for_status()
            .with_context(|| {
                format!("AnimeTosho returned an error for params '{params}' page {page}")
            })?
            .json::<Vec<AnimeToshoItem>>()
            .context("failed to decode AnimeTosho JSON")?;

        let page_count = items.len();
        if page_count == 0 {
            break;
        }

        harvested.extend(items);

        if page_count < ANIMETOSHO_PAGE_SIZE {
            break;
        }
    }

    Ok(harvested)
}

fn newznab_item_to_candidate(item: NewznabItem, facet: CorpusFacet) -> Option<HarvestedRelease> {
    let raw_title = item
        .title
        .or(item.description)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;

    let size_bytes = first_attr(&item.attrs, "size")
        .and_then(|value| value.parse::<i64>().ok())
        .or_else(|| {
            item.enclosure
                .as_ref()
                .and_then(|enclosure| enclosure.length.as_deref())
                .and_then(|value| value.parse::<i64>().ok())
        });

    let item_link = item.link.clone();
    Some(HarvestedRelease {
        source: CorpusSource::NzbGeek,
        facet,
        raw_title,
        published_at: item.pub_date.clone(),
        published_ts: item
            .pub_date
            .as_deref()
            .and_then(parse_rfc2822_to_unix_timestamp),
        size_bytes,
        link: item
            .comments
            .or(item_link.clone())
            .map(|url| redact_api_key(&url)),
        download_url: item
            .enclosure
            .and_then(|enclosure| enclosure.url)
            .or(item_link)
            .map(|url| redact_api_key(&url)),
        category_hint: item.category,
    })
}

fn animetosho_item_to_candidate(item: AnimeToshoItem) -> Option<HarvestedRelease> {
    let raw_title = item
        .title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;

    let published_at = item
        .timestamp
        .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
        .map(|dt| dt.to_rfc3339());

    Some(HarvestedRelease {
        source: CorpusSource::AnimeTosho,
        facet: CorpusFacet::Anime,
        raw_title,
        published_at,
        published_ts: item.timestamp,
        size_bytes: item.total_size,
        link: item.link,
        download_url: item.nzb_url.or(item.torrent_url),
        category_hint: Some("Anime".to_string()),
    })
}

fn evaluate_candidates(candidates: Vec<HarvestedRelease>) -> Vec<EvaluatedCandidate> {
    let mut seen_raw = HashSet::new();
    let mut evaluated = Vec::new();

    for harvested in candidates {
        let raw_title = harvested.raw_title.trim();
        if !is_candidate_title_worth_parsing(raw_title) {
            continue;
        }

        let raw_key = raw_title.to_ascii_lowercase();
        if !seen_raw.insert(raw_key.clone()) {
            continue;
        }

        let parsed = parse_for_corpus(raw_title, Some(harvested.facet));
        let label = build_training_label(&parsed, harvested.facet);
        if label.title.trim().is_empty() {
            continue;
        }

        let parser_snapshot = build_parser_snapshot(&parsed);
        let complexity = classify_complexity(&parsed);
        let title_key = canonical_title_key(&parsed, raw_title);
        let field_density = count_populated_fields(&parsed);
        let parse_confidence_score = (parsed.parse_confidence * 1000.0).round() as i32;
        let daily_series = harvested.facet == CorpusFacet::Series
            && parsed
                .episode
                .as_ref()
                .is_some_and(|episode| episode.air_date.is_some());
        let review_notes = build_review_notes(&parsed, complexity, daily_series);

        evaluated.push(EvaluatedCandidate {
            source: harvested.source,
            facet: harvested.facet,
            raw_title: raw_title.to_string(),
            raw_key,
            title_key,
            published_at: harvested.published_at,
            published_ts: harvested.published_ts,
            size_bytes: harvested.size_bytes,
            link: harvested.link,
            download_url: harvested.download_url,
            category_hint: harvested.category_hint,
            daily_series,
            complexity,
            field_density,
            parse_confidence_score,
            parser_snapshot,
            label,
            review_notes,
        });
    }

    evaluated
}

fn select_corpus(
    candidates: &[EvaluatedCandidate],
    total: usize,
    max_per_title: usize,
) -> Vec<CorpusSample> {
    let targets = balanced_facet_targets(total);
    let daily_target = daily_series_target(targets.series);
    let mut selected = Vec::new();
    let mut selected_raw = HashSet::new();
    let mut title_counts = HashMap::new();

    let movie_pool = candidates_for_facet(candidates, CorpusFacet::Movie);
    selected.extend(select_pool(
        &movie_pool,
        targets.movie,
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let series_pool = candidates_for_facet(candidates, CorpusFacet::Series);
    let daily_pool = series_pool
        .iter()
        .copied()
        .filter(|candidate| candidate.daily_series)
        .collect::<Vec<_>>();
    selected.extend(select_pool(
        &daily_pool,
        daily_target.min(targets.series),
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let series_pack_target = season_pack_target(targets.series);
    let series_pack_pool = series_pool
        .iter()
        .copied()
        .filter(|candidate| is_pack_kind(candidate.label.kind))
        .filter(|candidate| !selected_raw.contains(&candidate.raw_key))
        .collect::<Vec<_>>();
    selected.extend(select_pool(
        &series_pack_pool,
        series_pack_target.min(targets.series),
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let remaining_series_pool = series_pool
        .iter()
        .copied()
        .filter(|candidate| !selected_raw.contains(&candidate.raw_key))
        .collect::<Vec<_>>();
    let remaining_series_target = targets.series.saturating_sub(
        selected
            .iter()
            .filter(|candidate| candidate.facet == CorpusFacet::Series)
            .count(),
    );
    selected.extend(select_pool(
        &remaining_series_pool,
        remaining_series_target,
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let anime_pool = candidates_for_facet(candidates, CorpusFacet::Anime);
    let anime_pack_target = season_pack_target(targets.anime);
    let anime_pack_pool = anime_pool
        .iter()
        .copied()
        .filter(|candidate| is_pack_kind(candidate.label.kind))
        .collect::<Vec<_>>();
    selected.extend(select_pool(
        &anime_pack_pool,
        anime_pack_target.min(targets.anime),
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let animetosho_anime_pool = anime_pool
        .iter()
        .copied()
        .filter(|candidate| candidate.source == CorpusSource::AnimeTosho)
        .filter(|candidate| !selected_raw.contains(&candidate.raw_key))
        .collect::<Vec<_>>();
    let animetosho_target = animetosho_anime_target(targets.anime).min(targets.anime);
    selected.extend(select_pool(
        &animetosho_anime_pool,
        animetosho_target,
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    let remaining_anime_pool = anime_pool
        .iter()
        .copied()
        .filter(|candidate| !selected_raw.contains(&candidate.raw_key))
        .collect::<Vec<_>>();
    let remaining_anime_target = targets.anime.saturating_sub(
        selected
            .iter()
            .filter(|candidate| candidate.facet == CorpusFacet::Anime)
            .count(),
    );
    selected.extend(select_pool(
        &remaining_anime_pool,
        remaining_anime_target,
        max_per_title,
        &mut selected_raw,
        &mut title_counts,
    ));

    if selected.len() < total {
        let remaining = candidates
            .iter()
            .filter(|candidate| !selected_raw.contains(&candidate.raw_key))
            .collect::<Vec<_>>();
        selected.extend(select_pool(
            &remaining,
            total - selected.len(),
            max_per_title,
            &mut selected_raw,
            &mut title_counts,
        ));
    }

    selected.truncate(total);
    selected.into_iter().map(to_corpus_sample).collect()
}

fn candidates_for_facet(
    candidates: &[EvaluatedCandidate],
    facet: CorpusFacet,
) -> Vec<&EvaluatedCandidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.facet == facet)
        .collect()
}

fn select_pool<'a>(
    pool: &[&'a EvaluatedCandidate],
    target: usize,
    max_per_title: usize,
    selected_raw: &mut HashSet<String>,
    title_counts: &mut HashMap<String, usize>,
) -> Vec<&'a EvaluatedCandidate> {
    if target == 0 || pool.is_empty() {
        return Vec::new();
    }

    let mut selected = Vec::new();
    let complexity_targets = complexity_targets(target);
    for bucket in [
        ComplexityBucket::Simple,
        ComplexityBucket::Standard,
        ComplexityBucket::Complex,
    ] {
        let mut bucket_pool = pool
            .iter()
            .copied()
            .filter(|candidate| candidate.complexity == bucket)
            .collect::<Vec<_>>();
        sort_pool(&mut bucket_pool);
        take_candidates(
            &bucket_pool,
            complexity_targets[&bucket],
            max_per_title,
            &mut selected,
            selected_raw,
            title_counts,
        );
    }

    if selected.len() < target {
        let mut remaining = pool.to_vec();
        sort_pool(&mut remaining);
        take_candidates(
            &remaining,
            target - selected.len(),
            max_per_title,
            &mut selected,
            selected_raw,
            title_counts,
        );
    }

    selected
}

fn sort_pool(pool: &mut Vec<&EvaluatedCandidate>) {
    pool.sort_by(|left, right| {
        right
            .field_density
            .cmp(&left.field_density)
            .then(
                right
                    .parse_confidence_score
                    .cmp(&left.parse_confidence_score),
            )
            .then(right.published_ts.cmp(&left.published_ts))
            .then(left.raw_title.len().cmp(&right.raw_title.len()))
            .then(left.raw_title.cmp(&right.raw_title))
    });
}

fn take_candidates<'a>(
    pool: &[&'a EvaluatedCandidate],
    target: usize,
    max_per_title: usize,
    selected: &mut Vec<&'a EvaluatedCandidate>,
    selected_raw: &mut HashSet<String>,
    title_counts: &mut HashMap<String, usize>,
) {
    if target == 0 {
        return;
    }

    let start_len = selected.len();
    for candidate in pool {
        if selected.len() - start_len >= target {
            break;
        }
        if selected_raw.contains(&candidate.raw_key) {
            continue;
        }
        if title_counts.get(&candidate.title_key).copied().unwrap_or(0) >= max_per_title {
            continue;
        }

        selected.push(candidate);
        selected_raw.insert(candidate.raw_key.clone());
        *title_counts.entry(candidate.title_key.clone()).or_default() += 1;
    }
}

fn to_corpus_sample(candidate: &EvaluatedCandidate) -> CorpusSample {
    CorpusSample {
        source: candidate.source,
        facet: candidate.facet,
        complexity: candidate.complexity,
        raw_title: candidate.raw_title.clone(),
        title_key: candidate.title_key.clone(),
        published_at: candidate.published_at.clone(),
        size_bytes: candidate.size_bytes,
        link: candidate.link.clone(),
        download_url: candidate.download_url.clone(),
        category_hint: candidate.category_hint.clone(),
        daily_series: candidate.daily_series,
        parser_first_pass: candidate.parser_snapshot.clone(),
        label: candidate.label.clone(),
        review_notes: candidate.review_notes.clone(),
    }
}

fn build_summary(
    samples: &[CorpusSample],
    files: &BTreeMap<String, String>,
    requested_total: usize,
) -> CorpusSummary {
    let mut counts_by_facet = BTreeMap::new();
    let mut counts_by_source = BTreeMap::new();
    let mut counts_by_complexity = BTreeMap::new();
    let mut unique_titles_by_facet = BTreeMap::new();
    let mut titles_per_facet: HashMap<CorpusFacet, HashSet<String>> = HashMap::new();

    for sample in samples {
        *counts_by_facet
            .entry(sample.facet.as_str().to_string())
            .or_insert(0) += 1;
        *counts_by_source
            .entry(sample.source.as_str().to_string())
            .or_insert(0) += 1;
        *counts_by_complexity
            .entry(sample.complexity.as_str().to_string())
            .or_insert(0) += 1;
        titles_per_facet
            .entry(sample.facet)
            .or_default()
            .insert(sample.title_key.clone());
    }

    for (facet, titles) in titles_per_facet {
        unique_titles_by_facet.insert(facet.as_str().to_string(), titles.len());
    }

    let daily_series_target = daily_series_target(balanced_facet_targets(requested_total).series);
    let daily_series_actual = samples
        .iter()
        .filter(|sample| sample.facet == CorpusFacet::Series && sample.daily_series)
        .count();
    let season_pack_series_target =
        season_pack_target(balanced_facet_targets(requested_total).series);
    let season_pack_series_actual = samples
        .iter()
        .filter(|sample| sample.facet == CorpusFacet::Series)
        .filter(|sample| is_pack_kind(sample.label.kind))
        .count();
    let season_pack_anime_target =
        season_pack_target(balanced_facet_targets(requested_total).anime);
    let season_pack_anime_actual = samples
        .iter()
        .filter(|sample| sample.facet == CorpusFacet::Anime)
        .filter(|sample| is_pack_kind(sample.label.kind))
        .count();
    let review_row_count = samples
        .iter()
        .filter(|sample| !sample.review_notes.is_empty())
        .count();

    CorpusSummary {
        generated_at: Utc::now().to_rfc3339(),
        requested_total,
        actual_total: samples.len(),
        daily_series_target,
        daily_series_actual,
        pack_series_target: season_pack_series_target,
        pack_series_actual: season_pack_series_actual,
        pack_anime_target: season_pack_anime_target,
        pack_anime_actual: season_pack_anime_actual,
        counts_by_facet,
        counts_by_source,
        counts_by_complexity,
        unique_titles_by_facet,
        review_row_count,
        files: files.clone(),
    }
}

fn write_corpus_files(
    output_dir: &Path,
    samples: &[CorpusSample],
) -> Result<BTreeMap<String, String>> {
    let structured_path = output_dir.join("structured_samples.jsonl");
    let oumi_path = output_dir.join("oumi_training.jsonl");
    let review_path = output_dir.join("review_samples.jsonl");

    write_jsonl(&structured_path, samples)?;

    let oumi_rows = samples
        .iter()
        .map(sample_to_oumi_row)
        .collect::<Result<Vec<_>>>()?;
    write_jsonl(&oumi_path, &oumi_rows)?;

    let review_rows = samples
        .iter()
        .filter(|sample| !sample.review_notes.is_empty())
        .map(|sample| ReviewSample {
            source: sample.source,
            facet: sample.facet,
            complexity: sample.complexity,
            raw_title: sample.raw_title.clone(),
            title_key: sample.title_key.clone(),
            review_notes: sample.review_notes.clone(),
            parser_first_pass: sample.parser_first_pass.clone(),
            label: sample.label.clone(),
        })
        .collect::<Vec<_>>();
    write_jsonl(&review_path, &review_rows)?;

    let mut files = BTreeMap::new();
    files.insert(
        "structured_samples".to_string(),
        structured_path.display().to_string(),
    );
    files.insert("oumi_training".to_string(), oumi_path.display().to_string());
    files.insert(
        "review_samples".to_string(),
        review_path.display().to_string(),
    );
    Ok(files)
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row).context("failed to serialize JSONL row")?);
        out.push('\n');
    }
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn sample_to_oumi_row(sample: &CorpusSample) -> Result<OumiRow> {
    Ok(OumiRow {
        messages: vec![
            OumiMessage {
                role: "system",
                content: RELEASE_PARSER_SYSTEM_PROMPT.to_string(),
            },
            OumiMessage {
                role: "user",
                content: format!("Parse this release title:\n{}", sample.raw_title),
            },
            OumiMessage {
                role: "assistant",
                content: serde_json::to_string(&sample.label)
                    .context("failed to serialize Oumi assistant label")?,
            },
        ],
    })
}

fn build_training_label(parsed: &ParsedReleaseMetadata, facet: CorpusFacet) -> TrainingLabel {
    let title_variants = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    TrainingLabel {
        facet_hint: facet,
        kind: release_kind(parsed),
        title: parsed.normalized_title.clone(),
        title_variants,
        year: parsed.year,
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
        episode: parsed.episode.as_ref().map(build_episode_label),
        flags: FlagsLabel {
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
        },
        fps: parsed.fps,
        missing_fields: parsed.missing_fields.clone(),
    }
}

fn build_parser_snapshot(parsed: &ParsedReleaseMetadata) -> ParserSnapshot {
    ParserSnapshot {
        normalized_title: parsed.normalized_title.clone(),
        normalized_title_variants: parsed.normalized_title_variants.clone(),
        release_group: parsed.release_group.clone(),
        languages_audio: parsed.languages_audio.clone(),
        languages_subtitles: parsed.languages_subtitles.clone(),
        imdb_id: parsed.imdb_id.clone(),
        tmdb_id: parsed.tmdb_id.clone(),
        year: parsed.year,
        quality: parsed.quality.clone(),
        source: parsed.source.clone(),
        video_codec: parsed.video_codec.clone(),
        video_encoding: parsed.video_encoding.clone(),
        audio: parsed.audio.clone(),
        audio_codecs: parsed.audio_codecs.clone(),
        audio_channels: parsed.audio_channels.clone(),
        is_dual_audio: parsed.is_dual_audio,
        is_atmos: parsed.is_atmos,
        is_dolby_vision: parsed.is_dolby_vision,
        detected_hdr: parsed.detected_hdr,
        has_hdr_fallback: parsed.has_hdr_fallback,
        is_hdr10plus: parsed.is_hdr10plus,
        is_hlg: parsed.is_hlg,
        is_10bit: parsed.is_10bit,
        is_proper_upload: parsed.is_proper_upload,
        is_repack: parsed.is_repack,
        is_remux: parsed.is_remux,
        is_bd_disk: parsed.is_bd_disk,
        is_ai_enhanced: parsed.is_ai_enhanced,
        is_hardcoded_subs: parsed.is_hardcoded_subs,
        is_uncensored: parsed.is_uncensored,
        is_dubs_only: parsed.is_dubs_only,
        streaming_service: parsed.streaming_service.clone(),
        edition: parsed.edition.clone(),
        anime_version: parsed.anime_version,
        episode: parsed.episode.as_ref().map(build_episode_label),
        parser_version: parsed.parser_version,
        parse_confidence: parsed.parse_confidence,
        missing_fields: parsed.missing_fields.clone(),
        parse_hints: parsed.parse_hints.clone(),
    }
}

fn build_episode_label(episode: &ParsedEpisodeMetadata) -> EpisodeLabel {
    EpisodeLabel {
        season: episode.season,
        episode_numbers: episode.episode_numbers.clone(),
        absolute_episode: episode.absolute_episode,
        air_date: episode.air_date.map(|date| date.to_string()),
        daily_part: episode.daily_part,
        absolute_episode_numbers: episode.absolute_episode_numbers.clone(),
        special_absolute_episode_numbers: episode.special_absolute_episode_numbers.clone(),
        full_season: episode.full_season,
        partial_season: episode.is_partial_season,
        multi_season: episode.is_multi_season,
        season_part: episode.season_part,
        season_extra: episode.is_season_extra,
        split_episode: episode.is_split_episode,
        mini_series: episode.is_mini_series,
        special_kind: episode.special_kind.map(special_kind_to_string),
        release_type: episode_release_type_to_string(episode.release_type).to_string(),
        raw: episode.raw.clone(),
    }
}

fn release_kind(parsed: &ParsedReleaseMetadata) -> ReleaseKind {
    let Some(episode) = parsed.episode.as_ref() else {
        return ReleaseKind::Movie;
    };

    if episode.special_kind.is_some() || !episode.special_absolute_episode_numbers.is_empty() {
        return ReleaseKind::Special;
    }

    if episode.full_season
        || episode.is_partial_season
        || episode.is_multi_season
        || episode.release_type == ParsedEpisodeReleaseType::SeasonPack
    {
        return ReleaseKind::SeasonPack;
    }

    if episode.episode_numbers.len() > 1 || episode.absolute_episode_numbers.len() > 1 {
        return ReleaseKind::MultiEpisode;
    }

    ReleaseKind::Episode
}

fn classify_complexity(parsed: &ParsedReleaseMetadata) -> ComplexityBucket {
    let mut score = 0;
    if parsed.year.is_some() {
        score += 1;
    }
    if parsed.quality.is_some() {
        score += 1;
    }
    if parsed.source.is_some() {
        score += 1;
    }
    if parsed.video_codec.is_some() || parsed.video_encoding.is_some() {
        score += 1;
    }
    if parsed.audio.is_some() || !parsed.audio_codecs.is_empty() {
        score += 1;
    }
    if parsed.audio_channels.is_some() {
        score += 1;
    }
    if parsed.release_group.is_some() {
        score += 1;
    }
    if !parsed.languages_audio.is_empty()
        || !parsed.languages_subtitles.is_empty()
        || parsed.is_dual_audio
    {
        score += 2;
    }
    if parsed.detected_hdr
        || parsed.is_dolby_vision
        || parsed.is_hdr10plus
        || parsed.is_hlg
        || parsed.is_10bit
    {
        score += 2;
    }
    if parsed.is_proper_upload
        || parsed.is_repack
        || parsed.is_remux
        || parsed.is_bd_disk
        || parsed.is_ai_enhanced
        || parsed.edition.is_some()
        || parsed.streaming_service.is_some()
    {
        score += 2;
    }
    if let Some(episode) = parsed.episode.as_ref() {
        score += 1;
        if episode.air_date.is_some()
            || episode.absolute_episode.is_some()
            || !episode.absolute_episode_numbers.is_empty()
            || episode.full_season
            || episode.is_partial_season
            || episode.is_multi_season
            || episode.special_kind.is_some()
            || episode.daily_part.is_some()
        {
            score += 2;
        }
    }

    match score {
        0..=4 => ComplexityBucket::Simple,
        5..=8 => ComplexityBucket::Standard,
        _ => ComplexityBucket::Complex,
    }
}

fn count_populated_fields(parsed: &ParsedReleaseMetadata) -> usize {
    let mut count = 0;
    count += usize::from(!parsed.normalized_title.is_empty());
    count += usize::from(!parsed.normalized_title_variants.is_empty());
    count += usize::from(parsed.release_group.is_some());
    count += usize::from(!parsed.languages_audio.is_empty());
    count += usize::from(!parsed.languages_subtitles.is_empty());
    count += usize::from(parsed.imdb_id.is_some());
    count += usize::from(parsed.tmdb_id.is_some());
    count += usize::from(parsed.year.is_some());
    count += usize::from(parsed.quality.is_some());
    count += usize::from(parsed.source.is_some());
    count += usize::from(parsed.video_codec.is_some());
    count += usize::from(parsed.video_encoding.is_some());
    count += usize::from(parsed.audio.is_some());
    count += parsed.audio_codecs.len();
    count += usize::from(parsed.audio_channels.is_some());
    count += usize::from(parsed.streaming_service.is_some());
    count += usize::from(parsed.edition.is_some());
    count += usize::from(parsed.anime_version.is_some());
    count += usize::from(parsed.episode.is_some());
    count += parsed.missing_fields.len();
    count
}

fn canonical_title_key(parsed: &ParsedReleaseMetadata, raw_title: &str) -> String {
    if !parsed.normalized_title.is_empty() {
        return parsed.normalized_title.to_ascii_lowercase();
    }

    parsed
        .normalized_title_variants
        .first()
        .cloned()
        .unwrap_or_else(|| raw_title.to_string())
        .to_ascii_lowercase()
}

fn build_review_notes(
    parsed: &ParsedReleaseMetadata,
    complexity: ComplexityBucket,
    daily_series: bool,
) -> Vec<String> {
    let mut notes = Vec::new();

    if parsed.parse_confidence < 0.8 {
        notes.push(format!(
            "low_parse_confidence:{:.2}",
            parsed.parse_confidence
        ));
    }
    if !parsed.missing_fields.is_empty() {
        notes.push(format!(
            "missing_fields:{}",
            parsed.missing_fields.join(",")
        ));
    }
    if complexity == ComplexityBucket::Complex {
        notes.push("complex_release".to_string());
    }
    if daily_series {
        notes.push("daily_series".to_string());
    }

    notes
}

fn balanced_facet_targets(total: usize) -> FacetTargets {
    let base = total / 3;
    let remainder = total % 3;
    FacetTargets {
        movie: base + usize::from(remainder > 0),
        series: base + usize::from(remainder > 1),
        anime: base,
    }
}

fn daily_series_target(series_target: usize) -> usize {
    let desired = series_target / SERIES_DAILY_DIVISOR;
    desired
        .clamp(SERIES_DAILY_MIN, SERIES_DAILY_MAX)
        .min(series_target)
}

fn animetosho_anime_target(anime_target: usize) -> usize {
    let desired = anime_target / ANIMETOSHO_ANIME_DIVISOR;
    desired
        .clamp(ANIMETOSHO_ANIME_MIN, ANIMETOSHO_ANIME_MAX)
        .min(anime_target)
}

fn season_pack_target(facet_target: usize) -> usize {
    let desired = facet_target / FACET_SEASON_PACK_DIVISOR;
    desired
        .clamp(FACET_SEASON_PACK_MIN, FACET_SEASON_PACK_MAX)
        .min(facet_target)
}

fn is_pack_kind(kind: ReleaseKind) -> bool {
    matches!(kind, ReleaseKind::SeasonPack | ReleaseKind::MultiEpisode)
}

fn query_terms_for_style(style: AniDbQueryStyle) -> &'static [&'static str] {
    match style {
        AniDbQueryStyle::Absolute => ABSOLUTE_EPISODE_QUERY_TERMS,
        AniDbQueryStyle::SeasonEpisode => SEASON_EPISODE_QUERY_TERMS,
    }
}

fn url_encode(input: &str) -> String {
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char)
            }
            b' ' => output.push_str("%20"),
            _ => {
                output.push('%');
                output.push_str(&format!("{byte:02X}"));
            }
        }
    }
    output
}

fn complexity_targets(target: usize) -> HashMap<ComplexityBucket, usize> {
    let simple = target * 3 / 10;
    let complex = target / 4;
    let standard = target - simple - complex;
    HashMap::from([
        (ComplexityBucket::Simple, simple),
        (ComplexityBucket::Standard, standard),
        (ComplexityBucket::Complex, complex),
    ])
}

fn is_candidate_title_worth_parsing(raw_title: &str) -> bool {
    raw_title.len() >= 8 && raw_title.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn redact_api_key(url: &str) -> String {
    let mut redacted = url.to_string();
    if let Some(start) = redacted.find("apikey=") {
        let value_start = start + "apikey=".len();
        let value_end = redacted[value_start..]
            .find('&')
            .map(|offset| value_start + offset)
            .unwrap_or(redacted.len());
        redacted.replace_range(value_start..value_end, "[redacted]");
    }
    redacted
}

fn parse_rfc2822_to_unix_timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(value)
        .map(|dt| dt.with_timezone(&Utc).timestamp())
        .ok()
}

fn first_attr<'a>(attrs: &'a [NewznabAttr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == name)
        .map(|attr| attr.value.as_str())
}

fn parse_for_corpus(raw: &str, facet: Option<CorpusFacet>) -> ParsedReleaseMetadata {
    let facet_hint = match facet {
        Some(CorpusFacet::Movie) => ContextFacetHint::Movie,
        Some(CorpusFacet::Series) => ContextFacetHint::Series,
        Some(CorpusFacet::Anime) => ContextFacetHint::Anime,
        None => ContextFacetHint::Unknown,
    };
    best_parse_for_target(
        raw,
        &ReleaseParseContext {
            facet_hint,
            title: ContextTitle::default(),
            aliases: Vec::new(),
            known_years: Vec::new(),
            imdb_ids: Vec::new(),
            episodes: Vec::new(),
        },
    )
}

fn special_kind_to_string(kind: ParsedSpecialKind) -> String {
    match kind {
        ParsedSpecialKind::Special => "special",
        ParsedSpecialKind::Ova => "ova",
        ParsedSpecialKind::Oad => "oad",
        ParsedSpecialKind::Ncop => "ncop",
        ParsedSpecialKind::Nced => "nced",
        ParsedSpecialKind::Extra => "extra",
    }
    .to_string()
}

fn episode_release_type_to_string(kind: ParsedEpisodeReleaseType) -> &'static str {
    match kind {
        ParsedEpisodeReleaseType::SingleEpisode => "single_episode",
        ParsedEpisodeReleaseType::MultiEpisode => "multi_episode",
        ParsedEpisodeReleaseType::SeasonPack => "season_pack",
        ParsedEpisodeReleaseType::RangePack => "range_pack",
        ParsedEpisodeReleaseType::Daily => "daily",
        ParsedEpisodeReleaseType::Unknown => "unknown",
    }
}

#[derive(Debug, Clone, Copy)]
struct FacetTargets {
    movie: usize,
    series: usize,
    anime: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluated(
        raw_title: &str,
        facet: CorpusFacet,
        complexity: ComplexityBucket,
    ) -> EvaluatedCandidate {
        let parsed = parse_for_corpus(raw_title, Some(facet));
        EvaluatedCandidate {
            source: CorpusSource::AnimeTosho,
            facet,
            raw_title: raw_title.to_string(),
            raw_key: raw_title.to_ascii_lowercase(),
            title_key: canonical_title_key(&parsed, raw_title),
            published_at: None,
            published_ts: None,
            size_bytes: None,
            link: None,
            download_url: None,
            category_hint: None,
            daily_series: false,
            complexity,
            field_density: count_populated_fields(&parsed),
            parse_confidence_score: (parsed.parse_confidence * 1000.0).round() as i32,
            parser_snapshot: build_parser_snapshot(&parsed),
            label: build_training_label(&parsed, facet),
            review_notes: vec![],
        }
    }

    #[test]
    fn balanced_targets_sum_to_requested_total() {
        let targets = balanced_facet_targets(1000);
        assert_eq!(targets.movie, 334);
        assert_eq!(targets.series, 333);
        assert_eq!(targets.anime, 333);
        assert_eq!(targets.movie + targets.series + targets.anime, 1000);
    }

    #[test]
    fn daily_target_reserves_small_slice() {
        assert_eq!(daily_series_target(333), 27);
        assert_eq!(daily_series_target(36), 18);
    }

    #[test]
    fn animetosho_target_reserves_meaningful_anime_slice() {
        assert_eq!(animetosho_anime_target(333), 111);
        assert_eq!(animetosho_anime_target(90), 90);
    }

    #[test]
    fn season_pack_target_reserves_small_slice() {
        assert_eq!(season_pack_target(333), 23);
        assert_eq!(season_pack_target(12), 12);
    }

    #[test]
    fn multi_episode_counts_as_pack_kind() {
        assert!(is_pack_kind(ReleaseKind::MultiEpisode));
        assert!(is_pack_kind(ReleaseKind::SeasonPack));
        assert!(!is_pack_kind(ReleaseKind::Episode));
    }

    #[test]
    fn query_terms_include_absolute_and_sxex_patterns() {
        assert!(query_terms_for_style(AniDbQueryStyle::Absolute).contains(&"24"));
        assert!(query_terms_for_style(AniDbQueryStyle::Absolute).contains(&"1-12"));
        assert!(query_terms_for_style(AniDbQueryStyle::SeasonEpisode).contains(&"S02E01"));
        assert!(query_terms_for_style(AniDbQueryStyle::SeasonEpisode).contains(&"S01E01-S01E12"));
    }

    #[test]
    fn animetosho_seed_list_uses_explicit_bleach_and_frieren_aids() {
        assert_eq!(ANIMETOSHO_AID_SEEDS.len(), 2);
        assert_eq!(ANIMETOSHO_AID_SEEDS[0].title, "Bleach");
        assert_eq!(ANIMETOSHO_AID_SEEDS[0].anidb_aid, 2369);
        assert_eq!(ANIMETOSHO_AID_SEEDS[0].style, AniDbQueryStyle::Absolute);
        assert_eq!(ANIMETOSHO_AID_SEEDS[1].title, "Frieren");
        assert_eq!(ANIMETOSHO_AID_SEEDS[1].anidb_aid, 18886);
        assert_eq!(
            ANIMETOSHO_AID_SEEDS[1].style,
            AniDbQueryStyle::SeasonEpisode
        );
    }

    #[test]
    fn complexity_marks_basic_movie_as_simple() {
        let parsed = parse_for_corpus("Movie.2024.1080p", None);
        assert_eq!(classify_complexity(&parsed), ComplexityBucket::Simple);
    }

    #[test]
    fn complexity_marks_multilang_hdr_release_as_complex() {
        let parsed = parse_for_corpus(
            "Show.S01E01.REPACK.2160p.NF.WEB-DL.DoVi.HDR10Plus.10bit.DUAL.DDP5.1.Atmos.H.265-GROUP",
            None,
        );
        assert_eq!(classify_complexity(&parsed), ComplexityBucket::Complex);
    }

    #[test]
    fn redact_api_key_replaces_secret_value() {
        let redacted = redact_api_key(
            "https://api.nzbgeek.info/api?t=get&id=abc123&apikey=supersecretvalue&cat=2000",
        );
        assert!(redacted.contains("apikey=[redacted]"));
        assert!(!redacted.contains("supersecretvalue"));
    }

    #[test]
    fn select_pool_respects_title_cap() {
        let same_a = evaluated(
            "Show.Name.S01E01.1080p.WEB-DL-GRP",
            CorpusFacet::Series,
            ComplexityBucket::Standard,
        );
        let same_b = evaluated(
            "Show.Name.S01E02.1080p.WEB-DL-GRP",
            CorpusFacet::Series,
            ComplexityBucket::Standard,
        );
        let other = evaluated(
            "Other.Show.S01E01.1080p.WEB-DL-GRP",
            CorpusFacet::Series,
            ComplexityBucket::Standard,
        );
        let pool = vec![&same_a, &same_b, &other];
        let mut selected_raw = HashSet::new();
        let mut title_counts = HashMap::new();

        let selected = select_pool(&pool, 3, 1, &mut selected_raw, &mut title_counts);

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].title_key, "show name");
        assert_eq!(selected[1].title_key, "other show");
    }
}
