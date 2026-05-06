use std::collections::HashMap;

use scryer_domain::{
    AppPermission, AppPermissionMask, LibraryPermission, LibraryPermissionMask, MediaFacet, User,
    UserAuthorization,
};

use crate::{AppError, AppResult, AppUseCase};

impl AppUseCase {
    pub async fn load_user_authorization(&self, actor: &User) -> AppResult<UserAuthorization> {
        let app = self
            .services
            .catalog
            .libraries
            .app_permission_mask_for_user(&actor.id)
            .await?;

        let grants = self
            .services
            .catalog
            .libraries
            .permission_masks_for_user(&actor.id)
            .await?;
        let mut libraries = HashMap::with_capacity(grants.len());
        for grant in grants {
            libraries.insert(grant.library_id, grant.permissions);
        }

        Ok(UserAuthorization {
            app,
            libraries,
            default_library: LibraryPermissionMask::NONE,
            loaded: true,
        })
    }

    pub async fn attach_user_authorization(&self, mut actor: User) -> AppResult<User> {
        actor.authorization = self.load_user_authorization(&actor).await?;
        Ok(actor)
    }

    async fn authorization_for_actor(&self, actor: &User) -> AppResult<UserAuthorization> {
        if actor.authorization.loaded {
            Ok(actor.authorization.clone())
        } else {
            self.load_user_authorization(actor).await
        }
    }

    pub async fn require_app_permission(
        &self,
        actor: &User,
        permission: AppPermission,
    ) -> AppResult<()> {
        let authorization = self.authorization_for_actor(actor).await?;
        if authorization
            .app
            .contains(AppPermissionMask::from_permission(permission))
        {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ))
        }
    }

    pub async fn has_app_permission(
        &self,
        actor: &User,
        permission: AppPermission,
    ) -> AppResult<bool> {
        Ok(self
            .authorization_for_actor(actor)
            .await?
            .has_app_permission(permission))
    }

    pub async fn has_any_app_permission(
        &self,
        actor: &User,
        permissions: AppPermissionMask,
    ) -> AppResult<bool> {
        Ok(self
            .authorization_for_actor(actor)
            .await?
            .has_any_app_permission(permissions))
    }

    pub async fn require_library_permission(
        &self,
        actor: &User,
        library_id: &str,
        permission: LibraryPermission,
    ) -> AppResult<()> {
        let authorization = self.authorization_for_actor(actor).await?;
        let permissions = authorization.library_permissions(library_id);
        if permissions.contains(LibraryPermissionMask::from_permission(permission)) {
            Ok(())
        } else {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        }
    }

    pub async fn has_any_library_permission(
        &self,
        actor: &User,
        permission: LibraryPermission,
    ) -> AppResult<bool> {
        Ok(!self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty())
    }

    pub async fn authorized_library_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        let required = LibraryPermissionMask::from_permission(permission);
        if libraries.is_empty() && authorization.default_library.contains(required) {
            return Ok(match facet {
                Some(facet) => vec![scryer_domain::default_library_id_for_facet(&facet)],
                None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                    .into_iter()
                    .map(|facet| scryer_domain::default_library_id_for_facet(&facet))
                    .collect(),
            });
        }
        if authorization
            .app
            .contains(AppPermissionMask::MANAGE_CATALOG_SETTINGS)
        {
            return Ok(libraries.into_iter().map(|library| library.id).collect());
        }
        Ok(libraries
            .into_iter()
            .filter(|library| {
                authorization
                    .library_permissions(&library.id)
                    .contains(required)
            })
            .map(|library| library.id)
            .collect())
    }

    pub async fn list_libraries_for_permission(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        permission: LibraryPermission,
    ) -> AppResult<Vec<scryer_domain::Library>> {
        let libraries = self.services.catalog.libraries.list(facet).await?;
        let authorization = self.authorization_for_actor(actor).await?;
        if authorization
            .app
            .contains(AppPermissionMask::MANAGE_CATALOG_SETTINGS)
            || authorization
                .app
                .contains(AppPermissionMask::MANAGE_PERMISSIONS)
        {
            return Ok(libraries);
        }
        let required = LibraryPermissionMask::from_permission(permission);
        Ok(libraries
            .into_iter()
            .filter(|library| {
                authorization
                    .library_permissions(&library.id)
                    .contains(required)
            })
            .collect())
    }
}
