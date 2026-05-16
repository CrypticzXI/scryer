use super::*;
use std::future::Future;

pub(crate) const INHERIT_QUALITY_PROFILE_VALUE: &str = "__inherit__";
pub(crate) const NATIVE_DOWNLOAD_CLIENT_TYPES: [&str; 4] =
    ["nzbget", "sabnzbd", "qbittorrent", "weaver"];
pub(crate) const INDEXER_PROVIDER_NZBGEEK: &str = "nzbgeek";

pub(crate) fn parsed_episode_lookup_season(
    ep_meta: &ParsedEpisodeMetadata,
    default_season: &str,
) -> String {
    if ep_meta.season == Some(0) {
        "0".to_string()
    } else {
        default_season.to_string()
    }
}

/// Return the accepted input kinds for a download client type, checking
/// the plugin provider first (WASM plugins), then falling back to known
/// native client capabilities.
///
/// An empty vec means the client has not declared any capabilities and
/// will not receive any downloads.
pub fn accepted_inputs_for_client(
    client_type: &str,
    plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
) -> Vec<DownloadSourceKind> {
    if let Some(provider) = plugin_provider {
        let inputs = provider.accepted_inputs_for_provider(client_type);
        if !inputs.is_empty() {
            return inputs
                .iter()
                .filter_map(|s| DownloadSourceKind::parse(s))
                .collect();
        }
    }
    native_accepted_inputs(client_type)
}

/// Native client capabilities. Returns the accepted input kinds for
/// built-in download client types.
fn native_accepted_inputs(client_type: &str) -> Vec<DownloadSourceKind> {
    match client_type {
        "nzbget" | "sabnzbd" | "weaver" => vec![DownloadSourceKind::NzbFile],
        "qbittorrent" => vec![
            DownloadSourceKind::TorrentFile,
            DownloadSourceKind::MagnetUri,
        ],
        _ => vec![],
    }
}

/// Lower the calling thread's scheduling priority via `nice(10)`.
///
/// Call this at the top of CPU-heavy `spawn_blocking` closures (AVIF encoding,
/// alass alignment, audio decoding) so they don't starve the async runtime.
/// Safe to call on any Unix platform; silently ignored on Windows.
#[cfg(unix)]
pub fn nice_thread() {
    unsafe {
        libc::nice(10);
    }
}

#[cfg(not(unix))]
pub fn nice_thread() {}

pub(crate) fn normalize_release_attempt_hint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn normalize_release_attempt_title(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

pub(crate) fn normalize_release_password(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
        .map(str::to_string)
}

pub(crate) fn is_obfuscated_release_name(parsed: &ParsedReleaseMetadata) -> bool {
    if parsed
        .release_group
        .as_ref()
        .is_some_and(|group| !group.trim().is_empty())
    {
        return false;
    }

    parsed
        .raw_title
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 8)
        .any(|token| {
            let has_alpha = token.chars().any(|ch| ch.is_ascii_alphabetic());
            let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
            let hex_like = token.chars().all(|ch| ch.is_ascii_hexdigit());
            (has_alpha && has_digit) || hex_like
        })
}

pub(crate) fn has_usable_release_title_signal(parsed: &ParsedReleaseMetadata) -> bool {
    let normalized_title = parsed.normalized_title.trim();
    if normalized_title.is_empty() {
        return false;
    }

    if matches!(
        normalized_title.to_ascii_uppercase().as_str(),
        "MOVIE" | "VIDEO" | "FILE" | "DOWNLOAD" | "UNKNOWN"
    ) {
        return false;
    }

    !is_obfuscated_release_name(parsed)
}

pub(crate) fn parse_usable_release_title(raw: &str) -> Option<ParsedReleaseMetadata> {
    let parsed = parse_release_metadata(raw);
    has_usable_release_title_signal(&parsed).then_some(parsed)
}

pub(crate) fn normalize_release_selection_signature(
    source_hint: Option<&str>,
    source_title: Option<&str>,
    source_kind: Option<DownloadSourceKind>,
) -> Option<String> {
    let source_hint = source_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let source_title = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let source_kind = source_kind.map(|value| value.as_str().to_string());

    if source_hint.is_none() && source_title.is_none() && source_kind.is_none() {
        return None;
    }

    Some(format!(
        "{}|{}|{}",
        source_kind.unwrap_or_default(),
        source_hint.unwrap_or_default(),
        source_title.unwrap_or_default()
    ))
}

pub(crate) fn sha256_hex(input: impl AsRef<str>) -> String {
    let hash = aws_lc_digest::digest(&aws_lc_digest::SHA256, input.as_ref().as_bytes());
    to_hex(hash.as_ref())
}

pub(crate) fn to_hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(unix)]
pub(crate) fn statvfs_path(path: &str) -> Option<libc::statvfs> {
    use std::ffi::CString;
    let c_path = CString::new(path).ok()?;
    unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut buf) == 0 {
            Some(buf)
        } else {
            None
        }
    }
}

fn normalize_tag(raw: String) -> String {
    raw.trim().to_lowercase()
}

fn normalize_show_text(raw: String) -> Option<String> {
    let value = raw.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

pub(crate) fn normalize_show_text_opt(raw: Option<String>) -> Option<String> {
    raw.and_then(normalize_show_text)
}

pub(crate) fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in raw {
        let normalized = normalize_tag(value.clone());
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

pub(crate) fn sanitize_ids(ids: Vec<ExternalId>) -> Vec<ExternalId> {
    ids.into_iter()
        .filter_map(|id| {
            let source = id.source.trim().to_lowercase();
            let value = id.value.trim().to_string();
            if source.is_empty() || value.is_empty() {
                None
            } else {
                Some(ExternalId { source, value })
            }
        })
        .collect()
}

pub(crate) async fn await_cancellable<T, F>(
    cancel_token: Option<&tokio_util::sync::CancellationToken>,
    future: F,
) -> Option<T>
where
    F: Future<Output = T>,
{
    let Some(token) = cancel_token else {
        return Some(future.await);
    };

    tokio::pin!(future);
    tokio::select! {
        _ = token.cancelled() => None,
        value = &mut future => Some(value),
    }
}

pub(crate) async fn await_cancellable_app_result<T, F>(
    cancel_token: Option<&tokio_util::sync::CancellationToken>,
    future: F,
) -> AppResult<Option<T>>
where
    F: Future<Output = AppResult<T>>,
{
    match await_cancellable(cancel_token, future).await {
        Some(result) => result.map(Some),
        None => Ok(None),
    }
}
