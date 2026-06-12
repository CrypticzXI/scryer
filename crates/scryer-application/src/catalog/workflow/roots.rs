#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LibraryRootFolder {
    pub library_id: String,
    pub library_name: String,
    pub facet: MediaFacet,
    pub path: String,
    pub normalized_path: String,
}
pub(crate) fn normalize_library_root_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(windows)]
    {
        trimmed
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }

    #[cfg(not(windows))]
    {
        trimmed.replace('\\', "/").trim_end_matches('/').to_string()
    }
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

    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}{separator}"))
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
