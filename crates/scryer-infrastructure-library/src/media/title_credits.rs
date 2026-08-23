use std::collections::BTreeMap;

use chrono::Utc;
use scryer_application::{AppResult, TitleCredit};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx};

/// Replace a title's cached SMG credits with `credits`, preserving the order SMG
/// returned them in. An empty slice clears the cache.
pub async fn replace_title_credits_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    credits: &[TitleCredit],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM title_credits WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;

    let now = Utc::now();
    for (position, credit) in credits.iter().enumerate() {
        tx.execute(
            "INSERT INTO title_credits (
                title_id, position, kind, person_id, person_name, person_original_name,
                person_image_url, person_source, person_external_id, character_name,
                language, billing_order, episode_count, created_at, updated_at
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I32(position as i32),
                SqlArg::Text(credit.kind.clone()),
                SqlArg::Text(credit.person_id.clone()),
                SqlArg::Text(credit.person_name.clone()),
                SqlArg::Text(credit.person_original_name.clone()),
                SqlArg::Text(credit.person_image_url.clone()),
                SqlArg::Text(credit.person_source.clone()),
                SqlArg::Text(credit.person_external_id.clone()),
                SqlArg::Text(credit.character_name.clone()),
                SqlArg::Text(credit.language.clone()),
                SqlArg::I32(credit.billing_order),
                SqlArg::OptI32(credit.episode_count),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

pub async fn load_title_credits(
    exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<TitleCredit>>> {
    if title_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = std::iter::repeat_n("{}", title_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT
            title_id,
            kind,
            person_id,
            person_name,
            person_original_name,
            person_image_url,
            person_source,
            person_external_id,
            character_name,
            language,
            billing_order,
            episode_count
           FROM title_credits
          WHERE title_id IN ({placeholders})
          ORDER BY title_id, position"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;

    let mut credits_by_title = BTreeMap::<String, Vec<TitleCredit>>::new();
    for row in &rows {
        let title_id = row.text("title_id")?;
        credits_by_title
            .entry(title_id)
            .or_default()
            .push(TitleCredit {
                kind: row.text("kind")?,
                person_id: row.text("person_id")?,
                person_name: row.text("person_name")?,
                person_original_name: row.text("person_original_name")?,
                person_image_url: row.text("person_image_url")?,
                person_source: row.text("person_source")?,
                person_external_id: row.text("person_external_id")?,
                character_name: row.text("character_name")?,
                language: row.text("language")?,
                billing_order: row.i32("billing_order")?,
                episode_count: row.opt_i32("episode_count")?,
            });
    }
    Ok(credits_by_title)
}
