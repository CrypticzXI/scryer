use std::collections::HashSet;

use crate::LibraryFile;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

#[derive(Clone, Debug)]
pub(crate) struct MovieScanScope {
    canonical_folder_path: Option<String>,
    discovered_paths: HashSet<String>,
}

impl MovieScanScope {
    pub(crate) fn from_scan_inputs(
        cleanup_canonical_folder_path: Option<&str>,
        title_folder_path: Option<&str>,
        scan_folder_path: Option<&str>,
        discovered_files: &[LibraryFile],
    ) -> Self {
        let canonical_folder_path = Self::normalize_folder_path(cleanup_canonical_folder_path)
            .or_else(|| Self::normalize_folder_path(title_folder_path))
            .or_else(|| Self::normalize_folder_path(scan_folder_path))
            .or_else(|| {
                discovered_files
                    .iter()
                    .map(|file| file.path.as_str())
                    .min()
                    .and_then(Self::file_parent_folder_path)
            });
        let discovered_paths = discovered_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();

        Self {
            canonical_folder_path,
            discovered_paths,
        }
    }

    pub(crate) fn from_title_folder_or_file(
        title_folder_path: Option<&str>,
        file_path: &str,
    ) -> Option<Self> {
        let canonical_folder_path = Self::normalize_folder_path(title_folder_path)
            .or_else(|| Self::file_parent_folder_path(file_path))?;

        Some(Self {
            canonical_folder_path: Some(canonical_folder_path),
            discovered_paths: HashSet::new(),
        })
    }

    pub(crate) fn file_is_inside_canonical_folder(&self, file_path: &str) -> bool {
        self.canonical_folder_path
            .as_deref()
            .is_some_and(|folder_path| Self::path_is_in_folder(file_path, folder_path))
    }

    pub(crate) fn file_is_outside_canonical_folder(&self, file_path: &str) -> bool {
        self.canonical_folder_path
            .as_deref()
            .is_some_and(|folder_path| !Self::path_is_in_folder(file_path, folder_path))
    }

    pub(crate) fn file_is_in_scan_scope(&self, file_path: &str) -> bool {
        if let Some(folder_path) = self.canonical_folder_path.as_deref() {
            Self::path_is_in_folder(file_path, folder_path)
        } else {
            self.discovered_paths.contains(file_path)
        }
    }

    pub(crate) fn normalize_folder_path(path: Option<&str>) -> Option<String> {
        path.map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToString::to_string)
    }

    pub(crate) fn file_parent_folder_path(file_path: &str) -> Option<String> {
        let path = stored_path_to_path_buf(file_path);
        path.parent()
            .map(path_to_stored_string)
            .and_then(|path| Self::normalize_folder_path(Some(path.as_str())))
    }

    pub(crate) fn path_is_in_folder(file_path: &str, folder_path: &str) -> bool {
        let file_path = stored_path_to_path_buf(file_path);
        let folder_path = stored_path_to_path_buf(folder_path);
        file_path == folder_path || file_path.starts_with(folder_path)
    }
}
