#![recursion_limit = "256"]

mod common;

use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use common::TestContext;
use scryer_application::recycle_bin::{RecycleBinConfig, RecycleManifest, recycle_file};
use scryer_application::{
    LibraryRootDraft, RECYCLE_BIN_ENABLED_KEY, RECYCLE_BIN_PATH_KEY, SETTINGS_SCOPE_MEDIA,
    SETTINGS_SOURCE_TYPED_GRAPHQL, TitleRepository, UpdateRecycleBinSettings,
};
use scryer_domain::{
    AppPermission, AppPermissionMask, Library, LibraryPermission, LibraryPermissionMask,
    MediaFacet, Title, User, UserAuthorization,
};
use scryer_infrastructure::SettingDefinitionSeed;
use serde_json::{Value, json};

async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

fn actor(
    username: &str,
    app_permissions: impl IntoIterator<Item = AppPermission>,
    library_permissions: impl IntoIterator<Item = (String, LibraryPermissionMask)>,
) -> User {
    let mut user = User::new_admin(username);
    user.authorization = UserAuthorization {
        app: AppPermissionMask::from_permissions(app_permissions),
        libraries: library_permissions.into_iter().collect::<HashMap<_, _>>(),
        loaded: true,
        ..Default::default()
    };
    user
}

fn config_actor() -> User {
    actor(
        "config",
        [AppPermission::ManageSystemSettings],
        std::iter::empty(),
    )
}

fn catalog_actor() -> User {
    actor(
        "catalog",
        [AppPermission::ManageCatalogSettings],
        std::iter::empty(),
    )
}

fn manage_titles_actor(username: &str, library_ids: &[String]) -> User {
    actor(
        username,
        std::iter::empty(),
        library_ids.iter().map(|library_id| {
            (
                library_id.clone(),
                LibraryPermissionMask::from_permissions([LibraryPermission::ManageTitles]),
            )
        }),
    )
}

fn no_permission_actor() -> User {
    actor("none", std::iter::empty(), std::iter::empty())
}

async fn seed_recycle_bin_setting_definition(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_MEDIA.into(),
                key_name: RECYCLE_BIN_ENABLED_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_MEDIA.into(),
                key_name: RECYCLE_BIN_PATH_KEY.into(),
                data_type: "string".into(),
                default_value_json: "\"\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed recycle bin setting definition");
}

async fn set_custom_recycle_bin_path(ctx: &TestContext, path: &Path) {
    ctx.settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_MEDIA,
            RECYCLE_BIN_PATH_KEY,
            None,
            json!(path.to_string_lossy().to_string()).to_string(),
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("set custom recycle bin path");
}

async fn seed_library(ctx: &TestContext, name: &str, root: &Path) -> Library {
    ctx.app
        .create_library(
            &catalog_actor(),
            MediaFacet::Movie,
            name.to_string(),
            vec![LibraryRootDraft {
                path: root.to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library")
}

async fn seed_title(ctx: &TestContext, id: &str, library: &Library) {
    let title = Title {
        id: id.to_string(),
        name: format!("{} Title", library.name),
        facet: MediaFacet::Movie,
        library_id: library.id.clone(),
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    TitleRepository::create(&ctx.titles, title)
        .await
        .expect("seed title");
}

async fn seed_recycled_file(root: &Path, title_id: &str, name: &str) -> String {
    seed_recycled_file_in_bin(root, &root.join(".scryer-recycle"), title_id, name).await
}

async fn seed_recycled_file_in_bin(
    root: &Path,
    recycle_base_path: &Path,
    title_id: &str,
    name: &str,
) -> String {
    let source_path = root.join(format!("{name}.mkv"));
    std::fs::write(&source_path, format!("{name} content")).expect("write source file");
    let config = RecycleBinConfig {
        enabled: true,
        base_path: recycle_base_path.to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    };
    let result = recycle_file(
        &config,
        &source_path,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: source_path.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 128,
            title_id: Some(title_id.to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("recycle file")
    .expect("file recycled");

    result
        .recycled_path
        .parent()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .expect("entry id")
}

#[tokio::test]
async fn recycle_bin_settings_permissions_are_split_between_read_and_update() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    let manage_actor = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let config_actor = config_actor();

    assert!(
        ctx.app
            .get_recycle_bin_settings(&manage_actor)
            .await
            .expect("manage-title users can read")
            .enabled
    );
    assert!(
        ctx.app
            .get_recycle_bin_settings(&config_actor)
            .await
            .expect("config users can read")
            .enabled
    );
    assert!(
        ctx.app
            .get_recycle_bin_settings(&no_permission_actor())
            .await
            .is_err(),
        "users without config or manage-title access cannot read"
    );
    assert!(
        ctx.app
            .update_recycle_bin_settings(
                &manage_actor,
                UpdateRecycleBinSettings { enabled: false },
            )
            .await
            .is_err(),
        "manage-title users cannot update the setting"
    );

    let updated = ctx
        .app
        .update_recycle_bin_settings(&config_actor, UpdateRecycleBinSettings { enabled: false })
        .await
        .expect("config user updates setting");
    assert!(!updated.enabled);
}

#[tokio::test]
async fn graphql_recycle_bin_settings_and_scoped_item_args_work() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;

    let body = gql(&ctx, "query { recycleBinSettings { enabled } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["recycleBinSettings"]["enabled"], true);

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateRecycleBinSettingsInput!) {
            updateRecycleBinSettings(input: $input) { enabled }
        }"#,
        json!({ "input": { "enabled": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["updateRecycleBinSettings"]["enabled"], false);

    let body = gql(
        &ctx,
        r#"query($libraryIds: [String!]) {
            recycledItems(libraryIds: $libraryIds) {
                totalCount
                items { id libraryId libraryName }
            }
        }"#,
        json!({ "libraryIds": null }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["recycledItems"]["totalCount"], 0);

    let body = gql(
        &ctx,
        r#"mutation($libraryIds: [String!]) {
            emptyRecycleBin(libraryIds: $libraryIds)
        }"#,
        json!({ "libraryIds": null }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["emptyRecycleBin"], 0);
}

#[tokio::test]
async fn recycled_items_are_filtered_to_manage_title_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file(root_a.path(), "title-a", "movie-a").await;
    seed_recycled_file(root_b.path(), "title-b", "movie-b").await;

    let manager_a = manage_titles_actor("manager-a", std::slice::from_ref(&library_a.id));
    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );

    let items = ctx
        .app
        .list_recycled_items(&manager_a, None)
        .await
        .expect("list authorized items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].library_id, library_a.id);
    assert_eq!(items[0].library_name, library_a.name);

    let filtered_out = ctx
        .app
        .list_recycled_items(&manager_a, Some(vec![library_b.id.clone()]))
        .await
        .expect("selected unauthorized library is intersected away");
    assert!(filtered_out.is_empty());

    let selected = ctx
        .app
        .list_recycled_items(&manager_both, Some(vec![library_b.id.clone()]))
        .await
        .expect("selected authorized library is listed");
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].library_id, library_b.id);

    let config_only = ctx
        .app
        .list_recycled_items(&config_actor(), None)
        .await
        .expect("config-only user can access page but not items");
    assert!(config_only.is_empty());
}

#[tokio::test]
async fn empty_recycle_bin_only_purges_selected_authorized_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file(root_a.path(), "title-a", "movie-a").await;
    seed_recycled_file(root_b.path(), "title-b", "movie-b").await;

    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );
    let removed = ctx
        .app
        .empty_recycle_bin(&manager_both, Some(vec![library_a.id.clone()]))
        .await
        .expect("empty selected library");
    assert_eq!(removed, 1);

    let remaining = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list remaining items");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].library_id, library_b.id);
}

#[tokio::test]
async fn custom_recycle_bin_path_lists_entries_once_across_libraries() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");
    let recycle_root = tempfile::tempdir().expect("custom recycle root");
    set_custom_recycle_bin_path(&ctx, recycle_root.path()).await;

    let library_a = seed_library(&ctx, "Movies A", root_a.path()).await;
    let library_b = seed_library(&ctx, "Movies B", root_b.path()).await;
    seed_title(&ctx, "title-a", &library_a).await;
    seed_title(&ctx, "title-b", &library_b).await;
    seed_recycled_file_in_bin(root_a.path(), recycle_root.path(), "title-a", "movie-a").await;
    seed_recycled_file_in_bin(root_b.path(), recycle_root.path(), "title-b", "movie-b").await;

    let manager_both = manage_titles_actor(
        "manager-both",
        &[library_a.id.clone(), library_b.id.clone()],
    );
    let items = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list custom recycle bin");
    assert_eq!(items.len(), 2);

    let removed = ctx
        .app
        .empty_recycle_bin(&manager_both, Some(vec![library_a.id.clone()]))
        .await
        .expect("empty selected library");
    assert_eq!(removed, 1);

    let remaining = ctx
        .app
        .list_recycled_items(&manager_both, None)
        .await
        .expect("list remaining custom recycle bin");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].library_id, library_b.id);
}

#[tokio::test]
async fn recycle_bin_config_resolution_deduplicates_roots_by_base_path() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let root_path = root.path().to_string_lossy().to_string();

    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![root_path.clone(), format!("{root_path}/")])
        .await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].0, root_path);
    assert_eq!(configs[0].1.base_path, root.path().join(".scryer-recycle"));
}

#[tokio::test]
async fn recycle_bin_config_resolution_keeps_distinct_default_roots() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root_a = tempfile::tempdir().expect("library root a");
    let root_b = tempfile::tempdir().expect("library root b");

    let configs = ctx
        .app
        .recycle_bin_configs_for_media_roots(vec![
            root_a.path().to_string_lossy().to_string(),
            root_b.path().to_string_lossy().to_string(),
        ])
        .await;
    assert_eq!(configs.len(), 2);
    assert!(
        configs
            .iter()
            .any(|(_, config)| { config.base_path == root_a.path().join(".scryer-recycle") })
    );
    assert!(
        configs
            .iter()
            .any(|(_, config)| { config.base_path == root_b.path().join(".scryer-recycle") })
    );
}

#[tokio::test]
async fn disabled_recycle_bin_paths_are_inert_and_direct_delete_new_files() {
    let ctx = TestContext::new().await;
    seed_recycle_bin_setting_definition(&ctx).await;
    let root = tempfile::tempdir().expect("library root");
    let library = seed_library(&ctx, "Movies A", root.path()).await;
    seed_title(&ctx, "title-a", &library).await;
    let entry_id = seed_recycled_file(root.path(), "title-a", "movie-a").await;
    let manager = manage_titles_actor("manager", std::slice::from_ref(&library.id));
    let config_actor = config_actor();

    ctx.app
        .update_recycle_bin_settings(&config_actor, UpdateRecycleBinSettings { enabled: false })
        .await
        .expect("disable recycle bin");

    let items = ctx
        .app
        .list_recycled_items(&manager, None)
        .await
        .expect("disabled list is inert");
    assert!(items.is_empty());
    assert_eq!(
        ctx.app
            .empty_recycle_bin(&manager, None)
            .await
            .expect("disabled empty is inert"),
        0
    );
    assert!(
        ctx.app
            .restore_recycled_item(&manager, &entry_id)
            .await
            .is_err(),
        "disabled restore should not traverse stored entries"
    );

    let new_source = root.path().join("new-delete.mkv");
    std::fs::write(&new_source, b"delete directly").expect("write new source");
    let config = ctx
        .app
        .recycle_bin_config_for_media_root(Some(root.path().to_string_lossy().as_ref()))
        .await;
    let result = recycle_file(
        &config,
        &new_source,
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: new_source.to_string_lossy().to_string(),
            original_file_id: None,
            size_bytes: 64,
            title_id: Some("title-a".to_string()),
            media_root: None,
            reason: "file_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        },
    )
    .await
    .expect("direct delete succeeds");
    assert!(result.is_none());
    assert!(!new_source.exists());
}
