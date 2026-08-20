//! Detection for the explicit native-command artifact marker.

use wasmparser::{Parser, Payload};

pub(crate) const COMMAND_ABI_CUSTOM_SECTION: &str = "scryer.plugin.command_abi";
pub(crate) const COMMAND_ABI_VERSION: u16 = 1;

/// Return the declared native command ABI version, if present.
///
/// A marker is deliberately authoritative: a malformed or unsupported marker
/// rejects the artifact rather than letting it fall through to legacy Extism
/// validation.
pub(crate) fn command_abi_version(wasm: &[u8]) -> Result<Option<u16>, String> {
    let mut marker = None;
    for payload in Parser::new(0).parse_all(wasm) {
        let Payload::CustomSection(section) =
            payload.map_err(|error| format!("failed to parse plugin WASM: {error}"))?
        else {
            continue;
        };
        if section.name() != COMMAND_ABI_CUSTOM_SECTION {
            continue;
        }
        if marker.replace(section.data()).is_some() {
            return Err(format!(
                "plugin contains duplicate '{COMMAND_ABI_CUSTOM_SECTION}' custom sections"
            ));
        }
    }

    let Some(marker) = marker else {
        return Ok(None);
    };
    let bytes: [u8; 2] = marker.try_into().map_err(|_| {
        "plugin command ABI marker must contain exactly a two-byte little-endian version"
            .to_string()
    })?;
    let version = u16::from_le_bytes(bytes);
    if version != COMMAND_ABI_VERSION {
        return Err(format!("unsupported plugin command ABI version {version}"));
    }
    Ok(Some(version))
}

/// Synthesize marked artifacts for tests in this crate.
///
/// Runtime selection is decided by the custom section, so any test that wants to
/// exercise a command-ABI path needs to be able to produce one; keeping the
/// encoder here means those tests agree with the reader they are testing.
#[cfg(test)]
pub(crate) mod test_support {
    use super::COMMAND_ABI_CUSTOM_SECTION;

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

    pub(crate) fn append_marker(mut wasm: Vec<u8>, marker: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        encode_u32_leb(COMMAND_ABI_CUSTOM_SECTION.len() as u32, &mut body);
        body.extend_from_slice(COMMAND_ABI_CUSTOM_SECTION.as_bytes());
        body.extend_from_slice(marker);
        wasm.push(0);
        encode_u32_leb(body.len() as u32, &mut wasm);
        wasm.extend_from_slice(&body);
        wasm
    }

    /// A minimal module carrying the current command marker.
    pub(crate) fn command_marked_wasm() -> Vec<u8> {
        append_marker(
            wat::parse_str("(module)").expect("valid wat"),
            &super::COMMAND_ABI_VERSION.to_le_bytes(),
        )
    }

    /// A minimal module with no marker at all — a legacy reactor artifact.
    pub(crate) fn unmarked_wasm() -> Vec<u8> {
        wat::parse_str("(module)").expect("valid wat")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::append_marker;
    use super::*;

    #[test]
    fn accepts_current_command_marker() {
        let wasm = append_marker(wat::parse_str("(module)").unwrap(), &1_u16.to_le_bytes());
        assert_eq!(command_abi_version(&wasm).unwrap(), Some(1));
    }

    #[test]
    fn rejects_malformed_or_unknown_marker() {
        let malformed = append_marker(wat::parse_str("(module)").unwrap(), &[1]);
        assert!(command_abi_version(&malformed).is_err());
        let unknown = append_marker(wat::parse_str("(module)").unwrap(), &2_u16.to_le_bytes());
        assert!(command_abi_version(&unknown).is_err());
    }
}
