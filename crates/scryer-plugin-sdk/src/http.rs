//! Guest-side HTTP helpers for Scryer plugins.
//!
//! This module preserves the existing Extism host HTTP ABI so existing plugins
//! and newer SDK consumers can share the same runtime behavior.

pub use extism_manifest::HttpRequest;

#[cfg(target_arch = "wasm32")]
pub use extism_pdk::http::{HttpResponse, request};
