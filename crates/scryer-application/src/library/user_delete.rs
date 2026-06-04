use super::*;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use scryer_domain::{
    MediaFacet, RootFolderEntry, Title, is_image_file, is_subtitle_file, is_video_file,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::fs;

const DELETE_PREVIEW_SAMPLE_PATH_LIMIT: usize = 5;
const LARGE_DELETE_MEDIA_THRESHOLD: usize = 50;
const DELETE_TYPED_CONFIRMATION_VALUE: &str = "DELETE";
const DELETE_TYPED_CONFIRMATION_PROMPT: &str = "Type DELETE to confirm this large delete.";
const BULK_DELETE_PREVIEW_CONCURRENCY: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletePreview {
    pub fingerprint: String,
    pub total_file_count: i32,
    pub media_count: i32,
    pub subtitle_count: i32,
    pub image_count: i32,
    pub other_count: i32,
    pub directory_count: i32,
    pub requires_typed_confirmation: bool,
    pub typed_confirmation_prompt: Option<String>,
    pub target_label: String,
    pub sample_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlePreviewResult {
    pub title_id: String,
    pub preview: Option<DeletePreview>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlesPreview {
    pub preview: DeletePreview,
    pub items: Vec<DeleteTitlePreviewResult>,
}

#[derive(Clone, Debug)]
enum UserDeleteContext {
    Title(TitleDeleteContext),
    MediaFile(MediaFileDeleteContext),
    Subtitle(SubtitleDeleteContext),
}

#[derive(Clone, Debug)]
struct TitleDeleteContext {
    title_id: String,
    title_name: String,
    facet: MediaFacet,
    folder_path: Option<String>,
    root_folders: Vec<RootFolderEntry>,
    other_titles: Vec<TrackedTitleFolder>,
}

#[derive(Clone, Debug)]
struct TrackedTitleFolder {
    title_id: String,
    title_name: String,
    folder_path: String,
}

#[derive(Clone, Debug)]
struct MediaFileDeleteContext {
    title_id: String,
    file_id: String,
    file_path: String,
    subtitle_paths: Vec<String>,
}

#[derive(Clone, Debug)]
struct SubtitleDeleteContext {
    title_id: String,
    subtitle_download_id: String,
    file_path: String,
}

#[derive(Clone, Debug)]
enum ExternalSubtitleRemovalMode {
    DeleteOnly,
    DeleteAndBlocklist { reason: Option<String> },
}

#[derive(Clone, Debug)]
struct UserDeleteManifest {
    fingerprint: String,
    target_label: String,
    entries: Vec<DeleteManifestEntry>,
    media_count: usize,
    subtitle_count: usize,
    image_count: usize,
    other_count: usize,
    directory_count: usize,
}

#[derive(Clone, Debug)]
struct DeleteManifestEntry {
    path: PathBuf,
    delete_kind: DeletePathKind,
    preview_category: Option<DeletePreviewCategory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletePathKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletePreviewCategory {
    Media,
    Subtitle,
    Image,
    Other,
}

impl DeletePathKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
        }
    }
}

impl DeletePreviewCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Subtitle => "subtitle",
            Self::Image => "image",
            Self::Other => "other",
        }
    }
}

impl UserDeleteManifest {
    fn to_preview(&self) -> DeletePreview {
        DeletePreview {
            fingerprint: self.fingerprint.clone(),
            total_file_count: (self.media_count
                + self.subtitle_count
                + self.image_count
                + self.other_count) as i32,
            media_count: self.media_count as i32,
            subtitle_count: self.subtitle_count as i32,
            image_count: self.image_count as i32,
            other_count: self.other_count as i32,
            directory_count: self.directory_count as i32,
            requires_typed_confirmation: self.media_count > LARGE_DELETE_MEDIA_THRESHOLD,
            typed_confirmation_prompt: (self.media_count > LARGE_DELETE_MEDIA_THRESHOLD)
                .then(|| DELETE_TYPED_CONFIRMATION_PROMPT.to_string()),
            target_label: self.target_label.clone(),
            sample_paths: self
                .entries
                .iter()
                .take(DELETE_PREVIEW_SAMPLE_PATH_LIMIT)
                .map(|entry| entry.path.display().to_string())
                .collect(),
        }
    }
}

impl AppUseCase {
    pub async fn preview_delete_title_files(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<DeletePreview> {
        let context = self.resolve_title_delete_context(title_id).await?;
        self.require_delete_context_permission(actor, &context)
            .await?;
        let manifest = self.build_delete_manifest(context).await?;
        Ok(manifest.to_preview())
    }

    pub async fn preview_delete_titles_files(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<DeleteTitlesPreview> {
        let mut items = Vec::with_capacity(title_ids.len());
        let title_infos = self
            .services
            .catalog
            .titles
            .list_delete_preview_info()
            .await?;
        let titles_by_id = title_infos
            .into_iter()
            .map(|title| (title.title_id.clone(), title))
            .collect::<HashMap<_, _>>();
        let requested_facets = title_ids
            .iter()
            .filter_map(|title_id| titles_by_id.get(title_id).map(|title| title.facet.clone()))
            .collect::<HashSet<_>>();
        let mut root_folders_by_facet = HashMap::new();
        for facet in requested_facets {
            let root_folders = self
                .root_folders_for_facet(&facet)
                .await
                .map_err(|error| error.to_string());
            root_folders_by_facet.insert(facet, root_folders);
        }
        let tracked_title_folders = titles_by_id
            .values()
            .cloned()
            .filter_map(tracked_title_folder_from_preview_info)
            .collect::<Vec<_>>();

        let titles_by_id = Arc::new(titles_by_id);
        let root_folders_by_facet = Arc::new(root_folders_by_facet);
        let tracked_title_folders = Arc::new(tracked_title_folders);

        for chunk in title_ids.chunks(BULK_DELETE_PREVIEW_CONCURRENCY) {
            let mut tasks = Vec::with_capacity(chunk.len());
            for title_id in chunk {
                let app = self.clone();
                let actor = actor.clone();
                let title_id = title_id.clone();
                let titles_by_id = Arc::clone(&titles_by_id);
                let root_folders_by_facet = Arc::clone(&root_folders_by_facet);
                let tracked_title_folders = Arc::clone(&tracked_title_folders);
                tasks.push(tokio::spawn(async move {
                    let result = app
                        .preview_delete_title_files_with_shared_context(
                            &actor,
                            &title_id,
                            &titles_by_id,
                            &root_folders_by_facet,
                            &tracked_title_folders,
                        )
                        .await;
                    DeleteTitlePreviewResult {
                        title_id,
                        preview: result.as_ref().ok().cloned(),
                        error: result.err().map(|error| error.to_string()),
                    }
                }));
            }

            for task in tasks {
                let item = task.await.map_err(|error| {
                    AppError::Repository(format!("delete preview task failed: {error}"))
                })?;
                items.push(item);
            }
        }

        let preview = aggregate_delete_title_previews(&items);
        Ok(DeleteTitlesPreview { preview, items })
    }

    async fn preview_delete_title_files_with_shared_context(
        &self,
        actor: &User,
        title_id: &str,
        titles_by_id: &HashMap<String, TitleDeletePreviewInfo>,
        root_folders_by_facet: &HashMap<MediaFacet, Result<Vec<RootFolderEntry>, String>>,
        tracked_title_folders: &[TrackedTitleFolder],
    ) -> AppResult<DeletePreview> {
        let title = titles_by_id
            .get(title_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let root_folders = match root_folders_by_facet.get(&title.facet) {
            Some(Ok(root_folders)) => root_folders.clone(),
            Some(Err(error)) => {
                return Err(AppError::Repository(format!(
                    "failed to load {} root folders for delete preview: {error}",
                    title.facet.as_str()
                )));
            }
            None => {
                return Err(AppError::Repository(format!(
                    "missing {} root folders for delete preview",
                    title.facet.as_str()
                )));
            }
        };
        let other_titles = tracked_title_folders
            .iter()
            .filter(|candidate| candidate.title_id.as_str() != title.title_id.as_str())
            .cloned()
            .collect();
        let folder_path =
            if let Some(folder_path) = normalize_optional_path_string(title.folder_path) {
                Some(folder_path)
            } else {
                self.services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.title_id)
                    .await
                    .ok()
                    .and_then(|media_files| {
                        infer_title_folder_path_from_media_files(&root_folders, &media_files)
                    })
            };

        let manifest = self
            .build_delete_manifest(UserDeleteContext::Title(TitleDeleteContext {
                title_id: title.title_id,
                title_name: title.title_name,
                facet: title.facet,
                folder_path,
                root_folders,
                other_titles,
            }))
            .await?;
        Ok(manifest.to_preview())
    }

    pub async fn preview_delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
    ) -> AppResult<DeletePreview> {
        let context = self.resolve_media_file_delete_context(file_id).await?;
        self.require_delete_context_permission(actor, &context)
            .await?;
        let manifest = self.build_delete_manifest(context).await?;
        Ok(manifest.to_preview())
    }

    pub async fn preview_delete_external_subtitle_file(
        &self,
        actor: &User,
        external_subtitle_id: &str,
    ) -> AppResult<DeletePreview> {
        let context = self
            .resolve_subtitle_delete_context(external_subtitle_id)
            .await?;
        self.require_delete_context_permission(actor, &context)
            .await?;
        let manifest = self.build_delete_manifest(context).await?;
        Ok(manifest.to_preview())
    }

    pub(crate) async fn execute_delete_title_files(
        &self,
        title_id: &str,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let context = self.resolve_title_delete_context(title_id).await?;
        self.execute_delete_context(context, preview_fingerprint, typed_confirmation)
            .await
    }

    pub(crate) async fn execute_delete_media_file(
        &self,
        file_id: &str,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let context = self.resolve_media_file_delete_context(file_id).await?;
        self.execute_delete_context(context, preview_fingerprint, typed_confirmation)
            .await
    }

    pub async fn delete_external_subtitle(
        &self,
        actor: &User,
        external_subtitle_id: &str,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let context = self
            .resolve_subtitle_delete_context(external_subtitle_id)
            .await?;
        self.require_delete_context_permission(actor, &context)
            .await?;
        self.remove_external_subtitle(
            external_subtitle_id,
            ExternalSubtitleRemovalMode::DeleteOnly,
            preview_fingerprint,
            typed_confirmation,
        )
        .await
    }

    pub async fn blocklist_external_subtitle(
        &self,
        actor: &User,
        external_subtitle_id: &str,
        reason: Option<&str>,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let context = self
            .resolve_subtitle_delete_context(external_subtitle_id)
            .await?;
        self.require_delete_context_permission(actor, &context)
            .await?;
        self.remove_external_subtitle(
            external_subtitle_id,
            ExternalSubtitleRemovalMode::DeleteAndBlocklist {
                reason: reason.map(str::to_string),
            },
            preview_fingerprint,
            typed_confirmation,
        )
        .await
    }

    async fn remove_external_subtitle(
        &self,
        external_subtitle_id: &str,
        mode: ExternalSubtitleRemovalMode,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let subtitle = self
            .services
            .workflow
            .subtitle_downloads
            .get(external_subtitle_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("external subtitle {}", external_subtitle_id))
            })?;

        if matches!(mode, ExternalSubtitleRemovalMode::DeleteAndBlocklist { .. })
            && (subtitle.source_kind != scryer_domain::ExternalSubtitleSourceKind::Downloaded
                || subtitle.provider.as_deref().is_none_or(str::is_empty)
                || subtitle.provider_file_id.is_none())
        {
            return Err(AppError::Validation(
                "only provider-backed downloaded subtitles can be blocklisted".into(),
            ));
        }

        self.execute_delete_context(
            UserDeleteContext::Subtitle(SubtitleDeleteContext {
                title_id: subtitle.title_id.clone(),
                subtitle_download_id: subtitle.id.clone(),
                file_path: subtitle.file_path.clone(),
            }),
            preview_fingerprint,
            typed_confirmation,
        )
        .await?;

        let deleted = self
            .services
            .workflow
            .subtitle_downloads
            .delete(external_subtitle_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("external subtitle {}", external_subtitle_id))
            })?;

        if deleted.source_kind == scryer_domain::ExternalSubtitleSourceKind::Discovered {
            self.services
                .workflow
                .subtitle_downloads
                .delete_probe_cache_entry(&deleted.media_file_id, &deleted.file_path)
                .await?;
        }

        if let ExternalSubtitleRemovalMode::DeleteAndBlocklist { reason } = mode {
            self.services
                .workflow
                .subtitle_downloads
                .blocklist(
                    &deleted.media_file_id,
                    deleted.provider.as_deref().unwrap_or_default(),
                    deleted.provider_file_id.as_deref().unwrap_or_default(),
                    &deleted.language,
                    reason.as_deref(),
                )
                .await?;
        }

        Ok(())
    }

    async fn build_delete_manifest(
        &self,
        context: UserDeleteContext,
    ) -> AppResult<UserDeleteManifest> {
        tokio::task::spawn_blocking(move || build_delete_manifest_sync(context))
            .await
            .map_err(|error| AppError::Repository(format!("delete preview task failed: {error}")))?
    }

    async fn execute_delete_context(
        &self,
        context: UserDeleteContext,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let manifest = self.build_delete_manifest(context).await?;
        if manifest.fingerprint != preview_fingerprint {
            return Err(AppError::Validation(
                "delete preview is stale; refresh the delete dialog and confirm again".into(),
            ));
        }

        if manifest.media_count > LARGE_DELETE_MEDIA_THRESHOLD
            && typed_confirmation.unwrap_or_default().trim() != DELETE_TYPED_CONFIRMATION_VALUE
        {
            return Err(AppError::Validation(format!(
                "typed confirmation is required; enter {DELETE_TYPED_CONFIRMATION_VALUE}"
            )));
        }

        for entry in &manifest.entries {
            delete_single_path(entry).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to delete {}: {error}",
                    entry.path.display()
                ))
            })?;
        }

        Ok(())
    }

    async fn require_delete_context_permission(
        &self,
        actor: &User,
        context: &UserDeleteContext,
    ) -> AppResult<()> {
        let title_id = match context {
            UserDeleteContext::Title(context) => &context.title_id,
            UserDeleteContext::MediaFile(context) => &context.title_id,
            UserDeleteContext::Subtitle(context) => &context.title_id,
        };
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await
    }

    async fn resolve_title_delete_context(&self, title_id: &str) -> AppResult<UserDeleteContext> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        let root_folders = self.root_folders_for_facet(&title.facet).await?;
        let other_titles = self
            .services
            .catalog
            .titles
            .list(None, None)
            .await?
            .into_iter()
            .filter(|candidate| candidate.id != title.id)
            .filter_map(tracked_title_folder_from_title)
            .collect();
        let folder_path =
            if let Some(folder_path) = normalize_optional_path_string(title.folder_path) {
                Some(folder_path)
            } else {
                self.services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await
                    .ok()
                    .and_then(|media_files| {
                        infer_title_folder_path_from_media_files(&root_folders, &media_files)
                    })
            };

        Ok(UserDeleteContext::Title(TitleDeleteContext {
            title_id: title.id,
            title_name: title.name,
            facet: title.facet,
            folder_path,
            root_folders,
            other_titles,
        }))
    }

    async fn resolve_media_file_delete_context(
        &self,
        file_id: &str,
    ) -> AppResult<UserDeleteContext> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        let subtitles = self
            .services
            .workflow
            .subtitle_downloads
            .list_for_media_file(file_id)
            .await?;

        Ok(UserDeleteContext::MediaFile(MediaFileDeleteContext {
            title_id: media_file.title_id,
            file_id: media_file.id,
            file_path: media_file.file_path,
            subtitle_paths: subtitles
                .into_iter()
                .map(|record| record.file_path)
                .collect(),
        }))
    }

    async fn resolve_subtitle_delete_context(
        &self,
        subtitle_download_id: &str,
    ) -> AppResult<UserDeleteContext> {
        let subtitle = self
            .services
            .workflow
            .subtitle_downloads
            .get(subtitle_download_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("external subtitle {}", subtitle_download_id))
            })?;

        Ok(UserDeleteContext::Subtitle(SubtitleDeleteContext {
            title_id: subtitle.title_id,
            subtitle_download_id: subtitle.id,
            file_path: subtitle.file_path,
        }))
    }
}

fn tracked_title_folder_from_title(title: Title) -> Option<TrackedTitleFolder> {
    let folder_path = normalize_optional_path_string(title.folder_path)?;
    Some(TrackedTitleFolder {
        title_id: title.id,
        title_name: title.name,
        folder_path,
    })
}

fn tracked_title_folder_from_preview_info(
    title: TitleDeletePreviewInfo,
) -> Option<TrackedTitleFolder> {
    let folder_path = normalize_optional_path_string(title.folder_path)?;
    Some(TrackedTitleFolder {
        title_id: title.title_id,
        title_name: title.title_name,
        folder_path,
    })
}

fn normalize_optional_path_string(path: Option<String>) -> Option<String> {
    path.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn infer_title_folder_path_from_media_files(
    root_folders: &[RootFolderEntry],
    media_files: &[crate::TitleMediaFile],
) -> Option<String> {
    infer_title_folder_path_from_paths(
        root_folders,
        media_files
            .iter()
            .map(|media_file| stored_path_to_path_buf(&media_file.file_path)),
    )
}

fn infer_title_folder_path_from_paths<I>(
    root_folders: &[RootFolderEntry],
    file_paths: I,
) -> Option<String>
where
    I: IntoIterator<Item = PathBuf>,
{
    let normalized_roots = root_folders
        .iter()
        .filter_map(|entry| normalize_optional_path_string(Some(entry.path.clone())))
        .filter_map(|path| normalize_absolute_path(Path::new(&path)).ok())
        .collect::<Vec<_>>();
    if normalized_roots.is_empty() {
        return None;
    }

    let mut candidates = BTreeSet::new();
    for file_path in file_paths {
        let normalized_file_path = match normalize_absolute_path(&file_path) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let Some(parent) = normalized_file_path.parent() else {
            continue;
        };

        for root in &normalized_roots {
            if !normalized_file_path.starts_with(root) {
                continue;
            }

            let Ok(relative_parent) = parent.strip_prefix(root) else {
                continue;
            };
            let Some(Component::Normal(first_segment)) = relative_parent.components().next() else {
                continue;
            };

            candidates.insert(root.join(first_segment));
            break;
        }
    }

    if candidates.len() != 1 {
        return None;
    }

    candidates.into_iter().next().map(path_to_stored_string)
}

fn build_delete_manifest_sync(context: UserDeleteContext) -> AppResult<UserDeleteManifest> {
    match context {
        UserDeleteContext::Title(context) => build_title_delete_manifest(context),
        UserDeleteContext::MediaFile(context) => build_media_file_delete_manifest(context),
        UserDeleteContext::Subtitle(context) => build_subtitle_delete_manifest(context),
    }
}

fn build_title_delete_manifest(context: TitleDeleteContext) -> AppResult<UserDeleteManifest> {
    let TitleDeleteContext {
        title_id,
        title_name,
        facet,
        folder_path,
        root_folders,
        other_titles,
    } = context;

    let Some(raw_folder_path) = folder_path else {
        return Ok(finalize_manifest(
            "title",
            &title_id,
            title_name,
            Vec::new(),
        ));
    };

    let normalized_folder = normalize_absolute_path(&stored_path_to_path_buf(&raw_folder_path))?;
    let normalized_roots = normalize_root_folders(root_folders, facet)?;

    if normalized_roots.contains(&normalized_folder) {
        return Err(AppError::Validation(format!(
            "refusing to delete title folder {} because it matches a configured root folder",
            normalized_folder.display()
        )));
    }

    if normalized_roots
        .iter()
        .all(|root| !normalized_folder.starts_with(root))
    {
        return Err(AppError::Validation(format!(
            "refusing to delete title folder {} because it is outside the configured root folders",
            normalized_folder.display()
        )));
    }

    for tracked in other_titles {
        let other_path = normalize_absolute_path(&stored_path_to_path_buf(&tracked.folder_path))?;
        if other_path == normalized_folder || other_path.starts_with(&normalized_folder) {
            return Err(AppError::Validation(format!(
                "refusing to delete title folder {} because it includes tracked title folder {} ({})",
                normalized_folder.display(),
                tracked.title_name,
                other_path.display()
            )));
        }
    }

    match std::fs::symlink_metadata(&normalized_folder) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(AppError::Validation(format!(
                    "refusing to delete title folder {} because it is a symlink",
                    normalized_folder.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(AppError::Validation(format!(
                    "refusing to delete title folder {} because it is not a directory",
                    normalized_folder.display()
                )));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(finalize_manifest(
                "title",
                &title_id,
                title_name,
                Vec::new(),
            ));
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect title folder {}: {error}",
                normalized_folder.display()
            )));
        }
    }

    let entries = collect_directory_manifest_entries(&normalized_folder)?;
    Ok(finalize_manifest("title", &title_id, title_name, entries))
}

fn build_media_file_delete_manifest(
    context: MediaFileDeleteContext,
) -> AppResult<UserDeleteManifest> {
    let mut paths = BTreeSet::new();
    let file_path = normalize_absolute_path(&stored_path_to_path_buf(&context.file_path))?;
    paths.insert(file_path);

    for raw_path in context.subtitle_paths {
        let subtitle_path = normalize_absolute_path(&stored_path_to_path_buf(&raw_path))?;
        paths.insert(subtitle_path);
    }

    let entries = collect_leaf_manifest_entries(paths)?;
    Ok(finalize_manifest(
        "media_file",
        &context.file_id,
        context.file_path,
        entries,
    ))
}

fn build_subtitle_delete_manifest(context: SubtitleDeleteContext) -> AppResult<UserDeleteManifest> {
    let mut paths = BTreeSet::new();
    let subtitle_path = normalize_absolute_path(&stored_path_to_path_buf(&context.file_path))?;
    paths.insert(subtitle_path);
    let entries = collect_leaf_manifest_entries(paths)?;
    Ok(finalize_manifest(
        "subtitle",
        &context.subtitle_download_id,
        context.file_path,
        entries,
    ))
}

fn normalize_root_folders(
    root_folders: Vec<RootFolderEntry>,
    facet: MediaFacet,
) -> AppResult<Vec<PathBuf>> {
    let mut normalized = Vec::with_capacity(root_folders.len());
    for entry in root_folders {
        normalized.push(
            normalize_absolute_path(&stored_path_to_path_buf(&entry.path)).map_err(|error| {
                AppError::Validation(format!(
                    "configured {} root folder {} is invalid: {error}",
                    facet.as_str(),
                    entry.path
                ))
            })?,
        );
    }
    if normalized.is_empty() {
        return Err(AppError::Validation(format!(
            "no configured {} root folders are available for safe deletion",
            facet.as_str()
        )));
    }
    Ok(normalized)
}

fn normalize_absolute_path(path: &Path) -> AppResult<PathBuf> {
    if !path.is_absolute() {
        return Err(AppError::Validation(format!(
            "path must be absolute: {}",
            path.display()
        )));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(AppError::Validation(format!(
                        "path cannot escape its root: {}",
                        path.display()
                    )));
                }
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }

    Ok(normalized)
}

fn collect_directory_manifest_entries(root: &Path) -> AppResult<Vec<DeleteManifestEntry>> {
    let mut file_entries = Vec::new();
    let mut directory_entries = Vec::new();
    walk_directory_entries(root, &mut file_entries, &mut directory_entries)?;

    file_entries.sort_by(|left, right| left.path.cmp(&right.path));
    directory_entries.sort_by(|left, right| {
        directory_depth(&right.path)
            .cmp(&directory_depth(&left.path))
            .then_with(|| left.path.cmp(&right.path))
    });
    file_entries.extend(directory_entries);
    Ok(file_entries)
}

fn walk_directory_entries(
    directory: &Path,
    file_entries: &mut Vec<DeleteManifestEntry>,
    directory_entries: &mut Vec<DeleteManifestEntry>,
) -> AppResult<()> {
    let mut child_paths = Vec::new();
    let read_dir = std::fs::read_dir(directory).map_err(|error| {
        AppError::Repository(format!(
            "failed to read directory {}: {error}",
            directory.display()
        ))
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|error| {
            AppError::Repository(format!(
                "failed to read directory entry in {}: {error}",
                directory.display()
            ))
        })?;
        child_paths.push(entry.path());
    }
    child_paths.sort();

    for path in child_paths {
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect filesystem entry {}: {error}",
                path.display()
            ))
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            file_entries.push(DeleteManifestEntry {
                path,
                delete_kind: DeletePathKind::Symlink,
                preview_category: Some(DeletePreviewCategory::Other),
            });
            continue;
        }
        if metadata.is_dir() {
            walk_directory_entries(&path, file_entries, directory_entries)?;
            continue;
        }

        file_entries.push(DeleteManifestEntry {
            path: path.clone(),
            delete_kind: DeletePathKind::File,
            preview_category: Some(classify_path_for_preview(&path)),
        });
    }

    directory_entries.push(DeleteManifestEntry {
        path: directory.to_path_buf(),
        delete_kind: DeletePathKind::Directory,
        preview_category: None,
    });

    Ok(())
}

fn collect_leaf_manifest_entries(paths: BTreeSet<PathBuf>) -> AppResult<Vec<DeleteManifestEntry>> {
    let mut entries = Vec::new();
    for path in paths {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to inspect filesystem entry {}: {error}",
                    path.display()
                )));
            }
        };
        let file_type = metadata.file_type();
        if metadata.is_dir() {
            return Err(AppError::Validation(format!(
                "refusing to delete directory path {} from a single-file delete flow",
                path.display()
            )));
        }
        entries.push(DeleteManifestEntry {
            path: path.clone(),
            delete_kind: if file_type.is_symlink() {
                DeletePathKind::Symlink
            } else {
                DeletePathKind::File
            },
            preview_category: Some(if file_type.is_symlink() {
                DeletePreviewCategory::Other
            } else {
                classify_path_for_preview(&path)
            }),
        });
    }
    Ok(entries)
}

fn classify_path_for_preview(path: &Path) -> DeletePreviewCategory {
    if is_video_file(path) {
        DeletePreviewCategory::Media
    } else if is_subtitle_file(path) {
        DeletePreviewCategory::Subtitle
    } else if is_image_file(path) {
        DeletePreviewCategory::Image
    } else {
        DeletePreviewCategory::Other
    }
}

fn directory_depth(path: &Path) -> usize {
    path.components().count()
}

fn finalize_manifest(
    intent_kind: &str,
    target_id: &str,
    target_label: String,
    entries: Vec<DeleteManifestEntry>,
) -> UserDeleteManifest {
    let mut media_count = 0usize;
    let mut subtitle_count = 0usize;
    let mut image_count = 0usize;
    let mut other_count = 0usize;
    let mut directory_count = 0usize;

    for entry in &entries {
        match entry.preview_category {
            Some(DeletePreviewCategory::Media) => media_count += 1,
            Some(DeletePreviewCategory::Subtitle) => subtitle_count += 1,
            Some(DeletePreviewCategory::Image) => image_count += 1,
            Some(DeletePreviewCategory::Other) => other_count += 1,
            None => directory_count += 1,
        }
    }

    let fingerprint = build_delete_manifest_fingerprint(intent_kind, target_id, &entries);
    UserDeleteManifest {
        fingerprint,
        target_label,
        entries,
        media_count,
        subtitle_count,
        image_count,
        other_count,
        directory_count,
    }
}

fn aggregate_delete_title_previews(items: &[DeleteTitlePreviewResult]) -> DeletePreview {
    let mut total_file_count = 0i32;
    let mut media_count = 0i32;
    let mut subtitle_count = 0i32;
    let mut image_count = 0i32;
    let mut other_count = 0i32;
    let mut directory_count = 0i32;
    let mut requires_typed_confirmation = false;
    let mut sample_paths = Vec::new();
    let mut fingerprint_parts = vec!["intent:bulk_title_delete".to_string()];

    for item in items {
        fingerprint_parts.push(format!("title:{}", item.title_id));
        let Some(preview) = item.preview.as_ref() else {
            fingerprint_parts.push("preview:failed".to_string());
            continue;
        };
        fingerprint_parts.push(format!("preview:{}", preview.fingerprint));
        total_file_count += preview.total_file_count;
        media_count += preview.media_count;
        subtitle_count += preview.subtitle_count;
        image_count += preview.image_count;
        other_count += preview.other_count;
        directory_count += preview.directory_count;
        requires_typed_confirmation |= preview.requires_typed_confirmation;
        for path in &preview.sample_paths {
            if sample_paths.len() >= DELETE_PREVIEW_SAMPLE_PATH_LIMIT {
                break;
            }
            sample_paths.push(path.clone());
        }
    }

    if media_count as usize > LARGE_DELETE_MEDIA_THRESHOLD {
        requires_typed_confirmation = true;
    }

    DeletePreview {
        fingerprint: sha256_hex(fingerprint_parts.join("\n")),
        total_file_count,
        media_count,
        subtitle_count,
        image_count,
        other_count,
        directory_count,
        requires_typed_confirmation,
        typed_confirmation_prompt: requires_typed_confirmation
            .then(|| DELETE_TYPED_CONFIRMATION_PROMPT.to_string()),
        target_label: format!("{} selected titles", items.len()),
        sample_paths,
    }
}

fn build_delete_manifest_fingerprint(
    intent_kind: &str,
    target_id: &str,
    entries: &[DeleteManifestEntry],
) -> String {
    let mut payload = vec![
        format!("intent:{intent_kind}"),
        format!("target:{target_id}"),
    ];
    payload.extend(entries.iter().map(|entry| {
        let category = entry
            .preview_category
            .map(DeletePreviewCategory::as_str)
            .unwrap_or("directory");
        format!(
            "{}:{category}:{}",
            entry.delete_kind.as_str(),
            entry.path.display()
        )
    }));
    sha256_hex(payload.join("\n"))
}

async fn delete_single_path(entry: &DeleteManifestEntry) -> AppResult<()> {
    match entry.delete_kind {
        DeletePathKind::File => fs::remove_file(&entry.path)
            .await
            .map_err(|error| AppError::Repository(error.to_string())),
        DeletePathKind::Directory => fs::remove_dir(&entry.path)
            .await
            .map_err(|error| AppError::Repository(error.to_string())),
        DeletePathKind::Symlink => match fs::remove_file(&entry.path).await {
            Ok(()) => Ok(()),
            Err(remove_file_error) => {
                fs::remove_dir(&entry.path)
                    .await
                    .map_err(|remove_dir_error| {
                        AppError::Repository(format!("{remove_file_error}; {remove_dir_error}"))
                    })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_title_folder_path_from_paths_uses_title_root_under_configured_root() {
        let root_folders = vec![RootFolderEntry {
            path: "/data/anime".to_string(),
            is_default: true,
        }];

        let inferred = infer_title_folder_path_from_paths(
            &root_folders,
            vec![
                PathBuf::from("/data/anime/Emberfall/Season 01/Emberfall - S01E01.mkv"),
                PathBuf::from("/data/anime/Emberfall/Season 02/Emberfall - S02E01.mkv"),
            ],
        );

        assert_eq!(inferred.as_deref(), Some("/data/anime/Emberfall"));
    }

    #[test]
    fn infer_title_folder_path_from_paths_returns_none_for_mixed_title_roots() {
        let root_folders = vec![RootFolderEntry {
            path: "/data/anime".to_string(),
            is_default: true,
        }];

        let inferred = infer_title_folder_path_from_paths(
            &root_folders,
            vec![
                PathBuf::from("/data/anime/Emberfall/Season 01/Emberfall - S01E01.mkv"),
                PathBuf::from("/data/anime/Stormleaf/Season 01/Stormleaf - S01E01.mkv"),
            ],
        );

        assert_eq!(inferred, None);
    }
}
