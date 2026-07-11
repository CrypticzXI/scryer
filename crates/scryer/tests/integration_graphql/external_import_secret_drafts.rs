use super::*;
use async_graphql::Variables;

async fn schema_exec_with_variables(
    ctx: &TestContext,
    query: &str,
    variables: Value,
    user: &User,
) -> Value {
    let req = async_graphql::Request::new(query)
        .variables(Variables::from_json(variables))
        .data(user.clone());
    let resp = ctx.schema.execute(req).await;
    serde_json::to_value(&resp).expect("serialize gql response")
}

fn assert_graphql_error_contains(body: &Value, expected: &str) {
    let errors = body["errors"].as_array().expect("expected GraphQL errors");
    assert!(
        errors.iter().any(|error| error["message"]
            .as_str()
            .is_some_and(|message| message.contains(expected))),
        "expected GraphQL error containing {expected:?}: {body}"
    );
}

fn secret_draft_input(label: &str) -> Value {
    json!({
        "instanceApiKeys": [
            {
                "instanceId": format!("sonarr-{label}"),
                "kind": "SONARR",
                "apiKey": format!("sonarr-key-{label}")
            }
        ],
        "downloadClientApiKeyOverrides": [
            {
                "dedupKey": format!("download-client-api-{label}"),
                "apiKey": format!("download-client-api-key-{label}")
            }
        ],
        "downloadClientPasswordOverrides": [
            {
                "dedupKey": format!("download-client-password-{label}"),
                "password": format!("download-client-password-{label}")
            }
        ],
        "indexerApiKeyOverrides": [
            {
                "dedupKey": format!("indexer-api-{label}"),
                "apiKey": format!("indexer-api-key-{label}")
            }
        ]
    })
}

fn empty_secret_draft_input() -> Value {
    json!({
        "instanceApiKeys": [],
        "downloadClientApiKeyOverrides": [],
        "downloadClientPasswordOverrides": [],
        "indexerApiKeyOverrides": []
    })
}

async fn create_admin(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(User::new_admin(username))
        .await
        .expect("test admin should create")
}

async fn create_denied_user(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: UserAuthorization::default(),
        })
        .await
        .expect("denied user should create")
}

const SAVE_DRAFT: &str = r#"
mutation SaveDraft($input: SaveExternalImportSetupSecretDraftInput!) {
  saveExternalImportSetupSecretDraft(input: $input) {
    overwroteAnotherUserDraft
    updatedAt
  }
}
"#;

const CLEAR_DRAFT: &str = r#"
mutation ClearDraft {
  clearExternalImportSetupSecretDraft {
    clearedAt
  }
}
"#;

const READ_DRAFT: &str = r#"
query ReadDraft {
  externalImportSetupSecretDraftStatus {
    hasDraft
    ownedByCurrentUser
    updatedAt
  }
  externalImportSetupSecretDraft {
    updatedAt
    instanceApiKeys {
      instanceId
      kind
      apiKey
    }
    downloadClientApiKeyOverrides {
      dedupKey
      apiKey
    }
    downloadClientPasswordOverrides {
      dedupKey
      password
    }
    indexerApiKeyOverrides {
      dedupKey
      apiKey
    }
  }
}
"#;

#[tokio::test]
async fn graphql_external_import_setup_secret_draft_round_trips_typed_owner_scoped_secrets() {
    let ctx = TestContext::new().await;
    let owner = create_admin(&ctx, "secret-draft-owner").await;
    let other = create_admin(&ctx, "secret-draft-other").await;
    let denied = create_denied_user(&ctx, "secret-draft-denied").await;

    let initial = schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &owner).await;
    assert_no_errors(&initial);
    assert_eq!(
        initial["data"]["externalImportSetupSecretDraftStatus"]["hasDraft"],
        false
    );
    assert!(initial["data"]["externalImportSetupSecretDraft"].is_null());

    let denied_save = schema_exec_with_variables(
        &ctx,
        SAVE_DRAFT,
        json!({ "input": secret_draft_input("denied") }),
        &denied,
    )
    .await;
    assert_graphql_field_denied(&denied_save, "saveExternalImportSetupSecretDraft");

    let empty_save = schema_exec_with_variables(
        &ctx,
        SAVE_DRAFT,
        json!({ "input": empty_secret_draft_input() }),
        &owner,
    )
    .await;
    assert_graphql_error_contains(
        &empty_save,
        "external import setup secret draft cannot be empty; clear the draft instead",
    );

    let owner_save = schema_exec_with_variables(
        &ctx,
        SAVE_DRAFT,
        json!({ "input": secret_draft_input("owner") }),
        &owner,
    )
    .await;
    assert_no_errors(&owner_save);
    assert_eq!(
        owner_save["data"]["saveExternalImportSetupSecretDraft"]["overwroteAnotherUserDraft"],
        false
    );
    let owner_updated_at = owner_save["data"]["saveExternalImportSetupSecretDraft"]["updatedAt"]
        .as_str()
        .expect("owner save updatedAt");

    let owner_read = schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &owner).await;
    assert_no_errors(&owner_read);
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraftStatus"]["hasDraft"],
        true
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraftStatus"]["ownedByCurrentUser"],
        true
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraft"]["updatedAt"],
        owner_updated_at
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraft"]["instanceApiKeys"][0]["apiKey"],
        "sonarr-key-owner"
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraft"]["downloadClientApiKeyOverrides"][0]["apiKey"],
        "download-client-api-key-owner"
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraft"]["downloadClientPasswordOverrides"][0]
            ["password"],
        "download-client-password-owner"
    );
    assert_eq!(
        owner_read["data"]["externalImportSetupSecretDraft"]["indexerApiKeyOverrides"][0]["apiKey"],
        "indexer-api-key-owner"
    );

    let other_read = schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &other).await;
    assert_no_errors(&other_read);
    assert_eq!(
        other_read["data"]["externalImportSetupSecretDraftStatus"]["hasDraft"],
        true
    );
    assert_eq!(
        other_read["data"]["externalImportSetupSecretDraftStatus"]["ownedByCurrentUser"],
        false
    );
    assert_eq!(
        other_read["data"]["externalImportSetupSecretDraftStatus"]["updatedAt"],
        owner_updated_at
    );
    assert!(other_read["data"]["externalImportSetupSecretDraft"].is_null());

    let other_save = schema_exec_with_variables(
        &ctx,
        SAVE_DRAFT,
        json!({ "input": secret_draft_input("other") }),
        &other,
    )
    .await;
    assert_no_errors(&other_save);
    assert_eq!(
        other_save["data"]["saveExternalImportSetupSecretDraft"]["overwroteAnotherUserDraft"],
        true
    );

    let owner_after_overwrite =
        schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &owner).await;
    assert_no_errors(&owner_after_overwrite);
    assert_eq!(
        owner_after_overwrite["data"]["externalImportSetupSecretDraftStatus"]["ownedByCurrentUser"],
        false
    );
    assert!(owner_after_overwrite["data"]["externalImportSetupSecretDraft"].is_null());

    let owner_clear = schema_exec_with_variables(&ctx, CLEAR_DRAFT, json!({}), &owner).await;
    assert_no_errors(&owner_clear);
    assert!(
        owner_clear["data"]["clearExternalImportSetupSecretDraft"]["clearedAt"].is_string(),
        "expected clearedAt timestamp: {owner_clear}"
    );

    // The owner no longer owns the draft, so their clear must not remove the
    // other user's draft (previously observable as `cleared: false`).
    let other_still_owns = schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &other).await;
    assert_no_errors(&other_still_owns);
    assert_eq!(
        other_still_owns["data"]["externalImportSetupSecretDraftStatus"]["hasDraft"],
        true
    );

    let other_clear = schema_exec_with_variables(&ctx, CLEAR_DRAFT, json!({}), &other).await;
    assert_no_errors(&other_clear);
    assert!(
        other_clear["data"]["clearExternalImportSetupSecretDraft"]["clearedAt"].is_string(),
        "expected clearedAt timestamp: {other_clear}"
    );

    let final_read = schema_exec_with_variables(&ctx, READ_DRAFT, json!({}), &other).await;
    assert_no_errors(&final_read);
    assert_eq!(
        final_read["data"]["externalImportSetupSecretDraftStatus"]["hasDraft"],
        false
    );
    assert!(final_read["data"]["externalImportSetupSecretDraft"].is_null());
}
