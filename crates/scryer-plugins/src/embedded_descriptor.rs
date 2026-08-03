use wasmparser::{Parser, Payload};

use crate::types::PluginDescriptor;

pub(crate) const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1: &str = "scryer.plugin-descriptor.v1";
const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_PREFIX: &str = "scryer.plugin-descriptor.";
const PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct EmbeddedDescriptorModule {
    pub(crate) descriptor: PluginDescriptor,
}

pub(crate) fn embedded_descriptor_from_wasm(
    wasm: &[u8],
) -> Result<Option<EmbeddedDescriptorModule>, String> {
    let mut descriptor_payload = None;
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(section) =
            payload.map_err(|error| format!("failed to parse plugin WASM: {error}"))?
        {
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
    }

    let Some(payload) = descriptor_payload else {
        return Ok(None);
    };
    if payload.len() > PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES {
        return Err(format!(
            "embedded plugin descriptor exceeds the {} byte limit",
            PLUGIN_DESCRIPTOR_CUSTOM_SECTION_MAX_BYTES
        ));
    }
    let descriptor = serde_json::from_slice(payload)
        .map_err(|error| format!("embedded plugin descriptor is invalid JSON: {error}"))?;
    Ok(Some(EmbeddedDescriptorModule { descriptor }))
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
    fn extracts_descriptor() {
        let wasm = append_custom_section(
            indexer_module(),
            PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1,
            descriptor_json(),
        );
        let embedded = embedded_descriptor_from_wasm(&wasm)
            .unwrap()
            .expect("descriptor should be embedded");
        assert_eq!(embedded.descriptor.id, "newznab");
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
}
