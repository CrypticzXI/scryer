use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, RETRY_AFTER, USER_AGENT};
use scryer_outbound_http::{blocking_reqwest_client, send_blocking_reqwest_request};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
use xtask_support::{TaskContext, ok, run_checked, step};

const APPS: &[&str] = &["sonarr", "radarr"];
const FETCH_WORKERS: usize = 36;
const GITHUB_REPO_API_BASE: &str = "https://api.github.com/repos/TRaSH-Guides/Guides";
const GITHUB_API_BASE: &str = "https://api.github.com/repos/TRaSH-Guides/Guides/contents/docs/json";
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/TRaSH-Guides/Guides";
const REQUEST_USER_AGENT: &str = "scryer-xtask-trash-guides";
const SOURCE_REVISION_ENV: &str = "SCRYER_TRASH_GUIDES_REVISION";
const ACCEPT_STEMS_ENV: &str = "SCRYER_TRASH_GUIDES_ACCEPT_STEMS";

const QUALITY_OUTPUT: &str =
    "crates/scryer-application/src/quality/trash_guides_release_groups.generated.rs";
const PARSER_OUTPUT: &str =
    "crates/scryer-release-parser/src/trash_guides_parser_knowledge.generated.rs";
const SUMMARY_OUTPUT: &str = "xtask-trash-guides/generated/latest-summary.txt";
const MANIFEST_OUTPUT: &str = "xtask-trash-guides/generated/stem-classification.json";

struct LegacySeedGroup {
    name: &'static str,
    tier: DistilledTier,
    context: DistilledContext,
}

const LEGACY_SEED_GROUPS: &[LegacySeedGroup] = &[
    LegacySeedGroup {
        name: "BDMV",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "BDVD",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "BakedFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "D3US",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "DaddySubs",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "DeadFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Deadmau RAWS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Erai-raws",
        tier: DistilledTier::Bronze,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "GSK_kun",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Iznjie Biznjie",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Judgment",
        tier: DistilledTier::Bronze,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "M2TS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "Mr.Deadpool",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "NAN0",
        tier: DistilledTier::Gold,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "NoGrop",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "NoobSubs",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "PiRaTeS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "PMR",
        tier: DistilledTier::Gold,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "QAS",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "SpaceFish",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "SubsPlus+",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "tenshi",
        tier: DistilledTier::Silver,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "VISIONPLUSHDR-X",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "VISIONPLUSHDR1000",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "WtF Anime",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
    LegacySeedGroup {
        name: "YTS.AG",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "YTS.LT",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "YTS.MX",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "jennaortega",
        tier: DistilledTier::Banned,
        context: DistilledContext::Any,
    },
    LegacySeedGroup {
        name: "mal lu zen",
        tier: DistilledTier::Banned,
        context: DistilledContext::Anime,
    },
];

#[derive(Debug, Deserialize)]
struct GitHubEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubCommit {
    sha: String,
}

#[derive(Debug, Clone)]
struct FetchTask {
    app: String,
    filename: String,
}

#[derive(Debug, Deserialize)]
struct UpstreamCf {
    name: String,
    #[serde(default)]
    trash_id: String,
    #[serde(default)]
    specifications: Vec<UpstreamSpec>,
    #[serde(default)]
    trash_scores: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct UpstreamSpec {
    name: String,
    implementation: String,
    #[serde(default)]
    required: Value,
    #[serde(default)]
    negate: Value,
    #[serde(default)]
    fields: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct UpstreamRecord {
    app: String,
    stem: String,
    source_path: String,
    trash_id: String,
    cf_name: String,
    spec_name: String,
    implementation: String,
    value: String,
    trash_scores_json: String,
    required_json: String,
    negate_json: String,
    complete_json: String,
}

#[derive(Debug)]
struct FetchedRecords {
    source_revision: String,
    records: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledTier {
    Gold,
    Silver,
    Bronze,
    Banned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledContext {
    Web,
    BluRay,
    UhdBluRay,
    Remux,
    Anime,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DistilledFacet {
    Movie,
    Series,
    Anime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GroupMatchKindSpec {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupRuleIdentity {
    matcher: String,
    match_kind: GroupMatchKindSpec,
    facet: DistilledFacet,
    context: DistilledContext,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupRuleKey {
    matcher: String,
    match_kind: GroupMatchKindSpec,
    tier: DistilledTier,
    facet: DistilledFacet,
    context: DistilledContext,
}

#[derive(Debug, Clone)]
struct GroupRuleRecord {
    key: GroupRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ParserSignalKindSpec {
    AiEnhanced,
    Proper,
    Repack,
    DubsOnly,
    HardcodedSubs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TokenPatternKindSpec {
    Sequence,
    RequiredTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TokenPatternSpec {
    kind: TokenPatternKindSpec,
    tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ServiceAliasKey {
    token: String,
    service: String,
}

#[derive(Debug, Clone)]
struct ServiceAliasRecord {
    key: ServiceAliasKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SignalRuleKey {
    kind: ParserSignalKindSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct SignalRuleRecord {
    key: SignalRuleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CategoryScopeSpec {
    Any,
    Anime,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BlockedTitleKey {
    code: String,
    facet: DistilledFacet,
    category: CategoryScopeSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct BlockedTitleRecord {
    key: BlockedTitleKey,
    provenance: Vec<UpstreamRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FactRuleKey {
    code: String,
    facet: DistilledFacet,
    category: CategoryScopeSpec,
    pattern: TokenPatternSpec,
}

#[derive(Debug, Clone)]
struct FactRuleRecord {
    key: FactRuleKey,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LocaleGroupFactRuleKey {
    code: String,
    matcher: String,
    match_kind: GroupMatchKindSpec,
    facet: DistilledFacet,
    source_context: DistilledContext,
}

#[derive(Debug, Clone)]
struct LocaleGroupFactRuleRecord {
    key: LocaleGroupFactRuleKey,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DetectionOwner {
    NativeFact,
    ExistingNative,
    ManagedRego,
    CustomRuleOnly,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EffectBinding {
    HardBlock,
    PersonaScore,
    LocaleScore,
    Informational,
    None,
}

#[derive(Debug, Clone, Copy)]
struct StemClassification {
    detection_owner: DetectionOwner,
    effect_binding: EffectBinding,
    reason: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct StemClassificationManifest {
    source_revision: String,
    synced_at: String,
    stems: Vec<StemClassificationManifestRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StemClassificationManifestRecord {
    app: String,
    stem: String,
    detection_owner: DetectionOwner,
    effect_binding: EffectBinding,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MetadataRuleRecord {
    app: String,
    facet: DistilledFacet,
    stem: String,
    trash_id: String,
    cf_name: String,
    spec_name: String,
    implementation: String,
    value: String,
    trash_scores_json: String,
    required_json: String,
    negate_json: String,
    complete_json: String,
    reason: String,
    source_path: String,
}

#[derive(Debug, Default)]
struct DistilledCatalog {
    group_rules: Vec<GroupRuleRecord>,
    service_alias_rules: Vec<ServiceAliasRecord>,
    signal_rules: Vec<SignalRuleRecord>,
    blocked_title_rules: Vec<BlockedTitleRecord>,
    fact_rules: Vec<FactRuleRecord>,
    locale_group_fact_rules: Vec<LocaleGroupFactRuleRecord>,
    no_release_group_facets: Vec<DistilledFacet>,
    source_records: Vec<MetadataRuleRecord>,
    inactive_records: Vec<MetadataRuleRecord>,
    ignored_records: Vec<MetadataRuleRecord>,
}

pub fn run_sync(ctx: &TaskContext) -> Result<()> {
    step("Fetching TRaSH Guides custom formats");
    let fetched = fetch_all_records()?;
    ok(format!(
        "Fetched {} custom-format specifications at {}",
        fetched.records.len(),
        fetched.source_revision
    ));
    let manifest_output = ctx.path(MANIFEST_OUTPUT);
    enforce_stem_coverage(&manifest_output, &fetched.records)?;

    step("Distilling Scryer-native rule sets");
    let distilled = distill_records(&fetched.records)?;
    ok(format!(
        "Distilled {} release-group rules, {} service aliases, {} parser signals, {} blocked title rules",
        distilled.group_rules.len(),
        distilled.service_alias_rules.len(),
        distilled.signal_rules.len(),
        distilled.blocked_title_rules.len()
    ));

    let synced_at = Utc::now().format("%Y-%m-%d").to_string();

    step("Writing generated outputs");
    let quality_output = ctx.path(QUALITY_OUTPUT);
    let parser_output = ctx.path(PARSER_OUTPUT);
    let summary_output = ctx.path(SUMMARY_OUTPUT);

    write_if_changed(
        &quality_output,
        &render_quality_output(&distilled, &synced_at, &fetched.source_revision)?,
    )?;
    write_if_changed(
        &parser_output,
        &render_parser_output(&distilled, &synced_at, &fetched.source_revision)?,
    )?;
    write_if_changed(
        &summary_output,
        &render_summary(&distilled, &synced_at, &fetched.source_revision),
    )?;
    write_if_changed(
        &manifest_output,
        &render_stem_manifest(&fetched.records, &synced_at, &fetched.source_revision)?,
    )?;
    format_generated_rust(ctx)?;
    ok("Generated TRaSH distillation artifacts refreshed");

    Ok(())
}

fn fetch_all_records() -> Result<FetchedRecords> {
    let client = blocking_reqwest_client().context("failed to build HTTP client")?;
    let source_revision = resolve_source_revision(&client)?;
    let mut tasks = Vec::new();

    for app in APPS {
        let listing_url = format!("{GITHUB_API_BASE}/{app}/cf?ref={source_revision}");
        let listing = get_json::<Vec<GitHubEntry>>(&client, &listing_url)
            .with_context(|| format!("failed to list {app} custom formats"))?;

        for entry in listing {
            if !entry.name.ends_with(".json") {
                continue;
            }
            tasks.push(FetchTask {
                app: (*app).to_string(),
                filename: entry.name,
            });
        }
    }

    let worker_count = tasks.len().clamp(1, FETCH_WORKERS);
    let chunk_size = tasks.len().div_ceil(worker_count).max(1);
    let mut records = Vec::new();

    thread::scope(|scope| -> Result<()> {
        let mut workers = Vec::new();
        for chunk in tasks.chunks(chunk_size) {
            let client = client.clone();
            let source_revision = source_revision.clone();
            workers.push(scope.spawn(move || -> Result<Vec<UpstreamRecord>> {
                let mut chunk_records = Vec::new();
                for task in chunk {
                    chunk_records.extend(fetch_records_for_task(&client, task, &source_revision)?);
                }
                Ok(chunk_records)
            }));
        }

        for worker in workers {
            records.extend(
                worker
                    .join()
                    .map_err(|panic| anyhow!("TRaSH fetch worker panicked: {panic:?}"))??,
            );
        }
        Ok(())
    })?;

    records.sort();
    Ok(FetchedRecords {
        source_revision,
        records,
    })
}

fn resolve_source_revision(client: &Client) -> Result<String> {
    let requested = std::env::var(SOURCE_REVISION_ENV)
        .ok()
        .filter(|revision| !revision.trim().is_empty())
        .unwrap_or_else(|| "master".to_string());
    let commit_url = format!("{GITHUB_REPO_API_BASE}/commits/{requested}");
    let resolved = get_json::<GitHubCommit>(client, &commit_url)
        .with_context(|| format!("failed to resolve TRaSH Guides revision {requested}"))?;
    if resolved.sha.is_empty() {
        bail!("TRaSH Guides resolved an empty source revision for {requested}");
    }
    Ok(resolved.sha)
}

fn fetch_records_for_task(
    client: &Client,
    task: &FetchTask,
    source_revision: &str,
) -> Result<Vec<UpstreamRecord>> {
    let raw_url = format!(
        "{GITHUB_RAW_BASE}/{source_revision}/docs/json/{}/cf/{}",
        task.app, task.filename
    );
    let source_path = format!("docs/json/{}/cf/{}", task.app, task.filename);
    let parsed = get_json::<UpstreamCf>(client, &raw_url)
        .with_context(|| format!("failed to fetch {}", task.filename))?;
    let stem = task.filename.trim_end_matches(".json").to_string();
    let mut records = Vec::new();

    for spec in parsed.specifications {
        let value = spec
            .fields
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| spec.fields.to_string());
        records.push(UpstreamRecord {
            app: task.app.clone(),
            stem: stem.clone(),
            source_path: source_path.clone(),
            trash_id: parsed.trash_id.clone(),
            cf_name: parsed.name.clone(),
            spec_name: spec.name,
            implementation: spec.implementation,
            value,
            trash_scores_json: json_string(parsed.trash_scores.as_ref()),
            required_json: json_string(Some(&spec.required)),
            negate_json: json_string(Some(&spec.negate)),
            complete_json: json_string(Some(&spec.fields)),
        });
    }

    Ok(records)
}

fn json_string(value: Option<&Value>) -> String {
    serde_json::to_string(value.unwrap_or(&Value::Null))
        .expect("serializing a serde_json::Value cannot fail")
}

fn get_json<T: for<'de> Deserialize<'de>>(client: &Client, url: &str) -> Result<T> {
    let mut last_error = None;
    for attempt in 1..=4 {
        match send_blocking_reqwest_request(
            client
                .get(url)
                .timeout(Duration::from_secs(30))
                .header(USER_AGENT, REQUEST_USER_AGENT)
                .header(ACCEPT, "application/json"),
        ) {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return response
                        .json::<T>()
                        .with_context(|| format!("failed to decode JSON from {url}"));
                }
                last_error = Some(anyhow!("unsuccessful response for {url}: HTTP {status}"));
                if attempt < 4 {
                    thread::sleep(retry_delay_for_response(status, &response, attempt));
                }
                continue;
            }
            Err(error) => last_error = Some(anyhow!("request failed for {url}: {error}")),
        }

        if attempt < 4 {
            thread::sleep(retry_delay_for_attempt(attempt));
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("request failed for {url}")))
}

fn retry_delay_for_response(
    status: StatusCode,
    response: &reqwest::blocking::Response,
    attempt: usize,
) -> Duration {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return retry_after_delay(response)
            .unwrap_or_else(|| Duration::from_secs((attempt as u64).max(1)));
    }

    retry_delay_for_attempt(attempt)
}

fn retry_delay_for_attempt(attempt: usize) -> Duration {
    Duration::from_millis(250 * attempt as u64)
}

fn retry_after_delay(response: &reqwest::blocking::Response) -> Option<Duration> {
    let retry_after = response.headers().get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = retry_after.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let when = DateTime::parse_from_rfc2822(retry_after).ok()?;
    let when = when.with_timezone(&Utc);
    if when <= Utc::now() {
        return Some(Duration::from_secs(0));
    }

    (when - Utc::now()).to_std().ok()
}

fn distill_records(records: &[UpstreamRecord]) -> Result<DistilledCatalog> {
    let mut group_rules = BTreeMap::<GroupRuleKey, Vec<UpstreamRecord>>::new();
    let mut service_alias_rules = BTreeMap::<ServiceAliasKey, Vec<UpstreamRecord>>::new();
    let mut signal_rules = BTreeMap::<SignalRuleKey, Vec<UpstreamRecord>>::new();
    let mut blocked_title_rules = BTreeMap::<BlockedTitleKey, Vec<UpstreamRecord>>::new();
    let mut fact_rules = BTreeMap::<FactRuleKey, Vec<UpstreamRecord>>::new();
    let mut locale_group_fact_rules =
        BTreeMap::<LocaleGroupFactRuleKey, Vec<UpstreamRecord>>::new();
    let mut no_release_group_facets = BTreeSet::<DistilledFacet>::new();
    let mut inactive_records = BTreeSet::<MetadataRuleRecord>::new();
    let mut ignored_records = BTreeSet::<MetadataRuleRecord>::new();
    let source_records = records
        .iter()
        .map(|record| metadata_record(record, classify_stem(&record.stem).reason))
        .collect::<Vec<_>>();

    for record in records {
        if let Some(group_target) = classify_active_group_stem(&record.stem) {
            let facet = facet_for_record(record);
            let matchers = match record.implementation.as_str() {
                "ReleaseGroupSpecification" => distill_group_matchers(record)?,
                "ReleaseTitleSpecification" => {
                    if let Some(matcher) = distill_title_spec_group_matcher(record) {
                        vec![matcher]
                    } else {
                        ignored_records
                            .insert(metadata_record(record, "group_title_spec_not_lossless"));
                        continue;
                    }
                }
                _ => {
                    ignored_records.insert(metadata_record(record, "group_spec_not_supported"));
                    continue;
                }
            };

            for (matcher, match_kind) in matchers {
                let key = GroupRuleKey {
                    matcher,
                    match_kind,
                    tier: group_target.0,
                    facet,
                    context: group_target.1,
                };
                group_rules.entry(key).or_default().push(record.clone());
            }
            continue;
        }

        if is_localized_stem(&record.stem) {
            if let Some(source_context) = locale_group_source_context(&record.stem) {
                let matchers = match record.implementation.as_str() {
                    "ReleaseGroupSpecification" => distill_group_matchers(record)?,
                    "ReleaseTitleSpecification" if !record_is_negated(record) => {
                        if let Some(matcher) = distill_title_spec_group_matcher(record) {
                            vec![matcher]
                        } else {
                            inactive_records.insert(metadata_record(
                                record,
                                "locale_group_title_spec_not_lossless",
                            ));
                            continue;
                        }
                    }
                    "ReleaseTitleSpecification" => {
                        inactive_records
                            .insert(metadata_record(record, "locale_group_negated_title_spec"));
                        continue;
                    }
                    _ => {
                        inactive_records
                            .insert(metadata_record(record, "locale_group_spec_not_supported"));
                        continue;
                    }
                };
                for (matcher, match_kind) in matchers {
                    locale_group_fact_rules
                        .entry(LocaleGroupFactRuleKey {
                            code: locale_fact_code(&record.stem),
                            matcher,
                            match_kind,
                            facet: facet_for_record(record),
                            source_context,
                        })
                        .or_default()
                        .push(record.clone());
                }
                continue;
            }
            if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record) {
                inactive_records.insert(metadata_record(
                    record,
                    "locale_marker_requires_positive_title_spec",
                ));
                continue;
            }
            match distill_named_patterns(record, ParserSignalKindSpec::Proper) {
                Ok(patterns) => {
                    for pattern in patterns {
                        fact_rules
                            .entry(FactRuleKey {
                                code: locale_fact_code(&record.stem),
                                facet: facet_for_record(record),
                                category: CategoryScopeSpec::Any,
                                pattern,
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
                Err(_) => {
                    inactive_records.insert(metadata_record(record, "locale_fact_not_lossless"));
                }
            }
            continue;
        }

        match record.stem.as_str() {
            "amzn" | "atvp" | "cr" | "dsnp" | "funi" | "hbo" | "hidive" | "hmax" | "hulu"
            | "max" | "nf" | "pcok" | "pmtp" | "stan" => {
                for token in distill_service_alias_tokens(record)? {
                    let key = ServiceAliasKey {
                        token,
                        service: canonical_service_name(&record.stem)?.to_string(),
                    };
                    service_alias_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                }
            }
            "upscaled" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::AiEnhanced)? {
                    let key = SignalRuleKey {
                        kind: ParserSignalKindSpec::AiEnhanced,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    fact_rules
                        .entry(FactRuleKey {
                            code: "trash.ai_enhanced".to_string(),
                            facet: facet_for_record(record),
                            category: CategoryScopeSpec::Any,
                            pattern,
                        })
                        .or_default()
                        .push(record.clone());
                }
            }
            "repack-proper" | "repack2" | "repack3" => {
                for (kind, pattern) in distill_repack_patterns(record)? {
                    let key = SignalRuleKey {
                        kind,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    for code in fact_codes_for_signal(kind) {
                        fact_rules
                            .entry(FactRuleKey {
                                code: (*code).to_string(),
                                facet: facet_for_record(record),
                                category: CategoryScopeSpec::Any,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "dubs-only" => {
                for pattern in distill_dubs_only_patterns(record)? {
                    let key = SignalRuleKey {
                        kind: ParserSignalKindSpec::DubsOnly,
                        pattern: pattern.clone(),
                    };
                    signal_rules.entry(key).or_default().push(record.clone());
                    fact_rules
                        .entry(FactRuleKey {
                            code: "trash.dubs_only".to_string(),
                            facet: facet_for_record(record),
                            category: CategoryScopeSpec::Any,
                            pattern,
                        })
                        .or_default()
                        .push(record.clone());
                }
            }
            "anime-raws" => {
                for pattern in distill_blocked_title_patterns(record, "trash_guides_anime_raws")? {
                    let key = BlockedTitleKey {
                        code: "trash_guides_anime_raws".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern,
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                }
            }
            "lq-release-title" => {
                for pattern in
                    distill_blocked_title_patterns(record, "trash_guides_lq_release_title")?
                {
                    let key = BlockedTitleKey {
                        code: "trash_guides_lq_release_title".to_string(),
                        facet: facet_for_record(record),
                        category: CategoryScopeSpec::Any,
                        pattern,
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                }
            }
            "fansub" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::HardcodedSubs)?
                {
                    let key = BlockedTitleKey {
                        code: "trash_guides_fansub".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern: pattern.clone(),
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                    for code in ["trash.hardcoded_subs", "trash.fansub"] {
                        fact_rules
                            .entry(FactRuleKey {
                                code: code.to_string(),
                                facet: DistilledFacet::Anime,
                                category: CategoryScopeSpec::Anime,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "fastsub" => {
                for pattern in distill_named_patterns(record, ParserSignalKindSpec::HardcodedSubs)?
                {
                    let key = BlockedTitleKey {
                        code: "trash_guides_fastsub".to_string(),
                        facet: DistilledFacet::Anime,
                        category: CategoryScopeSpec::Anime,
                        pattern: pattern.clone(),
                    };
                    blocked_title_rules
                        .entry(key)
                        .or_default()
                        .push(record.clone());
                    for code in ["trash.hardcoded_subs", "trash.fastsub"] {
                        fact_rules
                            .entry(FactRuleKey {
                                code: code.to_string(),
                                facet: DistilledFacet::Anime,
                                category: CategoryScopeSpec::Anime,
                                pattern: pattern.clone(),
                            })
                            .or_default()
                            .push(record.clone());
                    }
                }
            }
            "scene" | "obfuscated" | "retags" => {
                if record.implementation != "ReleaseTitleSpecification" || record_is_negated(record)
                {
                    inactive_records.insert(metadata_record(
                        record,
                        "native_fact_requires_positive_title_spec",
                    ));
                    continue;
                }
                match distill_named_patterns(record, ParserSignalKindSpec::Proper) {
                    Ok(patterns) => {
                        for pattern in patterns {
                            fact_rules
                                .entry(FactRuleKey {
                                    code: native_fact_code(&record.stem).to_string(),
                                    facet: facet_for_record(record),
                                    category: CategoryScopeSpec::Any,
                                    pattern,
                                })
                                .or_default()
                                .push(record.clone());
                        }
                    }
                    Err(_) => {
                        inactive_records
                            .insert(metadata_record(record, "native_fact_not_lossless"));
                    }
                }
            }
            "no-rlsgroup" => {
                if record.implementation == "ReleaseGroupSpecification"
                    && record.value.trim() == "."
                {
                    no_release_group_facets.insert(facet_for_record(record));
                } else {
                    inactive_records.insert(metadata_record(
                        record,
                        "no_release_group_requires_wildcard_group_spec",
                    ));
                }
            }
            "hdr" | "hdr10plus-boost" | "dv-boost" | "dv-disk" | "dv-wo-hdr-fallback"
            | "hybrid" | "remaster" | "truehd-atmos" | "ddplus-atmos" | "10bit" | "10-mono"
            | "20-stereo" | "line-mic-dubbed" => {
                ignored_records.insert(metadata_record(
                    record,
                    "custom_rule_only_existing_parser_signal",
                ));
            }
            _ => {
                ignored_records.insert(metadata_record(record, "unsupported_unreviewed_stem"));
            }
        }
    }

    seed_legacy_group_rules(&mut group_rules);

    let mut group_conflict_records = BTreeSet::<MetadataRuleRecord>::new();

    Ok(DistilledCatalog {
        group_rules: collapse_group_rules(group_rules, &mut group_conflict_records),
        service_alias_rules: service_alias_rules
            .into_iter()
            .map(|(key, provenance)| ServiceAliasRecord { key, provenance })
            .collect(),
        signal_rules: signal_rules
            .into_iter()
            .map(|(key, provenance)| SignalRuleRecord { key, provenance })
            .collect(),
        blocked_title_rules: blocked_title_rules
            .into_iter()
            .map(|(key, provenance)| BlockedTitleRecord { key, provenance })
            .collect(),
        fact_rules: fact_rules
            .into_iter()
            .map(|(key, _)| FactRuleRecord { key })
            .collect(),
        locale_group_fact_rules: locale_group_fact_rules
            .into_iter()
            .map(|(key, _)| LocaleGroupFactRuleRecord { key })
            .collect(),
        no_release_group_facets: no_release_group_facets.into_iter().collect(),
        source_records,
        inactive_records: inactive_records.into_iter().collect(),
        ignored_records: ignored_records
            .into_iter()
            .chain(group_conflict_records)
            .collect(),
    })
}

fn seed_legacy_group_rules(group_rules: &mut BTreeMap<GroupRuleKey, Vec<UpstreamRecord>>) {
    for seed in LEGACY_SEED_GROUPS {
        let facets: &[DistilledFacet] = match seed.context {
            DistilledContext::Anime => &[DistilledFacet::Anime],
            DistilledContext::Any => &[
                DistilledFacet::Movie,
                DistilledFacet::Series,
                DistilledFacet::Anime,
            ],
            _ => &[DistilledFacet::Movie, DistilledFacet::Series],
        };

        for facet in facets {
            let record = UpstreamRecord {
                app: "scryer".to_string(),
                stem: "legacy-release-groups".to_string(),
                source_path: "scryer://legacy-release-groups".to_string(),
                trash_id: String::new(),
                cf_name: "Legacy release-group carry-forward".to_string(),
                spec_name: seed.name.to_string(),
                implementation: "ScryerLegacySeed".to_string(),
                value: format!("{:?}:{:?}:{:?}", seed.tier, seed.context, facet),
                trash_scores_json: "null".to_string(),
                required_json: "null".to_string(),
                negate_json: "null".to_string(),
                complete_json: "null".to_string(),
            };
            let key = GroupRuleKey {
                matcher: seed.name.to_string(),
                match_kind: GroupMatchKindSpec::Exact,
                tier: seed.tier,
                facet: *facet,
                context: seed.context,
            };
            group_rules.entry(key).or_default().push(record);
        }
    }
}

fn collapse_group_rules(
    group_rules: BTreeMap<GroupRuleKey, Vec<UpstreamRecord>>,
    ignored_records: &mut BTreeSet<MetadataRuleRecord>,
) -> Vec<GroupRuleRecord> {
    let mut grouped =
        BTreeMap::<GroupRuleIdentity, BTreeMap<DistilledTier, Vec<UpstreamRecord>>>::new();

    for (key, provenance) in group_rules {
        let identity = GroupRuleIdentity {
            matcher: key.matcher,
            match_kind: key.match_kind,
            facet: key.facet,
            context: key.context,
        };
        grouped
            .entry(identity)
            .or_default()
            .entry(key.tier)
            .or_default()
            .extend(provenance);
    }

    grouped
        .into_iter()
        .map(|(identity, tier_records)| {
            let selected_tier = select_group_tier(tier_records.keys().copied());
            let mut provenance = tier_records
                .get(&selected_tier)
                .cloned()
                .unwrap_or_default();
            provenance.sort();
            provenance.dedup();

            for (tier, records) in tier_records {
                if tier == selected_tier {
                    continue;
                }
                for record in records {
                    ignored_records.insert(metadata_record(
                        &record,
                        "group_tier_conflict_lower_precedence",
                    ));
                }
            }

            GroupRuleRecord {
                key: GroupRuleKey {
                    matcher: identity.matcher,
                    match_kind: identity.match_kind,
                    tier: selected_tier,
                    facet: identity.facet,
                    context: identity.context,
                },
                provenance,
            }
        })
        .collect()
}

fn select_group_tier(tiers: impl IntoIterator<Item = DistilledTier>) -> DistilledTier {
    let mut saw_bronze = false;
    let mut saw_silver = false;
    let mut saw_gold = false;

    for tier in tiers {
        match tier {
            DistilledTier::Banned => return DistilledTier::Banned,
            DistilledTier::Gold => saw_gold = true,
            DistilledTier::Silver => saw_silver = true,
            DistilledTier::Bronze => saw_bronze = true,
        }
    }

    if saw_gold {
        DistilledTier::Gold
    } else if saw_silver {
        DistilledTier::Silver
    } else {
        let _ = saw_bronze;
        DistilledTier::Bronze
    }
}

fn metadata_record(record: &UpstreamRecord, reason: &str) -> MetadataRuleRecord {
    MetadataRuleRecord {
        app: record.app.clone(),
        facet: facet_for_record(record),
        stem: record.stem.clone(),
        trash_id: record.trash_id.clone(),
        cf_name: record.cf_name.clone(),
        spec_name: record.spec_name.clone(),
        implementation: record.implementation.clone(),
        value: record.value.clone(),
        trash_scores_json: record.trash_scores_json.clone(),
        required_json: record.required_json.clone(),
        negate_json: record.negate_json.clone(),
        complete_json: record.complete_json.clone(),
        reason: reason.to_string(),
        source_path: record.source_path.clone(),
    }
}

fn classify_active_group_stem(stem: &str) -> Option<(DistilledTier, DistilledContext)> {
    let tier = |value: &str| -> Option<DistilledTier> {
        let tier = value.parse::<u8>().ok()?;
        Some(match tier {
            1 => DistilledTier::Gold,
            2 => DistilledTier::Silver,
            _ => DistilledTier::Bronze,
        })
    };

    if let Some(value) = stem.strip_prefix("web-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Web));
    }
    if let Some(value) = stem.strip_prefix("hd-bluray-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::BluRay));
    }
    if let Some(value) = stem.strip_prefix("uhd-bluray-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::UhdBluRay));
    }
    if let Some(value) = stem.strip_prefix("remux-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Remux));
    }
    if let Some(value) = stem.strip_prefix("anime-bd-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Anime));
    }
    if let Some(value) = stem.strip_prefix("anime-web-tier-") {
        return tier(value).map(|mapped| (mapped, DistilledContext::Anime));
    }

    match stem {
        "lq" | "bad-dual-groups" => Some((DistilledTier::Banned, DistilledContext::Any)),
        "anime-lq-groups" | "anime-dual-audio" => {
            Some((DistilledTier::Banned, DistilledContext::Anime))
        }
        _ => None,
    }
}

fn facet_for_record(record: &UpstreamRecord) -> DistilledFacet {
    if is_anime_stem(&record.stem) {
        return DistilledFacet::Anime;
    }

    match record.app.as_str() {
        "radarr" => DistilledFacet::Movie,
        "sonarr" => DistilledFacet::Series,
        _ => DistilledFacet::Series,
    }
}

fn is_anime_stem(stem: &str) -> bool {
    stem.starts_with("anime-") || matches!(stem, "fansub" | "fastsub" | "dubs-only")
}

fn is_localized_stem(stem: &str) -> bool {
    stem.starts_with("french-") || stem.starts_with("german-") || stem.starts_with("asian-")
}

fn classify_stem(stem: &str) -> StemClassification {
    if classify_active_group_stem(stem).is_some() {
        return StemClassification {
            detection_owner: DetectionOwner::ExistingNative,
            effect_binding: EffectBinding::PersonaScore,
            reason: "reviewed_release_group_tier",
        };
    }
    if is_localized_stem(stem) {
        return StemClassification {
            detection_owner: DetectionOwner::NativeFact,
            effect_binding: EffectBinding::LocaleScore,
            reason: "reviewed_locale_fact",
        };
    }
    if is_service_alias_stem(stem) {
        return StemClassification {
            detection_owner: DetectionOwner::ExistingNative,
            effect_binding: EffectBinding::Informational,
            reason: "reviewed_service_alias",
        };
    }
    if matches!(
        stem,
        "upscaled"
            | "repack-proper"
            | "repack2"
            | "repack3"
            | "dubs-only"
            | "anime-raws"
            | "lq-release-title"
            | "fansub"
            | "fastsub"
            | "scene"
            | "obfuscated"
            | "retags"
            | "no-rlsgroup"
    ) {
        return StemClassification {
            detection_owner: DetectionOwner::NativeFact,
            effect_binding: if matches!(
                stem,
                "anime-raws" | "lq-release-title" | "fansub" | "fastsub"
            ) {
                EffectBinding::HardBlock
            } else if matches!(stem, "scene" | "obfuscated" | "retags" | "no-rlsgroup") {
                EffectBinding::Informational
            } else {
                EffectBinding::PersonaScore
            },
            reason: "reviewed_native_fact",
        };
    }
    if matches!(
        stem,
        "hdr"
            | "hdr10plus-boost"
            | "dv-boost"
            | "dv-disk"
            | "dv-wo-hdr-fallback"
            | "hybrid"
            | "remaster"
            | "truehd-atmos"
            | "ddplus-atmos"
            | "10bit"
            | "10-mono"
            | "20-stereo"
            | "line-mic-dubbed"
    ) {
        return StemClassification {
            detection_owner: DetectionOwner::CustomRuleOnly,
            effect_binding: EffectBinding::None,
            reason: "existing_parser_or_scoring_signal",
        };
    }
    StemClassification {
        detection_owner: DetectionOwner::Unsupported,
        effect_binding: EffectBinding::None,
        reason: "unsupported_unreviewed_stem",
    }
}

fn is_service_alias_stem(stem: &str) -> bool {
    matches!(
        stem,
        "amzn"
            | "atvp"
            | "cr"
            | "dsnp"
            | "funi"
            | "hbo"
            | "hidive"
            | "hmax"
            | "hulu"
            | "max"
            | "nf"
            | "pcok"
            | "pmtp"
            | "stan"
    )
}

fn record_is_negated(record: &UpstreamRecord) -> bool {
    record.negate_json == "true"
}

fn native_fact_code(stem: &str) -> &'static str {
    match stem {
        "scene" => "trash.scene",
        "obfuscated" => "trash.obfuscated",
        "retags" => "trash.retagged",
        "no-rlsgroup" => "trash.no_release_group",
        _ => unreachable!("native fact code requested for unsupported stem {stem}"),
    }
}

fn fact_codes_for_signal(kind: ParserSignalKindSpec) -> &'static [&'static str] {
    match kind {
        ParserSignalKindSpec::AiEnhanced => &["trash.ai_enhanced"],
        ParserSignalKindSpec::Proper => &["trash.proper"],
        ParserSignalKindSpec::Repack => &["trash.proper", "trash.repack"],
        ParserSignalKindSpec::DubsOnly => &["trash.dubs_only"],
        ParserSignalKindSpec::HardcodedSubs => &["trash.hardcoded_subs"],
    }
}

fn locale_fact_code(stem: &str) -> String {
    let (locale, semantic) = stem
        .split_once('-')
        .expect("localized stems always include a locale separator");
    let semantic = if semantic.contains("tier-01") {
        "group.tier1".to_string()
    } else if semantic.contains("tier-02") {
        "group.tier2".to_string()
    } else if semantic.contains("tier-03") {
        "group.tier3".to_string()
    } else if semantic.contains("lq") {
        "lq".to_string()
    } else if semantic.contains("scene") {
        "scene".to_string()
    } else {
        format!("marker.{}", semantic.replace('-', "_"))
    };
    format!("trash.locale.{locale}.{semantic}")
}

fn locale_group_source_context(stem: &str) -> Option<DistilledContext> {
    if stem.contains("anime-") && stem.contains("tier-") {
        Some(DistilledContext::Anime)
    } else if stem.contains("remux-tier-") {
        Some(DistilledContext::Remux)
    } else if stem.contains("uhd-bluray-tier-") {
        Some(DistilledContext::UhdBluRay)
    } else if stem.contains("bluray-tier-") {
        Some(DistilledContext::BluRay)
    } else if stem.contains("web-tier-") {
        Some(DistilledContext::Web)
    } else if stem.starts_with("asian-tier-") {
        Some(DistilledContext::Any)
    } else {
        None
    }
}

fn distill_group_matchers(record: &UpstreamRecord) -> Result<Vec<(String, GroupMatchKindSpec)>> {
    let pattern = record.value.trim();
    if pattern.is_empty() {
        bail!("empty release-group pattern for {}", record.source_path);
    }

    if pattern == r"Pahe(\.(ph|in))?\b" {
        return Ok(vec![
            ("Pahe".to_string(), GroupMatchKindSpec::Exact),
            ("Pahe.ph".to_string(), GroupMatchKindSpec::Exact),
            ("Pahe.in".to_string(), GroupMatchKindSpec::Exact),
        ]);
    }

    let inner = strip_group_anchors(pattern);
    if inner.ends_with(".*") && !inner[..inner.len() - 2].contains('|') {
        return Ok(vec![(
            unescape_group_literal(inner.trim_end_matches(".*"))?,
            GroupMatchKindSpec::Prefix,
        )]);
    }

    if inner.starts_with('(') && inner.ends_with(')') {
        let inner = &inner[1..inner.len() - 1];
        let alternatives = split_top_level_alternatives(inner);
        if alternatives.len() > 1 {
            return alternatives
                .into_iter()
                .map(|value| {
                    if value.ends_with(".*") {
                        Ok((
                            unescape_group_literal(value.trim_end_matches(".*"))?,
                            GroupMatchKindSpec::Prefix,
                        ))
                    } else {
                        Ok((unescape_group_literal(value)?, GroupMatchKindSpec::Exact))
                    }
                })
                .collect();
        }

        if let Some(value) = alternatives.first() {
            if value.ends_with(".*") {
                return Ok(vec![(
                    unescape_group_literal(value.trim_end_matches(".*"))?,
                    GroupMatchKindSpec::Prefix,
                )]);
            }

            return Ok(vec![(
                unescape_group_literal(value)?,
                GroupMatchKindSpec::Exact,
            )]);
        }
    }

    Ok(vec![(
        unescape_group_literal(inner)?,
        GroupMatchKindSpec::Exact,
    )])
}

fn distill_title_spec_group_matcher(
    record: &UpstreamRecord,
) -> Option<(String, GroupMatchKindSpec)> {
    let expected = sanitize_token(&record.spec_name)?;

    for alternative in split_top_level_alternatives(&record.value) {
        let mut candidate = alternative.trim();
        candidate = candidate.strip_prefix(r"\b").unwrap_or(candidate);
        candidate = candidate.strip_suffix(r"\b").unwrap_or(candidate);
        candidate = candidate.strip_prefix('(').unwrap_or(candidate);
        candidate = candidate.strip_suffix(')').unwrap_or(candidate);
        candidate = candidate.strip_prefix(r"\[").unwrap_or(candidate);
        candidate = candidate.strip_suffix(r"\]").unwrap_or(candidate);
        candidate = candidate.strip_prefix('-').unwrap_or(candidate);
        candidate = candidate.trim();
        candidate = candidate
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();

        let normalized = candidate
            .replace("[ .-]?", "")
            .replace("[ ._]?", "")
            .replace("[.-]?", "")
            .replace("[ _-]?", "");

        if normalized.contains(['|', '(', ')', '[', ']', '?', '+', '*', '{', '}']) {
            continue;
        }

        if sanitize_token(&normalized).as_deref() == Some(expected.as_str()) {
            return Some((record.spec_name.clone(), GroupMatchKindSpec::Exact));
        }
    }

    None
}

fn strip_group_anchors(pattern: &str) -> &str {
    let stripped = pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .strip_suffix('$')
        .unwrap_or(pattern);
    stripped
        .strip_prefix(r"\b")
        .unwrap_or(stripped)
        .strip_suffix(r"\b")
        .unwrap_or(stripped)
}

fn split_top_level_alternatives(input: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut depth = 0_i32;
    let mut escaped = false;
    let mut start = 0_usize;

    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                output.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    output.push(&input[start..]);
    output
}

fn unescape_group_literal(input: &str) -> Result<String> {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| anyhow!("unterminated escape in group pattern {input:?}"))?;
                match next {
                    '.' | '&' | '+' | '-' | '_' | '\'' | ' ' | '[' | ']' | '(' | ')' => {
                        output.push(next)
                    }
                    'b' => {}
                    other => output.push(other),
                }
            }
            '(' | ')' | '?' => {}
            _ => output.push(ch),
        }
    }
    let trimmed = output.trim();
    if trimmed.is_empty() {
        bail!("group literal reduced to empty string from {input:?}");
    }
    Ok(trimmed.to_string())
}

fn canonical_service_name(stem: &str) -> Result<&'static str> {
    match stem {
        "amzn" => Ok("Amazon"),
        "atvp" => Ok("Apple TV+"),
        "cr" => Ok("Crunchyroll"),
        "dsnp" => Ok("Disney+"),
        "funi" => Ok("Funimation"),
        "hbo" | "hmax" | "max" => Ok("HBO Max"),
        "hidive" => Ok("HIDIVE"),
        "hulu" => Ok("Hulu"),
        "nf" => Ok("Netflix"),
        "pcok" => Ok("Peacock"),
        "pmtp" => Ok("Paramount+"),
        "stan" => Ok("Stan"),
        _ => bail!("unknown service stem {stem}"),
    }
}

fn distill_service_alias_tokens(record: &UpstreamRecord) -> Result<Vec<String>> {
    let mut tokens = BTreeSet::new();
    if let Some(token) = sanitize_token(&record.spec_name) {
        tokens.insert(token);
    }

    for extracted in extract_explicit_tokens(&record.value) {
        tokens.insert(extracted);
    }

    if tokens.is_empty() {
        bail!(
            "failed to distill service alias tokens for {}",
            record.source_path
        );
    }

    Ok(tokens.into_iter().collect())
}

fn distill_named_patterns(
    record: &UpstreamRecord,
    _kind: ParserSignalKindSpec,
) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    if let Some(pattern) = pattern_from_spec_name(&record.spec_name) {
        patterns.insert(pattern);
    }
    if let Some(pattern) = pattern_from_spec_value(&record.value) {
        patterns.insert(pattern);
    }

    match record.stem.as_str() {
        "upscaled" if record.spec_name.eq_ignore_ascii_case("AI Upscales") => {
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["AI".to_string(), "ENHANCED".to_string()],
            });
        }
        "upscaled" if record.spec_name.eq_ignore_ascii_case("Upscaled") => {
            for token in ["UPSCALED", "UPREZ"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
        }
        "repack-proper" if record.spec_name.eq_ignore_ascii_case("Repack/Proper/Rerip") => {
            for token in ["PROPER", "REPACK", "RERIP"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
        }
        "repack-proper"
            if record
                .spec_name
                .eq_ignore_ascii_case("Not Higher Version Repack/Proper") =>
        {
            for token in ["REPACK2", "REPACK3"] {
                patterns.insert(TokenPatternSpec {
                    kind: TokenPatternKindSpec::Sequence,
                    tokens: vec![token.to_string()],
                });
            }
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["REAL".to_string(), "PROPER".to_string()],
            });
            patterns.insert(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens: vec!["REAL".to_string(), "REPACK".to_string()],
            });
        }
        _ => {}
    }

    if patterns.is_empty() {
        bail!(
            "failed to distill parser token patterns for {} {}",
            record.source_path,
            record.spec_name
        );
    }

    Ok(patterns.into_iter().collect())
}

fn distill_repack_patterns(
    record: &UpstreamRecord,
) -> Result<Vec<(ParserSignalKindSpec, TokenPatternSpec)>> {
    let mut output = Vec::new();
    for pattern in distill_named_patterns(record, ParserSignalKindSpec::Proper)? {
        let kind = if pattern
            .tokens
            .iter()
            .any(|token| token == "REPACK" || token == "RERIP")
        {
            ParserSignalKindSpec::Repack
        } else {
            ParserSignalKindSpec::Proper
        };
        output.push((kind, pattern));
    }
    Ok(output)
}

fn distill_dubs_only_patterns(record: &UpstreamRecord) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    if record.spec_name.eq_ignore_ascii_case("Dubbed") {
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::Sequence,
            tokens: vec!["DUB".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::Sequence,
            tokens: vec!["DUBBED".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["ENG".to_string(), "DUB".to_string()],
        });
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["FUNI".to_string(), "DUB".to_string()],
        });
    } else if let Some(pattern) = pattern_from_spec_name(&record.spec_name) {
        if pattern.tokens.as_slice() == ["KS"] {
            return Ok(Vec::new());
        }
        patterns.insert(pattern);
    }

    Ok(patterns.into_iter().collect())
}

fn distill_blocked_title_patterns(
    record: &UpstreamRecord,
    _code: &str,
) -> Result<Vec<TokenPatternSpec>> {
    let mut patterns = BTreeSet::new();
    if let Some(pattern) = pattern_from_spec_name(&record.spec_name) {
        patterns.insert(pattern);
    }
    if let Some(pattern) = pattern_from_spec_value(&record.value) {
        patterns.insert(pattern);
    }

    if record.spec_name.starts_with("BiTOR") {
        patterns.insert(TokenPatternSpec {
            kind: TokenPatternKindSpec::RequiredTokens,
            tokens: vec!["2160P".to_string(), "BITOR".to_string()],
        });
    }

    if patterns.is_empty() {
        bail!(
            "failed to distill blocked-title patterns for {} {}",
            record.source_path,
            record.spec_name
        );
    }

    Ok(patterns.into_iter().collect())
}

fn pattern_from_spec_name(name: &str) -> Option<TokenPatternSpec> {
    let cleaned = name
        .replace("(2160p)", " 2160p")
        .replace("(Not Dual Audio)", "")
        .replace("Rename", "")
        .replace(['/', '-', '_'], " ");
    let tokens = cleaned
        .split_whitespace()
        .filter_map(sanitize_token)
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }

    Some(TokenPatternSpec {
        kind: TokenPatternKindSpec::Sequence,
        tokens,
    })
}

fn pattern_from_spec_value(value: &str) -> Option<TokenPatternSpec> {
    if value.contains("(?=.*") {
        let tokens = extract_boundary_tokens(value);
        if tokens.len() >= 2 {
            return Some(TokenPatternSpec {
                kind: TokenPatternKindSpec::RequiredTokens,
                tokens,
            });
        }
    }

    if value.contains("[ ._-]?") || value.contains("[ ._-]") {
        let cleaned = value
            .replace(r"\b", " ")
            .replace(r"[ ._-]?", " ")
            .replace(r"[ ._-]", " ")
            .replace(['(', ')', '[', ']', '^', '$', '?', '*', '|'], " ");
        let tokens = cleaned
            .split_whitespace()
            .filter_map(sanitize_token)
            .collect::<Vec<_>>();
        if tokens.len() >= 2 {
            return Some(TokenPatternSpec {
                kind: TokenPatternKindSpec::Sequence,
                tokens,
            });
        }
    }

    None
}

fn sanitize_token(raw: &str) -> Option<String> {
    let token = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect::<String>();
    if token.is_empty() { None } else { Some(token) }
}

fn extract_explicit_tokens(pattern: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut current = String::new();
    for ch in pattern.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_uppercase());
            continue;
        }
        if let Some(token) = normalize_extracted_token(&current) {
            tokens.insert(token);
        }
        current.clear();
    }
    if let Some(token) = normalize_extracted_token(&current) {
        tokens.insert(token);
    }
    tokens
}

fn extract_boundary_tokens(pattern: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut input = pattern;
    while let Some(start) = input.find(r"\b") {
        let after_start = &input[start + 2..];
        let Some(end) = after_start.find(r"\b") else {
            break;
        };
        if let Some(token) = sanitize_token(&after_start[..end])
            && !tokens.contains(&token)
        {
            tokens.push(token);
        }
        input = &after_start[end + 2..];
    }
    tokens
}

fn normalize_extracted_token(token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }

    let blocklist = [
        "B",
        "DL",
        "RIP",
        "WEB",
        "JSON",
        "CF",
        "TITLE",
        "SOURCE",
        "SPECIFICATION",
        "VALUE",
        "LOOKBEHIND",
        "LOOKAHEAD",
        "TRUE",
        "FALSE",
        "HEVC",
    ];
    if blocklist.contains(&token) {
        return None;
    }

    Some(token.to_string())
}

fn write_if_changed(path: &Path, next: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if fs::read_to_string(path).ok().as_deref() == Some(next) {
        return Ok(());
    }

    fs::write(path, next).with_context(|| format!("failed to write {}", path.display()))
}

fn format_generated_rust(ctx: &TaskContext) -> Result<()> {
    let mut command = ctx.command_in("cargo", &ctx.repo_root);
    command.args(["fmt", "--all", "--", QUALITY_OUTPUT, PARSER_OUTPUT]);
    run_checked(&mut command).context("failed to rustfmt generated TRaSH outputs")
}

fn render_quality_output(
    catalog: &DistilledCatalog,
    synced_at: &str,
    source_revision: &str,
) -> Result<String> {
    let mut output = String::new();
    writeln!(
        output,
        "// Generated by `cargo xtask trash-guides sync`.\n// Do not edit by hand.\n"
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SYNCED_AT: &str = {};",
        rust_str(synced_at)
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SOURCE_REVISION: &str = {};",
        rust_str(source_revision)
    )?;
    writeln!(output)?;
    writeln!(output, "pub static GROUP_RULES: &[GroupRule] = &[")?;
    for rule in &catalog.group_rules {
        writeln!(
            output,
            "    GroupRule {{ matcher: {}, match_kind: GroupMatchKind::{}, entry: GroupEntry {{ name: {}, tier: GroupTier::{}, facet: RuleFacet::{}, source_context: SourceContext::{} }} }},",
            rust_str(&rule.key.matcher),
            match rule.key.match_kind {
                GroupMatchKindSpec::Exact => "Exact",
                GroupMatchKindSpec::Prefix => "Prefix",
            },
            rust_str(&rule.key.matcher),
            render_tier(rule.key.tier),
            render_facet(rule.key.facet),
            render_context(rule.key.context),
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "#[allow(dead_code)]\npub static ACTIVE_GROUP_RULE_METADATA: &[TrashGuideRuleMetadata] = &["
    )?;
    for rule in &catalog.group_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    TrashGuideRuleMetadata {{ matcher: {}, match_kind: GroupMatchKind::{}, tier: GroupTier::{}, facet: RuleFacet::{}, source_context: SourceContext::{}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                rust_str(&rule.key.matcher),
                match rule.key.match_kind {
                    GroupMatchKindSpec::Exact => "Exact",
                    GroupMatchKindSpec::Prefix => "Prefix",
                },
                render_tier(rule.key.tier),
                render_facet(rule.key.facet),
                render_context(rule.key.context),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    render_metadata_rules(
        &mut output,
        "INACTIVE_GROUP_RULES",
        &catalog.inactive_records,
        Some("localized_preserved_inactive"),
    )?;
    writeln!(output)?;
    render_metadata_rules(
        &mut output,
        "IGNORED_GROUP_RULES",
        &catalog.ignored_records,
        None,
    )?;

    Ok(output)
}

fn render_parser_output(
    catalog: &DistilledCatalog,
    synced_at: &str,
    source_revision: &str,
) -> Result<String> {
    let mut output = String::new();
    writeln!(
        output,
        "// Generated by `cargo xtask trash-guides sync`.\n// Do not edit by hand.\n"
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SYNCED_AT: &str = {};",
        rust_str(synced_at)
    )?;
    writeln!(
        output,
        "#[allow(dead_code)]\npub const TRASH_GUIDES_SOURCE_REVISION: &str = {};",
        rust_str(source_revision)
    )?;
    writeln!(output)?;

    writeln!(
        output,
        "pub static SERVICE_ALIAS_RULES: &[ServiceAliasRule] = &["
    )?;
    for rule in &catalog.service_alias_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    ServiceAliasRule {{ token: {}, service: {}, facet: RuleFacet::{}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                rust_str(&rule.key.token),
                rust_str(&rule.key.service),
                render_facet(facet_for_record(provenance)),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    writeln!(output, "pub static FACT_RULES: &[FactRule] = &[")?;
    for rule in &catalog.fact_rules {
        writeln!(
            output,
            "    FactRule {{ code: {}, facet: RuleFacet::{}, category: TitleCategoryScope::{}, pattern: {} }},",
            rust_str(&rule.key.code),
            render_facet(rule.key.facet),
            match rule.key.category {
                CategoryScopeSpec::Any => "Any",
                CategoryScopeSpec::Anime => "Anime",
            },
            render_token_pattern(&rule.key.pattern),
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "pub static LOCALE_GROUP_FACT_RULES: &[LocaleGroupFactRule] = &["
    )?;
    for rule in &catalog.locale_group_fact_rules {
        writeln!(
            output,
            "    LocaleGroupFactRule {{ code: {}, matcher: {}, match_kind: LocaleGroupMatchKind::{}, facet: RuleFacet::{}, source_context: LocaleSourceContext::{} }},",
            rust_str(&rule.key.code),
            rust_str(&rule.key.matcher),
            match rule.key.match_kind {
                GroupMatchKindSpec::Exact => "Exact",
                GroupMatchKindSpec::Prefix => "Prefix",
            },
            render_facet(rule.key.facet),
            render_context(rule.key.source_context),
        )?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "pub static NO_RELEASE_GROUP_FACT_FACETS: &[RuleFacet] = &["
    )?;
    for facet in &catalog.no_release_group_facets {
        writeln!(output, "    RuleFacet::{},", render_facet(*facet))?;
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "pub static TOKEN_SIGNAL_RULES: &[TokenSignalRule] = &["
    )?;
    for rule in &catalog.signal_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    TokenSignalRule {{ kind: ParserSignalKind::{}, pattern: {}, facet: RuleFacet::{}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                render_signal_kind(rule.key.kind),
                render_token_pattern(&rule.key.pattern),
                render_facet(facet_for_record(provenance)),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    writeln!(
        output,
        "pub static BLOCKED_TITLE_RULES: &[BlockedTitleRule] = &["
    )?;
    for rule in &catalog.blocked_title_rules {
        for provenance in &rule.provenance {
            writeln!(
                output,
                "    BlockedTitleRule {{ code: {}, facet: RuleFacet::{}, category: TitleCategoryScope::{}, pattern: {}, app: {}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, source_path: {} }},",
                rust_str(&rule.key.code),
                render_facet(rule.key.facet),
                match rule.key.category {
                    CategoryScopeSpec::Any => "Any",
                    CategoryScopeSpec::Anime => "Anime",
                },
                render_token_pattern(&rule.key.pattern),
                rust_str(&provenance.app),
                rust_str(&provenance.stem),
                rust_str(&provenance.trash_id),
                rust_str(&provenance.cf_name),
                rust_str(&provenance.spec_name),
                rust_str(&provenance.source_path),
            )?;
        }
    }
    writeln!(output, "];\n")?;

    render_metadata_rules(
        &mut output,
        "INACTIVE_PARSER_RULES",
        &catalog.inactive_records,
        None,
    )?;
    writeln!(output)?;
    render_metadata_rules(
        &mut output,
        "IGNORED_PARSER_RULES",
        &catalog.ignored_records,
        None,
    )?;
    writeln!(output)?;
    render_metadata_rules(
        &mut output,
        "UPSTREAM_RECORDS",
        &catalog.source_records,
        None,
    )?;

    Ok(output)
}

fn render_metadata_rules(
    output: &mut String,
    name: &str,
    records: &[MetadataRuleRecord],
    only_reason: Option<&str>,
) -> Result<()> {
    writeln!(
        output,
        "#[allow(dead_code)]\npub static {name}: &[MetadataRuleRecord] = &["
    )?;
    for record in records {
        if let Some(reason) = only_reason
            && record.reason != reason
        {
            continue;
        }
        writeln!(
            output,
            "    MetadataRuleRecord {{ app: {}, facet: RuleFacet::{}, stem: {}, trash_id: {}, cf_name: {}, spec_name: {}, implementation: {}, value: {}, trash_scores_json: {}, required_json: {}, negate_json: {}, complete_json: {}, reason: {}, source_path: {} }},",
            rust_str(&record.app),
            render_facet(record.facet),
            rust_str(&record.stem),
            rust_str(&record.trash_id),
            rust_str(&record.cf_name),
            rust_str(&record.spec_name),
            rust_str(&record.implementation),
            rust_str(&record.value),
            rust_str(&record.trash_scores_json),
            rust_str(&record.required_json),
            rust_str(&record.negate_json),
            rust_str(&record.complete_json),
            rust_str(&record.reason),
            rust_str(&record.source_path),
        )?;
    }
    writeln!(output, "];")?;
    Ok(())
}

fn render_summary(catalog: &DistilledCatalog, synced_at: &str, source_revision: &str) -> String {
    let mut output = String::new();
    let (movie_groups, series_groups, anime_groups) =
        count_group_rules_by_facet(&catalog.group_rules);
    let (movie_titles, series_titles, anime_titles) =
        count_blocked_title_rules_by_facet(&catalog.blocked_title_rules);
    let _ = writeln!(output, "TRaSH Guides sync summary");
    let _ = writeln!(output, "Synced at: {synced_at}");
    let _ = writeln!(output, "Source revision: {source_revision}");
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "Active release-group rules: {}",
        catalog.group_rules.len()
    );
    let _ = writeln!(
        output,
        "  by facet: movies={movie_groups}, series={series_groups}, anime={anime_groups}"
    );
    let _ = writeln!(
        output,
        "Active service aliases: {}",
        catalog.service_alias_rules.len()
    );
    let _ = writeln!(
        output,
        "Active parser token signals: {}",
        catalog.signal_rules.len()
    );
    let _ = writeln!(output, "Active guide facts: {}", catalog.fact_rules.len());
    let _ = writeln!(
        output,
        "Active blocked title rules: {}",
        catalog.blocked_title_rules.len()
    );
    let _ = writeln!(
        output,
        "  by facet: movies={movie_titles}, series={series_titles}, anime={anime_titles}"
    );
    let _ = writeln!(
        output,
        "Preserved inactive records: {}",
        catalog.inactive_records.len()
    );
    let _ = writeln!(output, "Ignored records: {}", catalog.ignored_records.len());
    output
}

fn render_stem_manifest(
    records: &[UpstreamRecord],
    synced_at: &str,
    source_revision: &str,
) -> Result<String> {
    let mut stems = BTreeMap::<(&str, &str), StemClassification>::new();
    for record in records {
        stems
            .entry((&record.app, &record.stem))
            .or_insert_with(|| classify_stem(&record.stem));
    }

    let manifest = StemClassificationManifest {
        source_revision: source_revision.to_string(),
        synced_at: synced_at.to_string(),
        stems: stems
            .into_iter()
            .map(
                |((app, stem), classification)| StemClassificationManifestRecord {
                    app: app.to_string(),
                    stem: stem.to_string(),
                    detection_owner: classification.detection_owner,
                    effect_binding: classification.effect_binding,
                    reason: classification.reason.to_string(),
                },
            )
            .collect(),
    };
    serde_json::to_string_pretty(&manifest)
        .map(|mut rendered| {
            rendered.push('\n');
            rendered
        })
        .context("failed to serialize TRaSH stem classification manifest")
}

fn enforce_stem_coverage(manifest_path: &Path, records: &[UpstreamRecord]) -> Result<()> {
    let accepts_new_stems = accepts_new_stems();
    let known_stems = match fs::read_to_string(manifest_path) {
        Ok(content) => serde_json::from_str::<StemClassificationManifest>(&content)
            .with_context(|| {
                format!(
                    "failed to parse reviewed TRaSH stem manifest {}",
                    manifest_path.display()
                )
            })?
            .stems
            .into_iter()
            .map(|record| (record.app, record.stem))
            .collect::<BTreeSet<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && accepts_new_stems => {
            BTreeSet::new()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "reviewed TRaSH stem manifest {} is missing; set {ACCEPT_STEMS_ENV}=1 only to bootstrap or explicitly accept the fetched inventory",
                manifest_path.display()
            )
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", manifest_path.display()));
        }
    };

    let unexpected = unexpected_stems(&known_stems, records);
    if !unexpected.is_empty() && !accepts_new_stems {
        let rendered = unexpected
            .iter()
            .map(|(app, stem)| format!("{app}/{stem}"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "unreviewed TRaSH stems detected: {rendered}; classify them in {} or set {ACCEPT_STEMS_ENV}=1 to intentionally regenerate the reviewed inventory",
            manifest_path.display()
        );
    }
    Ok(())
}

fn unexpected_stems(
    known_stems: &BTreeSet<(String, String)>,
    records: &[UpstreamRecord],
) -> BTreeSet<(String, String)> {
    records
        .iter()
        .map(|record| (record.app.clone(), record.stem.clone()))
        .filter(|stem| !known_stems.contains(stem))
        .collect()
}

fn accepts_new_stems() -> bool {
    matches!(
        std::env::var(ACCEPT_STEMS_ENV).as_deref(),
        Ok("1" | "true" | "TRUE")
    )
}

fn count_group_rules_by_facet(rules: &[GroupRuleRecord]) -> (usize, usize, usize) {
    let mut movie = 0;
    let mut series = 0;
    let mut anime = 0;

    for rule in rules {
        match rule.key.facet {
            DistilledFacet::Movie => movie += 1,
            DistilledFacet::Series => series += 1,
            DistilledFacet::Anime => anime += 1,
        }
    }

    (movie, series, anime)
}

fn count_blocked_title_rules_by_facet(rules: &[BlockedTitleRecord]) -> (usize, usize, usize) {
    let mut movie = 0;
    let mut series = 0;
    let mut anime = 0;

    for rule in rules {
        match rule.key.facet {
            DistilledFacet::Movie => movie += 1,
            DistilledFacet::Series => series += 1,
            DistilledFacet::Anime => anime += 1,
        }
    }

    (movie, series, anime)
}

fn render_tier(tier: DistilledTier) -> &'static str {
    match tier {
        DistilledTier::Gold => "Gold",
        DistilledTier::Silver => "Silver",
        DistilledTier::Bronze => "Bronze",
        DistilledTier::Banned => "Banned",
    }
}

fn render_context(context: DistilledContext) -> &'static str {
    match context {
        DistilledContext::Web => "Web",
        DistilledContext::BluRay => "BluRay",
        DistilledContext::UhdBluRay => "UhdBluRay",
        DistilledContext::Remux => "Remux",
        DistilledContext::Anime => "Anime",
        DistilledContext::Any => "Any",
    }
}

fn render_facet(facet: DistilledFacet) -> &'static str {
    match facet {
        DistilledFacet::Movie => "Movie",
        DistilledFacet::Series => "Series",
        DistilledFacet::Anime => "Anime",
    }
}

fn render_signal_kind(kind: ParserSignalKindSpec) -> &'static str {
    match kind {
        ParserSignalKindSpec::AiEnhanced => "AiEnhanced",
        ParserSignalKindSpec::Proper => "Proper",
        ParserSignalKindSpec::Repack => "Repack",
        ParserSignalKindSpec::DubsOnly => "DubsOnly",
        ParserSignalKindSpec::HardcodedSubs => "HardcodedSubs",
    }
}

fn render_token_pattern(pattern: &TokenPatternSpec) -> String {
    let tokens = pattern
        .tokens
        .iter()
        .map(|token| rust_str(token))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "TokenPattern {{ kind: TokenPatternKind::{}, tokens: &[{}] }}",
        match pattern.kind {
            TokenPatternKindSpec::Sequence => "Sequence",
            TokenPatternKindSpec::RequiredTokens => "RequiredTokens",
        },
        tokens
    )
}

fn rust_str(value: &str) -> String {
    format!("{value:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_maps_to_release_group_contexts() {
        assert_eq!(
            classify_active_group_stem("web-tier-01"),
            Some((DistilledTier::Gold, DistilledContext::Web))
        );
        assert_eq!(
            classify_active_group_stem("uhd-bluray-tier-03"),
            Some((DistilledTier::Bronze, DistilledContext::UhdBluRay))
        );
        assert_eq!(
            classify_active_group_stem("anime-lq-groups"),
            Some((DistilledTier::Banned, DistilledContext::Anime))
        );
    }

    #[test]
    fn record_facets_follow_app_and_anime_rule_boundaries() {
        let movie = UpstreamRecord {
            app: "radarr".to_string(),
            stem: "web-tier-01".to_string(),
            source_path: String::new(),
            trash_id: String::new(),
            cf_name: String::new(),
            spec_name: String::new(),
            implementation: String::new(),
            value: String::new(),
            ..Default::default()
        };
        assert_eq!(facet_for_record(&movie), DistilledFacet::Movie);

        let series = UpstreamRecord {
            app: "sonarr".to_string(),
            ..movie.clone()
        };
        assert_eq!(facet_for_record(&series), DistilledFacet::Series);

        let anime = UpstreamRecord {
            stem: "anime-web-tier-01".to_string(),
            ..series
        };
        assert_eq!(facet_for_record(&anime), DistilledFacet::Anime);
    }

    #[test]
    fn release_group_regex_expansion_handles_alias_and_prefix_cases() {
        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "web-tier-01".to_string(),
            source_path: "docs/json/sonarr/cf/web-tier-01.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "WEB Tier 01".to_string(),
            spec_name: "APEX".to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            value: "^(APEX|PAXA|PEXA|XEPA)$".to_string(),
            ..Default::default()
        };
        let expanded = distill_group_matchers(&record).expect("expand aliases");
        assert_eq!(
            expanded,
            vec![
                ("APEX".to_string(), GroupMatchKindSpec::Exact),
                ("PAXA".to_string(), GroupMatchKindSpec::Exact),
                ("PEXA".to_string(), GroupMatchKindSpec::Exact),
                ("XEPA".to_string(), GroupMatchKindSpec::Exact),
            ]
        );

        let prefix_record = UpstreamRecord {
            value: "^(alfaHD.*)$".to_string(),
            ..record
        };
        let expanded_prefix = distill_group_matchers(&prefix_record).expect("expand prefix");
        assert_eq!(
            expanded_prefix,
            vec![("alfaHD".to_string(), GroupMatchKindSpec::Prefix)]
        );
    }

    #[test]
    fn release_group_regex_expansion_handles_pahe_suffixes() {
        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "lq".to_string(),
            source_path: "docs/json/sonarr/cf/lq.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "LQ".to_string(),
            spec_name: "Pahe".to_string(),
            implementation: "ReleaseGroupSpecification".to_string(),
            value: r"Pahe(\.(ph|in))?\b".to_string(),
            ..Default::default()
        };
        let expanded = distill_group_matchers(&record).expect("expand pahe");
        assert_eq!(
            expanded,
            vec![
                ("Pahe".to_string(), GroupMatchKindSpec::Exact),
                ("Pahe.ph".to_string(), GroupMatchKindSpec::Exact),
                ("Pahe.in".to_string(), GroupMatchKindSpec::Exact),
            ]
        );
    }

    #[test]
    fn title_spec_group_distillation_activates_lossless_anime_groups() {
        let exact_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "anime-lq-groups".to_string(),
            source_path: "docs/json/sonarr/cf/anime-lq-groups.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Anime LQ Groups".to_string(),
            spec_name: "AnimeRG".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\b(AnimeRG)\b".to_string(),
            ..Default::default()
        };
        assert_eq!(
            distill_title_spec_group_matcher(&exact_record),
            Some(("AnimeRG".to_string(), GroupMatchKindSpec::Exact))
        );

        let bracket_record = UpstreamRecord {
            spec_name: "Cleo".to_string(),
            value: r"\[Cleo\]|-Cleo".to_string(),
            ..exact_record
        };
        assert_eq!(
            distill_title_spec_group_matcher(&bracket_record),
            Some(("Cleo".to_string(), GroupMatchKindSpec::Exact))
        );

        let complex_record = UpstreamRecord {
            spec_name: "Fish".to_string(),
            value: r"\b((Baked|Dead|Space)Fish)\b".to_string(),
            ..bracket_record
        };
        assert_eq!(distill_title_spec_group_matcher(&complex_record), None);
    }

    #[test]
    fn group_tier_conflicts_resolve_deterministically() {
        assert_eq!(
            select_group_tier([DistilledTier::Bronze, DistilledTier::Silver]),
            DistilledTier::Silver
        );
        assert_eq!(
            select_group_tier([DistilledTier::Silver, DistilledTier::Gold]),
            DistilledTier::Gold
        );
        assert_eq!(
            select_group_tier([DistilledTier::Gold, DistilledTier::Banned]),
            DistilledTier::Banned
        );
    }

    #[test]
    fn parser_rule_distillation_covers_service_and_upscaled_patterns() {
        let service_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "max".to_string(),
            source_path: "docs/json/sonarr/cf/max.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "MAX".to_string(),
            spec_name: "MAX Rename".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\[(MAX)\b|\b(MAX)\]".to_string(),
            ..Default::default()
        };
        let aliases = distill_service_alias_tokens(&service_record).expect("service aliases");
        assert!(aliases.contains(&"MAX".to_string()));

        let upscaled_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "upscaled".to_string(),
            source_path: "docs/json/sonarr/cf/upscaled.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Upscaled".to_string(),
            spec_name: "TheUpscaler".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\b(The[ ._-]?Upscaler)\b".to_string(),
            ..Default::default()
        };
        let patterns = distill_named_patterns(&upscaled_record, ParserSignalKindSpec::AiEnhanced)
            .expect("upscaled patterns");
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.tokens.as_slice() == ["THEUPSCALER"])
        );
    }

    #[test]
    fn blocked_title_patterns_handle_sequence_and_required_tokens() {
        let anime_raw_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "anime-raws".to_string(),
            source_path: "docs/json/sonarr/cf/anime-raws.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "Anime Raws".to_string(),
            spec_name: "AsukaRaws".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: "Asuka[ ._-]?(Raws)".to_string(),
            ..Default::default()
        };
        let patterns = distill_blocked_title_patterns(&anime_raw_record, "trash_guides_anime_raws")
            .expect("anime raw patterns");
        assert!(patterns.iter().any(|pattern| {
            pattern.tokens.as_slice() == ["ASUKARAWS"]
                || pattern.tokens.as_slice() == ["ASUKA", "RAWS"]
        }));

        let lq_record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "lq-release-title".to_string(),
            source_path: "docs/json/sonarr/cf/lq-release-title.json".to_string(),
            trash_id: "abc".to_string(),
            cf_name: "LQ".to_string(),
            spec_name: "BiTOR (2160p)".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: "(?=.*?(\\b2160p\\b))(?=.*?(\\bBiTOR\\b))".to_string(),
            ..Default::default()
        };
        let lq_patterns =
            distill_blocked_title_patterns(&lq_record, "trash_guides_lq_release_title")
                .expect("lq title patterns");
        assert!(lq_patterns.iter().any(|pattern| {
            pattern.kind == TokenPatternKindSpec::RequiredTokens
                && pattern.tokens.as_slice() == ["2160P", "BITOR"]
        }));
    }

    #[test]
    fn top_level_specification_polarity_and_fields_json_are_preserved() {
        let upstream = serde_json::from_str::<UpstreamCf>(
            r#"{
                "name": "Scene",
                "trash_id": "scene-id",
                "trash_scores": {"default": -100},
                "specifications": [{
                    "name": "Not GERMAN",
                    "implementation": "ReleaseTitleSpecification",
                    "required": true,
                    "negate": true,
                    "fields": {"value": "\\bGERMAN\\b", "extra": ["kept"]}
                }]
            }"#,
        )
        .expect("fixture should deserialize");
        let spec = upstream
            .specifications
            .first()
            .expect("fixture specification");
        assert_eq!(json_string(Some(&spec.required)), "true");
        assert_eq!(json_string(Some(&spec.negate)), "true");
        let complete_fields = serde_json::from_str::<Value>(&json_string(Some(&spec.fields)))
            .expect("complete fields JSON");
        assert_eq!(complete_fields, spec.fields);

        let record = UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "scene".to_string(),
            implementation: "ReleaseTitleSpecification".to_string(),
            value: r"\bGERMAN\b".to_string(),
            negate_json: json_string(Some(&spec.negate)),
            ..Default::default()
        };
        let catalog = distill_records(&[record]).expect("distill negated scene");
        assert!(catalog.fact_rules.is_empty());
        assert!(
            catalog
                .inactive_records
                .iter()
                .any(|record| record.reason == "native_fact_requires_positive_title_spec")
        );
    }

    #[test]
    fn coverage_check_rejects_new_stems() {
        let records = vec![UpstreamRecord {
            app: "sonarr".to_string(),
            stem: "new-upstream-stem".to_string(),
            ..Default::default()
        }];
        assert_eq!(
            unexpected_stems(&BTreeSet::new(), &records),
            BTreeSet::from([("sonarr".to_string(), "new-upstream-stem".to_string())])
        );
    }
}
