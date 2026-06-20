use super::*;

#[tokio::test]
async fn graphql_typed_security_settings_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let body = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
            passwordMinLength
            skipLoginForLocalIps
            mfaRequirePasswordLogin
            effectiveFormLoginEnabled
            envOverrideActive
            envOverrideDescription
          }
        }
        "#,
        Some(admin),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["securitySettings"]["formLoginEnabled"], false);
    assert_eq!(body["data"]["securitySettings"]["passwordMinLength"], 8);
    assert_eq!(
        body["data"]["securitySettings"]["skipLoginForLocalIps"],
        false
    );
    assert_eq!(
        body["data"]["securitySettings"]["mfaRequirePasswordLogin"],
        false
    );
    assert_eq!(
        body["data"]["securitySettings"]["effectiveFormLoginEnabled"],
        false
    );
    assert_eq!(body["data"]["securitySettings"]["envOverrideActive"], false);
    assert!(body["data"]["securitySettings"]["envOverrideDescription"].is_null());
}

#[tokio::test]
async fn graphql_auth_runtime_suppresses_mfa_requirements_when_login_is_disabled() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: true
            totpRequireJellyfinLogin: true
          }) {
            formLoginEnabled
            mfaRequireConfigStepUp
            mfaRequirePasswordLogin
            totpRequireJellyfinLogin
            effectiveFormLoginEnabled
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["totpRequireJellyfinLogin"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        false
    );
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "auth.mfa.require_config_step_up",
            None,
            "true",
            "test",
            None,
        )
        .await
        .unwrap();

    let runtime = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            mfaRequirePasswordLogin
            mfaRequireConfigStepUp
            totpRequireJellyfinLogin
          }
        }
        "#,
        Some(admin),
    )
    .await;

    assert_no_errors(&runtime);
    assert_eq!(
        runtime["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["totpRequireJellyfinLogin"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["mfaRequirePasswordLogin"],
        false
    );
    assert_eq!(
        runtime["data"]["authRuntimeState"]["mfaRequireConfigStepUp"],
        false
    );
}

#[tokio::test]
async fn graphql_typed_security_settings_round_trip_updates_runtime() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let admin = ctx
        .app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 12
            skipLoginForLocalIps: true
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
          }) {
            formLoginEnabled
            passwordMinLength
            skipLoginForLocalIps
            effectiveFormLoginEnabled
            envOverrideActive
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSecuritySettings"]["formLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["passwordMinLength"],
        12
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["skipLoginForLocalIps"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateSecuritySettings"]["envOverrideActive"],
        false
    );

    let auth_runtime = schema_exec(
        &ctx,
        r#"
        query AuthRuntimeState {
          authRuntimeState {
            effectiveFormLoginEnabled
            skipLoginForLocalIps
          }
        }
        "#,
        None,
    )
    .await;
    assert_no_errors(&auth_runtime);
    assert_eq!(
        auth_runtime["data"]["authRuntimeState"]["effectiveFormLoginEnabled"],
        true
    );
    assert_eq!(
        auth_runtime["data"]["authRuntimeState"]["skipLoginForLocalIps"],
        true
    );

    let me_with_local_bypass = gql(&ctx, "{ me { username } }", json!({})).await;
    assert_no_errors(&me_with_local_bypass);
    assert_eq!(
        me_with_local_bypass["data"]["me"]["username"],
        admin.username
    );

    let read = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
            passwordMinLength
            effectiveFormLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["securitySettings"]["formLoginEnabled"], true);
    assert_eq!(read["data"]["securitySettings"]["passwordMinLength"], 12);
    assert_eq!(
        read["data"]["securitySettings"]["effectiveFormLoginEnabled"],
        true
    );
}

#[tokio::test]
async fn graphql_typed_security_settings_reject_short_password_minimum() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: false
            passwordMinLength: 7
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
          }) {
            formLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;

    let errors = update["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        message.contains("password minimum length must be at least 8"),
        "expected minimum-length validation error: {update}"
    );
}

#[tokio::test]
async fn graphql_typed_security_settings_reject_enable_with_default_admin_password() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let update = schema_exec(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
          }) {
            formLoginEnabled
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;

    let errors = update["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        message.contains("change the default admin password before enabling form login"),
        "expected default admin password validation error: {update}"
    );

    let read = schema_exec(
        &ctx,
        r#"
        query SecuritySettings {
          securitySettings {
            formLoginEnabled
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["securitySettings"]["formLoginEnabled"], false);
}

#[tokio::test]
async fn graphql_delay_profiles_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let upsert = gql(
        &ctx,
        r#"
        mutation UpsertDelayProfile($input: DelayProfileInput!) {
          upsertDelayProfile(input: $input) {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({
          "input": {
            "id": "balanced-delay",
            "name": "Balanced Delay",
            "usenetDelayMinutes": 30,
            "torrentDelayMinutes": 90,
            "preferredProtocol": "usenet",
            "minAgeMinutes": 15,
            "bypassScoreThreshold": 320,
            "appliesToFacets": ["movie", "series"],
            "tags": ["4k", "hdr"],
            "priority": 5,
            "enabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&upsert);
    assert_eq!(upsert["data"]["upsertDelayProfile"]["id"], "balanced-delay");
    assert_eq!(
        upsert["data"]["upsertDelayProfile"]["appliesToFacets"][1],
        "series"
    );

    let read = gql(
        &ctx,
        r#"
        query DelayProfiles {
          delayProfiles {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    let profile = &read["data"]["delayProfiles"][0];
    assert_eq!(profile["id"], "balanced-delay");
    assert_eq!(profile["name"], "Balanced Delay");
    assert_eq!(profile["usenetDelayMinutes"], 30);
    assert_eq!(profile["torrentDelayMinutes"], 90);
    assert_eq!(profile["preferredProtocol"], "usenet");
    assert_eq!(profile["minAgeMinutes"], 15);
    assert_eq!(profile["bypassScoreThreshold"], 320);
    assert_eq!(profile["appliesToFacets"][0], "movie");
    assert_eq!(profile["appliesToFacets"][1], "series");
    assert_eq!(profile["tags"][0], "4k");
    assert_eq!(profile["priority"], 5);
    assert_eq!(profile["enabled"], true);

    let delete = gql(
        &ctx,
        r#"
        mutation DeleteDelayProfile($id: ID!) {
          deleteDelayProfile(id: $id) {
            id
          }
        }
        "#,
        json!({
          "id": "balanced-delay"
        }),
    )
    .await;
    assert_no_errors(&delete);
    assert_eq!(delete["data"]["deleteDelayProfile"]["id"], "balanced-delay");
}
