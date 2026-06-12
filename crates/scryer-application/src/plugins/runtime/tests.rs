#[cfg(test)]
mod indexer_config_reconciliation_tests {
    use super::*;

    fn field(
        key: &str,
        role: Option<scryer_domain::ConfigFieldRole>,
        field_type: scryer_domain::ConfigFieldType,
        required: bool,
        default_value: Option<&str>,
    ) -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            key: key.to_string(),
            label: key.to_string(),
            field_type,
            required,
            default_value: default_value.map(str::to_string),
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role,
            host_binding: None,
            options: Vec::new(),
            help_text: None,
        }
    }

    #[test]
    fn auto_create_allows_defaulted_connection_url_only() {
        let fields = vec![field(
            "base_url",
            Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            scryer_domain::ConfigFieldType::String,
            true,
            Some("https://indexer.example"),
        )];

        assert!(indexer_config_can_be_auto_created(&fields));
    }

    #[test]
    fn auto_create_skips_required_user_secret_without_default() {
        let fields = vec![
            field(
                "base_url",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                scryer_domain::ConfigFieldType::String,
                true,
                Some("https://indexer.example"),
            ),
            field(
                "api_key",
                None,
                scryer_domain::ConfigFieldType::Password,
                true,
                None,
            ),
        ];

        assert!(!indexer_config_can_be_auto_created(&fields));
    }
}
#[cfg(test)]
mod sdk_compatibility_tests {
    use super::*;

    fn current_sdk_minor_line_constraint() -> String {
        let sdk_version = semver::Version::parse(SDK_VERSION).expect("valid SDK_VERSION");
        format!(
            ">={}.{}.0, <{}.{}.0",
            sdk_version.major,
            sdk_version.minor,
            sdk_version.major,
            sdk_version.minor + 1
        )
    }

    #[test]
    fn downloaded_plugin_release_host_compatibility_rejects_legacy_minor_line_constraint() {
        let release = DownloadedPluginReleaseContract {
            version: "0.2.0".to_string(),
            sdk_version: Some("2.3.0".to_string()),
            sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
            scryer_constraint: None,
        };

        assert!(!downloaded_plugin_release_is_host_compatible(
            "jellyfin", &release
        ));
    }

    #[test]
    fn downloaded_plugin_release_preserves_explicit_minor_line_override() {
        let release = DownloadedPluginReleaseContract {
            version: "0.2.0".to_string(),
            sdk_version: Some(SDK_VERSION.to_string()),
            sdk_constraint: current_sdk_minor_line_constraint(),
            scryer_constraint: None,
        };

        assert_eq!(
            normalized_release_sdk_constraint(&release),
            current_sdk_minor_line_constraint()
        );
        assert!(downloaded_plugin_release_is_host_compatible(
            "jellyfin", &release
        ));
    }

    #[test]
    fn installation_sdk_contract_is_host_compatible_rejects_legacy_minor_line_constraint() {
        let installation = PluginInstallation {
            id: "install-1".to_string(),
            plugin_id: "jellyfin".to_string(),
            name: "Jellyfin".to_string(),
            description: "Jellyfin notifications".to_string(),
            version: "0.2.0".to_string(),
            sdk_version: "2.3.0".to_string(),
            sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
            scryer_constraint: None,
            plugin_type: "notification".to_string(),
            provider_type: "jellyfin".to_string(),
            source_kind: PluginSourceKind::Downloaded,
            is_enabled: true,
            is_builtin: false,
            wasm_encoding: PluginWasmEncoding::Identity,
            wasm_digest_algo: None,
            source_url: None,
            support_tier: PluginSupportTier::Official,
            publisher: None,
            docs_url: None,
            source_repo: None,
            manifest_url: None,
            wasm_digest: None,
            artifact_digest: None,
            descriptor_json: None,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!installation_sdk_contract_is_host_compatible(&installation));
    }

    #[test]
    fn latest_compatible_child_release_skips_legacy_minor_line_constraint() {
        let catalog = ChildCatalog {
            schema_version: "scryer.plugin.child_catalog.v2".to_string(),
            id: "email".to_string(),
            name: "Email".to_string(),
            description: "Email notifications".to_string(),
            plugin_type: "notification".to_string(),
            provider_type: "email".to_string(),
            publisher: "scryer".to_string(),
            support_tier: PluginSupportTier::Official,
            docs_url: "https://github.com/scryer-media/scryer-plugins".to_string(),
            source_repo: "https://github.com/scryer-media/scryer-plugins".to_string(),
            releases: vec![
                ChildCatalogRelease {
                    version: "0.1.0".to_string(),
                    sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
                    artifact_manifest_url: "https://example.invalid/email-v0.1.0.manifest.json"
                        .to_string(),
                },
                ChildCatalogRelease {
                    version: "0.2.0".to_string(),
                    sdk_constraint: current_sdk_minor_line_constraint(),
                    artifact_manifest_url: "https://example.invalid/email-v0.2.0.manifest.json"
                        .to_string(),
                },
            ],
        };

        let selected = latest_compatible_child_release(&catalog).expect("compatible release");

        assert_eq!(selected.version, "0.2.0");
    }
}
#[cfg(test)]
mod catalog_artifact_selection_tests {
    use super::*;
    use crate::services::RuntimePerformanceClass;
    use std::collections::HashSet;

    fn artifact(required_features: &[&str], url: &str) -> CatalogV3PluginArtifact {
        CatalogV3PluginArtifact {
            runtime: CATALOG_V3_RUNTIME_WASIP1.to_string(),
            required_features: required_features
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            url: url.to_string(),
            mirror_urls: Vec::new(),
            signature_url: format!("{url}.sig"),
            signature_mirror_urls: Vec::new(),
            digests: vec!["sha256:artifact".to_string()],
            wasm_digests: vec!["sha256:wasm".to_string()],
            bytes: 1234,
        }
    }

    fn release(artifacts: Vec<CatalogV3PluginArtifact>) -> CatalogV3PluginRelease {
        CatalogV3PluginRelease {
            version: "1.0.0".to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            min_scryer_version: None,
            artifacts,
        }
    }

    fn plugin(releases: Vec<CatalogV3PluginRelease>) -> CatalogV3PluginEntry {
        CatalogV3PluginEntry {
            id: "alpha".to_string(),
            name: "Alpha".to_string(),
            description: "Alpha plugin".to_string(),
            plugin_type: "indexer".to_string(),
            provider_type: "alpha".to_string(),
            publisher: "scryer".to_string(),
            support_tier: PluginSupportTier::Official,
            status: PluginLifecycleStatus::Active,
            docs_url: "https://example.invalid/docs".to_string(),
            source_repo: "https://github.com/scryer-media/alpha".to_string(),
            required_signer: RequiredSigner {
                github_repository: "scryer-media/alpha".to_string(),
                github_workflow: None,
            },
            releases,
        }
    }

    #[test]
    fn catalog_selection_skips_sdk2_release_and_selects_sdk3_release() {
        let mut sdk2_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-sdk2.zst",
        )]);
        sdk2_release.version = "1.0.0".to_string();
        sdk2_release.sdk_constraint = ">=2.3.0, <3.0.0".to_string();

        let mut sdk3_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-sdk3.zst",
        )]);
        sdk3_release.version = "2.0.0".to_string();
        sdk3_release.sdk_constraint = ">=3.0.0, <4.0.0".to_string();

        let plugin = plugin(vec![sdk2_release, sdk3_release]);

        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &HashSet::new(),
            RuntimePerformanceClass::Slow,
        )
        .expect("SDK 3 release");

        assert_eq!(selected_release.version, "2.0.0");
        assert_eq!(selected_artifact.url, "https://example.invalid/plugin-sdk3.zst");
    }

    #[test]
    fn empty_feature_set_selects_baseline_artifact() {
        let release = release(vec![
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-simd.br",
            ),
            artifact(&[], "https://example.invalid/plugin.zst"),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &HashSet::new(),
            RuntimePerformanceClass::Slow,
        )
        .expect("baseline artifact");

        assert_eq!(selected.required_features, Vec::<String>::new());
        assert_eq!(selected.url, "https://example.invalid/plugin.zst");
    }

    #[test]
    fn simd128_feature_set_selects_simd128_but_not_relaxed_simd() {
        let release = release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(&["simd128"], "https://example.invalid/plugin-simd.br"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.br",
            ),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &HashSet::from(["simd128".to_string()]),
            RuntimePerformanceClass::Slow,
        )
        .expect("simd128 artifact");

        assert_eq!(selected.required_features, vec!["simd128".to_string()]);
        assert_eq!(selected.url, "https://example.invalid/plugin-simd.br");
    }

    #[test]
    fn full_simd_feature_set_selects_relaxed_simd_artifact() {
        let release = release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(&["simd128"], "https://example.invalid/plugin-simd.zst"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.br",
            ),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.zst",
            ),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &HashSet::from(["simd128".to_string(), "relaxed-simd".to_string()]),
            RuntimePerformanceClass::Slow,
        )
        .expect("relaxed simd artifact");

        assert_eq!(
            selected.required_features,
            vec!["simd128".to_string(), "relaxed-simd".to_string()]
        );
        assert_eq!(selected.url, "https://example.invalid/plugin-relaxed.zst");
    }

    #[test]
    fn portable_native_build_can_select_simd_artifact_from_runtime_features() {
        let plugin = plugin(vec![release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.zst",
            ),
        ])]);

        let (_, selected) = select_catalog_release_and_artifact(
            &plugin,
            &HashSet::from(["simd128".to_string(), "relaxed-simd".to_string()]),
            RuntimePerformanceClass::Slow,
        )
        .expect("runtime feature selection should not depend on native build class");

        assert_eq!(
            selected.required_features,
            vec!["simd128".to_string(), "relaxed-simd".to_string()]
        );
    }

    #[test]
    fn catalog_selection_skips_release_requiring_newer_scryer() {
        let compatible_release = release(vec![artifact(&[], "https://example.invalid/plugin.zst")]);
        let mut newer_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-v2.zst",
        )]);
        newer_release.version = "2.0.0".to_string();
        newer_release.min_scryer_version = Some("999.0.0".to_string());

        let plugin = plugin(vec![compatible_release, newer_release]);

        let (selected_release, _) = select_catalog_release_and_artifact(
            &plugin,
            &HashSet::new(),
            RuntimePerformanceClass::Slow,
        )
        .expect("compatible release");
        let blocked_release = latest_host_blocked_catalog_release(&plugin, &HashSet::new())
            .expect("newer host-blocked release");

        assert_eq!(selected_release.version, "1.0.0");
        assert_eq!(blocked_release.version, "2.0.0");
        assert_eq!(
            blocked_release.min_scryer_version,
            Some("999.0.0".to_string())
        );
    }
}

#[cfg(test)]
mod signature_bundle_decode_tests {
    use super::*;

    #[tokio::test]
    async fn plain_signature_bundle_is_left_unchanged() {
        let bundle = br#"{"base64Signature":"signature"}"#.to_vec();

        let decoded = decode_signature_bundle(
            bundle.clone(),
            "https://example.test/plugin.tar.zst.bundle",
        )
        .await
        .expect("plain bundle should decode");

        assert_eq!(decoded, bundle);
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[tokio::test]
    async fn zstd_signature_bundle_is_decoded_from_url() {
        let bundle = br#"{"base64Signature":"signature"}"#.to_vec();
        let compressed = compress_zstd(bundle.clone(), 3)
            .await
            .expect("bundle should compress");

        let decoded = decode_signature_bundle(
            compressed,
            "https://example.test/catalog.json.bundle.zst",
        )
        .await
        .expect("zstd bundle should decode");

        assert_eq!(decoded, bundle);
    }
}

#[cfg(test)]
mod plugin_http_client_tests {
    use super::{PluginHttpClientProfile, plugin_http_client};

    #[test]
    fn plugin_http_client_profiles_are_cached() {
        let default_a = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should build") as *const _;
        let default_b = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should stay cached")
            as *const _;
        let rule_pack_a = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should build")
            as *const _;
        let rule_pack_b = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should stay cached")
            as *const _;

        assert_eq!(default_a, default_b);
        assert_eq!(rule_pack_a, rule_pack_b);
        assert_ne!(default_a, rule_pack_a);
    }
}
#[cfg(all(test, feature = "runtime-plugin-trust"))]
#[path = "../app_usecase_plugins_tests.rs"]
mod app_usecase_plugins_tests;
