use std::env;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::path::PathBuf;

use scryer_application::application_upgrade::{
    InstallationAssessment, InstallationEvidence, InstallationOs, classify_installation,
};

#[cfg(windows)]
const SCRYER_REGISTRY_KEY: &str = "Software\\Scryer Media\\Scryer";
static WRITABILITY_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn collect_installation_assessment() -> InstallationAssessment {
    let executable_path = env::current_exe().ok();
    let (windows_distribution_owner, windows_legacy_msi_registry_key_exists) =
        windows_registry_evidence();

    let evidence = InstallationEvidence {
        disable_self_upgrade: env::var("SCRYER_DISABLE_SELF_UPGRADE").ok(),
        package: env::var("SCRYER_PACKAGE").ok(),
        executable_dir_writable: executable_dir_writable(executable_path.as_deref()),
        docker_env_present: Path::new("/.dockerenv").exists(),
        os: current_os(),
        windows_session_zero: windows_session_zero(),
        windows_executable_under_program_files: executable_under_program_files(
            executable_path.as_deref(),
        ),
        windows_distribution_owner,
        windows_legacy_msi_registry_key_exists,
        tray_supervised: env::var("SCRYER_TRAY_SUPERVISED")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true")),
        executable_path,
    };

    classify_installation(&evidence)
}

fn current_os() -> InstallationOs {
    match env::consts::OS {
        "windows" => InstallationOs::Windows,
        "macos" => InstallationOs::Macos,
        "linux" => InstallationOs::Linux,
        _ => InstallationOs::Other,
    }
}

fn executable_dir_writable(executable_path: Option<&Path>) -> bool {
    let Some(directory) = executable_path.and_then(Path::parent) else {
        return false;
    };
    let unique_suffix = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos()),
        WRITABILITY_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let probe_path = directory.join(format!(".scryer-write-probe-{unique_suffix}"));

    let created = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe_path)
        .is_ok();
    if !created {
        return false;
    }

    fs::remove_file(probe_path).is_ok()
}

#[cfg(windows)]
fn windows_session_zero() -> bool {
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, ProcessIdToSessionId};

    let mut session_id = u32::MAX;
    // SAFETY: The process ID is valid and `session_id` points to writable memory.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) != 0 && session_id == 0 }
}

#[cfg(not(windows))]
fn windows_session_zero() -> bool {
    false
}

#[cfg(windows)]
fn windows_registry_evidence() -> (Option<String>, bool) {
    use std::ptr;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
    };

    let mut key: HKEY = ptr::null_mut();
    let key_path = wide(SCRYER_REGISTRY_KEY);
    // SAFETY: The registry path is nul-terminated and `key` points to writable memory.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return (None, false);
    }

    let owner = registry_string_value(key, "DistributionOwner", REG_SZ);
    // SAFETY: This function owns the registry key returned by `RegOpenKeyExW`.
    unsafe { RegCloseKey(key) };
    (owner, true)
}

#[cfg(not(windows))]
fn windows_registry_evidence() -> (Option<String>, bool) {
    (None, false)
}

#[cfg(windows)]
fn registry_string_value(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
    expected_type: u32,
) -> Option<String> {
    use std::ptr;
    use windows_sys::Win32::System::Registry::RegQueryValueExW;

    let name = wide(name);
    let mut value_type = 0_u32;
    let mut byte_len = 0_u32;
    // SAFETY: The key and value name are valid; output pointers are writable.
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status != 0 || value_type != expected_type || byte_len == 0 {
        return None;
    }

    let mut value = vec![0_u16; (byte_len as usize).div_ceil(2)];
    // SAFETY: The buffer is allocated for the reported byte length and all pointers are valid.
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            value.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != 0 || value_type != expected_type {
        return None;
    }

    let terminator = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    Some(String::from_utf16_lossy(&value[..terminator]))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn executable_under_program_files(executable_path: Option<&Path>) -> bool {
    let Some(executable_path) = executable_path else {
        return false;
    };
    let executable = executable_path.to_string_lossy().to_ascii_lowercase();

    ["ProgramFiles", "ProgramW6432"].into_iter().any(|name| {
        env::var_os(name).is_some_and(|program_files| {
            let program_files = PathBuf::from(program_files);
            let program_files = program_files.to_string_lossy().to_ascii_lowercase();
            let program_files = program_files.trim_end_matches(['\\', '/']);
            executable == program_files
                || executable
                    .strip_prefix(program_files)
                    .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
        })
    })
}

#[cfg(not(windows))]
fn executable_under_program_files(_executable_path: Option<&Path>) -> bool {
    false
}
