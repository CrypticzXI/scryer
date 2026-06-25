use super::*;

fn ui_settings_test_user(username: &str) -> User {
    User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
        },
    }
}

async fn create_ui_settings_test_user(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(ui_settings_test_user(username))
        .await
        .expect("create UI settings test user")
}

#[tokio::test]
async fn graphql_anonymous_ui_settings_round_trip_is_shared() {
    let ctx = TestContext::new().await;

    let update = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
            dateTimeFormat
            highlightColor
            secondaryColor
            highContrastMode
            reduceMotion
            density
            sidebarMode
            defaultLandingView
            tableColumns {
              facet
              tableViewMode
              columnId
              columnOrder
              visible
            }
          }
        }
        "#,
        json!({
            "input": {
                "theme": "pride",
                "dateTimeFormat": "iso24h",
                "highlightColor": "#ff3366",
                "secondaryColor": "#2277aa",
                "highContrastMode": true,
                "reduceMotion": true,
                "density": "compact",
                "sidebarMode": "collapsed",
                "defaultLandingView": "calendar",
                "tableColumns": [
                    {
                        "facet": "movies",
                        "tableViewMode": "compact",
                        "columnId": "name",
                        "columnOrder": 0,
                        "visible": true
                    },
                    {
                        "facet": "series",
                        "tableViewMode": "posterTable",
                        "columnId": "episodes",
                        "columnOrder": 1,
                        "visible": false
                    }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            dateTimeFormat
            highlightColor
            secondaryColor
            highContrastMode
            reduceMotion
            density
            sidebarMode
            defaultLandingView
            tableColumns {
              facet
              tableViewMode
              columnId
              columnOrder
              visible
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["myUiSettings"];
    assert_eq!(settings["theme"], json!("pride"));
    assert_eq!(settings["dateTimeFormat"], json!("iso24h"));
    assert_eq!(settings["highlightColor"], json!("#ff3366"));
    assert_eq!(settings["secondaryColor"], json!("#2277aa"));
    assert_eq!(settings["highContrastMode"], json!(true));
    assert_eq!(settings["reduceMotion"], json!(true));
    assert_eq!(settings["density"], json!("compact"));
    assert_eq!(settings["sidebarMode"], json!("collapsed"));
    assert_eq!(settings["defaultLandingView"], json!("calendar"));
    assert_eq!(
        settings["tableColumns"],
        json!([
            {
                "facet": "movies",
                "tableViewMode": "compact",
                "columnId": "name",
                "columnOrder": 0,
                "visible": true
            },
            {
                "facet": "series",
                "tableViewMode": "posterTable",
                "columnId": "episodes",
                "columnOrder": 1,
                "visible": false
            }
        ])
    );

    let old_client_update = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
            dateTimeFormat
          }
        }
        "#,
        json!({
            "input": {
                "theme": "dark",
                "highlightColor": "#ff3366",
                "secondaryColor": "#2277aa",
                "highContrastMode": true,
                "reduceMotion": true,
                "density": "compact",
                "sidebarMode": "collapsed",
                "defaultLandingView": "calendar",
                "tableColumns": []
            }
        }),
    )
    .await;
    assert_no_errors(&old_client_update);
    assert_eq!(
        old_client_update["data"]["setMyUiSettings"]["dateTimeFormat"],
        json!("iso24h")
    );
}

#[tokio::test]
async fn graphql_ui_settings_are_isolated_per_logged_in_user() {
    let ctx = TestContext::new().await;
    let user_a = create_ui_settings_test_user(&ctx, "ui_user_a").await;
    let user_b = create_ui_settings_test_user(&ctx, "ui_user_b").await;

    let update_a = schema_exec(
        &ctx,
        r##"
        mutation SetMyUiSettings {
          setMyUiSettings(input: {
            theme: system
            dateTimeFormat: iso24h
            highlightColor: "#112233"
            secondaryColor: "#445566"
            highContrastMode: false
            reduceMotion: true
            density: compact
            sidebarMode: collapsed
            defaultLandingView: series
            tableColumns: [
              {
                facet: series
                tableViewMode: posterTable
                columnId: "episodes"
                columnOrder: 0
                visible: true
              }
            ]
          }) {
            theme
            dateTimeFormat
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "##,
        Some(user_a.clone()),
    )
    .await;
    assert_no_errors(&update_a);

    let read_a = schema_exec(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            dateTimeFormat
            density
            sidebarMode
            defaultLandingView
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "#,
        Some(user_a),
    )
    .await;
    assert_no_errors(&read_a);
    assert_eq!(read_a["data"]["myUiSettings"]["theme"], json!("system"));
    assert_eq!(
        read_a["data"]["myUiSettings"]["dateTimeFormat"],
        json!("iso24h")
    );
    assert_eq!(
        read_a["data"]["myUiSettings"]["tableColumns"],
        json!([
            {
                "facet": "series",
                "tableViewMode": "posterTable",
                "columnId": "episodes",
                "columnOrder": 0,
                "visible": true
            }
        ])
    );

    let read_b = schema_exec(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            dateTimeFormat
            density
            sidebarMode
            defaultLandingView
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "#,
        Some(user_b),
    )
    .await;
    assert_no_errors(&read_b);
    assert_eq!(read_b["data"]["myUiSettings"]["theme"], json!("dark"));
    assert_eq!(
        read_b["data"]["myUiSettings"]["dateTimeFormat"],
        json!("locale")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["density"],
        json!("comfortable")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["sidebarMode"],
        json!("expanded")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["defaultLandingView"],
        json!("movies")
    );
    assert_eq!(read_b["data"]["myUiSettings"]["tableColumns"], json!([]));
}

#[tokio::test]
async fn graphql_ui_settings_reject_invalid_table_column() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
          }
        }
        "#,
        json!({
            "input": {
                "theme": "dark",
                "dateTimeFormat": "locale",
                "highlightColor": null,
                "secondaryColor": null,
                "highContrastMode": false,
                "reduceMotion": false,
                "density": "comfortable",
                "sidebarMode": "expanded",
                "defaultLandingView": "movies",
                "tableColumns": [
                    {
                        "facet": "movies",
                        "tableViewMode": "compact",
                        "columnId": "poster",
                        "columnOrder": 0,
                        "visible": true
                    }
                ]
            }
        }),
    )
    .await;

    let errors = body["errors"].as_array().expect("GraphQL errors");
    assert!(
        errors
            .first()
            .and_then(|error| error["message"].as_str())
            .is_some_and(|message| message.contains("unsupported compact table column")),
        "expected unsupported table column validation error: {body}"
    );
}
