use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, MediaServerConnectionRepository};
use scryer_domain::{
    AppPermissionMask, LibraryPermissionMask, MediaServerConnection,
    MediaServerDefaultLibraryGrant, MediaServerPathMapping, MediaServerProvider,
};

use crate::config_store::{current_encryption_key, decrypt_value, maybe_encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const CONNECTION_COLUMNS: &str = "id, provider, display_name, base_url, enabled, login_enabled,
    linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at";

#[derive(Clone)]
pub struct MediaServerConnectionStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl MediaServerConnectionStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl MediaServerConnectionRepository for MediaServerConnectionStore {
    async fn list(
        &self,
        provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        let encryption_key = self.encryption_key()?;
        let (sql, args) = match provider {
            Some(provider) => (
                format!(
                    "SELECT {CONNECTION_COLUMNS} FROM media_server_connections WHERE provider = {{}} ORDER BY display_name"
                ),
                vec![SqlArg::Text(provider.as_str().to_string())],
            ),
            None => (
                format!(
                    "SELECT {CONNECTION_COLUMNS} FROM media_server_connections ORDER BY provider, display_name"
                ),
                Vec::new(),
            ),
        };
        fetch_connections(&self.datastore, &sql, &args, encryption_key.as_ref()).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<MediaServerConnection>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_connection(
            &self.datastore,
            &format!("SELECT {CONNECTION_COLUMNS} FROM media_server_connections WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        let encryption_key = self.encryption_key()?;
        let insert_args = connection_insert_args(&connection);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_media_server_connection",
            move |tx| {
                let connection = connection.clone();
                let insert_args = insert_args.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO media_server_connections (
                             id, provider, display_name, base_url, enabled, login_enabled,
                             linking_enabled, auto_add_enabled, default_app_permissions,
                             created_at, updated_at
                         ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &insert_args,
                    )
                    .await?;
                    replace_connection_details(tx, &connection, encryption_key.as_ref()).await?;
                    Ok(connection)
                })
            },
        )
        .await
    }

    async fn update(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        let encryption_key = self.encryption_key()?;
        let args = vec![
            SqlArg::Text(connection.provider.as_str().to_string()),
            SqlArg::Text(connection.display_name.clone()),
            SqlArg::Text(connection.base_url.clone()),
            SqlArg::Bool(connection.enabled),
            SqlArg::Bool(connection.login_enabled),
            SqlArg::Bool(connection.linking_enabled),
            SqlArg::Bool(connection.auto_add_enabled),
            SqlArg::I64(connection.default_app_permissions.bits() as i64),
            SqlArg::Timestamp(connection.updated_at),
            SqlArg::Text(connection.id.clone()),
        ];
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_media_server_connection",
            move |tx| {
                let connection = connection.clone();
                let args = args.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE media_server_connections
                            SET provider = {},
                                display_name = {},
                                base_url = {},
                                enabled = {},
                                login_enabled = {},
                                linking_enabled = {},
                                auto_add_enabled = {},
                                default_app_permissions = {},
                                updated_at = {}
                          WHERE id = {}",
                        &args,
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!(
                            "media server connection {}",
                            connection.id
                        )));
                    }
                    replace_connection_details(tx, &connection, encryption_key.as_ref()).await?;
                    Ok(connection)
                })
            },
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_media_server_connection",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM media_server_connections WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!("media server connection {id}")));
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn has_external_accounts(&self, id: &str) -> AppResult<bool> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id FROM user_external_accounts WHERE connection_id = {} LIMIT 1",
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(row.is_some())
    }

    async fn has_notification_channels(&self, id: &str) -> AppResult<bool> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id FROM notification_channels WHERE media_server_connection_id = {}
             UNION ALL
             SELECT id FROM notification_subscriptions
              WHERE target_kind = 'media_server_connection'
                AND target_id = {}
             LIMIT 1",
            &[SqlArg::Text(id.to_string()), SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(row.is_some())
    }
}

fn connection_insert_args(connection: &MediaServerConnection) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(connection.id.clone()),
        SqlArg::Text(connection.provider.as_str().to_string()),
        SqlArg::Text(connection.display_name.clone()),
        SqlArg::Text(connection.base_url.clone()),
        SqlArg::Bool(connection.enabled),
        SqlArg::Bool(connection.login_enabled),
        SqlArg::Bool(connection.linking_enabled),
        SqlArg::Bool(connection.auto_add_enabled),
        SqlArg::I64(connection.default_app_permissions.bits() as i64),
        SqlArg::Timestamp(connection.created_at),
        SqlArg::Timestamp(connection.updated_at),
    ]
}

async fn replace_connection_details(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    connection: &MediaServerConnection,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()> {
    for table in [
        "jellyfin_media_server_details",
        "plex_media_server_details",
        "emby_media_server_details",
        "media_server_path_mappings",
        "media_server_default_library_grants",
    ] {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &format!("DELETE FROM {table} WHERE connection_id = {{}}"),
            &[SqlArg::Text(connection.id.clone())],
        )
        .await?;
    }

    match connection.provider {
        MediaServerProvider::Jellyfin => {
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
                 VALUES ({}, {}, {}, {})",
                &[
                    SqlArg::Text(connection.id.clone()),
                    encrypted_api_key_arg(encryption_key, connection.api_key.as_ref())?,
                    SqlArg::Timestamp(connection.created_at),
                    SqlArg::Timestamp(connection.updated_at),
                ],
            )
            .await?;
        }
        MediaServerProvider::Plex => {
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO plex_media_server_details (connection_id, machine_id, api_key, created_at, updated_at)
                 VALUES ({}, {}, {}, {}, {})",
                &[
                    SqlArg::Text(connection.id.clone()),
                    SqlArg::OptText(connection.machine_id.clone()),
                    encrypted_api_key_arg(encryption_key, connection.api_key.as_ref())?,
                    SqlArg::Timestamp(connection.created_at),
                    SqlArg::Timestamp(connection.updated_at),
                ],
            )
            .await?;
        }
        MediaServerProvider::Emby => {
            SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO emby_media_server_details (connection_id, api_key, created_at, updated_at)
                 VALUES ({}, {}, {}, {})",
                &[
                    SqlArg::Text(connection.id.clone()),
                    encrypted_api_key_arg(encryption_key, connection.api_key.as_ref())?,
                    SqlArg::Timestamp(connection.created_at),
                    SqlArg::Timestamp(connection.updated_at),
                ],
            )
            .await?;
        }
    }

    for mapping in &connection.path_mappings {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO media_server_path_mappings (
                 id, connection_id, source_path, destination_path, sort_order
             ) VALUES ({}, {}, {}, {}, {})",
            &[
                SqlArg::Text(scryer_domain::Id::new().0),
                SqlArg::Text(connection.id.clone()),
                SqlArg::Text(mapping.source_path.clone()),
                SqlArg::Text(mapping.destination_path.clone()),
                SqlArg::I64(mapping.sort_order),
            ],
        )
        .await?;
    }

    for grant in &connection.default_library_grants {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO media_server_default_library_grants (
                 connection_id, library_id, permissions
             ) VALUES ({}, {}, {})",
            &[
                SqlArg::Text(connection.id.clone()),
                SqlArg::Text(grant.library_id.clone()),
                SqlArg::I64(grant.permissions.bits() as i64),
            ],
        )
        .await?;
    }

    Ok(())
}

fn encrypted_api_key_arg(
    encryption_key: Option<&EncryptionKey>,
    api_key: Option<&String>,
) -> AppResult<SqlArg> {
    let value = api_key
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| maybe_encrypt_value(encryption_key, value))
        .transpose()?;
    Ok(SqlArg::OptText(value))
}

async fn fetch_connections(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<MediaServerConnection>> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let mut connections = Vec::with_capacity(rows.len());
    for row in rows {
        connections.push(row_to_connection(datastore, &row, encryption_key).await?);
    }
    Ok(connections)
}

async fn fetch_optional_connection(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<MediaServerConnection>> {
    match SqlRuntime::fetch_optional(datastore.read_exec(), sql, args).await? {
        Some(row) => row_to_connection(datastore, &row, encryption_key)
            .await
            .map(Some),
        None => Ok(None),
    }
}

async fn row_to_connection(
    datastore: &StoreDatastore,
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<MediaServerConnection> {
    let id = row.text("id")?;
    let provider = MediaServerProvider::parse(&row.text("provider")?)
        .ok_or_else(|| AppError::Repository("invalid media server provider".into()))?;
    let mut connection = MediaServerConnection {
        id: id.clone(),
        provider: provider.clone(),
        display_name: row.text("display_name")?,
        base_url: row.text("base_url")?,
        enabled: row.bool("enabled")?,
        login_enabled: row.bool("login_enabled")?,
        linking_enabled: row.bool("linking_enabled")?,
        auto_add_enabled: row.bool("auto_add_enabled")?,
        default_app_permissions: AppPermissionMask::from_bits_retain(
            row.i64("default_app_permissions")? as u64,
        ),
        default_library_grants: Vec::new(),
        machine_id: None,
        api_key: None,
        path_mappings: Vec::new(),
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    };

    match provider {
        MediaServerProvider::Jellyfin => {
            connection.api_key = load_api_key(
                datastore,
                "jellyfin_media_server_details",
                &id,
                encryption_key,
            )
            .await?;
        }
        MediaServerProvider::Plex => {
            connection.machine_id = load_plex_machine_id(datastore, &id).await?;
            connection.api_key =
                load_api_key(datastore, "plex_media_server_details", &id, encryption_key).await?;
        }
        MediaServerProvider::Emby => {
            connection.api_key =
                load_api_key(datastore, "emby_media_server_details", &id, encryption_key).await?;
        }
    }
    connection.path_mappings = load_path_mappings(datastore, &id).await?;
    connection.default_library_grants = load_default_library_grants(datastore, &id).await?;
    Ok(connection)
}

async fn load_api_key(
    datastore: &StoreDatastore,
    table: &str,
    id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<String>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        &format!("SELECT api_key FROM {table} WHERE connection_id = {{}}"),
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.map(|row| {
        row.opt_text("api_key")?
            .map(|raw| decrypt_value(encryption_key, raw, "media server api_key", false))
            .transpose()
    })
    .transpose()
    .map(Option::flatten)
}

async fn load_plex_machine_id(datastore: &StoreDatastore, id: &str) -> AppResult<Option<String>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT machine_id FROM plex_media_server_details WHERE connection_id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.map(|row| row.opt_text("machine_id"))
        .transpose()
        .map(Option::flatten)
}

async fn load_path_mappings(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Vec<MediaServerPathMapping>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT source_path, destination_path, sort_order
           FROM media_server_path_mappings
          WHERE connection_id = {}
          ORDER BY sort_order ASC",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(MediaServerPathMapping {
                source_path: row.text("source_path")?,
                destination_path: row.text("destination_path")?,
                sort_order: row.i64("sort_order")?,
            })
        })
        .collect()
}

async fn load_default_library_grants(
    datastore: &StoreDatastore,
    id: &str,
) -> AppResult<Vec<MediaServerDefaultLibraryGrant>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT library_id, permissions
           FROM media_server_default_library_grants
          WHERE connection_id = {}
          ORDER BY library_id ASC",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(MediaServerDefaultLibraryGrant {
                library_id: row.text("library_id")?,
                permissions: LibraryPermissionMask::from_bits_retain(
                    row.i64("permissions")? as u64,
                ),
            })
        })
        .collect()
}
