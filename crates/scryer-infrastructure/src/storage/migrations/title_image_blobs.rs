use scryer_application::{AppError, AppResult};
use sqlx::Row;

const MIGRATION_PAGE_SIZE: i64 = 128;

pub(crate) async fn migrate_title_image_blobs_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    sqlx::raw_sql(
        "CREATE TABLE title_image_blobs (
            digest TEXT PRIMARY KEY,
            format TEXT NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            bytes BLOB NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE title_image_variants_migrated (
            id TEXT PRIMARY KEY,
            title_image_id TEXT NOT NULL,
            variant_key TEXT NOT NULL,
            blob_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE (title_image_id, variant_key),
            FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE,
            FOREIGN KEY (blob_digest) REFERENCES title_image_blobs(digest) ON DELETE RESTRICT
        );",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    let mut after_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, title_image_id, variant_key, format, width, height, bytes, digest,
                    created_at, updated_at
               FROM title_image_variants
              WHERE id > ?1
              ORDER BY id
              LIMIT ?2",
        )
        .bind(&after_id)
        .bind(MIGRATION_PAGE_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
        if rows.is_empty() {
            break;
        }

        for row in rows {
            let variant = SqliteLegacyVariant::from_row(&row)?;
            after_id.clone_from(&variant.id);
            if variant.actual_digest() != variant.digest {
                continue;
            }

            sqlx::query(
                "INSERT INTO title_image_blobs (
                    digest, format, width, height, bytes, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (digest) DO NOTHING",
            )
            .bind(&variant.digest)
            .bind(&variant.format)
            .bind(variant.width)
            .bind(variant.height)
            .bind(&variant.bytes)
            .bind(&variant.created_at)
            .bind(&variant.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;

            let blob = sqlx::query(
                "SELECT format, width, height, bytes
                   FROM title_image_blobs
                  WHERE digest = ?1",
            )
            .bind(&variant.digest)
            .fetch_one(&mut **tx)
            .await
            .map_err(repo_err)?;
            if !sqlite_blob_matches(&blob, &variant)? {
                continue;
            }

            sqlx::query(
                "INSERT INTO title_image_variants_migrated (
                    id, title_image_id, variant_key, blob_digest, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(&variant.id)
            .bind(&variant.title_image_id)
            .bind(&variant.variant_key)
            .bind(&variant.digest)
            .bind(&variant.created_at)
            .bind(&variant.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        }
    }

    sqlx::raw_sql(
        "DROP TABLE title_image_variants;
         ALTER TABLE title_image_variants_migrated RENAME TO title_image_variants;
         CREATE INDEX idx_title_image_variants_image_variant
             ON title_image_variants(title_image_id, variant_key);
         CREATE INDEX idx_title_image_variants_blob_digest
             ON title_image_variants(blob_digest);",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    clear_missing_preferred_local_paths_sqlite(tx).await
}

pub(crate) async fn migrate_title_image_blobs_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    sqlx::raw_sql(
        "CREATE TABLE title_image_blobs (
            digest text PRIMARY KEY,
            format text NOT NULL,
            width bigint NOT NULL,
            height bigint NOT NULL,
            bytes bytea NOT NULL,
            created_at timestamptz NOT NULL,
            updated_at timestamptz NOT NULL
        );
        CREATE TABLE title_image_variants_migrated (
            id text NOT NULL,
            title_image_id text NOT NULL,
            variant_key text NOT NULL,
            blob_digest text NOT NULL,
            created_at timestamptz NOT NULL DEFAULT now(),
            updated_at timestamptz NOT NULL DEFAULT now()
        );",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    let mut after_id = String::new();
    loop {
        let rows = sqlx::query(
            "SELECT id, title_image_id, variant_key, format, width, height, bytes, digest,
                    created_at, updated_at
               FROM title_image_variants
              WHERE id > $1
              ORDER BY id
              LIMIT $2",
        )
        .bind(&after_id)
        .bind(MIGRATION_PAGE_SIZE)
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
        if rows.is_empty() {
            break;
        }

        for row in rows {
            let variant = PostgresLegacyVariant::from_row(&row)?;
            after_id.clone_from(&variant.id);
            if variant.actual_digest() != variant.digest {
                continue;
            }

            sqlx::query(
                "INSERT INTO title_image_blobs (
                    digest, format, width, height, bytes, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (digest) DO NOTHING",
            )
            .bind(&variant.digest)
            .bind(&variant.format)
            .bind(variant.width)
            .bind(variant.height)
            .bind(&variant.bytes)
            .bind(variant.created_at)
            .bind(variant.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;

            let blob = sqlx::query(
                "SELECT format, width, height, bytes
                   FROM title_image_blobs
                  WHERE digest = $1",
            )
            .bind(&variant.digest)
            .fetch_one(&mut **tx)
            .await
            .map_err(repo_err)?;
            if !postgres_blob_matches(&blob, &variant)? {
                continue;
            }

            sqlx::query(
                "INSERT INTO title_image_variants_migrated (
                    id, title_image_id, variant_key, blob_digest, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&variant.id)
            .bind(&variant.title_image_id)
            .bind(&variant.variant_key)
            .bind(&variant.digest)
            .bind(variant.created_at)
            .bind(variant.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
        }
    }

    sqlx::raw_sql(
        "DROP TABLE title_image_variants;
         ALTER TABLE title_image_variants_migrated RENAME TO title_image_variants;
         ALTER TABLE title_image_variants
             ADD CONSTRAINT title_image_variants_pkey PRIMARY KEY (id);
         ALTER TABLE title_image_variants
             ADD CONSTRAINT title_image_variants_title_image_id_variant_key_key
             UNIQUE (title_image_id, variant_key);
         ALTER TABLE title_image_variants
             ADD CONSTRAINT title_image_variants_title_image_id_fkey
             FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE;
         ALTER TABLE title_image_variants
             ADD CONSTRAINT title_image_variants_blob_digest_fkey
             FOREIGN KEY (blob_digest) REFERENCES title_image_blobs(digest) ON DELETE RESTRICT;
         CREATE INDEX idx_title_image_variants_image_variant
             ON title_image_variants(title_image_id, variant_key);
         CREATE INDEX idx_title_image_variants_blob_digest
             ON title_image_variants(blob_digest);",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    clear_missing_preferred_local_paths_postgres(tx).await
}

#[derive(Debug)]
struct SqliteLegacyVariant {
    id: String,
    title_image_id: String,
    variant_key: String,
    format: String,
    width: i64,
    height: i64,
    bytes: Vec<u8>,
    digest: String,
    created_at: String,
    updated_at: String,
}

impl SqliteLegacyVariant {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> AppResult<Self> {
        Ok(Self {
            id: row.try_get("id").map_err(repo_err)?,
            title_image_id: row.try_get("title_image_id").map_err(repo_err)?,
            variant_key: row.try_get("variant_key").map_err(repo_err)?,
            format: row.try_get("format").map_err(repo_err)?,
            width: row.try_get("width").map_err(repo_err)?,
            height: row.try_get("height").map_err(repo_err)?,
            bytes: row.try_get("bytes").map_err(repo_err)?,
            digest: row.try_get("digest").map_err(repo_err)?,
            created_at: row.try_get("created_at").map_err(repo_err)?,
            updated_at: row.try_get("updated_at").map_err(repo_err)?,
        })
    }

    fn actual_digest(&self) -> String {
        format!("blake3:{}", blake3::hash(&self.bytes).to_hex())
    }
}

#[derive(Debug)]
struct PostgresLegacyVariant {
    id: String,
    title_image_id: String,
    variant_key: String,
    format: String,
    width: i64,
    height: i64,
    bytes: Vec<u8>,
    digest: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl PostgresLegacyVariant {
    fn from_row(row: &sqlx::postgres::PgRow) -> AppResult<Self> {
        Ok(Self {
            id: row.try_get("id").map_err(repo_err)?,
            title_image_id: row.try_get("title_image_id").map_err(repo_err)?,
            variant_key: row.try_get("variant_key").map_err(repo_err)?,
            format: row.try_get("format").map_err(repo_err)?,
            width: row.try_get("width").map_err(repo_err)?,
            height: row.try_get("height").map_err(repo_err)?,
            bytes: row.try_get("bytes").map_err(repo_err)?,
            digest: row.try_get("digest").map_err(repo_err)?,
            created_at: row.try_get("created_at").map_err(repo_err)?,
            updated_at: row.try_get("updated_at").map_err(repo_err)?,
        })
    }

    fn actual_digest(&self) -> String {
        format!("blake3:{}", blake3::hash(&self.bytes).to_hex())
    }
}

fn sqlite_blob_matches(
    row: &sqlx::sqlite::SqliteRow,
    variant: &SqliteLegacyVariant,
) -> AppResult<bool> {
    Ok(
        row.try_get::<String, _>("format").map_err(repo_err)? == variant.format
            && row.try_get::<i64, _>("width").map_err(repo_err)? == variant.width
            && row.try_get::<i64, _>("height").map_err(repo_err)? == variant.height
            && row.try_get::<Vec<u8>, _>("bytes").map_err(repo_err)? == variant.bytes,
    )
}

fn postgres_blob_matches(
    row: &sqlx::postgres::PgRow,
    variant: &PostgresLegacyVariant,
) -> AppResult<bool> {
    Ok(
        row.try_get::<String, _>("format").map_err(repo_err)? == variant.format
            && row.try_get::<i64, _>("width").map_err(repo_err)? == variant.width
            && row.try_get::<i64, _>("height").map_err(repo_err)? == variant.height
            && row.try_get::<Vec<u8>, _>("bytes").map_err(repo_err)? == variant.bytes,
    )
}

async fn clear_missing_preferred_local_paths_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    sqlx::raw_sql(
        "UPDATE titles
            SET poster_local_path = NULL
          WHERE poster_local_path IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM title_images ti
                  JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
                 WHERE ti.title_id = titles.id
                   AND ti.kind = 'poster'
                   AND tiv.variant_key = 'w250'
            );
         UPDATE titles
            SET background_local_path = NULL
          WHERE background_local_path IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM title_images ti
                  JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
                 WHERE ti.title_id = titles.id
                   AND ti.kind = 'fanart'
                   AND tiv.variant_key = 'w1280'
            );",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn clear_missing_preferred_local_paths_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    sqlx::raw_sql(
        "UPDATE titles
            SET poster_local_path = NULL
          WHERE poster_local_path IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM title_images ti
                  JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
                 WHERE ti.title_id = titles.id
                   AND ti.kind = 'poster'
                   AND tiv.variant_key = 'w250'
            );
         UPDATE titles
            SET background_local_path = NULL
          WHERE background_local_path IS NOT NULL
            AND NOT EXISTS (
                SELECT 1
                  FROM title_images ti
                  JOIN title_image_variants tiv ON tiv.title_image_id = ti.id
                 WHERE ti.title_id = titles.id
                   AND ti.kind = 'fanart'
                   AND tiv.variant_key = 'w1280'
            );",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_hook_deduplicates_valid_bytes_and_skips_only_corrupt_variants() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool should open");
        sqlx::raw_sql(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE titles (
                 id TEXT PRIMARY KEY,
                 poster_local_path TEXT,
                 background_local_path TEXT
             );
             CREATE TABLE title_images (
                 id TEXT PRIMARY KEY,
                 title_id TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
             );
             CREATE TABLE title_image_variants (
                 id TEXT PRIMARY KEY,
                 title_image_id TEXT NOT NULL,
                 variant_key TEXT NOT NULL,
                 path TEXT,
                 format TEXT NOT NULL,
                 width INTEGER NOT NULL,
                 height INTEGER NOT NULL,
                 bytes BLOB NOT NULL,
                 digest TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 UNIQUE (title_image_id, variant_key),
                 FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE
             );
             INSERT INTO titles (id, poster_local_path) VALUES
                 ('title-a', '/images/titles/title-a/poster/w250?v=valid'),
                 ('title-b', '/images/titles/title-b/poster/w250?v=valid'),
                 ('title-c', '/images/titles/title-c/poster/w250?v=invalid');
             INSERT INTO title_images (id, title_id, kind) VALUES
                 ('image-a', 'title-a', 'poster'),
                 ('image-b', 'title-b', 'poster'),
                 ('image-c', 'title-c', 'poster');",
        )
        .execute(&pool)
        .await
        .expect("legacy fixture schema should load");

        let bytes = vec![4_u8, 5, 6];
        let digest = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        for (id, image_id, stored_digest) in [
            ("variant-a", "image-a", digest.as_str()),
            ("variant-b", "image-b", digest.as_str()),
            ("variant-c", "image-c", "blake3:invalid"),
        ] {
            sqlx::query(
                "INSERT INTO title_image_variants (
                    id, title_image_id, variant_key, format, width, height, bytes, digest,
                    created_at, updated_at
                 ) VALUES (?1, ?2, 'w250', 'avif', 250, 375, ?3, ?4, ?5, ?5)",
            )
            .bind(id)
            .bind(image_id)
            .bind(&bytes)
            .bind(stored_digest)
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("legacy variant should insert");
        }

        let mut tx = pool.begin().await.expect("transaction should start");
        migrate_title_image_blobs_sqlite(&mut tx)
            .await
            .expect("hook should migrate title image bytes");
        tx.commit().await.expect("transaction should commit");

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
            .fetch_one(&pool)
            .await
            .expect("blob count should load");
        let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
            .fetch_one(&pool)
            .await
            .expect("variant count should load");
        assert_eq!(blob_count, 1);
        assert_eq!(variant_count, 2);

        let transferred: Vec<u8> =
            sqlx::query_scalar("SELECT bytes FROM title_image_blobs WHERE digest = ?1")
                .bind(&digest)
                .fetch_one(&pool)
                .await
                .expect("transferred bytes should load");
        assert_eq!(transferred, bytes);

        let corrupt_local_path: Option<String> =
            sqlx::query_scalar("SELECT poster_local_path FROM titles WHERE id = 'title-c'")
                .fetch_one(&pool)
                .await
                .expect("corrupt title should load");
        assert!(corrupt_local_path.is_none());

        let legacy_column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('title_image_variants') WHERE name = 'bytes'",
        )
        .fetch_one(&pool)
        .await
        .expect("variant columns should load");
        assert_eq!(legacy_column_count, 0);
    }
}
