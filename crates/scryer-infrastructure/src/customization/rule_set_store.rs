use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppResult, RuleSetRepository};
use scryer_domain::{Id, MediaFacet, RuleSet};
use sqlx::Row;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::sqlite_services::SqliteServices;
use crate::storage::sql::json::{canonical_json_arg, json_text_or};

#[derive(Clone)]
pub struct RuleSetStore {
    datastore: StoreDatastore,
}

impl RuleSetStore {
    pub(crate) fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub fn from_sqlite_services(db: &SqliteServices) -> Self {
        Self::new(StoreDatastore::Sqlite {
            pool: db.pool().clone(),
            writer_gate: db.writer_gate(),
        })
    }
}

#[async_trait]
impl RuleSetRepository for RuleSetStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let sql =
            format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets ORDER BY priority DESC, name ASC");
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[]).await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let sql = format!(
            "SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE enabled = {{}} ORDER BY priority DESC, name ASC"
        );
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[SqlArg::Bool(true)]).await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let sql = format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE id = {{}}");
        fetch_optional_rule_set(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let args = rule_set_args(rule_set)?;
        execute_write(
            &self.datastore,
            "create_rule_set",
            "INSERT INTO rule_sets
                (id, name, description, rego_source, enabled, priority,
                 applied_facets, created_at, updated_at, is_managed, managed_key)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            args,
        )
        .await
    }

    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let args = rule_set_args(rule_set)?;
        execute_write(
            &self.datastore,
            "update_rule_set",
            "UPDATE rule_sets
                SET name = {}, description = {}, rego_source = {}, enabled = {},
                    priority = {}, applied_facets = {}, updated_at = {},
                    is_managed = {}, managed_key = {}
              WHERE id = {}",
            vec![
                args[1].clone(),
                args[2].clone(),
                args[3].clone(),
                args[4].clone(),
                args[5].clone(),
                args[6].clone(),
                args[8].clone(),
                args[9].clone(),
                args[10].clone(),
                args[0].clone(),
            ],
        )
        .await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_rule_set",
            "DELETE FROM rule_sets WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "record_rule_set_history",
            "INSERT INTO rule_set_history
                (id, rule_set_id, action, rego_source, actor_id, created_at)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(Id::new().0),
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(action.to_string()),
                SqlArg::OptText(rego_source.map(str::to_string)),
                SqlArg::OptText(actor_id.map(str::to_string)),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        let sql =
            format!("SELECT {RULE_SET_COLUMNS} FROM rule_sets WHERE managed_key = {{}} LIMIT 1");
        fetch_optional_rule_set(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(key.to_string())],
        )
        .await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_rule_set_by_managed_key",
            "DELETE FROM rule_sets WHERE managed_key = {}",
            vec![SqlArg::Text(key.to_string())],
        )
        .await
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        let pattern = format!("{prefix}%");
        let sql = format!(
            "SELECT {RULE_SET_COLUMNS}
               FROM rule_sets
              WHERE managed_key LIKE {{}}
              ORDER BY managed_key"
        );
        fetch_rule_sets(self.datastore.read_exec(), &sql, &[SqlArg::Text(pattern)]).await
    }
}

const RULE_SET_COLUMNS: &str = "id, name, description, rego_source, enabled, priority,
    applied_facets, created_at, updated_at, is_managed, managed_key";

async fn fetch_rule_sets(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<RuleSet>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .iter()
        .map(row_to_rule_set)
        .collect()
}

async fn fetch_optional_rule_set(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<RuleSet>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .as_ref()
        .map(row_to_rule_set)
        .transpose()
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move {
            SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
            Ok(())
        })
    })
    .await
}

fn rule_set_args(rule_set: &RuleSet) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(rule_set.id.clone()),
        SqlArg::Text(rule_set.name.clone()),
        SqlArg::Text(rule_set.description.clone()),
        SqlArg::Text(rule_set.rego_source.clone()),
        SqlArg::Bool(rule_set.enabled),
        SqlArg::I32(rule_set.priority),
        canonical_json_arg(&rule_set.applied_facets)?,
        SqlArg::Timestamp(rule_set.created_at),
        SqlArg::Timestamp(rule_set.updated_at),
        SqlArg::Bool(rule_set.is_managed),
        SqlArg::OptText(rule_set.managed_key.clone()),
    ])
}

fn row_to_rule_set(row: &SqlRow) -> AppResult<RuleSet> {
    Ok(RuleSet {
        id: row.text("id")?,
        name: row.text("name")?,
        description: row.text("description")?,
        rego_source: row.text("rego_source")?,
        enabled: row.bool("enabled")?,
        priority: row.i32("priority")?,
        applied_facets: applied_facets(row)?,
        created_at: timestamp_or_now(row, "created_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
        is_managed: row.bool("is_managed")?,
        managed_key: row.opt_text("managed_key")?,
    })
}

fn applied_facets(row: &SqlRow) -> AppResult<Vec<MediaFacet>> {
    let raw = json_text_or(row, "applied_facets", "[]")?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn timestamp_or_now(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            Ok(DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()))
        }
        SqlRow::Postgres(_) => row.timestamp(column),
    }
}
