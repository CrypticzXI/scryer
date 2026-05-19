use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

const STORED_PATH_PREFIX: &str = "scryer-path-v1:";
const STORED_PATH_UNIX_PREFIX: &str = "scryer-path-v1:u:";
const STORED_PATH_WINDOWS_PREFIX: &str = "scryer-path-v1:w:";

pub fn path_to_stored_string(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    if let Some(value) = path.to_str()
        && !value.starts_with(STORED_PATH_PREFIX)
    {
        return value.to_string();
    }

    encode_path(path)
}

pub fn stored_path_to_path_buf(stored: &str) -> PathBuf {
    decode_path(stored).unwrap_or_else(|| PathBuf::from(stored))
}

pub fn stored_path_to_display_string(stored: &str) -> String {
    if !stored.starts_with(STORED_PATH_PREFIX) {
        return stored.to_string();
    }

    stored_path_to_path_buf(stored)
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
fn encode_path(path: &Path) -> String {
    encode_percent_bytes(path.as_os_str().as_bytes(), STORED_PATH_UNIX_PREFIX)
}

#[cfg(windows)]
fn encode_path(path: &Path) -> String {
    let mut encoded = String::from(STORED_PATH_WINDOWS_PREFIX);
    for unit in path.as_os_str().encode_wide() {
        if is_safe_ascii(unit) {
            encoded.push(char::from_u32(unit as u32).unwrap_or_default());
        } else {
            encoded.push_str(&format!("%u{unit:04X}"));
        }
    }
    encoded
}

#[cfg(not(any(unix, windows)))]
fn encode_path(path: &Path) -> String {
    let mut encoded = String::from(STORED_PATH_UNIX_PREFIX);
    encoded.push_str(&path.to_string_lossy());
    encoded
}

fn decode_path(stored: &str) -> Option<PathBuf> {
    if let Some(encoded) = stored.strip_prefix(STORED_PATH_UNIX_PREFIX) {
        return decode_unix_path(encoded);
    }

    if let Some(encoded) = stored.strip_prefix(STORED_PATH_WINDOWS_PREFIX) {
        return decode_windows_path(encoded);
    }

    None
}

fn decode_unix_path(encoded: &str) -> Option<PathBuf> {
    let bytes = decode_percent_bytes(encoded)?;

    #[cfg(unix)]
    {
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }

    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

fn decode_windows_path(encoded: &str) -> Option<PathBuf> {
    let units = decode_windows_units(encoded)?;

    #[cfg(windows)]
    {
        Some(PathBuf::from(std::ffi::OsString::from_wide(&units)))
    }

    #[cfg(not(windows))]
    {
        let lossy = String::from_utf16_lossy(&units).replace('\\', "/");
        Some(PathBuf::from(lossy))
    }
}

fn encode_percent_bytes(bytes: &[u8], prefix: &str) -> String {
    let mut encoded = String::from(prefix);
    for &byte in bytes {
        if is_safe_ascii(byte as u16) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn decode_percent_bytes(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                decoded.push((hex_value(high)? << 4) | hex_value(low)?);
                index += 3;
            }
            byte if byte.is_ascii() => {
                decoded.push(byte);
                index += 1;
            }
            _ => return None,
        }
    }

    Some(decoded)
}

fn decode_windows_units(encoded: &str) -> Option<Vec<u16>> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes.get(index + 1).copied()? != b'u' {
                return None;
            }

            let h0 = u16::from(hex_value(*bytes.get(index + 2)?)?);
            let h1 = u16::from(hex_value(*bytes.get(index + 3)?)?);
            let h2 = u16::from(hex_value(*bytes.get(index + 4)?)?);
            let h3 = u16::from(hex_value(*bytes.get(index + 5)?)?);
            decoded.push((h0 << 12) | (h1 << 8) | (h2 << 4) | h3);
            index += 6;
            continue;
        }

        let byte = *bytes.get(index)?;
        if !byte.is_ascii() {
            return None;
        }
        decoded.push(u16::from(byte));
        index += 1;
    }

    Some(decoded)
}

fn is_safe_ascii(value: u16) -> bool {
    matches!(value, 0x20..=0x7E) && value != u16::from(b'%')
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_paths_stay_plain() {
        let path = Path::new("/library/Movie (2024)/Movie.mkv");
        assert_eq!(
            path_to_stored_string(path),
            "/library/Movie (2024)/Movie.mkv"
        );
    }

    #[test]
    fn reserved_prefix_round_trips() {
        let path = Path::new("scryer-path-v1:/library/Movie.mkv");
        let stored = path_to_stored_string(path);

        assert_ne!(stored, "scryer-path-v1:/library/Movie.mkv");
        assert_eq!(stored_path_to_path_buf(&stored), path);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_paths_round_trip() {
        let bytes = b"/library/\xFFmovie.mkv".to_vec();
        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes.clone()));
        let stored = path_to_stored_string(&path);

        assert!(stored.starts_with(STORED_PATH_UNIX_PREFIX));
        assert_eq!(stored_path_to_path_buf(&stored), path);
        assert_eq!(
            stored_path_to_display_string(&stored),
            path.to_string_lossy().into_owned()
        );
    }

    #[cfg(unix)]
    #[test]
    fn windows_paths_decode_lossily_on_unix() {
        let stored = "scryer-path-v1:w:C:\\Media\\%uD800.mkv";
        let decoded = stored_path_to_path_buf(stored);

        assert_eq!(decoded, PathBuf::from("C:/Media/\u{FFFD}.mkv"));
        assert_eq!(
            decoded
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("\u{FFFD}.mkv".to_string())
        );
        assert_eq!(
            stored_path_to_display_string(stored),
            "C:/Media/\u{FFFD}.mkv"
        );
    }

    #[cfg(windows)]
    #[test]
    fn non_utf8_windows_paths_round_trip() {
        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            u16::from(b'C'),
            u16::from(b':'),
            u16::from(b'\\'),
            0xD800,
            u16::from(b'.'),
            u16::from(b'm'),
            u16::from(b'k'),
            u16::from(b'v'),
        ]));
        let stored = path_to_stored_string(&path);

        assert!(stored.starts_with(STORED_PATH_WINDOWS_PREFIX));
        assert_eq!(stored_path_to_path_buf(&stored), path);
    }

    #[cfg(windows)]
    #[test]
    fn unix_paths_decode_lossily_on_windows() {
        let stored = "scryer-path-v1:u:/library/%FFmovie.mkv";
        let decoded = stored_path_to_path_buf(stored);
        let display = stored_path_to_display_string(stored);

        assert_eq!(
            decoded
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("\u{FFFD}movie.mkv".to_string())
        );
        assert!(display.contains("\u{FFFD}movie.mkv"));
        assert!(!display.starts_with(STORED_PATH_PREFIX));
    }
}
