const MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM: usize = 5;
const STANDBY_RETENTION_HOURS: i64 = 24;
fn annotated_auto_decision_code(candidate: &IndexerSearchResult) -> ReleaseAutoDecisionCode {
    candidate
        .auto_decision_code
        .as_deref()
        .and_then(ReleaseAutoDecisionCode::parse)
        .unwrap_or_else(|| {
            warn!(
                release_title = candidate.title.as_str(),
                "candidate missing auto decision annotation; defaulting to quality_blocked"
            );
            ReleaseAutoDecisionCode::QualityBlocked
        })
}
fn effective_auto_decision_code(
    candidate: &IndexerSearchResult,
    failed_source_kinds: &[DownloadSourceKind],
    db_blocklist: &std::collections::HashSet<String>,
) -> ReleaseAutoDecisionCode {
    if db_blocklist.contains(&candidate.title.to_ascii_lowercase()) {
        return ReleaseAutoDecisionCode::DbBlocklisted;
    }

    if let Some(source_kind) = candidate.source_kind
        && failed_source_kinds.contains(&source_kind)
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    annotated_auto_decision_code(candidate)
}
async fn record_release_decision(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    candidate: &IndexerSearchResult,
    decision_code: ReleaseAutoDecisionCode,
    now: &DateTime<Utc>,
) {
    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    let mut decision_candidate = candidate.clone();
    annotate_auto_decision(&mut decision_candidate, decision_code);
    let decision_record = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: decision_candidate.title.clone(),
        release_url: decision_candidate
            .download_url
            .clone()
            .or_else(|| decision_candidate.link.clone()),
        release_size_bytes: decision_candidate.size_bytes,
        decision_code: decision_code.as_str().to_string(),
        candidate_score,
        current_score: item.current_score,
        score_delta: item
            .current_score
            .map(|current_score| candidate_score - current_score),
        explanation_json: serialize_decision_explanation(&decision_candidate),
        created_at: now.to_rfc3339(),
    };

    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .insert_release_decision(&decision_record)
        .await;
}
impl AppUseCase {
    pub async fn list_release_decisions(
        &self,
        actor: &User,
        query: ReleaseDecisionsQuery,
    ) -> AppResult<Vec<ReleaseDecision>> {
        if let Some(wid) = query.wanted_item_id.as_deref() {
            let wanted = self
                .services
                .workflow
                .acquisition_scope_states
                .get_acquisition_scope_state_by_id(wid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("wanted item {wid}")))?;
            let library_id = if let Some(library_id) = wanted.library_id.as_deref() {
                library_id.to_string()
            } else {
                self.services
                    .catalog
                    .titles
                    .get_by_id(&wanted.title_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))?
                    .library_id
            };
            self.require_library_permission(
                actor,
                &library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            return self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_acquisition_scope_state(wid, query.limit, query.offset)
                .await;
        }
        if let Some(tid) = query.title_id.as_deref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(tid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {tid}")))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            return self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_title(tid, query.limit, query.offset)
                .await;
        }
        Ok(vec![])
    }
}
