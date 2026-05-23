use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use serde_json::{Map as JsonMap, Value as JsonValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportColumnKind {
    Generic,
    TimestampLike,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportColumnRule {
    pub(crate) name: String,
    pub(crate) nullable: bool,
    pub(crate) has_default: bool,
    pub(crate) nullable_foreign_key: bool,
    pub(crate) kind: ImportColumnKind,
}

pub(crate) fn strip_nonportable_backup_fields(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
) {
    if table == "plugin_installations" {
        object.remove("wasm_bytes");
    }
}

pub(crate) fn normalize_import_object_for_target(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
    columns: &[ImportColumnRule],
    line_number: usize,
) -> AppResult<()> {
    strip_nonportable_backup_fields(table, object);
    normalize_import_object(table, object, now)?;

    for column in columns {
        normalize_column_value(table, object, column, line_number)?;
    }

    for column in columns {
        if !column.nullable && !column.has_default && !object.contains_key(&column.name) {
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} is missing required column `{}` for the current schema",
                column.name
            )));
        }
    }

    Ok(())
}

fn normalize_import_object(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    match table {
        "settings_definitions" => normalize_settings_definition_import_object(object, now),
        "settings_values" => normalize_settings_value_import_object(object, now),
        "titles" => normalize_title_import_object(object),
        _ => {}
    }
    Ok(())
}

fn normalize_column_value(
    table: &str,
    object: &mut JsonMap<String, JsonValue>,
    column: &ImportColumnRule,
    line_number: usize,
) -> AppResult<()> {
    let Some(value) = object.get(&column.name).cloned() else {
        return Ok(());
    };

    if let JsonValue::String(text) = &value {
        if text.trim().is_empty() && column.nullable_foreign_key {
            object.insert(column.name.clone(), JsonValue::Null);
            return Ok(());
        }

        if text.trim().is_empty() && matches!(column.kind, ImportColumnKind::TimestampLike) {
            if column.nullable {
                object.insert(column.name.clone(), JsonValue::Null);
                return Ok(());
            }
            if column.has_default {
                object.remove(&column.name);
                return Ok(());
            }
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} contains a blank required timestamp column `{}`",
                column.name
            )));
        }
    }

    if matches!(value, JsonValue::Null) {
        if !column.nullable && column.has_default {
            object.remove(&column.name);
        } else if !column.nullable {
            return Err(AppError::Validation(format!(
                "backup row for {table}:{line_number} contains null for required column `{}`",
                column.name
            )));
        }
    }

    Ok(())
}

fn normalize_settings_value_import_object(
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) {
    if object
        .get("value_json")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        object.insert("value_json".to_string(), JsonValue::Object(JsonMap::new()));
    }

    if missing_or_blank(object.get("source")) {
        object.insert(
            "source".to_string(),
            JsonValue::String("system".to_string()),
        );
    }

    let now_rfc3339 = now.to_rfc3339();
    for field in ["created_at", "updated_at"] {
        if missing_or_blank(object.get(field)) {
            object.insert(field.to_string(), JsonValue::String(now_rfc3339.clone()));
        }
    }
}

fn normalize_settings_definition_import_object(
    object: &mut JsonMap<String, JsonValue>,
    now: DateTime<Utc>,
) {
    if object
        .get("default_value_json")
        .is_none_or(|value| missing_or_blank(Some(value)))
    {
        object.insert(
            "default_value_json".to_string(),
            JsonValue::String("null".to_string()),
        );
    }

    if object
        .get("validation_json")
        .is_some_and(|value| missing_or_blank(Some(value)))
    {
        object.insert("validation_json".to_string(), JsonValue::Null);
    }

    let now_rfc3339 = now.to_rfc3339();
    for field in ["created_at", "updated_at"] {
        if missing_or_blank(object.get(field)) {
            object.insert(field.to_string(), JsonValue::String(now_rfc3339.clone()));
        }
    }
}

fn normalize_title_import_object(object: &mut JsonMap<String, JsonValue>) {
    let record = object
        .get("record_json")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();

    for field in [
        "id",
        "name",
        "created_by",
        "created_at",
        "year",
        "overview",
        "poster_url",
        "banner_url",
        "background_url",
        "sort_title",
        "slug",
        "imdb_id",
        "runtime_minutes",
        "content_status",
        "language",
        "first_aired",
        "network",
        "studio",
        "country",
        "metadata_language",
        "metadata_fetched_at",
        "min_availability",
        "digital_release_date",
        "folder_path",
    ] {
        copy_title_record_field(object, &record, field, field);
    }

    copy_title_record_field(object, &record, "library_id", "library_id");
    copy_title_record_field(object, &record, "facet", "facet");

    object
        .entry("library_id".to_string())
        .or_insert_with(|| JsonValue::String(String::new()));
    object
        .entry("facet".to_string())
        .or_insert_with(|| JsonValue::String("movie".to_string()));
    let monitored = sqlite_bool_value(object.get("monitored"))
        .or_else(|| {
            record
                .get("monitored")
                .and_then(|value| sqlite_bool_value(Some(value)))
        })
        .unwrap_or(JsonValue::Bool(true));
    object.insert("monitored".to_string(), monitored);

    for (record_field, source_field) in [
        ("tags", "tags"),
        ("external_ids", "external_ids"),
        ("genres", "genres"),
        ("aliases", "aliases"),
        ("tagged_aliases", "tagged_aliases_json"),
    ] {
        if object.contains_key(source_field) {
            continue;
        }
        let value = record
            .get(record_field)
            .and_then(logical_json_value)
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        object.insert(source_field.to_string(), value);
    }
}

fn copy_title_record_field(
    object: &mut JsonMap<String, JsonValue>,
    record: &JsonMap<String, JsonValue>,
    record_field: &str,
    column: &str,
) {
    if object.contains_key(column) {
        return;
    }
    if let Some(value) = record.get(record_field).filter(|value| !value.is_null()) {
        object.insert(column.to_string(), value.clone());
    }
}

fn sqlite_bool_value(value: Option<&JsonValue>) -> Option<JsonValue> {
    match value {
        Some(JsonValue::Bool(value)) => Some(JsonValue::Bool(*value)),
        Some(JsonValue::Number(value)) => value.as_i64().map(|value| JsonValue::Bool(value != 0)),
        Some(JsonValue::String(value)) => match value.as_str() {
            "1" | "true" | "TRUE" => Some(JsonValue::Bool(true)),
            "0" | "false" | "FALSE" => Some(JsonValue::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

fn logical_json_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(value) => {
            Some(serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.clone())))
        }
        value => Some(value.clone()),
    }
}

fn missing_or_blank(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::{Map as JsonMap, Value as JsonValue, json};

    use super::{
        ImportColumnKind, ImportColumnRule, normalize_import_object_for_target,
        strip_nonportable_backup_fields,
    };

    #[test]
    fn plugin_installation_backup_rows_drop_wasm_bytes_but_keep_metadata() {
        let mut object = JsonMap::from_iter([
            (
                "plugin_id".to_string(),
                JsonValue::String("demo".to_string()),
            ),
            (
                "descriptor_json".to_string(),
                JsonValue::String("{\"name\":\"demo\"}".to_string()),
            ),
            ("is_enabled".to_string(), JsonValue::Bool(true)),
            (
                "wasm_bytes".to_string(),
                json!({
                    "__scryer_type": "blob",
                    "base64": "AQIDBA==",
                }),
            ),
        ]);

        strip_nonportable_backup_fields("plugin_installations", &mut object);

        assert!(!object.contains_key("wasm_bytes"));
        assert_eq!(
            object.get("plugin_id"),
            Some(&JsonValue::String("demo".to_string()))
        );
        assert_eq!(
            object.get("descriptor_json"),
            Some(&JsonValue::String("{\"name\":\"demo\"}".to_string()))
        );
        assert_eq!(object.get("is_enabled"), Some(&JsonValue::Bool(true)));
    }

    #[test]
    fn plugin_installation_import_normalization_ignores_wasm_bytes() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            (
                "plugin_id".to_string(),
                JsonValue::String("demo".to_string()),
            ),
            ("name".to_string(), JsonValue::String("Demo".to_string())),
            (
                "wasm_bytes".to_string(),
                json!({
                    "__scryer_type": "blob",
                    "base64": "AQIDBA==",
                }),
            ),
        ]);

        normalize_import_object_for_target(
            "plugin_installations",
            &mut object,
            now,
            &[
                ImportColumnRule {
                    name: "plugin_id".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "name".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
            ],
            3,
        )
        .expect("plugin installation row should normalize");

        assert!(!object.contains_key("wasm_bytes"));
    }

    #[test]
    fn settings_values_normalization_fills_required_fields() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            ("id".to_string(), JsonValue::String("setting-1".to_string())),
            (
                "setting_definition_id".to_string(),
                JsonValue::String("definition-1".to_string()),
            ),
            (
                "scope".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            ("scope_id".to_string(), JsonValue::Null),
            ("value_json".to_string(), JsonValue::Null),
            ("source".to_string(), JsonValue::String(String::new())),
            ("created_at".to_string(), JsonValue::Null),
        ]);

        normalize_import_object_for_target(
            "settings_values",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "value_json".to_string(),
                nullable: false,
                has_default: false,
                nullable_foreign_key: false,
                kind: ImportColumnKind::Generic,
            }],
            8,
        )
        .expect("normalization");

        assert_eq!(object.get("value_json"), Some(&json!({})));
        assert_eq!(
            object.get("source"),
            Some(&JsonValue::String("system".to_string()))
        );
        assert_eq!(
            object.get("created_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
        assert_eq!(
            object.get("updated_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
    }

    #[test]
    fn settings_definitions_normalization_fills_default_and_timestamps() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([
            (
                "id".to_string(),
                JsonValue::String("backup_matrix:backup_matrix:json_payload".to_string()),
            ),
            (
                "category".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            (
                "scope".to_string(),
                JsonValue::String("backup_matrix".to_string()),
            ),
            (
                "key_name".to_string(),
                JsonValue::String("json_payload".to_string()),
            ),
            (
                "data_type".to_string(),
                JsonValue::String("json".to_string()),
            ),
            ("default_value_json".to_string(), JsonValue::Null),
            (
                "validation_json".to_string(),
                JsonValue::String("   ".to_string()),
            ),
            ("is_sensitive".to_string(), JsonValue::Bool(false)),
        ]);

        normalize_import_object_for_target(
            "settings_definitions",
            &mut object,
            now,
            &[
                ImportColumnRule {
                    name: "default_value_json".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::Generic,
                },
                ImportColumnRule {
                    name: "created_at".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::TimestampLike,
                },
                ImportColumnRule {
                    name: "updated_at".to_string(),
                    nullable: false,
                    has_default: false,
                    nullable_foreign_key: false,
                    kind: ImportColumnKind::TimestampLike,
                },
            ],
            11,
        )
        .expect("settings definitions row should normalize");

        assert_eq!(
            object.get("default_value_json"),
            Some(&JsonValue::String("null".to_string()))
        );
        assert_eq!(object.get("validation_json"), Some(&JsonValue::Null));
        assert_eq!(
            object.get("created_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
        assert_eq!(
            object.get("updated_at"),
            Some(&JsonValue::String(now.to_rfc3339()))
        );
    }

    #[test]
    fn defaulted_required_nulls_are_omitted_for_target_defaults() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::from_iter([("created_at".to_string(), JsonValue::Null)]);

        normalize_import_object_for_target(
            "workflow_operations",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "created_at".to_string(),
                nullable: false,
                has_default: true,
                nullable_foreign_key: false,
                kind: ImportColumnKind::TimestampLike,
            }],
            3,
        )
        .expect("default should be used");

        assert!(!object.contains_key("created_at"));
    }

    #[test]
    fn missing_required_non_default_columns_fail_early() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 5, 15, 9, 30, 0)
            .single()
            .expect("fixed timestamp");
        let mut object = JsonMap::new();

        let error = normalize_import_object_for_target(
            "custom_table",
            &mut object,
            now,
            &[ImportColumnRule {
                name: "value_json".to_string(),
                nullable: false,
                has_default: false,
                nullable_foreign_key: false,
                kind: ImportColumnKind::Generic,
            }],
            11,
        )
        .expect_err("missing required column should fail");

        assert!(
            error
                .to_string()
                .contains("missing required column `value_json`")
        );
    }
}
