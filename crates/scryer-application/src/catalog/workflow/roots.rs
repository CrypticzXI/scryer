#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LibraryRootFolder {
    pub library_id: String,
    pub library_name: String,
    pub facet: MediaFacet,
    pub path: String,
    pub normalized_path: String,
}
pub(crate) fn normalize_library_root_path(path: &str) -> String {
    scryer_domain::normalize_library_root_path(path)
}
pub(crate) fn library_path_is_under_root(path: &str, root: &str) -> bool {
    let normalized_path = normalize_library_root_path(path);
    let normalized_root = normalize_library_root_path(root);
    if normalized_path.is_empty() || normalized_root.is_empty() {
        return false;
    }

    #[cfg(windows)]
    let separator = "\\";
    #[cfg(not(windows))]
    let separator = "/";

    let descendant_prefix = if normalized_root.ends_with(separator) {
        normalized_root.clone()
    } else {
        format!("{normalized_root}{separator}")
    };

    normalized_path == normalized_root || normalized_path.starts_with(&descendant_prefix)
}
pub(crate) fn library_root_paths_overlap(left: &str, right: &str) -> bool {
    library_path_is_under_root(left, right) || library_path_is_under_root(right, left)
}
pub(crate) fn library_root_folders_from_libraries(
    libraries: &[Library],
    facet: Option<&MediaFacet>,
) -> Vec<LibraryRootFolder> {
    let mut roots = Vec::new();
    for library in libraries {
        if facet.is_some_and(|facet| library.facet != *facet) {
            continue;
        }

        for root in &library.roots {
            let path = root.path.trim();
            let normalized_path = normalize_library_root_path(path);
            if normalized_path.is_empty() {
                continue;
            }
            roots.push(LibraryRootFolder {
                library_id: library.id.clone(),
                library_name: library.name.clone(),
                facet: library.facet.clone(),
                path: path.to_string(),
                normalized_path,
            });
        }
    }
    roots
}
fn submission_scopes_overlap(
    title_id: &str,
    existing: &SubmissionScope,
    requested: &SubmissionScope,
    episodes: &[Episode],
) -> bool {
    let existing_submission = submission_for_scope(title_id, existing);
    if wanted_item_candidates_for_submission_scope(title_id, requested, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&existing_submission, item, collection_id.as_deref())
        })
    {
        return true;
    }

    let requested_submission = submission_for_scope(title_id, requested);
    wanted_item_candidates_for_submission_scope(title_id, existing, episodes)
        .iter()
        .any(|(item, collection_id)| {
            submission_blocks_wanted_item(&requested_submission, item, collection_id.as_deref())
        })
}
impl AppUseCase {
    /// Return the configured root folders for a facet.
    ///
    /// Reads canonical roots from the facet's default library. Legacy
    /// `<facet>.root_folders` and `<facet>.path` settings are maintained only
    /// as compatibility mirrors and are reconciled during startup.
    pub async fn root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        let handler = self.facet_registry.get(facet);
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");

        if let Some(library) = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        {
            let entries = root_folder_entries_from_library_roots(&library.roots);

            if !entries.is_empty() {
                return Ok(entries);
            }
        }

        Ok(vec![scryer_domain::RootFolderEntry {
            path: default_path.to_string(),
            is_default: true,
        }])
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_title_root_folder_id_for_library(
        &self,
        library_id: &str,
        root_folder_id: Option<&str>,
    ) -> AppResult<String> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        let root = match root_folder_id
            .map(str::trim)
            .filter(|root_folder_id| !root_folder_id.is_empty())
        {
            Some(root_folder_id) => library
                .roots
                .iter()
                .find(|root| root.id == root_folder_id)
                .ok_or_else(|| {
                    AppError::Validation(
                        "rootFolderId must reference a root on the title library".to_string(),
                    )
                })?,
            None => library
                .roots
                .iter()
                .find(|root| root.is_default)
                .or_else(|| library.roots.first())
                .ok_or_else(|| {
                    AppError::Validation(
                        "title library must have at least one root folder".to_string(),
                    )
                })?,
        };
        Ok(root.id.clone())
    }

    pub(crate) async fn title_root_folder_path_override(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<String> {
        self.title_root_folder_path_for_parts(
            &title.root_folder_id,
            &title.library_id,
            &title.facet,
        )
        .await
    }

    pub async fn title_root_folder_path_for_parts(
        &self,
        root_folder_id: &str,
        library_id: &str,
        facet: &MediaFacet,
    ) -> AppResult<String> {
        let root_folder_id = root_folder_id.trim();
        if root_folder_id.is_empty() {
            return Err(AppError::Repository(
                "title root folder id cannot be empty".to_string(),
            ));
        }
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        if library.facet != *facet {
            return Err(AppError::Repository(format!(
                "title root folder library {library_id} does not match title facet {}",
                facet.as_str()
            )));
        }
        library
            .roots
            .into_iter()
            .find(|root| root.id == root_folder_id)
            .map(|root| root.path)
            .ok_or_else(|| {
                AppError::Repository(format!(
                    "title root folder id {root_folder_id} is not configured on library {library_id}"
                ))
            })
    }
}
impl AppUseCase {
    /// Return every configured library root for a facet across all libraries.
    pub(crate) async fn all_library_root_folders_for_facet(
        &self,
        facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<LibraryRootFolder>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(library_root_folders_from_libraries(&libraries, Some(facet)))
    }
}
impl AppUseCase {
    /// Return every configured library root across all facets and libraries.
    pub(crate) async fn all_library_root_folders(&self) -> AppResult<Vec<LibraryRootFolder>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(library_root_folders_from_libraries(&libraries, None))
    }
}
impl AppUseCase {
    /// Return the configured root folders for a concrete library.
    ///
    /// If a stale title points at a missing or empty library, fall back to the
    /// facet default roots so existing data remains importable.
    pub(crate) async fn root_folders_for_library(
        &self,
        library_id: &str,
        fallback_facet: &scryer_domain::MediaFacet,
    ) -> AppResult<Vec<scryer_domain::RootFolderEntry>> {
        if let Some(library) = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
        {
            if library.facet != *fallback_facet {
                warn!(
                    library_id = %library.id,
                    library_facet = library.facet.as_str(),
                    title_facet = fallback_facet.as_str(),
                    "library facet does not match title facet; falling back to facet default roots"
                );
                return self.root_folders_for_facet(fallback_facet).await;
            }

            let entries = root_folder_entries_from_library_roots(&library.roots);
            if !entries.is_empty() {
                return Ok(entries);
            }
            warn!(
                library_id = %library.id,
                facet = library.facet.as_str(),
                "library has no roots; falling back to facet default roots"
            );
        } else {
            warn!(
                library_id,
                facet = fallback_facet.as_str(),
                "library is missing; falling back to facet default roots"
            );
        }

        self.root_folders_for_facet(fallback_facet).await
    }
}

#[cfg(test)]
mod tests {
    use super::library_path_is_under_root;

    #[test]
    fn root_containment_handles_unix_root() {
        assert!(library_path_is_under_root("/media/movies", "/"));
    }

    #[test]
    fn root_containment_handles_windows_drive_root() {
        assert!(library_path_is_under_root("C:\\Media\\Movies", "C:\\"));
    }

    #[test]
    fn root_containment_handles_unc_share_root() {
        assert!(library_path_is_under_root(
            "\\\\server\\share\\Movies",
            "\\\\server\\share\\"
        ));
    }
}
