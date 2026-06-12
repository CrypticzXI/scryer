#[cfg(feature = "image-processing")]
pub(crate) mod processor;
pub(crate) mod title_image_store;

use scryer_application::TitleImageKind;

pub(crate) fn normalized_base_path_from_env() -> String {
    let Some(raw) = std::env::var("SCRYER_BASE_PATH").ok() else {
        return String::new();
    };

    let segments = raw
        .trim()
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    }
}

pub(crate) fn synthesize_local_title_image_url(
    base_path: &str,
    title_id: &str,
    kind: TitleImageKind,
    variant_key: &str,
    version_hash: &str,
) -> String {
    let version_value = version_hash
        .split_once(':')
        .map(|(_, digest)| digest)
        .unwrap_or(version_hash);
    let version = version_value.chars().take(16).collect::<String>();
    format!(
        "{base_path}/images/titles/{title_id}/{}/{variant_key}?v={version}",
        kind.as_str()
    )
}

pub(crate) fn content_type_for_format(format: String) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "webp" => "image/webp".to_string(),
        "avif" => "image/avif".to_string(),
        other => format!("image/{other}"),
    }
}
