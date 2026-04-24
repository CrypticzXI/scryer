use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::{ReleaseParserEvalArgs, TaskContext, ok, step};

#[derive(Debug, Deserialize)]
struct StructuredSample {
    facet: String,
    raw_title: String,
    label: ExpectedLabel,
}

#[derive(Debug, Deserialize)]
struct ExpectedLabel {
    kind: Option<String>,
    title: String,
    #[serde(default)]
    title_variants: Vec<String>,
    year: Option<i32>,
    source: Option<String>,
    episode: Option<ExpectedEpisode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ExpectedEpisode {
    season: Option<u32>,
    #[serde(default)]
    episode_numbers: Vec<u32>,
    #[serde(default)]
    absolute_episode_numbers: Vec<u32>,
    air_date: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct FacetEvalSummary {
    total: usize,
    exact_title: usize,
    full_match: usize,
    kind_match: usize,
    year_match: usize,
    episode_match: usize,
    source_match: usize,
}

#[derive(Debug, Serialize)]
struct EvalSummary {
    parser: String,
    input_path: String,
    total: usize,
    exact_title: usize,
    full_match: usize,
    kind_match: usize,
    year_match: usize,
    episode_match: usize,
    source_match: usize,
    mismatches_recorded: usize,
    facets: BTreeMap<String, FacetEvalSummary>,
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
    expected_title: String,
    actual_title: String,
    expected_kind: Option<String>,
    actual_kind: String,
    expected_year: Option<i32>,
    actual_year: Option<i32>,
    expected_source: Option<String>,
    actual_source: Option<String>,
    expected_episode: Option<ExpectedEpisode>,
    actual_episode: Option<ActualEpisode>,
    parser_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ActualEpisode {
    season: Option<u32>,
    episode_numbers: Vec<u32>,
    absolute_episode_numbers: Vec<u32>,
    air_date: Option<String>,
    release_type: String,
}

#[derive(Debug, Clone)]
struct ComparableParse {
    titles: Vec<String>,
    kind: String,
    year: Option<i32>,
    source: Option<String>,
    episode: Option<ActualEpisode>,
    parser_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GuessitRow {
    raw_title: String,
    parsed: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SonarrRow {
    raw_title: String,
    parsed: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RadarrRow {
    raw_title: String,
    parsed: Option<Value>,
    error: Option<String>,
}

pub(crate) fn run_v1_eval(ctx: &TaskContext, args: ReleaseParserEvalArgs) -> Result<()> {
    let input_path = resolve_input_path(ctx, args.input.as_ref())?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| input_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    fs::create_dir_all(&output_dir)?;

    step(format!(
        "Evaluating v1 release parser against {}",
        input_path.display()
    ));

    let file = File::open(&input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;
    let reader = BufReader::new(file);

    let mut summary = EvalSummary {
        parser: "scryer-release-parser-v1".to_string(),
        input_path: input_path.display().to_string(),
        total: 0,
        exact_title: 0,
        full_match: 0,
        kind_match: 0,
        year_match: 0,
        episode_match: 0,
        source_match: 0,
        mismatches_recorded: 0,
        facets: BTreeMap::new(),
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
        let parsed = scryer_release_parser::parse_release_metadata(&sample.raw_title);
        let comparable = comparable_from_v1(&parsed);
        score_parse(
            &sample,
            comparable,
            &mut summary,
            &mut mismatches,
            args.max_mismatches,
        );
    }

    write_eval_outputs(
        &output_dir,
        "release_parser_v1_eval_summary.json",
        "release_parser_v1_eval_mismatches.json",
        &summary,
        &mismatches,
    )?;

    Ok(())
}

pub(crate) fn run_guessit_eval(ctx: &TaskContext, args: ReleaseParserEvalArgs) -> Result<()> {
    let input_path = resolve_input_path(ctx, args.input.as_ref())?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| input_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    fs::create_dir_all(&output_dir)?;

    step(format!(
        "Evaluating guessit against {}",
        input_path.display()
    ));

    let file = File::open(&input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;
    let reader = BufReader::new(file);
    let samples = reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<StructuredSample>(&line)
                .context("failed to deserialize structured sample")
        })
        .collect::<Result<Vec<_>>>()?;

    let guessit_rows = run_guessit_batch(&input_path)?;
    let guessit_map = guessit_rows
        .into_iter()
        .map(|row| (row.raw_title.clone(), row))
        .collect::<BTreeMap<_, _>>();

    let mut summary = EvalSummary {
        parser: "guessit-3.8.0".to_string(),
        input_path: input_path.display().to_string(),
        total: 0,
        exact_title: 0,
        full_match: 0,
        kind_match: 0,
        year_match: 0,
        episode_match: 0,
        source_match: 0,
        mismatches_recorded: 0,
        facets: BTreeMap::new(),
    };
    let mut mismatches = Vec::<EvalMismatch>::new();

    for sample in samples {
        summary.total += 1;
        let row = guessit_map.get(&sample.raw_title);
        let comparable = comparable_from_guessit(row);
        score_parse(
            &sample,
            comparable,
            &mut summary,
            &mut mismatches,
            args.max_mismatches,
        );
    }

    write_eval_outputs(
        &output_dir,
        "guessit_eval_summary.json",
        "guessit_eval_mismatches.json",
        &summary,
        &mismatches,
    )?;

    Ok(())
}

pub(crate) fn run_sonarr_eval(ctx: &TaskContext, args: ReleaseParserEvalArgs) -> Result<()> {
    let input_path = resolve_input_path(ctx, args.input.as_ref())?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| input_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    fs::create_dir_all(&output_dir)?;

    step(format!(
        "Evaluating latest Sonarr parser against non-movie samples in {}",
        input_path.display()
    ));

    let file = File::open(&input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;
    let reader = BufReader::new(file);
    let samples = reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<StructuredSample>(&line)
                .context("failed to deserialize structured sample")
        })
        .collect::<Result<Vec<_>>>()?;

    let sonarr_source_dir = resolve_sonarr_source_dir();
    let sonarr_rows = run_sonarr_batch(ctx, &input_path, &sonarr_source_dir)?;
    let sonarr_map = sonarr_rows
        .into_iter()
        .map(|row| (row.raw_title.clone(), row))
        .collect::<BTreeMap<_, _>>();

    let mut summary = EvalSummary {
        parser: sonarr_parser_label(&sonarr_source_dir),
        input_path: input_path.display().to_string(),
        total: 0,
        exact_title: 0,
        full_match: 0,
        kind_match: 0,
        year_match: 0,
        episode_match: 0,
        source_match: 0,
        mismatches_recorded: 0,
        facets: BTreeMap::new(),
    };
    let mut mismatches = Vec::<EvalMismatch>::new();

    for sample in samples
        .into_iter()
        .filter(|sample| !sample.facet.eq_ignore_ascii_case("movie"))
    {
        summary.total += 1;
        let row = sonarr_map.get(&sample.raw_title);
        let comparable = comparable_from_sonarr(row);
        score_parse(
            &sample,
            comparable,
            &mut summary,
            &mut mismatches,
            args.max_mismatches,
        );
    }

    write_eval_outputs(
        &output_dir,
        "sonarr_eval_summary.json",
        "sonarr_eval_mismatches.json",
        &summary,
        &mismatches,
    )?;

    Ok(())
}

pub(crate) fn run_radarr_eval(ctx: &TaskContext, args: ReleaseParserEvalArgs) -> Result<()> {
    let input_path = resolve_input_path(ctx, args.input.as_ref())?;
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| input_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    fs::create_dir_all(&output_dir)?;

    step(format!(
        "Evaluating latest Radarr parser against movie samples in {}",
        input_path.display()
    ));

    let file = File::open(&input_path)
        .with_context(|| format!("failed to open {}", input_path.display()))?;
    let reader = BufReader::new(file);
    let samples = reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<StructuredSample>(&line)
                .context("failed to deserialize structured sample")
        })
        .collect::<Result<Vec<_>>>()?;

    let radarr_source_dir = resolve_radarr_source_dir();
    let radarr_rows = run_radarr_batch(ctx, &input_path, &radarr_source_dir)?;
    let radarr_map = radarr_rows
        .into_iter()
        .map(|row| (row.raw_title.clone(), row))
        .collect::<BTreeMap<_, _>>();

    let mut summary = EvalSummary {
        parser: radarr_parser_label(&radarr_source_dir),
        input_path: input_path.display().to_string(),
        total: 0,
        exact_title: 0,
        full_match: 0,
        kind_match: 0,
        year_match: 0,
        episode_match: 0,
        source_match: 0,
        mismatches_recorded: 0,
        facets: BTreeMap::new(),
    };
    let mut mismatches = Vec::<EvalMismatch>::new();

    for sample in samples
        .into_iter()
        .filter(|sample| sample.facet.eq_ignore_ascii_case("movie"))
    {
        summary.total += 1;
        let row = radarr_map.get(&sample.raw_title);
        let comparable = comparable_from_radarr(row);
        score_parse(
            &sample,
            comparable,
            &mut summary,
            &mut mismatches,
            args.max_mismatches,
        );
    }

    write_eval_outputs(
        &output_dir,
        "radarr_eval_summary.json",
        "radarr_eval_mismatches.json",
        &summary,
        &mismatches,
    )?;

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
        anyhow!(
            "no structured_samples_reviewed.jsonl found under {}",
            corpus_root.display()
        )
    })
}

fn write_eval_outputs(
    output_dir: &Path,
    summary_name: &str,
    mismatches_name: &str,
    summary: &EvalSummary,
    mismatches: &[EvalMismatch],
) -> Result<()> {
    let summary_path = output_dir.join(summary_name);
    let mismatches_path = output_dir.join(mismatches_name);
    fs::write(&summary_path, serde_json::to_vec_pretty(summary)?)?;
    fs::write(&mismatches_path, serde_json::to_vec_pretty(mismatches)?)?;

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

fn score_parse(
    sample: &StructuredSample,
    parsed: ComparableParse,
    summary: &mut EvalSummary,
    mismatches: &mut Vec<EvalMismatch>,
    max_mismatches: usize,
) {
    let title_match = matches_title(sample, &parsed.titles);
    let kind_match = sample
        .label
        .kind
        .as_deref()
        .is_none_or(|expected| expected.eq_ignore_ascii_case(&parsed.kind));
    let year_match = sample.label.year == parsed.year;
    let episode_match = matches_episode(sample.label.episode.as_ref(), parsed.episode.as_ref());
    let source_match =
        normalize_opt(sample.label.source.as_deref()) == normalize_opt(parsed.source.as_deref());
    let full_match = title_match && kind_match && year_match && episode_match && source_match;

    summary.exact_title += usize::from(title_match);
    summary.full_match += usize::from(full_match);
    summary.kind_match += usize::from(kind_match);
    summary.year_match += usize::from(year_match);
    summary.episode_match += usize::from(episode_match);
    summary.source_match += usize::from(source_match);

    let facet_summary = summary.facets.entry(sample.facet.clone()).or_default();
    facet_summary.total += 1;
    facet_summary.exact_title += usize::from(title_match);
    facet_summary.full_match += usize::from(full_match);
    facet_summary.kind_match += usize::from(kind_match);
    facet_summary.year_match += usize::from(year_match);
    facet_summary.episode_match += usize::from(episode_match);
    facet_summary.source_match += usize::from(source_match);

    if !full_match && mismatches.len() < max_mismatches {
        mismatches.push(EvalMismatch {
            raw_title: sample.raw_title.clone(),
            facet: sample.facet.clone(),
            title_match,
            kind_match,
            year_match,
            episode_match,
            source_match,
            expected_title: sample.label.title.clone(),
            actual_title: parsed.titles.first().cloned().unwrap_or_default(),
            expected_kind: sample.label.kind.clone(),
            actual_kind: parsed.kind,
            expected_year: sample.label.year,
            actual_year: parsed.year,
            expected_source: sample.label.source.clone(),
            actual_source: parsed.source,
            expected_episode: sample.label.episode.clone(),
            actual_episode: parsed.episode,
            parser_error: parsed.parser_error,
        });
        summary.mismatches_recorded = mismatches.len();
    }
}

fn comparable_from_v1(parsed: &scryer_release_parser::ParsedReleaseMetadata) -> ComparableParse {
    let mut titles = parsed.normalized_title_variants.clone();
    if !parsed.normalized_title.is_empty()
        && titles
            .iter()
            .all(|title| !title.eq_ignore_ascii_case(&parsed.normalized_title))
    {
        titles.push(parsed.normalized_title.clone());
    }

    let episode = parsed.episode.as_ref().map(|episode| ActualEpisode {
        season: episode.season,
        episode_numbers: episode.episode_numbers.clone(),
        absolute_episode_numbers: if episode.absolute_episode_numbers.is_empty() {
            episode.absolute_episode.into_iter().collect()
        } else {
            episode.absolute_episode_numbers.clone()
        },
        air_date: episode.air_date.map(|value| value.to_string()),
        release_type: format!("{:?}", episode.release_type),
    });

    ComparableParse {
        titles,
        kind: kind_label_v1(parsed).to_string(),
        year: parsed.year.map(|value| value as i32),
        source: parsed.source.clone(),
        episode,
        parser_error: None,
    }
}

fn comparable_from_guessit(row: Option<&GuessitRow>) -> ComparableParse {
    let Some(row) = row else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: Some("missing_guessit_result".to_string()),
        };
    };

    let Some(parsed) = row.parsed.as_ref() else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: row.error.clone(),
        };
    };

    let mut titles = Vec::new();
    titles.extend(strings_at(parsed, "title"));
    titles.extend(strings_at(parsed, "alternative_title"));
    titles.extend(strings_at(parsed, "alternative_titles"));
    dedupe_strings(&mut titles);

    let season = first_u32(parsed.get("season"));
    let mut episode_numbers = u32_values(parsed.get("episode"));
    let mut absolute_episode_numbers = u32_values(parsed.get("absolute_episode"));
    if absolute_episode_numbers.is_empty() && season.is_none() {
        absolute_episode_numbers = episode_numbers.clone();
    }
    dedupe_u32(&mut episode_numbers);
    dedupe_u32(&mut absolute_episode_numbers);

    let air_date = first_date_string(parsed.get("date"));
    let kind = guessit_kind(
        parsed,
        season,
        &episode_numbers,
        &absolute_episode_numbers,
        air_date.is_some(),
    );
    let source = normalize_guessit_source(parsed);

    ComparableParse {
        titles,
        kind,
        year: first_i32(parsed.get("year")),
        source,
        episode: if season.is_some()
            || !episode_numbers.is_empty()
            || !absolute_episode_numbers.is_empty()
            || air_date.is_some()
        {
            Some(ActualEpisode {
                season,
                episode_numbers,
                absolute_episode_numbers,
                air_date,
                release_type: "Guessit".to_string(),
            })
        } else {
            None
        },
        parser_error: row.error.clone(),
    }
}

fn comparable_from_sonarr(row: Option<&SonarrRow>) -> ComparableParse {
    let Some(row) = row else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: Some("missing_sonarr_result".to_string()),
        };
    };

    let Some(parsed) = row.parsed.as_ref() else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: row.error.clone(),
        };
    };

    let mut titles = strings_at(parsed, "series_title");
    titles.extend(strings_at(parsed, "series_title_without_year"));
    titles.extend(strings_at(parsed, "series_all_titles"));
    dedupe_strings(&mut titles);

    let season = first_u32(parsed.get("season_number"));
    let mut episode_numbers = u32_values(parsed.get("episode_numbers"));
    let mut absolute_episode_numbers = u32_values(parsed.get("absolute_episode_numbers"));
    absolute_episode_numbers.extend(u32_values(parsed.get("special_absolute_episode_numbers")));
    dedupe_u32(&mut episode_numbers);
    dedupe_u32(&mut absolute_episode_numbers);

    let air_date = first_string(parsed.get("air_date"));
    let year = first_i32(parsed.get("series_title_year")).or_else(|| year_from_air_date(&air_date));
    let kind = sonarr_kind(
        parsed,
        season,
        &episode_numbers,
        &absolute_episode_numbers,
        air_date.is_some(),
    );
    let source = normalize_sonarr_source(parsed, &row.raw_title);

    ComparableParse {
        titles,
        kind,
        year,
        source,
        episode: if season.is_some()
            || !episode_numbers.is_empty()
            || !absolute_episode_numbers.is_empty()
            || air_date.is_some()
        {
            Some(ActualEpisode {
                season,
                episode_numbers,
                absolute_episode_numbers,
                air_date,
                release_type: first_string(parsed.get("release_type"))
                    .unwrap_or_else(|| "Sonarr".to_string()),
            })
        } else {
            None
        },
        parser_error: row.error.clone(),
    }
}

fn comparable_from_radarr(row: Option<&RadarrRow>) -> ComparableParse {
    let Some(row) = row else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: Some("missing_radarr_result".to_string()),
        };
    };

    let Some(parsed) = row.parsed.as_ref() else {
        return ComparableParse {
            titles: Vec::new(),
            kind: "unknown".to_string(),
            year: None,
            source: None,
            episode: None,
            parser_error: row.error.clone(),
        };
    };

    let mut titles = strings_at(parsed, "movie_titles");
    titles.extend(strings_at(parsed, "primary_movie_title"));
    titles.extend(strings_at(parsed, "movie_title"));
    dedupe_strings(&mut titles);

    ComparableParse {
        titles,
        kind: "movie".to_string(),
        year: first_i32(parsed.get("year")),
        source: normalize_radarr_source(parsed, &row.raw_title),
        episode: None,
        parser_error: row.error.clone(),
    }
}

fn run_guessit_batch(input_path: &Path) -> Result<Vec<GuessitRow>> {
    let mut script = NamedTempFile::new().context("failed to create temp guessit script")?;
    script.write_all(
        br#"import json
import sys
from guessit import guessit

input_path = sys.argv[1]
with open(input_path, "r", encoding="utf-8") as handle:
    for line in handle:
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        raw_title = row["raw_title"]
        try:
            parsed = guessit(raw_title)
            print(json.dumps({"raw_title": raw_title, "parsed": parsed}, default=str))
        except Exception as exc:
            print(json.dumps({"raw_title": raw_title, "parsed": None, "error": str(exc)}))
"#,
    )?;
    script.flush()?;

    let output = Command::new("python3")
        .arg(script.path())
        .arg(input_path)
        .output()
        .context("failed to execute guessit batch script")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("guessit batch script failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<GuessitRow>(line).context("failed to decode guessit row")
        })
        .collect()
}

fn run_sonarr_batch(
    ctx: &TaskContext,
    input_path: &Path,
    sonarr_source_dir: &str,
) -> Result<Vec<SonarrRow>> {
    let script_path = ctx.path("xtask/fixtures/sonarr-parser/run_sonarr_parser.sh");
    let input_path = input_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", input_path.display()))?;
    let temp_dir = tempfile::tempdir().context("failed to create temp sonarr output dir")?;
    let output_path = temp_dir.path().join("sonarr_output.jsonl");

    let output = Command::new("bash")
        .arg(&script_path)
        .arg(&input_path)
        .arg(&output_path)
        .arg(sonarr_source_dir)
        .output()
        .with_context(|| format!("failed to execute {}", script_path.display()))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "sonarr parser fixture failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }

    let output = fs::read_to_string(&output_path)
        .with_context(|| format!("failed to read {}", output_path.display()))?;
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<SonarrRow>(line).context("failed to decode sonarr row"))
        .collect()
}

fn run_radarr_batch(
    ctx: &TaskContext,
    input_path: &Path,
    radarr_source_dir: &str,
) -> Result<Vec<RadarrRow>> {
    let script_path = ctx.path("xtask/fixtures/radarr-parser/run_radarr_parser.sh");
    let input_path = input_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", input_path.display()))?;
    let temp_dir = tempfile::tempdir().context("failed to create temp radarr output dir")?;
    let output_path = temp_dir.path().join("radarr_output.jsonl");

    let output = Command::new("bash")
        .arg(&script_path)
        .arg(&input_path)
        .arg(&output_path)
        .arg(radarr_source_dir)
        .output()
        .with_context(|| format!("failed to execute {}", script_path.display()))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "radarr parser fixture failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }

    let output = fs::read_to_string(&output_path)
        .with_context(|| format!("failed to read {}", output_path.display()))?;
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<RadarrRow>(line).context("failed to decode radarr row"))
        .collect()
}

fn resolve_sonarr_source_dir() -> String {
    env::var("SONARR_SOURCE_DIR")
        .unwrap_or_else(|_| "/Users/jeremy/dev/supporting-codebases/Sonarr".to_string())
}

fn resolve_radarr_source_dir() -> String {
    env::var("RADARR_SOURCE_DIR")
        .unwrap_or_else(|_| "/Users/jeremy/dev/supporting-codebases/Radarr".to_string())
}

fn sonarr_parser_label(sonarr_source_dir: &str) -> String {
    parser_label("sonarr-latest", sonarr_source_dir)
}

fn radarr_parser_label(radarr_source_dir: &str) -> String {
    parser_label("radarr-latest", radarr_source_dir)
}

fn parser_label(prefix: &str, source_dir: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(source_dir)
        .arg("rev-parse")
        .arg("--short=9")
        .arg("HEAD")
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|revision| format!("{prefix}-{}", revision.trim()))
        .unwrap_or_else(|| prefix.to_string())
}

fn matches_title(sample: &StructuredSample, actual_titles: &[String]) -> bool {
    let expected = normalize_title(sample.label.title.as_str());
    let mut normalized_actuals = actual_titles
        .iter()
        .map(|value| normalize_title(value))
        .collect::<Vec<_>>();
    normalized_actuals.extend(
        sample
            .label
            .title_variants
            .iter()
            .filter(|_| false)
            .map(|value| normalize_title(value)),
    );
    normalized_actuals.iter().any(|actual| actual == &expected)
}

fn matches_episode(expected: Option<&ExpectedEpisode>, actual: Option<&ActualEpisode>) -> bool {
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
            match (&expected.air_date, actual.air_date.as_deref()) {
                (Some(expected), Some(actual)) => expected == actual,
                (None, None) => true,
                (None, Some(_)) => true,
                (Some(_), None) => false,
            }
        }
        _ => false,
    }
}

fn kind_label_v1(parsed: &scryer_release_parser::ParsedReleaseMetadata) -> &'static str {
    let Some(episode) = parsed.episode.as_ref() else {
        return "movie";
    };

    if episode.full_season
        || episode.is_partial_season
        || episode.is_multi_season
        || episode.release_type == scryer_release_parser::ParsedEpisodeReleaseType::SeasonPack
    {
        return "season_pack";
    }

    if episode.episode_numbers.len() > 1
        || episode.absolute_episode_numbers.len() > 1
        || episode.release_type == scryer_release_parser::ParsedEpisodeReleaseType::MultiEpisode
    {
        return "multi_episode";
    }

    "episode"
}

fn guessit_kind(
    parsed: &Value,
    season: Option<u32>,
    episode_numbers: &[u32],
    absolute_episode_numbers: &[u32],
    has_air_date: bool,
) -> String {
    if parsed
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("movie"))
    {
        return "movie".to_string();
    }

    if season.is_some()
        && episode_numbers.is_empty()
        && absolute_episode_numbers.is_empty()
        && !has_air_date
    {
        return "season_pack".to_string();
    }

    if episode_numbers.len() > 1 || absolute_episode_numbers.len() > 1 {
        return "multi_episode".to_string();
    }

    if season.is_some()
        || !episode_numbers.is_empty()
        || !absolute_episode_numbers.is_empty()
        || has_air_date
    {
        return "episode".to_string();
    }

    "unknown".to_string()
}

fn sonarr_kind(
    parsed: &Value,
    season: Option<u32>,
    episode_numbers: &[u32],
    absolute_episode_numbers: &[u32],
    has_air_date: bool,
) -> String {
    let release_type = first_string(parsed.get("release_type")).unwrap_or_default();
    if bool_at(parsed, "full_season")
        || bool_at(parsed, "is_partial_season")
        || bool_at(parsed, "is_multi_season")
        || release_type.eq_ignore_ascii_case("SeasonPack")
    {
        return "season_pack".to_string();
    }

    if episode_numbers.len() > 1
        || absolute_episode_numbers.len() > 1
        || release_type.eq_ignore_ascii_case("MultiEpisode")
    {
        return "multi_episode".to_string();
    }

    if season.is_some()
        || !episode_numbers.is_empty()
        || !absolute_episode_numbers.is_empty()
        || has_air_date
        || release_type.eq_ignore_ascii_case("SingleEpisode")
    {
        return "episode".to_string();
    }

    "unknown".to_string()
}

fn normalize_guessit_source(parsed: &Value) -> Option<String> {
    let source = parsed.get("source").and_then(Value::as_str)?;
    let source_upper = source.to_ascii_uppercase();
    let mut other = strings_at(parsed, "other")
        .into_iter()
        .map(|value| value.to_ascii_uppercase())
        .collect::<Vec<_>>();
    other.extend(
        strings_at(parsed, "other")
            .into_iter()
            .map(|value| value.to_ascii_uppercase()),
    );
    let streaming_service = parsed
        .get("streaming_service")
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_uppercase());

    if source_upper.contains("WEB") {
        let has_rip = other.iter().any(|value| value.contains("RIP"));
        let override_to_webdl = has_rip
            && streaming_service.as_deref().is_some_and(|value| {
                matches!(
                    value,
                    "AMZN" | "AMAZON" | "CR" | "CRUNCHYROLL" | "DSNP" | "DISNEY+"
                )
            });
        if has_rip && !override_to_webdl {
            return Some("WEBRIP".to_string());
        }
        return Some("WEB-DL".to_string());
    }

    if source_upper.contains("BLU") || source_upper.contains("BD") {
        if other.iter().any(|value| value.contains("RIP")) {
            return Some("BDRIP".to_string());
        }
        return Some("BLURAY".to_string());
    }

    if source_upper.contains("HDTV") || source_upper == "TV" {
        return Some("HDTV".to_string());
    }

    if source_upper.contains("DVD") {
        return Some("DVD".to_string());
    }

    Some(source_upper.replace(' ', "-"))
}

fn normalize_sonarr_source(parsed: &Value, raw_title: &str) -> Option<String> {
    let source = first_string(parsed.get("quality_source"))?;
    match source.as_str() {
        "Web" => Some("WEB-DL".to_string()),
        "WebRip" if raw_title_has_service_webdl_convention(raw_title) => Some("WEB-DL".to_string()),
        "WebRip" => Some("WEBRIP".to_string()),
        "Television" | "TelevisionRaw" => Some("HDTV".to_string()),
        "DVD" => Some("DVD".to_string()),
        "Bluray" | "BlurayRaw" => Some("BLURAY".to_string()),
        "Unknown" => None,
        other => Some(other.to_ascii_uppercase()),
    }
}

fn normalize_radarr_source(parsed: &Value, raw_title: &str) -> Option<String> {
    let source = first_string(parsed.get("quality_source"))?;
    match source.as_str() {
        "WEBDL" => Some("WEB-DL".to_string()),
        "WEBRIP" if raw_title_has_service_webdl_convention(raw_title) => Some("WEB-DL".to_string()),
        "WEBRIP" => Some("WEBRIP".to_string()),
        "TV" => Some("HDTV".to_string()),
        "DVD" => Some("DVD".to_string()),
        "BLURAY" => Some("BLURAY".to_string()),
        "UNKNOWN" => None,
        other => Some(other.to_ascii_uppercase()),
    }
}

fn raw_title_has_service_webdl_convention(raw_title: &str) -> bool {
    let upper = raw_title.to_ascii_uppercase();
    [
        "AMZN",
        "AMAZON",
        "CR",
        "CRUNCHYROLL",
        "DSNP",
        "DISNEY",
        "NF",
        "NETFLIX",
    ]
    .iter()
    .any(|service| upper.contains(service))
}

fn strings_at(value: &Value, key: &str) -> Vec<String> {
    value.get(key).map_or_else(Vec::new, string_values)
}

fn string_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn first_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Array(values) => values.iter().find_map(|item| first_string(Some(item))),
        _ => None,
    })
}

fn bool_at(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn first_u32(value: Option<&Value>) -> Option<u32> {
    value.and_then(|value| match value {
        Value::Number(number) => number.as_u64().map(|value| value as u32).or_else(|| {
            number.as_f64().and_then(|value| {
                (value.is_finite() && value.fract() == 0.0 && value >= 0.0).then_some(value as u32)
            })
        }),
        Value::String(text) => text.parse::<u32>().ok(),
        Value::Array(values) => values.iter().find_map(|item| first_u32(Some(item))),
        _ => None,
    })
}

fn u32_values(value: Option<&Value>) -> Vec<u32> {
    match value {
        Some(Value::Number(_)) => first_u32(value).into_iter().collect(),
        Some(Value::String(text)) => text.parse::<u32>().ok().into_iter().collect(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|item| first_u32(Some(item)))
            .collect(),
        _ => Vec::new(),
    }
}

fn first_i32(value: Option<&Value>) -> Option<i32> {
    value.and_then(|value| match value {
        Value::Number(number) => number.as_i64().map(|value| value as i32),
        Value::String(text) => text.parse::<i32>().ok(),
        _ => None,
    })
}

fn year_from_air_date(air_date: &Option<String>) -> Option<i32> {
    air_date
        .as_deref()
        .and_then(|value| value.get(0..4))
        .and_then(|year| year.parse::<i32>().ok())
}

fn first_date_string(value: Option<&Value>) -> Option<String> {
    let text = value.and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(values) => values
            .iter()
            .find_map(|item| item.as_str().map(ToOwned::to_owned)),
        _ => None,
    })?;
    NaiveDate::parse_from_str(&text, "%Y-%m-%d")
        .ok()
        .map(|date| date.to_string())
}

fn dedupe_strings(values: &mut Vec<String>) {
    let mut unique = Vec::<String>::new();
    for value in values.drain(..) {
        if unique
            .iter()
            .all(|existing| !existing.eq_ignore_ascii_case(&value))
        {
            unique.push(value);
        }
    }
    *values = unique;
}

fn dedupe_u32(values: &mut Vec<u32>) {
    let mut unique = Vec::<u32>::new();
    for value in values.drain(..) {
        if unique.iter().all(|existing| *existing != value) {
            unique.push(value);
        }
    }
    *values = unique;
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
