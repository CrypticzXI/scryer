#[cfg(feature = "image-processing")]
pub(crate) mod processor;
pub(crate) mod title_image_store;

use scryer_application::{TitleImageKind, TitleImageStorageMode, TitleImageVariantRecord};

fn preferred_local_route_key_for_kind(kind: TitleImageKind) -> &'static str {
    match kind {
        TitleImageKind::Poster => "w500",
        TitleImageKind::Banner | TitleImageKind::Fanart => "master",
    }
}

pub(crate) fn required_persisted_variant_for_kind(kind: TitleImageKind) -> Option<&'static str> {
    match kind {
        TitleImageKind::Poster => Some("w500"),
        TitleImageKind::Banner | TitleImageKind::Fanart => None,
    }
}

pub(crate) fn materialize_local_title_image_path(
    title_id: &str,
    kind: TitleImageKind,
    storage_mode: TitleImageStorageMode,
    master_sha256: &str,
    variants: &[TitleImageVariantRecord],
) -> String {
    let (variant_key, version_hash) = match storage_mode {
        TitleImageStorageMode::Original => ("original", master_sha256),
        TitleImageStorageMode::AvifMaster => match kind {
            TitleImageKind::Poster => {
                let preferred_variant = preferred_local_route_key_for_kind(kind);
                if let Some(variant) = variants
                    .iter()
                    .find(|variant| variant.variant_key == preferred_variant)
                {
                    (preferred_variant, variant.sha256.as_str())
                } else {
                    ("original", master_sha256)
                }
            }
            TitleImageKind::Banner | TitleImageKind::Fanart => {
                (preferred_local_route_key_for_kind(kind), master_sha256)
            }
        },
    };

    synthesize_local_title_image_url("", title_id, kind, variant_key, version_hash)
}

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
    let version = version_hash.chars().take(16).collect::<String>();
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
