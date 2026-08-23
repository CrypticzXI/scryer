use super::*;

impl AppUseCase {
    pub async fn discover_plex_media_servers(
        &self,
        actor: &User,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .external_identity_verifier
            .discover_plex_servers(plex_auth_token)
            .await
    }

    pub(super) async fn resolve_plex_server_selection(
        &self,
        provider: &MediaServerProvider,
        existing_machine_id: Option<&str>,
        requested_machine_id: Option<String>,
        plex_auth_token: Option<&str>,
        plex_server_id: Option<&str>,
    ) -> AppResult<ResolvedPlexServerSelection> {
        if *provider != MediaServerProvider::Plex {
            return Ok(ResolvedPlexServerSelection::default());
        }

        let token = plex_auth_token
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected_server_id = plex_server_id
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(token) = token {
            let servers = self
                .services
                .integrations
                .external_identity_verifier
                .discover_plex_servers(token)
                .await?;
            if servers.is_empty() {
                return Err(AppError::Validation(
                    "Plex did not return any accessible servers for that account".into(),
                ));
            }
            let selected = if let Some(selected_server_id) = selected_server_id {
                servers
                    .iter()
                    .find(|server| server.id == selected_server_id)
                    .ok_or_else(|| {
                        AppError::Validation(
                            "The selected Plex server was not returned by Plex discovery".into(),
                        )
                    })?
            } else if servers.len() == 1 {
                &servers[0]
            } else {
                return Err(AppError::Validation(
                    "Select a Plex server before saving this connection".into(),
                ));
            };
            return Ok(ResolvedPlexServerSelection {
                machine_id: Some(selected.id.clone()),
            });
        }

        if selected_server_id.is_some() {
            return Err(AppError::Validation(
                "Sign in with Plex before selecting a Plex server".into(),
            ));
        }

        Ok(ResolvedPlexServerSelection {
            machine_id: normalize_optional_string(requested_machine_id)
                .or_else(|| existing_machine_id.map(ToString::to_string)),
        })
    }
}
