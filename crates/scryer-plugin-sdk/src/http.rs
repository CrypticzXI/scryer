//! Guest-side HTTP helpers for Scryer plugins.
//!
//! This module preserves the existing Extism host HTTP ABI so existing plugins
//! and newer SDK consumers can share the same runtime behavior.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct HttpRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::collections::HashMap;

    use extism_pdk::{Error, Memory, MemoryHandle, ToMemory};

    use super::HttpRequest;

    #[link(wasm_import_module = "scryer:host/http")]
    unsafe extern "C" {
        fn scryer_http_request(request_offset: i64, body_offset: i64) -> i64;
        fn scryer_http_status_code() -> i32;
        fn scryer_http_headers() -> i64;
    }

    /// A host HTTP response returned by Scryer's plugin runtime.
    pub struct HttpResponse {
        memory: Memory,
        status: u16,
        headers: HashMap<String, String>,
    }

    impl HttpResponse {
        pub fn into_memory(self) -> Memory {
            self.memory
        }

        pub fn status_code(&self) -> u16 {
            self.status
        }

        pub fn as_memory(&self) -> &Memory {
            &self.memory
        }

        pub fn body(&self) -> Vec<u8> {
            self.memory.to_vec()
        }

        pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
            Ok(serde_json::from_slice(&self.body())?)
        }

        pub fn headers(&self) -> &HashMap<String, String> {
            &self.headers
        }

        pub fn header(&self, name: impl AsRef<str>) -> Option<&str> {
            self.headers.get(name.as_ref()).map(String::as_str)
        }
    }

    /// Execute an HTTP request through Scryer's host-owned plugin HTTP surface.
    pub fn request<T: ToMemory>(req: &HttpRequest, body: Option<T>) -> Result<HttpResponse, Error> {
        let request_bytes = serde_json::to_vec(req)?;
        let request_memory = Memory::from_bytes(request_bytes)?;
        let body_memory = match body {
            Some(body) => Some(body.to_memory()?),
            None => None,
        };
        let body_offset = body_memory
            .as_ref()
            .map(|memory| memory.offset())
            .unwrap_or(0);

        let response_offset =
            unsafe { scryer_http_request(request_memory.offset() as i64, body_offset as i64) };
        let status = unsafe { scryer_http_status_code() } as u16;
        let response_length =
            extism_pdk::memory::internal::memory_length_unsafe(response_offset as u64);
        let headers_offset = unsafe { scryer_http_headers() };
        let headers = read_headers(headers_offset)?;

        Ok(HttpResponse {
            memory: Memory(MemoryHandle {
                offset: response_offset as u64,
                length: response_length,
            }),
            status,
            headers,
        })
    }

    fn read_headers(offset: i64) -> Result<HashMap<String, String>, Error> {
        if offset == 0 {
            return Ok(HashMap::new());
        }
        let Some(memory) = Memory::find(offset as u64) else {
            return Ok(HashMap::new());
        };
        let headers = serde_json::from_slice(&memory.to_vec())?;
        memory.free();
        Ok(headers)
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{HttpResponse, request};

impl HttpRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: None,
            headers: BTreeMap::new(),
        }
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::HttpRequest;

    #[test]
    fn http_request_builder_serializes_to_legacy_shape() {
        let request = HttpRequest::new("https://indexer.example/api")
            .with_method("POST")
            .with_header("X-Test", "one");

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["url"], "https://indexer.example/api");
        assert_eq!(json["method"], "POST");
        assert_eq!(json["headers"]["X-Test"], "one");
    }
}
