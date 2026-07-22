use std::collections::HashMap;

use wasmparser::{ExternalKind, Parser, Payload, Validator};

use crate::types::PluginDescriptor;

pub(crate) const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1: &str = "scryer.plugin-descriptor.v1";
const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_PREFIX: &str = "scryer.plugin-descriptor.";
const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct EmbeddedDescriptorModule {
    pub(crate) descriptor: PluginDescriptor,
    exports: HashMap<String, ExternalKind>,
}

impl EmbeddedDescriptorModule {
    pub(crate) fn missing_function_exports<'a>(
        &self,
        required: impl IntoIterator<Item = &'a str>,
    ) -> Vec<&'a str> {
        required
            .into_iter()
            .filter(|name| self.exports.get(*name) != Some(&ExternalKind::Func))
            .collect()
    }

    pub(crate) fn require_command_exports(&self) -> Result<(), String> {
        if self.exports.get("_start") != Some(&ExternalKind::Func) {
            return Err("command plugin must export a function named '_start'".to_string());
        }
        if self.exports.get("memory") != Some(&ExternalKind::Memory) {
            return Err("command plugin must export a linear memory named 'memory'".to_string());
        }
        Ok(())
    }
}

pub(crate) fn embedded_descriptor_from_wasm(
    wasm: &[u8],
) -> Result<Option<EmbeddedDescriptorModule>, String> {
    Validator::new()
        .validate_all(wasm)
        .map_err(|error| format!("embedded-descriptor WASM validation failed: {error}"))?;

    let mut descriptor_payload = None;
    let mut exports = HashMap::new();
    let mut unsupported_import = None;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(|error| format!("failed to parse plugin WASM: {error}"))? {
            Payload::CustomSection(section) => {
                if section.name() == PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1 {
                    if descriptor_payload.replace(section.data()).is_some() {
                        return Err(format!(
                            "plugin contains duplicate '{PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1}' custom sections"
                        ));
                    }
                } else if section
                    .name()
                    .starts_with(PLUGIN_DESCRIPTOR_CUSTOM_SECTION_PREFIX)
                {
                    return Err(format!(
                        "plugin uses unsupported embedded descriptor section '{}'",
                        section.name()
                    ));
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export
                        .map_err(|error| format!("failed to parse plugin export: {error}"))?;
                    exports.insert(export.name.to_string(), export.kind);
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    let import = import
                        .map_err(|error| format!("failed to parse plugin import: {error}"))?;
                    if !matches!(
                        import.module,
                        "wasi_snapshot_preview1"
                            | "extism:host/env"
                            | "extism:host/user"
                            | "scryer:host/http"
                    ) && unsupported_import.is_none()
                    {
                        unsupported_import =
                            Some((import.module.to_string(), import.name.to_string()));
                    }
                }
            }
            _ => {}
        }
    }

    let Some(payload) = descriptor_payload else {
        return Ok(None);
    };
    if let Some((module, name)) = unsupported_import {
        return Err(format!(
            "plugin imports unsupported host module '{module}': {name}"
        ));
    }
    if payload.len() > PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES {
        return Err(format!(
            "embedded plugin descriptor exceeds the {} byte limit",
            PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES
        ));
    }
    let descriptor = serde_json::from_slice(payload)
        .map_err(|error| format!("embedded plugin descriptor is invalid JSON: {error}"))?;
    Ok(Some(EmbeddedDescriptorModule {
        descriptor,
        exports,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_u32_leb(mut value: u32, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn append_custom_section(mut wasm: Vec<u8>, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        encode_u32_leb(name.len() as u32, &mut body);
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(payload);
        wasm.push(0);
        encode_u32_leb(body.len() as u32, &mut wasm);
        wasm.extend_from_slice(&body);
        wasm
    }

    fn indexer_module() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "scryer_describe"))
                (func (export "scryer_indexer_search")))"#,
        )
        .unwrap()
    }

    fn descriptor_json() -> &'static [u8] {
        include_bytes!("../builtins/newznab_indexer.descriptor.json")
    }

    #[test]
    fn extracts_descriptor_and_static_exports() {
        let wasm = append_custom_section(
            indexer_module(),
            PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1,
            descriptor_json(),
        );
        let embedded = embedded_descriptor_from_wasm(&wasm)
            .unwrap()
            .expect("descriptor should be embedded");
        assert_eq!(embedded.descriptor.id, "newznab");
        assert!(
            embedded
                .missing_function_exports(["scryer_describe", "scryer_indexer_search"])
                .is_empty()
        );
    }

    #[test]
    fn sectionless_module_uses_legacy_fallback() {
        assert!(
            embedded_descriptor_from_wasm(&indexer_module())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn sectionless_module_does_not_apply_embedded_metadata_import_policy() {
        let wasm = wat::parse_str(
            r#"(module
                (import "legacy:host" "call" (func))
                (memory (export "memory") 1)
                (func (export "scryer_describe"))
                (func (export "scryer_indexer_search")))"#,
        )
        .unwrap();

        assert!(embedded_descriptor_from_wasm(&wasm).unwrap().is_none());
    }

    #[test]
    fn duplicate_descriptor_section_is_rejected() {
        let wasm = append_custom_section(
            append_custom_section(
                indexer_module(),
                PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1,
                descriptor_json(),
            ),
            PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1,
            descriptor_json(),
        );
        let error = embedded_descriptor_from_wasm(&wasm).unwrap_err();
        assert!(error.contains("duplicate"));
    }

    #[test]
    fn unsupported_descriptor_section_version_is_rejected() {
        let wasm = append_custom_section(
            indexer_module(),
            "scryer.plugin-descriptor.v2",
            descriptor_json(),
        );
        let error = embedded_descriptor_from_wasm(&wasm).unwrap_err();
        assert!(error.contains("unsupported embedded descriptor section"));
    }

    #[test]
    fn unsupported_import_module_is_rejected() {
        let wasm = wat::parse_str(
            r#"(module
                (import "unexpected:host" "call" (func))
                (memory (export "memory") 1)
                (func (export "scryer_describe"))
                (func (export "scryer_indexer_search")))"#,
        )
        .unwrap();
        let wasm =
            append_custom_section(wasm, PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1, descriptor_json());
        let error = embedded_descriptor_from_wasm(&wasm).unwrap_err();
        assert!(error.contains("unsupported host module"));
    }
}
