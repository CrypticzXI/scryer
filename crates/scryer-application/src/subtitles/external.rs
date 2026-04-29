use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::Utc;
use scryer_domain::{ExternalSubtitleSourceKind, SUBTITLE_EXTENSIONS, SubtitleDownload};
use tokio::fs;

use crate::{AppError, AppResult, AppUseCase};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredExternalSubtitle {
    file_path: String,
    language: String,
    forced: bool,
    hearing_impaired: bool,
}

pub(crate) async fn reconcile_external_subtitles_for_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    episode_id: Option<&str>,
    video_path: &Path,
) -> AppResult<bool> {
    let existing = app
        .services
        .workflow
        .subtitle_downloads
        .list_for_media_file(media_file_id)
        .await?;

    let discovered = discover_external_subtitles_for_video(video_path).await?;
    let downloaded_paths = existing
        .iter()
        .filter(|record| record.source_kind == ExternalSubtitleSourceKind::Downloaded)
        .map(|record| record.file_path.clone())
        .collect::<HashSet<_>>();

    let mut desired_discovered = BTreeMap::new();
    for subtitle in discovered {
        if downloaded_paths.contains(&subtitle.file_path) {
            continue;
        }
        desired_discovered.insert(subtitle.file_path.clone(), subtitle);
    }

    let mut changed = false;
    let mut existing_discovered_by_path = BTreeMap::new();
    for record in &existing {
        let exists = fs::try_exists(Path::new(&record.file_path))
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if !exists {
            app.services
                .workflow
                .subtitle_downloads
                .delete(&record.id)
                .await?;
            changed = true;
            continue;
        }

        if record.source_kind == ExternalSubtitleSourceKind::Discovered {
            if desired_discovered.contains_key(&record.file_path) {
                existing_discovered_by_path.insert(record.file_path.clone(), record.clone());
            } else {
                app.services
                    .workflow
                    .subtitle_downloads
                    .delete(&record.id)
                    .await?;
                changed = true;
            }
        }
    }

    for discovered in desired_discovered.into_values() {
        if let Some(existing_record) = existing_discovered_by_path.get(&discovered.file_path) {
            let updated = build_discovered_external_subtitle_record(
                existing_record.id.clone(),
                media_file_id,
                title_id,
                episode_id,
                &discovered,
                &existing_record.downloaded_at,
            );
            if subtitle_records_differ(existing_record, &updated) {
                app.services
                    .workflow
                    .subtitle_downloads
                    .insert(&updated)
                    .await?;
                changed = true;
            }
        } else {
            let inserted = build_discovered_external_subtitle_record(
                scryer_domain::Id::new().0,
                media_file_id,
                title_id,
                episode_id,
                &discovered,
                &Utc::now().to_rfc3339(),
            );
            app.services
                .workflow
                .subtitle_downloads
                .insert(&inserted)
                .await?;
            changed = true;
        }
    }

    Ok(changed)
}

fn build_discovered_external_subtitle_record(
    id: String,
    media_file_id: &str,
    title_id: &str,
    episode_id: Option<&str>,
    discovered: &DiscoveredExternalSubtitle,
    downloaded_at: &str,
) -> SubtitleDownload {
    SubtitleDownload {
        id,
        media_file_id: media_file_id.to_string(),
        title_id: title_id.to_string(),
        episode_id: episode_id.map(str::to_string),
        source_kind: ExternalSubtitleSourceKind::Discovered,
        language: discovered.language.clone(),
        provider: None,
        provider_file_id: None,
        file_path: discovered.file_path.clone(),
        score: None,
        hearing_impaired: discovered.hearing_impaired,
        forced: discovered.forced,
        ai_translated: false,
        machine_translated: false,
        uploader: None,
        release_info: None,
        synced: false,
        downloaded_at: downloaded_at.to_string(),
    }
}

fn subtitle_records_differ(left: &SubtitleDownload, right: &SubtitleDownload) -> bool {
    left.media_file_id != right.media_file_id
        || left.title_id != right.title_id
        || left.episode_id != right.episode_id
        || left.source_kind != right.source_kind
        || left.language != right.language
        || left.provider != right.provider
        || left.provider_file_id != right.provider_file_id
        || left.file_path != right.file_path
        || left.score != right.score
        || left.hearing_impaired != right.hearing_impaired
        || left.forced != right.forced
        || left.ai_translated != right.ai_translated
        || left.machine_translated != right.machine_translated
        || left.uploader != right.uploader
        || left.release_info != right.release_info
        || left.synced != right.synced
}

async fn discover_external_subtitles_for_video(
    video_path: &Path,
) -> AppResult<Vec<DiscoveredExternalSubtitle>> {
    let Some(parent) = video_path.parent() else {
        return Ok(Vec::new());
    };
    let Some(video_stem) = video_path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(Vec::new());
    };

    let mut entries = fs::read_dir(parent)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let mut discovered = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    {
        let path = entry.path();
        if !path_has_subtitle_extension(&path) {
            continue;
        }
        if let Some(subtitle) = parse_discovered_external_subtitle(video_stem, &path) {
            discovered.push(subtitle);
        }
    }

    discovered.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(discovered)
}

fn path_has_subtitle_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| SUBTITLE_EXTENSIONS.contains(&ext.as_str()))
}

fn parse_discovered_external_subtitle(
    video_stem: &str,
    subtitle_path: &Path,
) -> Option<DiscoveredExternalSubtitle> {
    let subtitle_stem = subtitle_path.file_stem()?.to_str()?;
    let suffix = if subtitle_stem == video_stem {
        ""
    } else {
        subtitle_stem.strip_prefix(&format!("{video_stem}."))?
    };

    let (language, forced, hearing_impaired) = parse_sidecar_suffix_tokens(suffix);
    Some(DiscoveredExternalSubtitle {
        file_path: subtitle_path.to_string_lossy().to_string(),
        language,
        forced,
        hearing_impaired,
    })
}

fn parse_sidecar_suffix_tokens(suffix: &str) -> (String, bool, bool) {
    let mut language = None;
    let mut forced = false;
    let mut hearing_impaired = false;

    for token in suffix.split('.').filter(|token| !token.trim().is_empty()) {
        let normalized = token.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "forced" | "foreign") {
            forced = true;
            continue;
        }
        if matches!(
            normalized.as_str(),
            "hi" | "cc" | "sdh" | "hoh" | "hearingimpaired" | "hearing-impaired"
        ) {
            hearing_impaired = true;
            continue;
        }
        if language.is_none() {
            language = crate::media::language::normalize_detected_subtitle_language_code(token)
                .or_else(|| normalized.eq("und").then(|| "und".to_string()));
        }
    }

    (
        language.unwrap_or_else(|| "und".to_string()),
        forced,
        hearing_impaired,
    )
}

#[cfg(test)]
mod tests {
    use super::{discover_external_subtitles_for_video, parse_sidecar_suffix_tokens};
    use std::fs;

    #[test]
    fn parses_language_and_common_flags_from_sidecar_suffix() {
        assert_eq!(
            parse_sidecar_suffix_tokens("eng.forced.hi"),
            ("eng".to_string(), true, true)
        );
        assert_eq!(
            parse_sidecar_suffix_tokens("jpn.sdh"),
            ("jpn".to_string(), false, true)
        );
        assert_eq!(
            parse_sidecar_suffix_tokens("commentary"),
            ("und".to_string(), false, false)
        );
    }

    #[tokio::test]
    async fn discovers_same_stem_sidecars_only() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Show.S01E01.mkv");
        let english = tempdir.path().join("Example.Show.S01E01.eng.srt");
        let forced = tempdir.path().join("Example.Show.S01E01.jpn.forced.ass");
        let unrelated = tempdir.path().join("Other.Show.eng.srt");

        fs::write(&video, b"video").expect("video");
        fs::write(&english, b"subtitle").expect("subtitle");
        fs::write(&forced, b"subtitle").expect("subtitle");
        fs::write(&unrelated, b"subtitle").expect("subtitle");

        let discovered = discover_external_subtitles_for_video(&video)
            .await
            .expect("discover subtitles");

        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].language, "eng");
        assert_eq!(discovered[0].file_path, english.to_string_lossy());
        assert_eq!(discovered[1].language, "jpn");
        assert!(discovered[1].forced);
    }
}
