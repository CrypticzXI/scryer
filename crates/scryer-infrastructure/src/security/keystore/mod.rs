//! Platform-native keystore backends for the encryption master key.
//!
//! Each platform compiles only its own backend — no dead code from other platforms.
//! The priority chain in [`platform_keystores`] returns backends in descending
//! priority order; callers iterate and use the first one that returns a key.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::PathBuf;

const DISABLE_PLATFORM_KEYSTORE_ENV: &str = "SCRYER_DISABLE_PLATFORM_KEYSTORE";

/// A backend that can store and retrieve the encryption master key.
pub trait KeyStore: Send + Sync {
    /// Retrieve the base64-encoded encryption key, if stored.
    fn get_key(&self) -> Result<Option<String>, String>;

    /// Store the base64-encoded encryption key.
    fn set_key(&self, key_base64: &str) -> Result<(), String>;

    /// Delete the stored key.
    fn delete_key(&self) -> Result<(), String>;

    /// Human-readable name for log messages (e.g. "macOS Keychain").
    fn name(&self) -> &'static str;
}

/// Returns platform-native keystores in priority order.
///
/// `data_dir` is the application data directory (resolved by the binary crate)
/// and is used by the Linux `KeyFile` backend.
#[allow(clippy::vec_init_then_push)] // conditional cfg pushes can't use vec![]
pub fn platform_keystores(_data_dir: Option<PathBuf>) -> Vec<Box<dyn KeyStore>> {
    if platform_keystore_disabled() {
        return Vec::new();
    }

    let mut stores: Vec<Box<dyn KeyStore>> = Vec::new();

    #[cfg(target_os = "macos")]
    stores.push(Box::new(macos::MacOSKeychain));

    #[cfg(target_os = "windows")]
    stores.push(Box::new(windows::WindowsCredentialManager));

    #[cfg(target_os = "linux")]
    {
        stores.push(Box::new(linux::DockerSecret));
        if let Some(dir) = _data_dir {
            stores.push(Box::new(linux::KeyFile::new(dir)));
        }
    }

    stores
}

fn platform_keystore_disabled() -> bool {
    std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn platform_keystore_flag_defaults_to_enabled() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).ok();
        unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) };

        assert!(!platform_keystore_disabled());

        match original {
            Some(value) => unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, value) },
            None => unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) },
        }
    }

    #[test]
    fn platform_keystore_flag_disables_all_stores() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).ok();
        unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, "1") };

        assert!(platform_keystore_disabled());
        assert!(platform_keystores(None).is_empty());

        match original {
            Some(value) => unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, value) },
            None => unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) },
        }
    }
}
