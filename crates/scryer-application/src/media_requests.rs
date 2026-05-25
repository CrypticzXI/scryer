use super::*;
use crate::domain_events::new_global_domain_event;
use scryer_domain::{
    DomainEventPayload, LibraryPermission, MediaRequestStatus, MediaRequestSubmittedEventData,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct SubmitMediaRequestInput {
    pub library_id: String,
    pub facet: MediaFacet,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub external_ids: Vec<ExternalId>,
}

#[derive(Clone, Debug)]
pub struct SubmitMediaRequestOutcome {
    pub accepted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ListMediaRequestsInput {
    pub facet: Option<MediaFacet>,
    pub library_ids: Option<Vec<String>>,
    pub status: Option<MediaRequestStatus>,
}

impl AppUseCase {
    pub async fn submit_media_request(
        &self,
        actor: &User,
        input: SubmitMediaRequestInput,
    ) -> AppResult<SubmitMediaRequestOutcome> {
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(AppError::Validation("request title is required".into()));
        }

        let external_ids = normalize_media_request_external_ids(input.external_ids)?;
        if external_ids.is_empty() {
            return Err(AppError::Validation(
                "media requests must include SMG external identifiers".into(),
            ));
        }
        if !external_ids
            .iter()
            .any(is_smg_request_correlation_external_id)
        {
            return Err(AppError::Validation(
                "media requests must include a searchable SMG identifier".into(),
            ));
        }

        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(input.library_id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound("library not found".into()))?;

        if library.facet != input.facet {
            return Err(AppError::Validation(
                "library facet does not match requested media facet".into(),
            ));
        }

        self.require_library_permission(actor, &library.id, LibraryPermission::Request)
            .await?;
        self.ensure_request_subject_is_not_in_library(&library.id, &external_ids)
            .await?;

        let request = NewMediaRequest {
            id: Id::new().0,
            library_id: library.id.clone(),
            facet: input.facet,
            identity_fingerprint: media_request_identity_fingerprint(&external_ids),
            title,
            sort_title: normalized_optional_string(input.sort_title),
            slug: normalized_optional_string(input.slug),
            poster_url: normalized_optional_string(input.poster_url),
            year: input.year,
            overview: normalized_optional_string(input.overview),
            runtime_minutes: input.runtime_minutes,
            language: normalized_optional_string(input.language),
            content_status: normalized_optional_string(input.content_status),
            external_ids,
            created_by_user_id: actor.id.clone(),
        };
        let submitted_event = new_global_domain_event(
            Some(actor.id.clone()),
            DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
                request_id: request.id.clone(),
                library_id: request.library_id.clone(),
                facet: request.facet.clone(),
                title_name: request.title.clone(),
                external_ids: request.external_ids.clone(),
                poster_url: request.poster_url.clone(),
                year: request.year,
            }),
        );

        self.services
            .catalog
            .media_requests
            .submit(request, actor, submitted_event)
            .await?;

        Ok(SubmitMediaRequestOutcome { accepted: true })
    }

    pub async fn list_media_requests(
        &self,
        actor: &User,
        input: ListMediaRequestsInput,
    ) -> AppResult<Vec<MediaRequest>> {
        let allowed_libraries = self
            .list_libraries_for_permission(
                actor,
                input.facet.clone(),
                LibraryPermission::ManageTitles,
            )
            .await?;
        let allowed_ids = allowed_libraries
            .into_iter()
            .map(|library| library.id)
            .collect::<HashSet<_>>();

        let library_ids = match input.library_ids {
            Some(requested_ids) => requested_ids
                .into_iter()
                .filter(|id| allowed_ids.contains(id))
                .collect::<Vec<_>>(),
            None => allowed_ids.into_iter().collect::<Vec<_>>(),
        };

        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        self.services
            .catalog
            .media_requests
            .list(MediaRequestQuery {
                facet: input.facet,
                library_ids: Some(library_ids),
                status: input.status,
            })
            .await
    }

    async fn ensure_request_subject_is_not_in_library(
        &self,
        library_id: &str,
        external_ids: &[ExternalId],
    ) -> AppResult<()> {
        for (source, values) in group_external_id_values_by_source(external_ids) {
            let titles = self
                .services
                .catalog
                .titles
                .list_by_external_ids(&source, &values)
                .await?;
            if titles
                .into_iter()
                .any(|title| title.library_id == library_id)
            {
                return Err(AppError::Validation(
                    "title already exists in the target library".into(),
                ));
            }
        }
        Ok(())
    }
}

fn normalize_media_request_external_ids(
    external_ids: Vec<ExternalId>,
) -> AppResult<Vec<ExternalId>> {
    let mut seen = BTreeSet::new();
    for external_id in external_ids {
        let source = external_id.source.trim().to_ascii_lowercase();
        let value = external_id.value.trim().to_string();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        seen.insert((source, value));
    }

    Ok(seen
        .into_iter()
        .map(|(source, value)| ExternalId { source, value })
        .collect())
}

fn media_request_identity_fingerprint(external_ids: &[ExternalId]) -> String {
    sha256_hex(
        external_ids
            .iter()
            .map(|external_id| format!("{}:{}", external_id.source, external_id.value))
            .collect::<Vec<_>>()
            .join("|"),
    )
}

fn group_external_id_values_by_source(external_ids: &[ExternalId]) -> Vec<(String, Vec<String>)> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
    for external_id in external_ids {
        grouped
            .entry(external_id.source.clone())
            .or_default()
            .push(external_id.value.clone());
    }
    grouped.into_iter().collect()
}

fn is_smg_request_correlation_external_id(external_id: &ExternalId) -> bool {
    matches!(external_id.source.as_str(), "tvdb" | "imdb" | "tmdb")
}

fn normalized_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
