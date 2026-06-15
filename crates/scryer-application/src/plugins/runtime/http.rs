static PLUGIN_HTTP_RATE_LIMITS: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::new);
static DEFAULT_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(build_plugin_http_client);
#[cfg(test)]
static RULE_PACK_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(build_plugin_http_client);
#[derive(Clone, Copy)]
enum PluginHttpClientProfile {
    DefaultFetch,
    #[cfg(test)]
    RulePackFetch,
}
fn build_plugin_http_client() -> Result<OutboundHttpClient, String> {
    Ok(OutboundHttpClient::new(
        no_redirect_reqwest_client(),
        PLUGIN_HTTP_RATE_LIMITS.clone(),
    ))
}
fn plugin_http_client(profile: PluginHttpClientProfile) -> AppResult<&'static OutboundHttpClient> {
    let cached = match profile {
        PluginHttpClientProfile::DefaultFetch => &*DEFAULT_PLUGIN_HTTP_CLIENT,
        #[cfg(test)]
        PluginHttpClientProfile::RulePackFetch => &*RULE_PACK_PLUGIN_HTTP_CLIENT,
    };

    cached
        .as_ref()
        .map_err(|error| AppError::Repository(error.clone()))
}
fn map_plugin_outbound_error(label: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
            match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                Some(delay) => format!(
                    "failed to download {label}: rate limited, retry after {}s",
                    delay.as_secs()
                ),
                None => format!("failed to download {label}: rate limited"),
            },
        ),
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("failed to download {label}: {source}"))
        }
    }
}
async fn fetch_plugin_bytes(
    url: &str,
    label: &str,
    scope: impl Into<String>,
) -> AppResult<Vec<u8>> {
    let target = scryer_outbound_http::prepare_untrusted_public_http_target(url, "plugin artifact")
        .await
        .map_err(|error| AppError::Validation(error.to_string()))?;
    let outbound_http = plugin_http_client(PluginHttpClientProfile::DefaultFetch)?;
    let response = outbound_http
        .send(plugin_request_policy(scope, label), || {
            target.client().get(target.url().clone())
        })
        .await
        .map_err(|error| map_plugin_outbound_error(label, error))?;
    if response.status().is_redirection() {
        return Err(AppError::Validation(format!(
            "plugin artifact redirects are not allowed for {label}"
        )));
    }
    let response = response
        .error_for_status()
        .map_err(|error| AppError::Repository(format!("failed to download {label}: {error}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
    Ok(bytes.to_vec())
}
