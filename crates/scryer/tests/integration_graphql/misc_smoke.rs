use super::*;

#[tokio::test]
async fn graphql_indexers_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ indexers { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["indexers"].is_array());
}

#[tokio::test]
async fn graphql_download_client_configs_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadClientConfigs { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["downloadClientConfigs"].is_array());
}

#[tokio::test]
async fn graphql_runtime_browse_and_download_client_permissions() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let limited = ctx
        .app
        .create_user(
            &admin,
            "runtime_limited".to_string(),
            "limited-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create limited user");
    let manage_library = ctx
        .app
        .create_user(
            &admin,
            "runtime_manage_library".to_string(),
            "library-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                permissions: LibraryPermissionMask::from_permission(
                    LibraryPermission::ManageLibrary,
                ),
            }],
        )
        .await
        .expect("create manage-library user");
    let catalog_user = ctx
        .app
        .create_user(
            &admin,
            "runtime_catalog".to_string(),
            "catalog-pass1".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageCatalogSettings),
            vec![],
        )
        .await
        .expect("create catalog user");
    let system_user = ctx
        .app
        .create_user(
            &admin,
            "runtime_system".to_string(),
            "system-pass1".to_string(),
            AppPermissionMask::from_permission(scryer_domain::AppPermission::ManageSystemSettings),
            vec![],
        )
        .await
        .expect("create system user");

    let runtime_body = schema_exec(
        &ctx,
        "{ runtimeInfo { runtimePathStyle } }",
        Some(limited.clone()),
    )
    .await;
    assert_no_errors(&runtime_body);
    assert!(
        matches!(
            runtime_body["data"]["runtimeInfo"]["runtimePathStyle"].as_str(),
            Some("UNIX") | Some("WINDOWS")
        ),
        "runtimeInfo should be readable by authenticated non-admin users"
    );

    let browse_path = serde_json::to_string(&std::env::current_dir().unwrap().to_string_lossy())
        .expect("serialize current dir path");
    let browse_query = format!("{{ browsePath(path: {browse_path}) {{ path }} }}");
    let browse_body = schema_exec(&ctx, &browse_query, Some(manage_library.clone())).await;
    assert_no_errors(&browse_body);
    assert!(browse_body["data"]["browsePath"].is_array());

    let browse_denied = schema_exec(&ctx, &browse_query, Some(limited)).await;
    assert!(
        browse_denied.get("errors").is_some(),
        "browsePath should reject users without library-settings access: {browse_denied}"
    );

    let configs_denied = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(manage_library),
    )
    .await;
    assert!(
        configs_denied.get("errors").is_some(),
        "downloadClientConfigs should reject ManageLibrary-only users: {configs_denied}"
    );

    let catalog_configs_body = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(catalog_user.clone()),
    )
    .await;
    assert_no_errors(&catalog_configs_body);
    assert!(catalog_configs_body["data"]["downloadClientConfigs"].is_array());

    let routing_bootstrap_body = schema_exec(
        &ctx,
        r#"
        query {
            downloadClientConfigs { id name }
            indexers { id name }
            downloadClientRouting(scope: movie) { clientId category }
            indexerRouting(scope: movie) { indexerId categories }
        }
        "#,
        Some(catalog_user),
    )
    .await;
    assert_no_errors(&routing_bootstrap_body);
    assert!(routing_bootstrap_body["data"]["downloadClientConfigs"].is_array());
    assert!(routing_bootstrap_body["data"]["indexers"].is_array());
    assert!(routing_bootstrap_body["data"]["downloadClientRouting"].is_array());
    assert!(routing_bootstrap_body["data"]["indexerRouting"].is_array());

    let configs_body = schema_exec(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        Some(system_user),
    )
    .await;
    assert_no_errors(&configs_body);
    assert!(configs_body["data"]["downloadClientConfigs"].is_array());
}

// ---------------------------------------------------------------------------
// Wanted items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_wanted_items_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($statuses: [WantedStatusValue!], $mediaTypes: [WantedMediaTypeValue!]) {
            wantedItems(statuses: $statuses, mediaTypes: $mediaTypes) {
                items { id }
                total
            }
        }"#,
        json!({ "statuses": ["wanted"], "mediaTypes": ["movie"] }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["wantedItems"]["total"], 0,
        "should have no wanted items initially"
    );
}

// ---------------------------------------------------------------------------
// Rule sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_rule_sets_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Import history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_import_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ importHistory { id sourceTitle status } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["importHistory"].is_array());
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_calendar_episodes() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($start: Date!, $end: Date!) {
            calendarEpisodes(startDate: $start, endDate: $end) {
                episodeTitle seasonNumber episodeNumber
            }
        }"#,
        json!({ "start": "2024-01-01", "end": "2024-12-31" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["calendarEpisodes"].is_array());
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_unknown_field_returns_error() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ nonExistentField }", json!({})).await;
    assert!(
        body.get("errors").is_some(),
        "unknown field should return errors"
    );
}

#[tokio::test]
async fn graphql_invalid_mutation_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { addTitle(input: { name: "" }) { title { id } } }"#,
        json!({}),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "invalid input should return errors"
    );
}

#[tokio::test]
async fn graphql_batch_request_not_supported_via_single() {
    let ctx = TestContext::new().await;
    // Verify single requests work (batch is handled at the middleware level)
    let body = gql(&ctx, "{ titles { items { id } } }", json!({})).await;
    assert_no_errors(&body);
}
