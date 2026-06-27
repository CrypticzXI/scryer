use super::*;

#[tokio::test]
async fn graphql_media_settings_rejects_invalid_folder_template_tokens() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            folderTemplate
          }
        }
        "#,
        json!({
          "input": {
            "scope": "movie",
            "folderTemplate": "{quality}"
          }
        }),
    )
    .await;

    let errors = body["errors"]
        .as_array()
        .expect("invalid folder template should return graphql errors");
    assert!(!errors.is_empty());
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsupported folder template token"));
}

#[tokio::test]
async fn graphql_media_settings_rejects_invalid_rename_template_tokens() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameTemplate
          }
        }
        "#,
        json!({
          "input": {
            "scope": "movie",
            "renameTemplate": "{title|truncate:0}.{ext}"
          }
        }),
    )
    .await;

    let errors = body["errors"]
        .as_array()
        .expect("invalid rename template should return graphql errors");
    assert!(!errors.is_empty());
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsupported rename template token"));
}

#[tokio::test]
async fn graphql_typed_media_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            libraryPath
            rootFolders { path isDefault }
            requiredAudioLanguages
            folderTemplate
            renameEnabled
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({
          "input": {
            "scope": "anime",
            "rootFolders": [
              { "path": "/library/anime-main", "isDefault": true },
              { "path": "/library/anime-archive", "isDefault": false }
            ],
            "requiredAudioLanguages": ["eng", "jpn"],
            "folderTemplate": "{title|truncate:64|space:_} ({year})",
            "renameEnabled": false,
            "renameTemplate": "{title|truncate:64|space:_} [{quality}].{ext}",
            "renameCollisionPolicy": "replace_if_better",
            "renameMissingMetadataPolicy": "skip",
            "fillerPolicy": "skip_filler",
            "recapPolicy": "skip_recap",
            "monitorSpecials": true,
            "interSeasonMovies": false,
            "monitorFillerMovies": true,
            "nfoWriteOnImport": true,
            "plexmatchWriteOnImport": true
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let updated = &update["data"]["updateMediaSettings"];
    assert_eq!(updated["scope"], "anime");
    assert_eq!(updated["libraryPath"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["path"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["isDefault"], true);
    assert_eq!(updated["requiredAudioLanguages"][0], "eng");
    assert_eq!(updated["requiredAudioLanguages"][1], "jpn");
    assert_eq!(
        updated["folderTemplate"],
        "{title|truncate:64|space:_} ({year})"
    );
    assert_eq!(updated["renameEnabled"], false);
    assert_eq!(
        updated["renameTemplate"],
        "{title|truncate:64|space:_} [{quality}].{ext}"
    );
    assert_eq!(updated["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(updated["renameMissingMetadataPolicy"], "skip");
    assert_eq!(updated["fillerPolicy"], "skip_filler");
    assert_eq!(updated["recapPolicy"], "skip_recap");
    assert_eq!(updated["monitorSpecials"], true);
    assert_eq!(updated["interSeasonMovies"], false);
    assert_eq!(updated["monitorFillerMovies"], true);
    assert_eq!(updated["nfoWriteOnImport"], true);
    assert_eq!(updated["plexmatchWriteOnImport"], true);

    let read = gql(
        &ctx,
        r#"
        query MediaSettings($scope: ContentScopeValue!) {
          mediaSettings(scope: $scope) {
            scope
            libraryPath
            rootFolders { path isDefault }
            requiredAudioLanguages
            folderTemplate
            renameEnabled
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({ "scope": "anime" }),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["mediaSettings"];
    assert_eq!(settings["scope"], "anime");
    assert_eq!(settings["libraryPath"], "/library/anime-main");
    assert_eq!(settings["rootFolders"][1]["path"], "/library/anime-archive");
    assert_eq!(settings["requiredAudioLanguages"][0], "eng");
    assert_eq!(settings["requiredAudioLanguages"][1], "jpn");
    assert_eq!(
        settings["folderTemplate"],
        "{title|truncate:64|space:_} ({year})"
    );
    assert_eq!(settings["renameEnabled"], false);
    assert_eq!(
        settings["renameTemplate"],
        "{title|truncate:64|space:_} [{quality}].{ext}"
    );
    assert_eq!(settings["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(settings["renameMissingMetadataPolicy"], "skip");
    assert_eq!(settings["fillerPolicy"], "skip_filler");
    assert_eq!(settings["recapPolicy"], "skip_recap");
    assert_eq!(settings["monitorSpecials"], true);
    assert_eq!(settings["interSeasonMovies"], false);
    assert_eq!(settings["monitorFillerMovies"], true);
    assert_eq!(settings["nfoWriteOnImport"], true);
    assert_eq!(settings["plexmatchWriteOnImport"], true);
}

#[tokio::test]
async fn graphql_typed_library_paths_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/mnt/storage/movies",
            "seriesPath": "/mnt/storage/series",
            "animePath": "/mnt/storage/anime"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateLibraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );

    let read = gql(
        &ctx,
        r#"
        query LibraryPaths {
          libraryPaths {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["libraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["seriesPath"],
        "/mnt/storage/series"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["animePath"],
        "/mnt/storage/anime"
    );
}

#[tokio::test]
async fn graphql_typed_service_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
          updateServiceSettings(input: $input) {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({
          "input": {
            "tlsCertPath": "/etc/scryer/tls.crt",
            "tlsKeyPath": "/etc/scryer/tls.key"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateServiceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );

    let read = gql(
        &ctx,
        r#"
        query ServiceSettings {
          serviceSettings {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["serviceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );
    assert_eq!(
        read["data"]["serviceSettings"]["tlsKeyPath"],
        "/etc/scryer/tls.key"
    );
}

#[tokio::test]
async fn graphql_typed_subtitle_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "subtitles.opensubtitles_api_key",
            None,
            json!("smg-managed-key").to_string(),
            "test",
            None,
        )
        .await
        .expect("subtitle api key should seed");
    let update = gql(
        &ctx,
        r#"
        mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
          updateSubtitleSettings(input: $input) {
            enabled
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "languages": [
              { "code": "eng", "hearingImpaired": true, "forced": false },
              { "code": "spa", "hearingImpaired": false, "forced": true }
            ],
            "autoDownloadOnImport": true,
            "minimumScoreSeries": 95,
            "minimumScoreMovie": 85,
            "searchIntervalHours": 12,
            "includeAiTranslated": true,
            "includeMachineTranslated": false,
            "syncEnabled": true,
            "syncThresholdSeries": 91,
            "syncThresholdMovie": 74,
            "syncMaxOffsetSeconds": 48
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query SubtitleSettings {
          subtitleSettings {
            enabled
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["subtitleSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["autoDownloadOnImport"], true);
    assert_eq!(settings["minimumScoreSeries"], 95);
    assert_eq!(settings["minimumScoreMovie"], 85);
    assert_eq!(settings["searchIntervalHours"], 12);
    assert_eq!(settings["includeAiTranslated"], true);
    assert_eq!(settings["includeMachineTranslated"], false);
    assert_eq!(settings["syncEnabled"], true);
    assert_eq!(settings["syncThresholdSeries"], 91);
    assert_eq!(settings["syncThresholdMovie"], 74);
    assert_eq!(settings["syncMaxOffsetSeconds"], 48);
    assert_eq!(settings["languages"][0]["code"], "eng");
    assert_eq!(settings["languages"][0]["hearingImpaired"], true);
    assert_eq!(settings["languages"][1]["code"], "spa");
    assert_eq!(settings["languages"][1]["forced"], true);
}

#[tokio::test]
async fn graphql_typed_acquisition_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
          updateAcquisitionSettings(input: $input) {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "upgradeCooldownHours": 18,
            "sameTierMinDelta": 140,
            "crossTierMinDelta": 35,
            "forcedUpgradeDeltaBypass": 420,
            "pollIntervalSeconds": 45,
            "syncIntervalSeconds": 1800,
            "batchSize": 25
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query AcquisitionSettings {
          acquisitionSettings {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["acquisitionSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["upgradeCooldownHours"], 18);
    assert_eq!(settings["sameTierMinDelta"], 140);
    assert_eq!(settings["crossTierMinDelta"], 35);
    assert_eq!(settings["forcedUpgradeDeltaBypass"], 420);
    assert_eq!(settings["pollIntervalSeconds"], 45);
    assert_eq!(settings["syncIntervalSeconds"], 1800);
    assert_eq!(settings["batchSize"], 25);
}

#[tokio::test]
async fn graphql_typed_general_settings_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            keepHistoryForever
            historyRetentionDays
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], false);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 180);
    assert_eq!(read["data"]["generalSettings"]["pluginHttpCaBundlePem"], "");
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpTrustedCertificates"],
        json!([])
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_round_trip_and_forever_preserves_days() {
    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let first_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 45,
            "pluginHttpCaBundlePem": TEST_PLUGIN_HTTP_CA_CERT_PEM
          }
        }),
    )
    .await;
    assert_no_errors(&first_update);
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );

    let forever_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": true,
            "historyRetentionDays": 0,
            "pluginHttpCaBundlePem": TEST_PLUGIN_HTTP_CA_CERT_PEM
          }
        }),
    )
    .await;
    assert_no_errors(&forever_update);
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["keepHistoryForever"],
        true
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            keepHistoryForever
            historyRetentionDays
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], true);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 45);
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_rejects_invalid_days() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let body = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 0
          }
        }),
    )
    .await;

    assert!(
        body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected validation errors: {body}"
    );
}
