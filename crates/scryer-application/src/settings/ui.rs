use std::collections::HashSet;

use crate::{
    AppError, AppResult, AppUseCase, UiSettings, UiSettingsFacet, UiSettingsUpdate,
    UiTableColumnSetting, UiTableViewMode,
};
use scryer_domain::User;

const COMPACT_TABLE_COLUMNS: &[&str] = &[
    "select",
    "name",
    "library",
    "monitored",
    "quality",
    "episodes",
    "status",
    "size",
    "actions",
];
const POSTER_TABLE_COLUMNS: &[&str] = &[
    "poster",
    "name",
    "library",
    "monitored",
    "quality",
    "episodes",
    "status",
    "size",
    "actions",
];

impl AppUseCase {
    pub async fn get_my_ui_settings(&self, actor: &User) -> AppResult<UiSettings> {
        Ok(self
            .services
            .identity
            .ui_settings
            .get_by_user_id(&actor.id)
            .await?
            .unwrap_or_else(|| UiSettings::defaults_for_user(actor.id.clone())))
    }

    pub async fn set_my_ui_settings(
        &self,
        actor: &User,
        input: UiSettingsUpdate,
    ) -> AppResult<UiSettings> {
        let input = validate_ui_settings_update(input)?;
        self.services
            .identity
            .ui_settings
            .upsert(&actor.id, input)
            .await
    }
}

fn validate_ui_settings_update(mut input: UiSettingsUpdate) -> AppResult<UiSettingsUpdate> {
    input.highlight_color = normalize_optional_hex_color(input.highlight_color, "highlightColor")?;
    input.secondary_color = normalize_optional_hex_color(input.secondary_color, "secondaryColor")?;
    validate_table_columns(&mut input.table_columns)?;
    Ok(input)
}

fn normalize_optional_hex_color(
    value: Option<String>,
    field_name: &str,
) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(AppError::Validation(format!(
            "{field_name} must be a #RRGGBB hex color"
        )));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

fn validate_table_columns(columns: &mut [UiTableColumnSetting]) -> AppResult<()> {
    let mut seen = HashSet::new();
    for column in columns.iter() {
        validate_table_column(column)?;
        let key = (
            column.facet,
            column.table_view_mode,
            column.column_id.as_str(),
        );
        if !seen.insert(key) {
            return Err(AppError::Validation(format!(
                "duplicate UI table column setting for {} {} column {}",
                column.facet.as_str(),
                column.table_view_mode.as_str(),
                column.column_id
            )));
        }
        if column.column_order < 0 {
            return Err(AppError::Validation(
                "table column order must be greater than or equal to 0".into(),
            ));
        }
    }

    columns.sort_by(|left, right| {
        left.facet
            .as_str()
            .cmp(right.facet.as_str())
            .then_with(|| {
                left.table_view_mode
                    .as_str()
                    .cmp(right.table_view_mode.as_str())
            })
            .then_with(|| left.column_order.cmp(&right.column_order))
            .then_with(|| left.column_id.cmp(&right.column_id))
    });

    Ok(())
}

fn validate_table_column(column: &UiTableColumnSetting) -> AppResult<()> {
    let allowed_columns = match column.table_view_mode {
        UiTableViewMode::Compact => COMPACT_TABLE_COLUMNS,
        UiTableViewMode::PosterTable => POSTER_TABLE_COLUMNS,
    };
    if !allowed_columns.contains(&column.column_id.as_str()) {
        return Err(AppError::Validation(format!(
            "unsupported {} table column {:?}",
            column.table_view_mode.as_str(),
            column.column_id
        )));
    }
    if column.facet == UiSettingsFacet::Movies && column.column_id == "episodes" {
        return Err(AppError::Validation(
            "movies table settings cannot include the episodes column".into(),
        ));
    }
    Ok(())
}
