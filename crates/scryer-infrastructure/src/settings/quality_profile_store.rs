use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, QualityProfile, QualityProfileCriteria, QualityProfileRepository,
    ScoringConfig, VideoCodec,
};
use serde_json::{Value as JsonValue, json};
use sqlx::{Row, types::Json};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};

const QUALITY_PROFILE_COLUMNS: &str = "id, name, scope, scope_id, archival_quality,
    allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed,
    prefer_remux, allow_bd_disk, allow_upgrades, prefer_dual_audio,
    required_audio_languages, scoring_config";

#[derive(Clone)]
pub struct QualityProfileStore {
    datastore: StoreDatastore,
}

impl QualityProfileStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl QualityProfileRepository for QualityProfileStore {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        let scope = scope.trim().to_string();
        if scope.is_empty() {
            return Err(AppError::Validation(
                "scope is required to list quality profiles".into(),
            ));
        }

        let normalized_scope_id = normalize_scope_id(scope_id);
        let (sql, args) = if let Some(scope_id) = normalized_scope_id {
            (
                format!(
                    "SELECT {QUALITY_PROFILE_COLUMNS}
                       FROM quality_profiles
                      WHERE scope = {{}} AND scope_id = {{}}
                      ORDER BY name"
                ),
                vec![SqlArg::Text(scope), SqlArg::Text(scope_id)],
            )
        } else {
            (
                format!(
                    "SELECT {QUALITY_PROFILE_COLUMNS}
                       FROM quality_profiles
                      WHERE scope = {{}} AND scope_id IS NULL
                      ORDER BY name"
                ),
                vec![SqlArg::Text(scope)],
            )
        };

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(decode_quality_profile(&self.datastore, &row).await?);
        }
        Ok(out)
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let scope = scope.trim().to_string();
        if scope.is_empty() {
            return Err(AppError::Validation(
                "scope is required to replace quality profiles".into(),
            ));
        }

        let normalized_scope_id = normalize_scope_id(scope_id);
        SqlRuntime::run_in_transaction(&self.datastore, "replace_quality_profiles", move |tx| {
            let scope = scope.clone();
            let scope_id = normalized_scope_id.clone();
            let profiles = profiles.clone();
            Box::pin(async move {
                if let Some(scope_id) = scope_id.as_ref() {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM quality_profiles WHERE scope = {} AND scope_id = {}",
                        &[SqlArg::Text(scope.clone()), SqlArg::Text(scope_id.clone())],
                    )
                    .await?;
                } else {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM quality_profiles WHERE scope = {} AND scope_id IS NULL",
                        &[SqlArg::Text(scope.clone())],
                    )
                    .await?;
                }

                for profile in profiles {
                    upsert_quality_profile_tx(tx, &scope, scope_id.as_ref(), profile).await?;
                }

                Ok(())
            })
        })
        .await
    }
}

async fn decode_quality_profile(
    datastore: &StoreDatastore,
    row: &SqlRow,
) -> AppResult<QualityProfile> {
    let id = row.text("id")?;
    let archival_quality = row.opt_text("archival_quality")?.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    });
    let required_audio_languages: Vec<String> = serde_json::from_value(row_json_or_default(
        row,
        "required_audio_languages",
        json!([]),
    )?)
    .unwrap_or_default();
    let scoring_config: ScoringConfig =
        serde_json::from_value(row_json_or_default(row, "scoring_config", json!({}))?)
            .unwrap_or_default();

    Ok(QualityProfile {
        id: id.clone(),
        name: row.text("name")?,
        criteria: QualityProfileCriteria {
            quality_tiers: list_quality_profile_values(
                datastore,
                "quality_profile_quality_tiers",
                "quality_tier",
                "sort_order ASC",
                &id,
            )
            .await?,
            archival_quality,
            allow_unknown_quality: row.bool("allow_unknown_quality")?,
            source_allowlist: list_quality_profile_values(
                datastore,
                "quality_profile_source_allowlist",
                "source",
                "source ASC",
                &id,
            )
            .await?,
            source_blocklist: list_quality_profile_values(
                datastore,
                "quality_profile_source_blocklist",
                "source",
                "source ASC",
                &id,
            )
            .await?,
            video_codec_allowlist: list_quality_profile_values(
                datastore,
                "quality_profile_video_codec_allowlist",
                "codec",
                "codec ASC",
                &id,
            )
            .await?
            .into_iter()
            .map(|value| {
                VideoCodec::parse(value.as_str())
                    .ok_or_else(|| repo_err(format!("invalid stored video codec {value:?}")))
            })
            .collect::<AppResult<Vec<_>>>()?,
            video_codec_blocklist: list_quality_profile_values(
                datastore,
                "quality_profile_video_codec_blocklist",
                "codec",
                "codec ASC",
                &id,
            )
            .await?
            .into_iter()
            .map(|value| {
                VideoCodec::parse(value.as_str())
                    .ok_or_else(|| repo_err(format!("invalid stored video codec {value:?}")))
            })
            .collect::<AppResult<Vec<_>>>()?,
            audio_codec_allowlist: list_quality_profile_values(
                datastore,
                "quality_profile_audio_codec_allowlist",
                "codec",
                "codec ASC",
                &id,
            )
            .await?,
            audio_codec_blocklist: list_quality_profile_values(
                datastore,
                "quality_profile_audio_codec_blocklist",
                "codec",
                "codec ASC",
                &id,
            )
            .await?,
            atmos_preferred: row.bool("atmos_preferred")?,
            dolby_vision_allowed: row.bool("dolby_vision_allowed")?,
            detected_hdr_allowed: row.bool("detected_hdr_allowed")?,
            prefer_remux: row.bool("prefer_remux")?,
            allow_bd_disk: row.bool("allow_bd_disk")?,
            allow_upgrades: row.bool("allow_upgrades")?,
            prefer_dual_audio: row.bool("prefer_dual_audio").unwrap_or(false),
            required_audio_languages,
            scoring_persona: scoring_config.scoring_persona,
            scoring_overrides: scoring_config.scoring_overrides,
            cutoff_tier: scoring_config.cutoff_tier,
            min_score_to_grab: scoring_config.min_score_to_grab,
            facet_persona_overrides: scoring_config.facet_persona_overrides,
        },
    })
}

async fn upsert_quality_profile_tx(
    tx: &mut SqlTx<'_>,
    scope: &str,
    scope_id: Option<&String>,
    profile: QualityProfile,
) -> AppResult<()> {
    let id = profile.id.trim().to_string();
    if id.is_empty() {
        return Ok(());
    }

    let name = profile.name.trim().to_string();
    let criteria = profile.criteria;
    let archival_quality = criteria
        .archival_quality
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let quality_tiers = normalize_profile_string_values(criteria.quality_tiers);
    let source_allowlist = normalize_profile_string_values(criteria.source_allowlist);
    let source_blocklist = normalize_profile_string_values(criteria.source_blocklist);
    let video_codec_allowlist = normalize_profile_video_codecs(
        criteria
            .video_codec_allowlist
            .iter()
            .map(ToString::to_string)
            .collect(),
    );
    let video_codec_blocklist = normalize_profile_video_codecs(
        criteria
            .video_codec_blocklist
            .iter()
            .map(ToString::to_string)
            .collect(),
    );
    let audio_codec_allowlist = normalize_profile_string_values(criteria.audio_codec_allowlist);
    let audio_codec_blocklist = normalize_profile_string_values(criteria.audio_codec_blocklist);
    let required_audio_languages = serde_json::to_value(&criteria.required_audio_languages)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let scoring_config = serde_json::to_value(ScoringConfig {
        scoring_persona: criteria.scoring_persona,
        scoring_overrides: criteria.scoring_overrides,
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
        facet_persona_overrides: criteria.facet_persona_overrides,
    })
    .map_err(|error| AppError::Repository(error.to_string()))?;

    clear_quality_profile_value_rows(tx, &id).await?;

    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO quality_profiles
            (id, name, scope, scope_id, archival_quality, allow_unknown_quality,
             atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux,
             allow_bd_disk, allow_upgrades, prefer_dual_audio, required_audio_languages,
             scoring_config, created_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            scope = excluded.scope,
            scope_id = excluded.scope_id,
            archival_quality = excluded.archival_quality,
            allow_unknown_quality = excluded.allow_unknown_quality,
            atmos_preferred = excluded.atmos_preferred,
            dolby_vision_allowed = excluded.dolby_vision_allowed,
            detected_hdr_allowed = excluded.detected_hdr_allowed,
            prefer_remux = excluded.prefer_remux,
            allow_bd_disk = excluded.allow_bd_disk,
            allow_upgrades = excluded.allow_upgrades,
            prefer_dual_audio = excluded.prefer_dual_audio,
            required_audio_languages = excluded.required_audio_languages,
            scoring_config = excluded.scoring_config",
        &[
            SqlArg::Text(id.clone()),
            SqlArg::Text(name),
            SqlArg::Text(scope.to_string()),
            SqlArg::OptText(scope_id.cloned()),
            SqlArg::OptText(archival_quality),
            SqlArg::Bool(criteria.allow_unknown_quality),
            SqlArg::Bool(criteria.atmos_preferred),
            SqlArg::Bool(criteria.dolby_vision_allowed),
            SqlArg::Bool(criteria.detected_hdr_allowed),
            SqlArg::Bool(criteria.prefer_remux),
            SqlArg::Bool(criteria.allow_bd_disk),
            SqlArg::Bool(criteria.allow_upgrades),
            SqlArg::Bool(criteria.prefer_dual_audio),
            SqlArg::Json(required_audio_languages),
            SqlArg::Json(scoring_config),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await?;

    insert_quality_profile_quality_tiers(tx, &id, &quality_tiers).await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_allowlist",
        "source",
        &id,
        &source_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_source_blocklist",
        "source",
        &id,
        &source_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_allowlist",
        "codec",
        &id,
        &video_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_video_codec_blocklist",
        "codec",
        &id,
        &video_codec_blocklist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_allowlist",
        "codec",
        &id,
        &audio_codec_allowlist,
    )
    .await?;
    insert_quality_profile_values(
        tx,
        "quality_profile_audio_codec_blocklist",
        "codec",
        &id,
        &audio_codec_blocklist,
    )
    .await?;

    Ok(())
}

async fn clear_quality_profile_value_rows(tx: &mut SqlTx<'_>, profile_id: &str) -> AppResult<()> {
    for table in [
        "quality_profile_quality_tiers",
        "quality_profile_source_allowlist",
        "quality_profile_source_blocklist",
        "quality_profile_video_codec_allowlist",
        "quality_profile_video_codec_blocklist",
        "quality_profile_audio_codec_allowlist",
        "quality_profile_audio_codec_blocklist",
    ] {
        let sql = format!("DELETE FROM {table} WHERE profile_id = {{}}");
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &sql,
            &[SqlArg::Text(profile_id.to_string())],
        )
        .await?;
    }
    Ok(())
}

async fn insert_quality_profile_quality_tiers(
    tx: &mut SqlTx<'_>,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    for (index, value) in values.iter().enumerate() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO quality_profile_quality_tiers(profile_id, quality_tier, sort_order)
             VALUES ({}, {}, {})",
            &[
                SqlArg::Text(profile_id.to_string()),
                SqlArg::Text(value.clone()),
                SqlArg::I64(index as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_quality_profile_values(
    tx: &mut SqlTx<'_>,
    table: &str,
    column: &str,
    profile_id: &str,
    values: &[String],
) -> AppResult<()> {
    let sql = format!("INSERT INTO {table}(profile_id, {column}) VALUES ({{}}, {{}})");
    for value in values {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &sql,
            &[
                SqlArg::Text(profile_id.to_string()),
                SqlArg::Text(value.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn list_quality_profile_values(
    datastore: &StoreDatastore,
    table: &str,
    column: &str,
    order_by: &str,
    profile_id: &str,
) -> AppResult<Vec<String>> {
    let sql = format!(
        "SELECT {column} AS value
           FROM {table}
          WHERE profile_id = {{}}
          ORDER BY {order_by}"
    );
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        &sql,
        &[SqlArg::Text(profile_id.to_string())],
    )
    .await?;

    rows.iter().map(|row| row.text("value")).collect()
}

fn row_json_or_default(row: &SqlRow, column: &str, default: JsonValue) -> AppResult<JsonValue> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            let Some(raw) = raw else {
                return Ok(default);
            };
            serde_json::from_str(&raw).or(Ok(default))
        }
        SqlRow::Postgres(row) => {
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| value.0).unwrap_or(default))
        }
    }
}

fn normalize_profile_string_values(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    normalized
}

fn normalize_profile_video_codecs(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let key = value.trim().to_string();
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    normalized
}

fn normalize_scope_id(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    })
}
