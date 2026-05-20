use chrono::NaiveDate;

use crate::enrichment::{enrich_candidate, project_final_metadata};
use crate::{
    ContextAlias, ContextEpisode, ContextFacetHint, ContextTitle, ParseFamily,
    ParsedEpisodeReleaseType, ReleaseParseContext, VideoCodec, analyze_release_against_targets,
    analyze_release_for_target,
};

fn context(facet_hint: ContextFacetHint, title: &str) -> ReleaseParseContext {
    ReleaseParseContext {
        facet_hint,
        title: ContextTitle {
            name: title.to_string(),
        },
        aliases: Vec::new(),
        known_years: Vec::new(),
        imdb_ids: Vec::new(),
        episodes: Vec::new(),
    }
}

#[test]
fn lex_and_parse_standard_episode_release() {
    let analysis = analyze_release_for_target(
        "Show.Name.S01E02.1080p.WEB-DL.H264-Group",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(candidate.projected.normalized_title, "SHOW NAME");
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(candidate.projected.source.as_deref(), Some("WEB-DL"));
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SingleEpisode)
    );
}

#[test]
fn parses_sonarr_style_x_episode_release() {
    let analysis = analyze_release_for_target(
        "Show Name - 01x02 - The Episode WEBDL-1080p",
        &context(ContextFacetHint::Series, "Show Name"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![2]);
}

#[test]
fn parses_daily_release_with_part_marker() {
    let mut target = context(ContextFacetHint::Series, "Series Title");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Series.Title.2026.04.22.Part.2.720p.HULU.WEBRip.AAC2.0.H264-Group",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(
        episode.air_date,
        Some(NaiveDate::from_ymd_opt(2026, 4, 22).unwrap())
    );
    assert_eq!(episode.daily_part, Some(2));
}

#[test]
fn bounds_token_role_hypotheses_and_marks_pruning() {
    let analysis = analyze_release_for_target(
        "S01E01.2024.1080p.MULTI.AVC",
        &context(ContextFacetHint::Series, "Placeholder"),
    );
    assert!(
        analysis
            .annotations
            .iter()
            .all(|annotation| annotation.alternate_roles.len() <= 2)
    );
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "annotation:role_pruned")
            || analysis
                .annotations
                .iter()
                .all(|annotation| !annotation.role_pruned)
    );
}

#[test]
fn context_keeps_stacked_anime_aliases_as_title_variants() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Silver Horizon Beyond Journey's End",
    );
    target.aliases = vec![
        ContextAlias {
            name: "Sora no Vale".to_string(),
        },
        ContextAlias {
            name: "Silver Horizon Beyond the Vale".to_string(),
        },
    ];
    target.known_years.push(2023);

    let analysis = analyze_release_for_target(
        "[SubsPlease] Sora no Vale Silver Horizon Beyond the Vale - 01 [1080p] [HEVC]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title: &String| title.contains("SORA NO VALE"))
    );
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title: &String| title.contains("SILVER HORIZON BEYOND THE VALE"))
    );
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:title_alias_hit")
    );
}

#[test]
fn context_does_not_invent_absent_titles() {
    let mut target = context(ContextFacetHint::Series, "Completely Different Show");
    target.aliases = vec![ContextAlias {
        name: "Different Alias".to_string(),
    }];

    let analysis = analyze_release_for_target("Farwander.S08E05.1080p.WEB-DL", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "FARWANDER");
    assert!(
        !candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "COMPLETELY DIFFERENT SHOW")
    );
}

#[test]
fn resource_limits_emit_hints() {
    let huge = "A".repeat(5000);
    let analysis = analyze_release_for_target(&huge, &context(ContextFacetHint::Movie, "Huge"));
    assert!(
        analysis
            .parse_hints
            .iter()
            .any(|hint| hint == "input_truncated")
    );
}

#[test]
fn targeted_single_context_parse_is_not_title_ambiguous() {
    let mut target = context(ContextFacetHint::Movie, "Neon Cipher");
    target.known_years.push(2010);

    let analysis =
        analyze_release_against_targets("Neon.Cipher.2010.1080p.BluRay.x264-GRP", &[target]);

    assert_eq!(analysis.best_target_index, Some(0));
    assert_eq!(analysis.ambiguity_margin(), i32::MAX);
    assert!(!analysis.is_ambiguous());
}

#[test]
fn targeted_empty_context_bank_has_no_best_target() {
    let analysis = analyze_release_against_targets("Neon.Cipher.2010.1080p.BluRay.x264-GRP", &[]);

    assert_eq!(analysis.best_target_index, None);
    assert!(analysis.is_ambiguous());
}

#[test]
fn episode_context_supplies_soft_absolute_signal() {
    let mut target = context(ContextFacetHint::Anime, "Emberfall");
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(330),
        title: Some("Emberfall".to_string()),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target("[SubsPlease] Emberfall - 330 [1080p]", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert!(
        candidate
            .context_evidence
            .iter()
            .any(|code| code == "context:absolute_mapping_hit")
    );
}

#[test]
fn movie_parser_extracts_year_and_source_from_compound_tokens() {
    let mut target = context(ContextFacetHint::Movie, "protector");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("protector.2025.108010bit.webri6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::Movie);
    assert_eq!(candidate.projected.normalized_title, "PROTECTOR");
    assert_eq!(candidate.projected.year, Some(2025));
    assert_eq!(candidate.projected.source.as_deref(), Some("WEBRip"));
}

#[test]
fn daily_parser_projects_air_date_year_and_normalizes_web_source() {
    let mut target = context(
        ContextFacetHint::Series,
        "The 11th Hour With Stephanie Ruhle",
    );
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "The.11th.Hour.With.Stephanie.Ruhle.2026.04.21.720p.WEB.x264-NGP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
    assert_eq!(candidate.projected.source.as_deref(), Some("WEB-DL"));
}

#[test]
fn range_pack_parser_handles_bleach_batch_release() {
    let analysis = analyze_release_for_target(
        "Emberfall 1-366",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(candidate.projected.normalized_title, "EMBERFALL");
    assert_eq!(candidate.projected.audio, None);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::RangePack)
    );
}

#[test]
fn anime_context_prefers_season_pack_with_trailing_episode_range() {
    let analysis = analyze_release_for_target(
        "Emberfall Season 12 - (213 - 229) [Typis]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SeasonPack)
    );
}

#[test]
fn movie_context_avoids_daily_misparse_for_numeric_movie_title() {
    let mut target = context(
        ContextFacetHint::Movie,
        "Apollo 10 1 2 A Space Age Childhood Apollo 10 1 2 Uzay Caginda Cocuk Olmak",
    );
    target.known_years.push(2022);

    let analysis = analyze_release_for_target(
        "Apollo.10.1.2.A.Space.Age.Childhood-Apollo.10.1.2.Uzay.Caginda.Cocuk.Olmak.2022.Animasyon.1080p.NF.WEB-DL",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::Movie);
    assert_eq!(candidate.projected.year, Some(2022));
}

#[test]
fn series_context_supports_split_day_month_year_daily_release() {
    let mut target = context(ContextFacetHint::Series, "Kiskanmak");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Kiskanmak.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
}

#[test]
fn series_context_supports_hyphenated_day_month_year_daily_release() {
    let mut target = context(ContextFacetHint::Series, "Kiskanmak");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Kiskanmak.29.Blm.21-04-2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::DailyEpisode);
    assert_eq!(candidate.projected.year, Some(2026));
}

#[test]
fn split_season_episode_tokens_parse_as_standard_episode() {
    let analysis = analyze_release_for_target(
        "[Erai-raws] Youkoso Jitsuryoku Shijou Shugi no Kyoushitsu e S4-07 [1080p]",
        &context(
            ContextFacetHint::Anime,
            "Youkoso Jitsuryoku Shijou Shugi no Kyoushitsu e",
        ),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(4));
    assert_eq!(episode.episode_numbers, vec![7]);
}

#[test]
fn standard_episode_range_token_projects_multi_episode() {
    let analysis = analyze_release_for_target(
        "Ergo.Proxy.S01E01-11.BDRemux.1080p",
        &context(ContextFacetHint::Anime, "Ergo Proxy"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, (1..=11).collect::<Vec<_>>());
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::MultiEpisode);
}

#[test]
fn e_prefixed_episode_token_maps_to_season_one_episode() {
    let analysis = analyze_release_for_target(
        "One.Piece.E1158.1080p.WEB.H264",
        &context(ContextFacetHint::Series, "Tidebreaker"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1158]);
}

#[test]
fn season_pack_projection_preserves_single_season_number() {
    let analysis = analyze_release_for_target(
        "Crossing.Swords.S02.1080p.WEB-DL",
        &context(ContextFacetHint::Series, "Crossing Swords"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.season, Some(2));
}

#[test]
fn movie_title_keeps_numeric_tokens_that_are_part_of_the_name() {
    let mut target = context(ContextFacetHint::Movie, "Perc 30");
    target.known_years.push(2023);

    let analysis = analyze_release_for_target("Perc.30.[2023].720p.WEBRip-LAMA", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "PERC 30");
}

#[test]
fn movie_title_keeps_hyphenated_words_before_metadata_boundary() {
    let mut target = context(ContextFacetHint::Movie, "Erbsunde Veil Of Sin");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("Erbsunde-Veil.Of.Sin.[2024].720p.WEBRip-LAMA", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "ERBSUNDE VEIL OF SIN");
}

#[test]
fn bracketed_prefix_group_can_be_captured_as_release_group() {
    let analysis = analyze_release_for_target(
        "[SubsPlease] Emberfall - 330 [1080p]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("SubsPlease")
    );
}

#[test]
fn title_word_web_does_not_force_metadata_source_in_title_zone() {
    let mut target = context(ContextFacetHint::Movie, "The Web of Lies");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target("The.Web.of.Lies.2021.1080p.WEB-DL", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "THE WEB OF LIES");
}

#[test]
fn html_entity_and_ampersand_normalize_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Things Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Things.Heard.&amp;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "THINGS HEARD AND SEEN"
    );
}

#[test]
fn connector_title_variants_split_aka_and_slash_titles() {
    let mut target = context(ContextFacetHint::Movie, "Mon Cousin My Cousin");
    target.known_years.push(2020);

    let analysis =
        analyze_release_for_target("Mon Cousin / My Cousin 2020 1080p BluRay x264-GRP", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(
        projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "MON COUSIN"),
        "{:?}",
        projected.normalized_title_variants
    );
    assert!(
        projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "MY COUSIN"),
        "{:?}",
        projected.normalized_title_variants
    );

    let mut aka_target = context(ContextFacetHint::Movie, "Sydney A K A Hard Eight");
    aka_target.known_years.push(1996);
    let aka_analysis = analyze_release_for_target(
        "Sydney.A.K.A.Hard.Eight.1996.1080p.WEB-DL.H.264",
        &aka_target,
    );
    let aka_projected = &aka_analysis
        .best_candidate()
        .expect("best candidate")
        .projected;

    assert_eq!(aka_projected.normalized_title, "SYDNEY AKA HARD EIGHT");
    assert!(
        aka_projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "SYDNEY")
    );
    assert!(
        aka_projected
            .normalized_title_variants
            .iter()
            .any(|title| title == "HARD EIGHT")
    );
}

#[test]
fn double_encoded_html_entity_normalizes_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Things Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Things.Heard.&amp;amp;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "THINGS HEARD AND SEEN"
    );
}

#[test]
fn numeric_html_entity_normalizes_into_title_words() {
    let mut target = context(ContextFacetHint::Movie, "Things Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Things.Heard.&#x26;.Seen.2021.2160p.NF.WEB-DL.DD+5.1.Atmos.H.265-playWEB",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "THINGS HEARD AND SEEN"
    );
}

#[test]
fn colon_separated_re_zero_preserves_spaced_title_form() {
    let analysis = analyze_release_for_target(
        "[Judas] Re:Zero kara Hajimeru Isekai Seikatsu - S04E03 [1080p]",
        &context(
            ContextFacetHint::Anime,
            "Re Zero kara Hajimeru Isekai Seikatsu",
        ),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert!(
        candidate
            .projected
            .normalized_title
            .contains("RE ZERO KARA HAJIMERU ISEKAI SEIKATSU")
    );
}

#[test]
fn service_tagged_webrip_normalizes_to_webdl() {
    let analysis = analyze_release_for_target(
        "WWE.NXT.2026.04.21.NF.iNT.720p.WEBRip.H.264-HEEL",
        &context(ContextFacetHint::Series, "WWE NXT"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.source.as_deref(), Some("WEB-DL"));
}

#[test]
fn season_only_token_can_project_target_episode_from_required_context() {
    let mut target = context(ContextFacetHint::Series, "Invincible");
    target.known_years.push(2021);
    target.episodes = vec![ContextEpisode {
        season: Some(4),
        episode: Some(3),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target(
        "Invincible.2021.S04.1080p.AMZN.Webrip.AV1.10bit.EAC3.5.1-Goki.TAoE",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(4));
    assert_eq!(episode.episode_numbers, vec![3]);
}

#[test]
fn parenthesized_cjk_alt_title_and_prefix_group_do_not_pollute_primary_title() {
    let analysis = analyze_release_for_target(
        "[H3LL] Silver Horizon (銀界の地平線 第2期) - Beyond the Vale - S02E01 [1080p][x264 10bits][AAC][Multiple Subtitles].mkv",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "SILVER HORIZON BEYOND THE VALE"
    );
    assert!(
        candidate
            .projected
            .normalized_title_variants
            .iter()
            .any(|title| title.contains("SILVER HORIZON"))
    );
}

#[test]
fn empty_movie_title_falls_back_to_required_context_match_before_metadata_boundary() {
    let mut target = context(ContextFacetHint::Movie, "Ablam");
    target.known_years.push(2019);

    let analysis =
        analyze_release_for_target("Ablam.2019.Yerli.1080p.WEB-DL.x264.AAC-TSRG", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "ABLAM");
}

#[test]
fn fused_standard_episode_quality_suffix_is_split_correctly() {
    let analysis = analyze_release_for_target(
        "Oats.Studios.S01E101080p.NF.WEB-DL.DDP5.1.H.264-SPWEB",
        &context(ContextFacetHint::Series, "Oats Studios"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![10]);
}

#[test]
fn merged_alias_pattern_can_project_canonical_re_zero_title() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Re Zero Starting Life In Another World",
    );
    target.aliases = vec![
        ContextAlias {
            name: "Re Zero".to_string(),
        },
        ContextAlias {
            name: "ReZERO".to_string(),
        },
    ];

    let analysis = analyze_release_for_target(
        "ReZERO.Starting.Life.in.Another.World.S04E03.1080p.AMZN.WEB-DL",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.normalized_title,
        "RE ZERO STARTING LIFE IN ANOTHER WORLD"
    );
}

#[test]
fn enrichment_extracts_legacy_audio_and_hdr_fields_without_inventing_languages() {
    let mut target = context(ContextFacetHint::Movie, "Things Heard and Seen");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Things.Heard.And.Seen.2021.2160p.NF.WEB-DL.DDP5.1.Atmos.DV.HDR10+.DUAL-AUDIO.MULTISUB.10bit.x265",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.audio.as_deref(), Some("DDP"));
    assert_eq!(projected.audio_codecs, vec!["DDP".to_string()]);
    assert_eq!(projected.audio_channels.as_deref(), Some("5.1"),);
    assert!(projected.is_atmos);
    assert!(projected.is_dolby_vision);
    assert!(projected.detected_hdr);
    assert!(projected.has_hdr_fallback);
    assert!(projected.is_hdr10plus);
    assert!(projected.is_10bit);
    assert!(projected.is_dual_audio);
    assert!(projected.languages_audio.is_empty());
}

#[test]
fn enrichment_does_not_treat_plain_hdr_or_hlg_as_hdr_fallback() {
    let mut target = context(ContextFacetHint::Movie, "Movie");
    target.known_years.push(2024);

    let hdr = analyze_release_for_target("Movie.2024.1080p.WEB-DL.HDR.x264", &target);
    let hdr_candidate = hdr.best_candidate().expect("best candidate");
    let hdr_enrichment = enrich_candidate(&hdr.tokens, hdr_candidate, &hdr.raw_input);
    let hdr_projected = project_final_metadata(hdr_candidate.projected.clone(), &hdr_enrichment);

    assert!(hdr_projected.detected_hdr);
    assert!(!hdr_projected.has_hdr_fallback);

    let hlg = analyze_release_for_target("Movie.2024.1080p.WEB-DL.HLG.x264", &target);
    let hlg_candidate = hlg.best_candidate().expect("best candidate");
    let hlg_enrichment = enrich_candidate(&hlg.tokens, hlg_candidate, &hlg.raw_input);
    let hlg_projected = project_final_metadata(hlg_candidate.projected.clone(), &hlg_enrichment);

    assert!(hlg_projected.is_hlg);
    assert!(!hlg_projected.has_hdr_fallback);
}

#[test]
fn enrichment_does_not_treat_atmosphere_title_word_as_atmos() {
    let mut target = context(ContextFacetHint::Movie, "Atmosphere");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target("Atmosphere.2024.1080p.WEB-DL.HDR.x264", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.normalized_title, "ATMOSPHERE");
    assert!(!projected.is_atmos);
}

#[test]
fn enrichment_extracts_split_dts_x_audio() {
    let mut target = context(ContextFacetHint::Movie, "Movie");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target("Movie.2024.2160p.BluRay.DTS-X.7.1.H.265", &target);
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.audio.as_deref(), Some("DTSX"));
    assert_eq!(projected.audio_channels.as_deref(), Some("7.1"));
}

#[test]
fn enrichment_extracts_split_eac3_without_matching_title_substrings() {
    let analysis = analyze_release_for_target(
        "[YukiSubs] Sora no Vale - 29 (S02E01) (WEB 1080p HEVC EAC-3).mkv",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.audio.as_deref(), Some("EAC3"));

    let bleach = analyze_release_for_target(
        "Emberfall 1-366",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let bleach_projected = &bleach.best_candidate().expect("best candidate").projected;

    assert_eq!(bleach_projected.normalized_title, "EMBERFALL");
    assert_eq!(bleach_projected.audio, None);
}

#[test]
fn enrichment_canonicalizes_french_language_codes_to_fra() {
    let mut target = context(ContextFacetHint::Movie, "Esther Kahn");
    target.known_years.push(2000);

    let analysis = analyze_release_for_target(
        "Esther.Kahn.2000.REMASTERED.VOSTFR.1080p.FRA.BluRay.REMUX.AVC.DTS-HD.MA.5.1-MAD",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["fra".to_string()]);
    assert_eq!(projected.languages_subtitles, vec!["fra".to_string()]);
}

#[test]
fn enrichment_extracts_affixed_language_before_video_anchor() {
    let analysis = analyze_release_for_target(
        "Teenage.Mutant.Ninja.Turtles.S05E07.HebDub.XviD",
        &context(ContextFacetHint::Series, "Teenage Mutant Ninja Turtles"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["heb".to_string()]);
    assert_eq!(projected.video_codec.as_ref(), Some(&VideoCodec::Xvid));
}

#[test]
fn enrichment_extracts_short_language_code_inside_metadata_zone() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-GRP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["tur".to_string()]);
    assert_eq!(projected.streaming_service.as_deref(), Some("Disney+"));
    assert_eq!(projected.audio.as_deref(), Some("AAC"));
    assert_eq!(projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn enrichment_extracts_named_language_before_release_group_suffix() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.BluRay.x265.DDP.5.1.English-GRP",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["eng".to_string()]);
    assert_eq!(projected.release_group.as_deref(), Some("GRP"));
}

#[test]
fn enrichment_extracts_english_dub_gap_group_after_episode_identity() {
    let analysis = analyze_release_for_target(
        "[Yameii] Go For It, Nakamura-kun!! - S01E05 [English Dub] [CR WEB-DL 1080p H264 AAC] [994F6EBD] (Ganbare! Nakamura-kun!!)",
        &context(ContextFacetHint::Anime, "Go For It Nakamura-kun"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert_eq!(projected.languages_audio, vec!["eng".to_string()]);
    assert!(projected.is_dubs_only);
}

#[test]
fn enrichment_extracts_release_flags_before_quality_anchor() {
    let analysis = analyze_release_for_target(
        "O11CE.New.Generation.S01E17.DV.2160p.WEB.h265-EDITH",
        &context(ContextFacetHint::Series, "O11CE New Generation"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert!(projected.is_dolby_vision);
    assert!(projected.detected_hdr);
}

#[test]
fn enrichment_extracts_fused_and_plural_10bit_markers() {
    let mut target = context(ContextFacetHint::Movie, "Protector");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("protector.2025.108010bit.webri6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let enrichment = enrich_candidate(&analysis.tokens, candidate, &analysis.raw_input);
    let projected = project_final_metadata(candidate.projected.clone(), &enrichment);

    assert!(projected.is_10bit);
}

#[test]
fn remux_is_projected_as_structural_flag_not_edition() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.2160p.BluRay.REMUX.HEVC.TrueHD.7.1",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert!(candidate.projected.is_remux);
    assert_ne!(candidate.projected.edition.as_deref(), Some("REMUX"));
}

#[test]
fn remux_survives_when_proper_appears_first() {
    let mut target = context(ContextFacetHint::Movie, "13 Sins");
    target.known_years.push(2014);

    let analysis = analyze_release_for_target(
        "13.Sins.2014.PROPER.BluRay.1080p.DTS-HD.MA.5.1.AVC.HYBRID.REMUX-FraMeSToR",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(projected.is_remux);
    assert!(projected.is_proper_upload);
}

#[test]
fn leading_bdmv_group_is_source_not_release_group() {
    let analysis = analyze_release_for_target(
        "[BDMV] Emberfall [BD-BOX] [SET 1- 9]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert!(projected.is_bd_disk);
    assert_ne!(projected.release_group.as_deref(), Some("BDMV"));
}

#[test]
fn split_dvd_rip_source_projects_as_dvd() {
    let analysis = analyze_release_for_target(
        "[EDG] BLEACH EP 1-30 [DVD RIP X264 Hi10]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.source.as_deref(), Some("DVD"));
}

#[test]
fn range_pack_projects_absolute_range_without_doubling_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[HorribleSubs] Tokyo Ghoul [01-12] [720p] [Batch]",
        &context(ContextFacetHint::Anime, "Tokyo Ghoul"),
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert!(episode.episode_numbers.is_empty());
    assert_eq!(
        episode.absolute_episode_numbers,
        (1..=12).collect::<Vec<_>>()
    );
}

#[test]
fn numbered_ova_projects_special_absolute_episode_number() {
    let analysis = analyze_release_for_target(
        "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
        &context(ContextFacetHint::Anime, "Another Anime Show"),
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;
    let episode = projected.episode.as_ref().expect("episode");

    assert_eq!(projected.parse_family, ParseFamily::Special);
    assert_eq!(episode.special_kind, Some(crate::ParsedSpecialKind::Ova));
    assert_eq!(episode.special_absolute_episode_numbers, vec![1]);
}

#[test]
fn season_pack_range_sets_multi_season_contract_flag() {
    let analysis = analyze_release_for_target(
        "The.Great.S01-S03.NORDiC.1080p.MAX.WEB-DL.H.265-NORViNE",
        &context(ContextFacetHint::Series, "The Great"),
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert_eq!(episode.season, None);
    assert!(episode.full_season);
    assert!(episode.is_multi_season);
}

#[test]
fn parenthetical_standard_identity_beats_prefixed_absolute_number() {
    let mut target = context(
        ContextFacetHint::Anime,
        "Silver Horizon Beyond Journey's End",
    );
    target.aliases.push(ContextAlias {
        name: "Sora no Vale".to_string(),
    });

    let analysis = analyze_release_for_target(
        "[YukiSubs] Sora no Vale - 29 (S02E01) (WEB 1080p HEVC EAC-3).mkv",
        &target,
    );
    let episode = analysis
        .best_candidate()
        .expect("best candidate")
        .projected
        .episode
        .as_ref()
        .expect("episode");

    assert_eq!(episode.season, Some(2));
    assert_eq!(episode.episode_numbers, vec![1]);
}

#[test]
fn series_facet_with_episode_context_can_recover_absolute_episode() {
    let mut target = context(ContextFacetHint::Series, "Kiskanmak");
    target.known_years.push(2026);
    target.episodes.push(ContextEpisode {
        season: None,
        episode: None,
        absolute_number: Some(29),
        air_date: None,
        title: None,
        title_aliases: Vec::new(),
    });

    let analysis = analyze_release_for_target(
        "Kiskanmak.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.absolute_episode, Some(29));
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(candidate.projected.source.as_deref(), Some("WEB-DL"));
}

#[test]
fn movie_facet_with_episode_context_can_recover_absolute_episode() {
    let mut target = context(ContextFacetHint::Movie, "Kiskanmak");
    target.known_years.push(2026);
    target.episodes.push(ContextEpisode {
        season: None,
        episode: None,
        absolute_number: Some(29),
        air_date: None,
        title: None,
        title_aliases: Vec::new(),
    });

    let analysis = analyze_release_for_target(
        "Kiskanmak.29.Blm.21.04.2026.1080p.DSNP.WEB-DL.TR.AAC2.0.H.264-TURG",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.absolute_episode, Some(29));
}

#[test]
fn service_tokens_project_to_canonical_service_names() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playweb", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.streaming_service.as_deref(),
        Some("Amazon")
    );
}

#[test]
fn unicode_case_and_out_of_corpus_metadata_parse_without_fixture_bias() {
    let mut target = context(ContextFacetHint::Movie, "Éclair Monstra");
    target.known_years.push(2024);

    let analysis =
        analyze_release_for_target("éclair.monstra.2024.576p.WEB-DL.VVC.OPUS.2.0-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "ÉCLAIR MONSTRA");
    assert_eq!(candidate.projected.quality.as_deref(), Some("576p"));
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::Vvc)
    );
    assert_eq!(candidate.projected.audio.as_deref(), Some("OPUS"));
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn fused_standard_episode_suffix_accepts_unseen_resolution() {
    let analysis = analyze_release_for_target(
        "Out.Show.S01E01576p.WEB-DL.VVC-Group",
        &context(ContextFacetHint::Series, "Out Show"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::StandardEpisode);
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(candidate.projected.quality.as_deref(), Some("576p"));
}

#[test]
fn generic_external_ids_parse_beyond_imdb() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    let analysis = analyze_release_for_target(
        "Movie.Title.2024.1080p.WEB-DL.TMDB.12345.TVDB.67890.IMDB.tt7654321",
        &target,
    );
    let projected = &analysis.best_candidate().expect("best candidate").projected;

    assert_eq!(projected.imdb_id.as_deref(), Some("tt7654321"));
    assert_eq!(projected.tmdb_id.as_deref(), Some("12345"));
    assert_eq!(projected.tvdb_id.as_deref(), Some("67890"));
    assert!(
        projected
            .external_ids
            .iter()
            .any(|id| { id.source == "tmdb" && id.value == "12345" })
    );
    assert!(
        projected
            .external_ids
            .iter()
            .any(|id| { id.source == "tvdb" && id.value == "67890" })
    );
}

#[test]
fn numeric_anime_title_uses_context_before_absolute_episode() {
    let mut target = context(ContextFacetHint::Anime, "86");
    target.episodes.push(ContextEpisode {
        absolute_number: Some(11),
        ..Default::default()
    });

    let analysis = analyze_release_for_target("86 - 11 [1080p]", &target);
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.projected.normalized_title, "86");
    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert_eq!(episode.absolute_episode_numbers, vec![11]);
}

#[test]
fn short_service_alias_can_remain_a_title_word() {
    let mut target = context(ContextFacetHint::Movie, "Max Headroom");
    target.known_years.push(1985);

    let analysis = analyze_release_for_target("Max.Headroom.1985.480p.DVD.MPEG2.MP3-GRP", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.normalized_title, "MAX HEADROOM");
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::Mpeg2)
    );
    assert_eq!(candidate.projected.audio.as_deref(), Some("MP3"));
}

#[test]
fn release_group_preserves_dotted_and_hyphenated_suffixes() {
    let mut target = context(ContextFacetHint::Movie, "The Moment");
    target.known_years.push(2026);

    let analysis =
        analyze_release_for_target("the.moment.2026.1080p.bluray.x264.aac5.1-yts.bz", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("yts.bz"));
}

#[test]
fn leading_fansub_group_beats_trailing_batch_group() {
    let analysis = analyze_release_for_target(
        "[HorribleSubs] Tokyo Ghoul [01-12] [720p] [Batch]",
        &context(ContextFacetHint::Anime, "Tokyo Ghoul"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("HorribleSubs")
    );
}

#[test]
fn suffix_group_after_episode_subtitle_beats_earlier_hyphen_text() {
    let analysis = analyze_release_for_target(
        "Homes.Under.the.Hammer.S28E75-One.Man.and.His.Dog.WEB-DL.H.264-W45Ps",
        &context(ContextFacetHint::Series, "Homes Under the Hammer"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("W45Ps"));
}

#[test]
fn suffix_group_ignores_parenthetical_alt_title_and_language_markers() {
    let analysis = analyze_release_for_target(
        "Silver Horizon.Beyond.Journeys.End.S02E01.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG.(Sousou.no.Silver Horizon.Multi-Subs)",
        &context(ContextFacetHint::Anime, "Silver Horizon Beyond the Vale"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("VARYG"));
}

#[test]
fn suffix_group_does_not_capture_hyphenated_words_in_trailing_title() {
    let analysis = analyze_release_for_target(
        "Emberfall.S17E11.720p.DSNP.WEB-DL.AAC2.0.H.264-PiroRips.mkv (Emberfall - Iron Eclipse)",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("PiroRips")
    );
}

#[test]
fn release_group_skips_embedded_suffix_inside_large_metadata_bracket() {
    let mut target = context(ContextFacetHint::Movie, "The Peasants");
    target.known_years.push(2023);

    let analysis = analyze_release_for_target(
        "The.Peasants.[2023].[1080p.BluRay.x265.SDR.DDP.5.1.Dual-DarQ.HONE]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group, None);
}

#[test]
fn release_group_preserves_short_hyphenated_prefix() {
    let mut target = context(ContextFacetHint::Movie, "Lechindi Mahila Lokam");
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Lechindi.Mahila.Lokam.2026.Tamil.2160p.SNXT.WEB-DL.DDP5.1.H.265-PMi-XDMovies",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("PMi-XDMovies")
    );
}

#[test]
fn release_group_uses_terminal_component_for_two_part_p2p_suffix() {
    let mut target = context(ContextFacetHint::Series, "Invincible");
    target.known_years.push(2021);

    let analysis = analyze_release_for_target(
        "Invincible.2021.S04E07.DONT.DO.ANYTHING.RASH.1080p.AMZN.Webrip.AV1.10bit.EAC3.5.1-Goki-TAoE",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("TAoE"));
}

#[test]
fn bracketed_short_hyphenated_release_group_preserves_both_parts() {
    let analysis = analyze_release_for_target(
        "Emberfall - 224 - 3 vs 1 Battle! Rangiku's Crisis [C-W].avi",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.release_group.as_deref(), Some("C-W"));
}

#[test]
fn terminal_language_adjacent_token_can_be_release_group() {
    let mut target = context(
        ContextFacetHint::Movie,
        "Senden Geriye Kalan Reminders of Him",
    );
    target.known_years.push(2026);

    let analysis = analyze_release_for_target(
        "Senden.Geriye.Kalan.Reminders.of.Him.2026.WEBDLRip.m1080p.X265.10bit.AAC.5.1.Turkce.TurkSeeD",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("TurkSeeD")
    );
}

#[test]
fn compound_source_suffix_is_not_captured_as_release_group() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playWEB", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("playWEB")
    );
}

#[test]
fn enrichment_fills_split_video_codec_and_audio_channels() {
    let mut target = context(ContextFacetHint::Movie, "Tarzan of the Apes");
    target.known_years.push(1998);

    let analysis = analyze_release_for_target(
        "Tarzan.of.the.Apes.1998.DVDRip.HebDub.AAC2.0.H.264-T00LBAR",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::H264)
    );
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn parser_canonicalizes_h264_family_video_codec_tokens() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    for raw in [
        "Movie.Title.2024.1080p.BluRay.h264-GRP",
        "Movie.Title.2024.1080p.BluRay.x264-GRP",
        "Movie.Title.2024.1080p.BluRay.AVC-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let candidate = analysis.best_candidate().expect("best candidate");

        assert_eq!(
            candidate.projected.video_codec.as_ref(),
            Some(&VideoCodec::H264),
            "{raw}"
        );
    }
}

#[test]
fn parser_canonicalizes_h265_family_video_codec_tokens() {
    let mut target = context(ContextFacetHint::Movie, "Movie Title");
    target.known_years.push(2024);

    for raw in [
        "Movie.Title.2024.2160p.WEB-DL.hevc-GRP",
        "Movie.Title.2024.2160p.WEB-DL.h265-GRP",
        "Movie.Title.2024.2160p.WEB-DL.x265-GRP",
    ] {
        let analysis = analyze_release_for_target(raw, &target);
        let candidate = analysis.best_candidate().expect("best candidate");

        assert_eq!(
            candidate.projected.video_codec.as_ref(),
            Some(&VideoCodec::H265),
            "{raw}"
        );
    }
}

#[test]
fn enrichment_extracts_split_dts_ma_audio() {
    let mut target = context(ContextFacetHint::Movie, "The Wizard of Oz");
    target.known_years.push(1939);

    let analysis = analyze_release_for_target(
        "The.Wizard.of.Oz.1939.2160p.MA.WEB-DL.DTS-HD.MA.5.1.H.265-FLUX",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.audio.as_deref(), Some("DTSMA"));
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("5.1"));
}

#[test]
fn enrichment_extracts_fused_dts_hd_audio() {
    let mut target = context(ContextFacetHint::Movie, "Graveyard Shift");
    target.known_years.push(1990);

    let analysis = analyze_release_for_target(
        "Graveyard.Shift.1990.REMASTERED.1080p.BluRay.REMUX.Dts-HDMa5.1.AVC-d3g",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.audio.as_deref(), Some("DTSHD"));
}

#[test]
fn enrichment_extracts_bare_dd_with_split_channels() {
    let mut target = context(ContextFacetHint::Movie, "Askari");
    target.known_years.push(2001);

    let analysis =
        analyze_release_for_target("askari.2001.amzn.web-dl.dd.2.0.h.264-playweb", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.audio.as_deref(), Some("DD"));
    assert_eq!(candidate.projected.audio_channels.as_deref(), Some("2.0"));
}

#[test]
fn standalone_channel_count_without_audio_codec_is_not_projected() {
    let mut target = context(ContextFacetHint::Movie, "Tow");
    target.known_years.push(2025);

    let analysis =
        analyze_release_for_target("tow.2025.1080p.10biwebrip.6ch.x265.hevc-psa", &target);
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.projected.audio_channels, None);
}

#[test]
fn labeled_episode_range_stays_multi_episode() {
    let analysis = analyze_release_for_target(
        "[EDG] BLEACH EP 1-30 [DVD R2 X264 Hi10]",
        &context(ContextFacetHint::Anime, "Emberfall"),
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::RangePack)
    );
}

#[test]
fn labeled_single_episode_does_not_become_range_pack() {
    let mut target = context(ContextFacetHint::Anime, "Clockwork Cat");
    target.known_years.push(2005);
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(911),
        ..Default::default()
    }];

    let analysis = analyze_release_for_target(
        "[Ommex] Clockwork Cat (2005) Episode 911 [ENG SUB][1080p x265 AAC]",
        &target,
    );
    let candidate = analysis.best_candidate().expect("best candidate");

    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.release_type),
        Some(ParsedEpisodeReleaseType::SingleEpisode)
    );
    assert_eq!(
        candidate
            .projected
            .episode
            .as_ref()
            .map(|episode| episode.absolute_episode_numbers.clone()),
        Some(vec![911])
    );
}

fn midnight_alloy_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Midnight Alloy Dark Signal");
    target.aliases = vec![
        ContextAlias {
            name: "Midnight Alloy Dark".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Dark Signal".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy Kage Requiem".to_string(),
        },
        ContextAlias {
            name: "Midnight Alloy".to_string(),
        },
    ];
    target.known_years.push(2022);
    target
}

#[test]
fn midnight_alloy_part_one_and_two_release_projects_full_season_pack() {
    let analysis = analyze_release_for_target(
        "[Studio Nova] MIDNIGHT ALLOY Dark Signal (Season 1) [Part 1 + Part 2] [Dual Audio] [1080p][HEVC 10bit x265][AAC][Multi Sub] [Batch]",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::SeasonPack);
    assert_eq!(episode.season, Some(1));
    assert!(episode.full_season);
    assert!(!episode.is_partial_season);
}

#[test]
fn midnight_alloy_standalone_part_two_release_projects_partial_season_pack() {
    let analysis = analyze_release_for_target(
        "[EMBER] MIDNIGHT ALLOY‼ Dark Signal (2022) (Season 1 | Part 02) [1080p] [Dual Audio HEVC 10 bits WEBRip AAC] (Midnight Alloy Kage Requiem) (Batch)",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::SeasonPack);
    assert_eq!(episode.release_type, ParsedEpisodeReleaseType::SeasonPack);
    assert_eq!(episode.season, Some(1));
    assert!(episode.is_partial_season);
    assert_eq!(episode.season_part, Some(2));
}

#[test]
fn midnight_alloy_tilde_absolute_range_projects_absolute_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[Erai-raws] Midnight Alloy Kage no Requiem (2022) - 01 ~ 13 [1080p][Multiple Subtitle]",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(episode.season, None);
    assert!(episode.episode_numbers.is_empty());
    assert_eq!(
        episode.absolute_episode_numbers,
        (1..=13).collect::<Vec<_>>()
    );
}

#[test]
fn midnight_alloy_labeled_absolute_range_projects_absolute_episode_numbers() {
    let analysis = analyze_release_for_target(
        "MIDNIGHT ALLOY -Dark Signal- Episodes 14-24 | Midnight Alloy Kage no Requiem [Dual][1080p] - E.N.D (English Dub | Japanese Dub)",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    assert_eq!(episode.season, None);
    assert_eq!(
        episode.absolute_episode_numbers,
        (14..=24).collect::<Vec<_>>()
    );
}

#[test]
fn midnight_alloy_season_scoped_labeled_range_projects_episode_numbers() {
    let analysis = analyze_release_for_target(
        "[Anime Chap] MIDNIGHT ALLOY‼ Dark Signal 2022 - Season 1 (ONA) [WEB 1080p] {OP & ED Lyrics} Improved Subs (Episode 1 - 13) {Batch}",
        &midnight_alloy_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    assert_eq!(candidate.family, ParseFamily::EpisodeRangePack);
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, (1..=13).collect::<Vec<_>>());
    assert!(episode.absolute_episode_numbers.is_empty());
}

fn starfall_iron_eclipse_context() -> ReleaseParseContext {
    let mut target = context(ContextFacetHint::Anime, "Starfall Iron Eclipse");
    target.aliases = vec![
        ContextAlias {
            name: "Starfall".to_string(),
        },
        ContextAlias {
            name: "Starfall - Iron Eclipse".to_string(),
        },
        ContextAlias {
            name: "Starfall: Iron Eclipse".to_string(),
        },
    ];
    target.known_years.push(2022);
    target.episodes = vec![ContextEpisode {
        absolute_number: Some(14),
        title: Some("The Last 9 Signals".to_string()),
        ..Default::default()
    }];
    target
}

#[test]
fn anime_absolute_release_keeps_release_group_and_metadata_boundaries() {
    let analysis = analyze_release_for_target(
        "[Studio Nova] Starfall - Iron Eclipse - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC] [Dual Audio][ENG Subs]",
        &starfall_iron_eclipse_context(),
    );
    let candidate = analysis.best_candidate().expect("best candidate");
    let episode = candidate.projected.episode.as_ref().expect("episode");

    assert_eq!(candidate.family, ParseFamily::AnimeAbsolute);
    assert_eq!(episode.absolute_episode, Some(14));
    assert_eq!(episode.absolute_episode_numbers, vec![14]);
    assert_eq!(
        candidate.projected.release_group.as_deref(),
        Some("Studio Nova")
    );
    assert_eq!(candidate.projected.source.as_deref(), Some("BluRay"));
    assert_eq!(candidate.projected.quality.as_deref(), Some("1080p"));
    assert_eq!(
        candidate.projected.video_codec.as_ref(),
        Some(&VideoCodec::H265)
    );
    assert!(
        candidate
            .projected
            .audio_codecs
            .iter()
            .any(|codec| codec == "AAC")
    );
}

#[test]
fn target_bank_prefers_specific_title_when_alias_and_episode_title_align() {
    let mut classic_starfall = context(ContextFacetHint::Anime, "Starfall");
    classic_starfall.aliases = vec![ContextAlias {
        name: "Starfall".to_string(),
    }];

    let analysis = analyze_release_against_targets(
        "[Studio Nova] Starfall - 014 - The Last 9 Signals [BD][1080p][HEVC 10bit x265][AAC]",
        &[classic_starfall, starfall_iron_eclipse_context()],
    );
    let candidate = analysis
        .best_target()
        .and_then(|target| target.analysis.best_candidate())
        .expect("best candidate");

    assert_eq!(analysis.best_target_index, Some(1));
    assert!(analysis.ambiguity_margin() >= 0);
    assert_eq!(
        candidate.projected.normalized_title,
        "STARFALL IRON ECLIPSE"
    );
}
