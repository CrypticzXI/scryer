use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use serde_json::Value as JsonValue;

use crate::{
    EncryptionKey, decrypt_value as decrypt_at_rest_value, encrypt_value as encrypt_at_rest_value,
    is_encrypted,
};

pub fn current_encryption_key(
    state: &Arc<RwLock<Option<EncryptionKey>>>,
) -> AppResult<Option<EncryptionKey>> {
    state
        .read()
        .map(|value| value.clone())
        .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))
}

pub fn maybe_encrypt_optional(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    encrypt_optional_value(key, value, "config_json", false)
}

pub fn maybe_encrypt_value(key: Option<&EncryptionKey>, value: &str) -> AppResult<String> {
    encrypt_value(key, value, "config_json", false)
}

pub fn encrypt_optional_value(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
    label: &str,
    require_key: bool,
) -> AppResult<Option<String>> {
    value
        .map(|value| encrypt_value(key, value, label, require_key))
        .transpose()
}

pub fn encrypt_value(
    key: Option<&EncryptionKey>,
    value: &str,
    label: &str,
    require_key: bool,
) -> AppResult<String> {
    let Some(key) = key else {
        if require_key {
            return Err(AppError::Repository(format!(
                "{label} encryption requires encryption key"
            )));
        }
        return Ok(value.to_string());
    };
    encrypt_at_rest_value(key, value)
        .map_err(|error| AppError::Repository(format!("failed to encrypt {label}: {error}")))
}

pub fn decrypt_optional_value(
    key: Option<&EncryptionKey>,
    value: Option<String>,
    label: &str,
    require_key: bool,
) -> AppResult<Option<String>> {
    value
        .map(|value| decrypt_value(key, value, label, require_key))
        .transpose()
}

pub fn decrypt_value(
    key: Option<&EncryptionKey>,
    value: String,
    label: &str,
    require_key: bool,
) -> AppResult<String> {
    if !is_encrypted(&value) {
        return Ok(value);
    }

    let Some(key) = key else {
        if require_key {
            return Err(AppError::Repository(format!(
                "encrypted {label} requires encryption key"
            )));
        }
        return Ok(value);
    };

    decrypt_at_rest_value(key, &value)
        .map_err(|error| AppError::Repository(format!("failed to decrypt {label}: {error}")))
}

pub fn enabled_facets_from_json(value: JsonValue) -> AppResult<Vec<String>> {
    let JsonValue::Array(values) = value else {
        return Err(AppError::Repository(
            "enabled_facets must be a JSON array".to_string(),
        ));
    };

    Ok(values
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect())
}
