use std::collections::HashMap;

use crate::{IndexerSearchResult, ParsedReleaseMetadata, QualityProfile, QualityProfileDecision};

pub(crate) struct ReleaseRuntimeInfo<'a> {
    pub size_bytes: Option<i64>,
    pub published_at: Option<&'a str>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub is_password_protected: Option<bool>,
    pub extra: Option<&'a HashMap<String, serde_json::Value>>,
    pub indexer_languages: Option<&'a [String]>,
}

pub(crate) struct RuleContextInfo<'a> {
    pub title_id: Option<&'a str>,
    pub library_name: Option<&'a str>,
    pub category: Option<&'a str>,
    pub title_tags: &'a [String],
    pub has_existing_file: bool,
    pub existing_score: Option<i32>,
    pub search_mode: &'a str,
    pub runtime_minutes: Option<i32>,
    pub is_filler: bool,
}

pub(crate) fn build_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    decision: &QualityProfileDecision,
    release_runtime: ReleaseRuntimeInfo<'_>,
    context: RuleContextInfo<'_>,
    file: Option<scryer_rules::FileDoc>,
) -> scryer_rules::UserRuleInput {
    use scryer_rules::*;

    let category = context.category.unwrap_or("unknown");
    let is_anime = context
        .title_tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("anime"))
        || category.eq_ignore_ascii_case("anime");
    let has_release_group = parsed
        .release_group
        .as_ref()
        .is_some_and(|group| !group.trim().is_empty());
    let is_obfuscated = is_obfuscated_release(parsed);
    let is_retagged = is_retagged_release(parsed);
    let (episode_release_type, is_season_pack, is_multi_episode) = release_type_details(parsed);

    let languages_audio =
        crate::release_audio_language_hints(parsed, release_runtime.indexer_languages);

    UserRuleInput {
        release: ReleaseDoc {
            raw_title: parsed.raw_title.clone(),
            quality: parsed.quality.clone(),
            source: parsed.source.as_ref().map(ToString::to_string),
            video_codec: parsed.video_codec.as_ref().map(ToString::to_string),
            audio: parsed.audio.as_ref().map(ToString::to_string),
            audio_codecs: parsed
                .audio_codecs
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_channels: parsed.audio_channels.clone(),
            languages_audio,
            languages_subtitles: parsed.languages_subtitles.clone(),
            is_dual_audio: parsed.is_dual_audio,
            is_atmos: parsed.is_atmos,
            is_dolby_vision: parsed.is_dolby_vision,
            detected_hdr: parsed.detected_hdr,
            is_remux: parsed.is_remux,
            is_bd_disk: parsed.is_bd_disk,
            is_proper_upload: parsed.is_proper_upload,
            is_repack: parsed.is_repack,
            is_ai_enhanced: parsed.is_ai_enhanced,
            is_hardcoded_subs: parsed.is_hardcoded_subs,
            is_password_protected: release_runtime.is_password_protected,
            is_hdr10plus: parsed.is_hdr10plus,
            is_hlg: parsed.is_hlg,
            is_10bit: parsed.is_10bit,
            is_uncensored: parsed.is_uncensored,
            is_dubs_only: parsed.is_dubs_only,
            has_release_group,
            is_obfuscated,
            is_retagged,
            streaming_service: parsed.streaming_service.as_ref().map(ToString::to_string),
            edition: parsed.edition.clone(),
            anime_version: parsed.anime_version,
            episode_release_type,
            is_season_pack,
            is_multi_episode,
            release_group: parsed.release_group.clone(),
            year: parsed.year.and_then(|year| u32::try_from(year).ok()),
            parse_confidence: parsed.parse_confidence,
            size_bytes: release_runtime.size_bytes,
            age_days: release_runtime
                .published_at
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| (chrono::Utc::now() - value.with_timezone(&chrono::Utc)).num_days()),
            thumbs_up: release_runtime.thumbs_up,
            thumbs_down: release_runtime.thumbs_down,
            extra: release_runtime.extra.cloned().unwrap_or_default(),
        },
        profile: ProfileDoc {
            id: profile.id.clone(),
            name: profile.name.clone(),
            quality_tiers: profile.criteria.quality_tiers.clone(),
            archival_quality: profile.criteria.archival_quality.clone(),
            allow_unknown_quality: profile.criteria.allow_unknown_quality,
            source_allowlist: profile
                .criteria
                .source_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            source_blocklist: profile
                .criteria
                .source_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            video_codec_allowlist: profile
                .criteria
                .video_codec_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            video_codec_blocklist: profile
                .criteria
                .video_codec_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_codec_allowlist: profile
                .criteria
                .audio_codec_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            audio_codec_blocklist: profile
                .criteria
                .audio_codec_blocklist
                .iter()
                .map(ToString::to_string)
                .collect(),
            atmos_preferred: profile.criteria.atmos_preferred,
            dolby_vision_allowed: profile.criteria.dolby_vision_allowed,
            detected_hdr_allowed: profile.criteria.detected_hdr_allowed,
            prefer_remux: profile.criteria.prefer_remux,
            allow_bd_disk: profile.criteria.allow_bd_disk,
            allow_upgrades: profile.criteria.allow_upgrades,
            prefer_dual_audio: profile.criteria.prefer_dual_audio,
            required_audio_languages: profile.criteria.required_audio_languages.clone(),
        },
        context: ContextDoc {
            title_id: context.title_id.map(str::to_owned),
            library_name: context.library_name.map(str::to_owned),
            media_type: category.to_string(),
            category: category.to_string(),
            tags: context.title_tags.to_vec(),
            has_existing_file: context.has_existing_file,
            existing_score: context.existing_score,
            search_mode: context.search_mode.to_string(),
            runtime_minutes: context.runtime_minutes,
            is_anime,
            is_filler: context.is_filler,
        },
        builtin_score: BuiltinScoreDoc {
            total: decision.release_score,
            blocked: !decision.allowed,
            codes: decision
                .scoring_log
                .iter()
                .map(|entry| entry.code.clone())
                .collect(),
        },
        file,
    }
}

fn release_type_details(parsed: &ParsedReleaseMetadata) -> (Option<String>, bool, bool) {
    let Some(ref episode) = parsed.episode else {
        return (None, false, false);
    };

    let kind = match episode.release_type {
        crate::ParsedEpisodeReleaseType::SingleEpisode => "single_episode",
        crate::ParsedEpisodeReleaseType::MultiEpisode => "multi_episode",
        crate::ParsedEpisodeReleaseType::RangePack => "multi_episode",
        crate::ParsedEpisodeReleaseType::SeasonPack => "season_pack",
        crate::ParsedEpisodeReleaseType::Daily => "single_episode",
        crate::ParsedEpisodeReleaseType::Unknown => "unknown",
    };

    (
        Some(kind.to_string()),
        matches!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::SeasonPack
        ) || episode.full_season
            || episode.is_partial_season
            || episode.is_multi_season,
        matches!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::MultiEpisode
                | crate::ParsedEpisodeReleaseType::SeasonPack
        ) || episode.episode_numbers.len() > 1
            || episode.absolute_episode_numbers.len() > 1,
    )
}

fn is_obfuscated_release(parsed: &ParsedReleaseMetadata) -> bool {
    crate::helpers::is_obfuscated_release_name(parsed)
}

fn is_retagged_release(parsed: &ParsedReleaseMetadata) -> bool {
    let lower = parsed.raw_title.to_ascii_lowercase();
    [
        "[rartv]", "rarbg", "[tgx]", "eztvx", "ettv", "yts.mx", "yts",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[derive(Clone, Copy)]
pub(crate) struct SearchRuleInputContext<'a> {
    pub(crate) category: Option<&'a str>,
    pub(crate) library_name: Option<&'a str>,
    pub(crate) title_tags: &'a [String],
    pub(crate) runtime_minutes: Option<i32>,
}

pub(crate) fn build_search_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    result: &IndexerSearchResult,
    decision: &QualityProfileDecision,
    context: SearchRuleInputContext<'_>,
) -> scryer_rules::UserRuleInput {
    build_rule_input(
        parsed,
        profile,
        decision,
        ReleaseRuntimeInfo {
            size_bytes: result.size_bytes,
            published_at: result.published_at.as_deref(),
            thumbs_up: result.thumbs_up,
            thumbs_down: result.thumbs_down,
            is_password_protected: result
                .extra
                .get("password_protected")
                .and_then(|value| value.as_bool())
                .or_else(|| {
                    crate::release_password_protection_hint(result.password_hint.as_deref())
                }),
            extra: Some(&result.extra),
            indexer_languages: result.indexer_languages.as_deref(),
        },
        RuleContextInfo {
            title_id: None,
            library_name: context.library_name,
            category: context.category,
            title_tags: context.title_tags,
            has_existing_file: false,
            existing_score: None,
            search_mode: "auto",
            runtime_minutes: context.runtime_minutes,
            is_filler: false,
        },
        None,
    )
}

#[cfg(feature = "runtime-media-analysis")]
pub(crate) fn build_file_doc(analysis: &scryer_mediainfo::MediaAnalysis) -> scryer_rules::FileDoc {
    let audio_languages = crate::normalize_detected_audio_languages(
        analysis.audio_languages.iter().map(String::as_str),
    );
    let subtitle_languages = crate::normalize_detected_subtitle_languages(
        analysis.subtitle_languages.iter().map(String::as_str),
    );

    scryer_rules::FileDoc {
        video_codec: analysis.video_codec.clone(),
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format.clone(),
        dovi_profile: analysis.dovi_profile,
        dovi_bl_compat_id: analysis.dovi_bl_compat_id,
        video_frame_rate: analysis.video_frame_rate.clone(),
        video_profile: analysis.video_profile.clone(),
        audio_codec: analysis.audio_codec.clone(),
        audio_profile: analysis.audio_profile.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages,
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| scryer_rules::AudioStreamDoc {
                codec: stream.codec.clone(),
                profile: stream.profile.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages,
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| scryer_rules::SubtitleStreamDoc {
                codec: stream.codec.clone(),
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_subtitle_language_code),
                name: stream.name.clone(),
                forced: stream.forced,
                default: stream.default,
            })
            .collect(),
        has_multiaudio: analysis.has_multiaudio,
        duration_seconds: analysis.duration_seconds,
        num_chapters: analysis.num_chapters,
        container_format: analysis.container_format.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{QualityProfileCriteria, ScoringSource};
    use std::collections::HashMap;

    fn test_profile() -> QualityProfile {
        QualityProfile {
            id: "profile".to_string(),
            name: "Profile".to_string(),
            criteria: QualityProfileCriteria {
                quality_tiers: vec!["2160P".to_string(), "1080P".to_string()],
                archival_quality: Some("2160P".to_string()),
                allow_unknown_quality: false,
                source_allowlist: vec![],
                source_blocklist: vec![],
                video_codec_allowlist: vec![],
                video_codec_blocklist: vec![],
                audio_codec_allowlist: vec![],
                audio_codec_blocklist: vec![],
                atmos_preferred: false,
                dolby_vision_allowed: true,
                detected_hdr_allowed: true,
                prefer_remux: false,
                allow_bd_disk: false,
                allow_upgrades: true,
                prefer_dual_audio: false,
                required_audio_languages: vec![],
                scoring_persona: crate::ScoringPersona::Balanced,
                scoring_overrides: crate::ScoringOverrides::default(),
                cutoff_tier: None,
                min_score_to_grab: None,
                facet_persona_overrides: HashMap::new(),
            },
        }
    }

    fn test_decision() -> QualityProfileDecision {
        QualityProfileDecision {
            release_score: 1200,
            scoring_log: vec![crate::ScoringEntry {
                code: "quality_tier_0".to_string(),
                delta: 1200,
                source: ScoringSource::Builtin,
            }],
            allowed: true,
            block_codes: vec![],
            preference_score: 1200,
        }
    }

    fn test_parsed() -> ParsedReleaseMetadata {
        crate::parse_release_metadata("Test.Movie.2024.2160p.WEB-DL.H.265.DDP5.1-Group")
    }

    #[test]
    fn build_search_rule_input_keeps_file_null() {
        let input = build_search_rule_input(
            &test_parsed(),
            &test_profile(),
            &IndexerSearchResult {
                source: "test-indexer".to_string(),
                title: "Test Movie".to_string(),
                link: None,
                download_url: None,
                source_kind: None,
                size_bytes: Some(8_000_000_000),
                published_at: Some("2026-03-10T12:00:00Z".to_string()),
                thumbs_up: Some(5),
                thumbs_down: Some(1),
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: Some("secret".to_string()),
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: HashMap::from([("indexer".to_string(), serde_json::json!("test"))]),
                guid: None,
                info_url: None,
                provenance: None,
                candidate_token: None,
                queue_scope: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
            },
            &test_decision(),
            SearchRuleInputContext {
                category: Some("movie"),
                library_name: Some("Movies"),
                title_tags: &["anime".to_string()],
                runtime_minutes: Some(120),
            },
        );

        let value = serde_json::to_value(input).unwrap();
        assert!(value["file"].is_null());
        assert_eq!(value["context"]["library_name"], "Movies");
        assert_eq!(value["release"]["extra"]["indexer"], "test");
        assert_eq!(value["release"]["is_password_protected"], true);
    }

    #[test]
    fn build_search_rule_input_normalizes_password_placeholders() {
        for (password_hint, expected) in [
            (Some("0".to_string()), Some(false)),
            (Some("false".to_string()), Some(false)),
            (Some("no".to_string()), Some(false)),
            (Some("1".to_string()), Some(true)),
            (Some("true".to_string()), Some(true)),
            (Some("protected".to_string()), Some(true)),
            (Some("   ".to_string()), None),
        ] {
            let input = build_search_rule_input(
                &test_parsed(),
                &test_profile(),
                &IndexerSearchResult {
                    source: "test-indexer".to_string(),
                    title: "Test Movie".to_string(),
                    link: None,
                    download_url: None,
                    source_kind: None,
                    size_bytes: Some(8_000_000_000),
                    published_at: Some("2026-03-10T12:00:00Z".to_string()),
                    thumbs_up: Some(5),
                    thumbs_down: Some(1),
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint,
                    parsed_release_metadata: None,
                    quality_profile_decision: None,
                    extra: HashMap::new(),
                    guid: None,
                    info_url: None,
                    provenance: None,
                    candidate_token: None,
                    queue_scope: None,
                    auto_eligible: None,
                    auto_decision_code: None,
                    auto_decision_summary: None,
                },
                &test_decision(),
                SearchRuleInputContext {
                    category: Some("movie"),
                    library_name: Some("Movies"),
                    title_tags: &[],
                    runtime_minutes: Some(120),
                },
            );

            let value = serde_json::to_value(input).unwrap();
            match expected {
                Some(value_expected) => {
                    assert_eq!(value["release"]["is_password_protected"], value_expected);
                }
                None => {
                    assert!(
                        value["release"]["is_password_protected"].is_null(),
                        "empty password hints should normalize away"
                    );
                }
            }
        }
    }

    #[test]
    fn build_search_rule_input_preserves_protection_hint_from_extra() {
        let input = build_search_rule_input(
            &test_parsed(),
            &test_profile(),
            &IndexerSearchResult {
                source: "test-indexer".to_string(),
                title: "Test Movie".to_string(),
                link: None,
                download_url: None,
                source_kind: None,
                size_bytes: Some(8_000_000_000),
                published_at: Some("2026-03-10T12:00:00Z".to_string()),
                thumbs_up: Some(5),
                thumbs_down: Some(1),
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: Some("1".to_string()),
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: HashMap::from([(
                    "password_protected".to_string(),
                    serde_json::Value::from(true),
                )]),
                guid: None,
                info_url: None,
                provenance: None,
                candidate_token: None,
                queue_scope: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
            },
            &test_decision(),
            SearchRuleInputContext {
                category: Some("movie"),
                library_name: Some("Movies"),
                title_tags: &[],
                runtime_minutes: Some(120),
            },
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["release"]["is_password_protected"], true);
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[test]
    fn build_rule_input_populates_post_download_file_doc() {
        let analysis = scryer_mediainfo::analyze_file(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("scryer-mediainfo")
                .join("tests")
                .join("media")
                .join("h264_aac.mkv"),
        )
        .unwrap();

        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: Some(1234),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: Some("title-1"),
                library_name: Some("Movies"),
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: true,
                existing_score: Some(900),
                search_mode: "post_download",
                runtime_minutes: Some(120),
                is_filler: false,
            },
            Some(build_file_doc(&analysis)),
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["context"]["search_mode"], "post_download");
        assert_eq!(value["context"]["existing_score"], 900);
        assert!(value["release"]["is_password_protected"].is_null());
        assert_eq!(value["file"]["num_chapters"], 0);
        assert_eq!(value["file"]["audio_profile"], "LC");
        assert_eq!(value["file"]["audio_streams"][0]["codec"], "aac");
        assert_eq!(value["file"]["audio_streams"][0]["profile"], "LC");
    }

    #[test]
    fn indexer_languages_merged_into_languages_audio() {
        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&["English".to_string(), "French".to_string()]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        let langs = &input.release.languages_audio;
        assert!(
            langs.contains(&"eng".to_string()),
            "should contain eng from indexer"
        );
        assert!(
            langs.contains(&"fra".to_string()),
            "should contain fra from indexer"
        );
    }

    #[test]
    fn indexer_languages_deduplicates_with_parsed() {
        // The test title parses no languages, but let's verify dedup with a title that does
        let parsed = crate::parse_release_metadata("Test.Movie.2024.FRENCH.2160p.WEB-DL");
        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&["French".to_string()]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        let fra_count = input
            .release
            .languages_audio
            .iter()
            .filter(|l| *l == "fra")
            .count();
        assert_eq!(fra_count, 1, "French should not be duplicated");
    }

    #[test]
    fn indexer_languages_support_full_iso_language_names() {
        let input = build_rule_input(
            &test_parsed(),
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: Some(&[
                    "Filipino".to_string(),
                    "English, Middle (1100-1500)".to_string(),
                ]),
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        assert!(input.release.languages_audio.contains(&"fil".to_string()));
        assert!(input.release.languages_audio.contains(&"enm".to_string()));
    }

    #[test]
    fn build_rule_input_exposes_episode_release_type_fields() {
        let parsed = crate::parse_release_metadata("Test.Show.S01.COMPLETE.1080p.WEB-DL-Group");
        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: Some("series-1"),
                library_name: Some("Series"),
                category: Some("series"),
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["release"]["episode_release_type"], "season_pack");
        assert_eq!(value["release"]["is_season_pack"], true);
        assert_eq!(value["release"]["is_multi_episode"], true);
    }

    #[test]
    fn build_rule_input_exposes_release_provenance_flags() {
        let mut parsed = test_parsed();
        parsed.raw_title = "Test.Movie.2024.1080p.WEB-DL.A1B2C3D4E5.RARBG".to_string();
        parsed.release_group = None;

        let input = build_rule_input(
            &parsed,
            &test_profile(),
            &test_decision(),
            ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "auto",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        let value = serde_json::to_value(input).unwrap();
        assert_eq!(value["release"]["has_release_group"], false);
        assert_eq!(value["release"]["is_obfuscated"], true);
        assert_eq!(value["release"]["is_retagged"], true);
    }
}
