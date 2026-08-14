use super::*;

fn names(values: &Value) -> Vec<&str> {
    values
        .as_array()
        .expect("GraphQL introspection list")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect()
}

#[tokio::test]
async fn graphql_emby_enums_inputs_outputs_and_mutations_match_the_public_contract() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mode: __type(name: "EmbyConnectionModeValue") { enumValues { name } }
          setup: __type(name: "EmbyLocalSetupMethodValue") { enumValues { name } }
          address: __type(name: "EmbyConnectAddressStatusValue") { enumValues { name } }
          userType: __type(name: "EmbyConnectUserTypeValue") { enumValues { name } }
          create: __type(name: "CreateMediaServerConnectionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          update: __type(name: "UpdateMediaServerConnectionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          login: __type(name: "LoginWithEmbyInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          link: __type(name: "LinkEmbyAccountInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          connection: __type(name: "MediaServerConnectionPayload") {
            fields { name }
          }
          mutation: __type(name: "MutationRoot") { fields { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(
        names(&body["data"]["mode"]["enumValues"]),
        ["LOCAL", "CONNECT"]
    );
    assert_eq!(
        names(&body["data"]["setup"]["enumValues"]),
        ["API_KEY", "ADMIN_CREDENTIALS"]
    );
    assert_eq!(
        names(&body["data"]["address"]["enumValues"]),
        [
            "REACHABLE",
            "UNREACHABLE",
            "INVALID_URL",
            "SERVER_ID_MISMATCH"
        ]
    );
    assert_eq!(
        names(&body["data"]["userType"]["enumValues"]),
        ["LINKED_USER", "GUEST", "UNKNOWN"]
    );

    let credential_fields = [
        "embyConnectionMode",
        "embyLocalSetupMethod",
        "embyConnectEnabled",
        "embyConnectUsernameOrEmail",
        "embyConnectPassword",
        "embyConnectServerId",
    ];
    let create_fields = names(&body["data"]["create"]["inputFields"]);
    let update_fields = names(&body["data"]["update"]["inputFields"]);
    for field in credential_fields {
        assert!(
            create_fields.contains(&field),
            "create input missing {field}"
        );
        assert!(
            update_fields.contains(&field),
            "update input missing {field}"
        );
    }

    let login_fields = names(&body["data"]["login"]["inputFields"]);
    assert_eq!(
        login_fields,
        [
            "connectionId",
            "mode",
            "username",
            "password",
            "totpCode",
            "persistSession"
        ]
    );
    let link_fields = names(&body["data"]["link"]["inputFields"]);
    assert_eq!(
        link_fields,
        ["connectionId", "mode", "username", "password"]
    );

    let connection_fields = names(&body["data"]["connection"]["fields"]);
    assert!(connection_fields.contains(&"embyServerIdPresent"));
    assert!(connection_fields.contains(&"embyConnectEnabled"));
    for forbidden in [
        "embyServerId",
        "apiKey",
        "accessToken",
        "accessKey",
        "connectAccessToken",
    ] {
        assert!(
            !connection_fields.contains(&forbidden),
            "public connection payload must not expose {forbidden}"
        );
    }

    let mutations = names(&body["data"]["mutation"]["fields"]);
    for mutation in [
        "discoverEmbyConnectServers",
        "testEmbyConnect",
        "loginWithEmby",
        "linkEmbyAccount",
    ] {
        assert!(
            mutations.contains(&mutation),
            "MutationRoot missing {mutation}"
        );
    }
}

#[tokio::test]
async fn graphql_emby_connection_payload_reports_presence_without_echoing_secrets_or_server_id() {
    let ctx = TestContext::new().await;
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO media_server_connections (
             id, provider, display_name, base_url, enabled, login_enabled,
             linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
         ) VALUES (?, 'emby', 'Emby Contract', 'https://emby.example.test', 1, 1, 1, 0, 0, ?, ?)",
    )
    .bind("emby-contract")
    .bind(&now)
    .bind(&now)
    .execute(ctx.db.pool())
    .await
    .expect("insert Emby connection fixture");
    sqlx::query(
        "INSERT INTO emby_media_server_details (
             connection_id, api_key, server_id, connect_enabled, created_at, updated_at
         ) VALUES (?, ?, ?, 1, ?, ?)",
    )
    .bind("emby-contract")
    .bind("emby-static-key-must-not-echo")
    .bind("emby-server-id-must-not-echo")
    .bind(&now)
    .bind(&now)
    .execute(ctx.db.pool())
    .await
    .expect("insert Emby detail fixture");
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let body = schema_exec(
        &ctx,
        r#"
        query EmbyConnections {
          mediaServerConnections(provider: EMBY) {
            id
            provider
            apiKeyPresent
            embyServerIdPresent
            embyConnectEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&body);
    let connections = body["data"]["mediaServerConnections"]
        .as_array()
        .expect("Emby connections");
    let connection = connections
        .iter()
        .find(|connection| connection["id"] == "emby-contract")
        .expect("fixture connection");
    assert_eq!(connection["provider"], "EMBY");
    assert_eq!(connection["apiKeyPresent"], true);
    assert_eq!(connection["embyServerIdPresent"], true);
    assert_eq!(connection["embyConnectEnabled"], true);

    let encoded = serde_json::to_string(&body).expect("encode GraphQL response");
    assert!(!encoded.contains("emby-static-key-must-not-echo"));
    assert!(!encoded.contains("emby-server-id-must-not-echo"));
}

#[tokio::test]
async fn graphql_emby_validation_errors_do_not_echo_submitted_credentials() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let request = async_graphql::Request::new(
        r#"
        mutation InvalidEmbySetup($input: CreateMediaServerConnectionInput!) {
          createMediaServerConnection(input: $input) { id }
        }
        "#,
    )
    .variables(async_graphql::Variables::from_json(json!({
        "input": {
            "provider": "EMBY",
            "displayName": "Invalid Emby",
            "baseUrl": "https://emby.example.test",
            "apiKey": "submitted-static-key",
            "adminUsername": "submitted-admin-name",
            "adminPassword": "submitted-admin-password",
            "embyConnectionMode": "LOCAL",
            "embyLocalSetupMethod": "API_KEY",
            "embyConnectEnabled": false
        }
    })))
    .data(admin);
    let response = ctx.schema.execute(request).await;
    let body = serde_json::to_value(response).expect("serialize GraphQL response");
    let encoded = serde_json::to_string(&body).expect("encode GraphQL error");
    assert!(
        body["errors"].is_array(),
        "invalid credential matrix must fail"
    );
    for secret in [
        "submitted-static-key",
        "submitted-admin-name",
        "submitted-admin-password",
    ] {
        assert!(!encoded.contains(secret), "GraphQL error echoed {secret}");
    }
}
