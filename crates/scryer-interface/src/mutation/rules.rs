use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::types::*;
use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_domain::AppPermission;

fn parse_facets(input: Option<Vec<String>>) -> Vec<scryer_domain::MediaFacet> {
    input
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| scryer_domain::MediaFacet::parse(&s))
        .collect()
}

#[derive(Default)]
pub(crate) struct RulesMutations;

#[Object]
impl RulesMutations {
    async fn create_rule_set(
        &self,
        ctx: &Context<'_>,
        input: CreateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .create_rule_set(
                &actor,
                input.name,
                input.description.unwrap_or_default(),
                input.rego_source,
                parse_facets(input.applied_facets),
                input.priority.unwrap_or(0),
                input.enabled,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    async fn update_rule_set(
        &self,
        ctx: &Context<'_>,
        input: UpdateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .update_rule_set(
                &actor,
                String::from(input.id),
                input.name,
                input.description,
                input.rego_source,
                input.applied_facets.map(|f| parse_facets(Some(f))),
                input.priority,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    async fn delete_rule_set(&self, ctx: &Context<'_>, id: ID) -> GqlResult<DeleteRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id = id.to_string();
        app.delete_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteRuleSetPayload { id: ID::from(id) })
    }

    async fn toggle_rule_set(
        &self,
        ctx: &Context<'_>,
        input: ToggleRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .toggle_rule_set(&actor, input.id.as_ref(), input.enabled)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    async fn set_title_required_audio(
        &self,
        ctx: &Context<'_>,
        input: SetTitleRequiredAudioInput,
    ) -> GqlResult<SetTitleRequiredAudioPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = String::from(input.title_id);
        let facet_value = input.facet;
        let facet = input.facet.into_domain();
        let languages = input.languages;
        app.set_title_required_audio(&actor, &title_id, facet.as_str(), languages.clone())
            .await
            .map_err(to_gql_error)?;
        Ok(SetTitleRequiredAudioPayload {
            title_id: ID::from(title_id),
            facet: facet_value,
            languages,
            updated: true,
        })
    }

    async fn validate_rule_set(
        &self,
        ctx: &Context<'_>,
        input: ValidateRuleSetInput,
    ) -> GqlResult<RuleValidationResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set_id = input
            .rule_set_id
            .map(String::from)
            .unwrap_or_else(|| "r_validation_test".to_string());
        let result = app
            .validate_rule_set(&actor, &input.rego_source, &rule_set_id)
            .await
            .map_err(to_gql_error)?;

        Ok(RuleValidationResultPayload {
            valid: result.valid,
            errors: result.errors,
        })
    }
}
