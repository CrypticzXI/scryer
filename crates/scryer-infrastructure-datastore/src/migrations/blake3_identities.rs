//! Plan 149 / WP4: re-key the two identity digests that are stored as lookup
//! keys rather than compared values.
//!
//! Both are recomputed rather than dropped. A `DELETE` would be simpler but
//! loses real state: pending media requests would lose their dedup identity and
//! unmatched-scan rows would vanish from review until the next scan.
//!
//! Neither engine can compute BLAKE3, so this runs as a Rust hook. Both
//! backfills are idempotent — a row already carrying the new value recomputes to
//! the same value — and each is a no-op once complete.

use std::collections::BTreeMap;

use scryer_application::{AppError, AppResult, HashDomain, blake3_identity_hex};
use sqlx::Row;

/// Recomputed `media_requests.identity_fingerprint`, keyed by request id.
///
/// The input mirrors `media_request_identity_fingerprint`: `source:value` pairs
/// joined with `|`, in the `ORDER BY source, external_id` the loader uses.
fn media_request_fingerprints(rows: &[(String, String, String)]) -> Vec<(String, String)> {
    let mut by_request: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (request_id, source, external_id) in rows {
        by_request
            .entry(request_id.as_str())
            .or_default()
            .push(format!("{source}:{external_id}"));
    }
    by_request
        .into_iter()
        .map(|(request_id, parts)| {
            (
                request_id.to_string(),
                blake3_identity_hex(HashDomain::MediaRequestIdentity, parts.join("|")),
            )
        })
        .collect()
}

/// Recomputed `library_scan_unmatched_items.id`.
///
/// Mirrors `build_library_scan_unmatched_item_id`: the digest covers
/// `facet:library_id:item_path` and only its first 24 hex characters are used.
/// A NULL `library_id` interpolates as the empty string, exactly as the
/// producer's `&str` did for a row written before the column existed.
fn unmatched_item_id(facet: &str, library_id: Option<&str>, item_path: &str) -> String {
    let library_id = library_id.unwrap_or_default();
    let fingerprint = blake3_identity_hex(
        HashDomain::LibraryScanUnmatchedItem,
        format!("{facet}:{library_id}:{item_path}"),
    );
    format!("library_scan_unmatched:{}", &fingerprint[..24])
}

fn repo_err(error: sqlx::Error) -> AppError {
    AppError::Repository(error.to_string())
}

pub async fn backfill_blake3_identities_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT request_id, source, external_id
           FROM media_request_external_ids
          ORDER BY request_id, source, external_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let external_ids = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("request_id").map_err(repo_err)?,
                row.try_get::<String, _>("source").map_err(repo_err)?,
                row.try_get::<String, _>("external_id").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (request_id, fingerprint) in media_request_fingerprints(&external_ids) {
        sqlx::query("UPDATE media_requests SET identity_fingerprint = ?1 WHERE id = ?2")
            .bind(&fingerprint)
            .bind(&request_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    // Re-key in one pass. Ids are content-derived and the table has no inbound
    // foreign keys, so a plain UPDATE per row is safe; a collision would mean two
    // rows already shared a (facet, library_id, item_path), which the producer
    // cannot emit.
    let rows =
        sqlx::query("SELECT id, facet, library_id, item_path FROM library_scan_unmatched_items")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
    for row in &rows {
        let id = row.try_get::<String, _>("id").map_err(repo_err)?;
        let facet = row.try_get::<String, _>("facet").map_err(repo_err)?;
        let library_id = row
            .try_get::<Option<String>, _>("library_id")
            .map_err(repo_err)?;
        let item_path = row.try_get::<String, _>("item_path").map_err(repo_err)?;
        let next_id = unmatched_item_id(&facet, library_id.as_deref(), &item_path);
        if next_id == id {
            continue;
        }
        sqlx::query("UPDATE library_scan_unmatched_items SET id = ?1 WHERE id = ?2")
            .bind(&next_id)
            .bind(&id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    Ok(())
}

pub async fn backfill_blake3_identities_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT request_id, source, external_id
           FROM media_request_external_ids
          ORDER BY request_id, source, external_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let external_ids = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("request_id").map_err(repo_err)?,
                row.try_get::<String, _>("source").map_err(repo_err)?,
                row.try_get::<String, _>("external_id").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (request_id, fingerprint) in media_request_fingerprints(&external_ids) {
        sqlx::query("UPDATE media_requests SET identity_fingerprint = $1 WHERE id = $2")
            .bind(&fingerprint)
            .bind(&request_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    let rows =
        sqlx::query("SELECT id, facet, library_id, item_path FROM library_scan_unmatched_items")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
    for row in &rows {
        let id = row.try_get::<String, _>("id").map_err(repo_err)?;
        let facet = row.try_get::<String, _>("facet").map_err(repo_err)?;
        let library_id = row
            .try_get::<Option<String>, _>("library_id")
            .map_err(repo_err)?;
        let item_path = row.try_get::<String, _>("item_path").map_err(repo_err)?;
        let next_id = unmatched_item_id(&facet, library_id.as_deref(), &item_path);
        if next_id == id {
            continue;
        }
        sqlx::query("UPDATE library_scan_unmatched_items SET id = $1 WHERE id = $2")
            .bind(&next_id)
            .bind(&id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_request_fingerprint_groups_and_orders_external_ids() {
        let rows = vec![
            ("req-1".to_string(), "tmdb".to_string(), "42".to_string()),
            ("req-1".to_string(), "imdb".to_string(), "tt7".to_string()),
            ("req-2".to_string(), "tvdb".to_string(), "9".to_string()),
        ];
        let out = media_request_fingerprints(&rows);
        assert_eq!(out.len(), 2);

        // Grouping preserves the caller's row order within a request, matching
        // the loader's ORDER BY source, external_id.
        let expected_req_1 =
            blake3_identity_hex(HashDomain::MediaRequestIdentity, "tmdb:42|imdb:tt7");
        assert_eq!(out[0], ("req-1".to_string(), expected_req_1));
    }

    #[test]
    fn unmatched_item_id_is_stable_and_null_library_reads_as_empty() {
        let with_empty = unmatched_item_id("movie", Some(""), "/media/a");
        let with_null = unmatched_item_id("movie", None, "/media/a");
        assert_eq!(with_empty, with_null);
        assert!(with_null.starts_with("library_scan_unmatched:"));
        assert_eq!(with_null.len(), "library_scan_unmatched:".len() + 24);
    }

    #[test]
    fn unmatched_item_id_separates_facets() {
        assert_ne!(
            unmatched_item_id("movie", Some("lib"), "/media/a"),
            unmatched_item_id("series", Some("lib"), "/media/a")
        );
    }
}
