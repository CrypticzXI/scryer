use std::collections::HashSet;
use std::env;
use std::ffi::{CString, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG_DIR: &str = "/config";
const DEFAULT_DB_PATH: &str = "/config/scryer.db";
const PORTABLE_PAYLOAD_NAME: &str = "scryer-portable";
const HASWELL_PAYLOAD_NAME: &str = "scryer-haswell";
const CORTEX_A76_PAYLOAD_NAME: &str = "scryer-cortex-a76";
const LAUNCHER_UID_DEFAULT: u32 = 1000;
const LAUNCHER_GID_DEFAULT: u32 = 1000;
const X86_REQUIRED_FEATURES: &[&str] = &[
    "avx",
    "avx2",
    "bmi1",
    "bmi2",
    "f16c",
    "fma",
    "lzcnt",
    "movbe",
    "pclmulqdq",
    "popcnt",
    "rdrand",
    "sse3",
    "sse4.1",
    "sse4.2",
    "ssse3",
    "xsave",
    "xsaveopt",
];
const ARM_REQUIRED_FEATURES: &[&str] = &[
    "aes", "crc32", "dotprod", "fp16", "lse", "neon", "rdm", "sha2",
];

fn main() {
    let config = LaunchConfig::from_env(env::args_os().skip(1).collect());
    let ops = RealLauncherOps;
    if let Err(error) = run_with_ops(&ops, &config) {
        eprintln!("failed to launch scryer: {error}");
        std::process::exit(1);
    }
    unreachable!("launcher exec unexpectedly returned without replacing the process");
}

#[derive(Clone, Debug)]
struct LaunchConfig {
    args: Vec<OsString>,
    cpuinfo_path: PathBuf,
    payload_root: PathBuf,
    db_path: PathBuf,
    container_arch_override: Option<String>,
    requested_uid: Option<String>,
    requested_gid: Option<String>,
    umask: Option<String>,
}

impl LaunchConfig {
    fn from_env(args: Vec<OsString>) -> Self {
        let db_path = resolved_db_path(env::var("SCRYER_DB_PATH").ok());
        Self {
            args,
            cpuinfo_path: env::var_os("SCRYER_CPUINFO_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/proc/cpuinfo")),
            payload_root: env::var_os("SCRYER_PAYLOAD_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/opt/scryer")),
            db_path,
            container_arch_override: env::var("SCRYER_CONTAINER_ARCH").ok(),
            requested_uid: env::var("PUID").ok(),
            requested_gid: env::var("PGID").ok(),
            umask: env::var("UMASK").ok(),
        }
    }

    fn launch_args(&self) -> Vec<OsString> {
        let mut args = Vec::with_capacity(self.args.len() + 2);
        args.push(OsString::from("--data-dir"));
        args.push(OsString::from(CONFIG_DIR));
        args.extend(self.args.iter().cloned());
        args
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Arch {
    Amd64,
    Arm64,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lane {
    Portable,
    Haswell,
    CortexA76,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeLaunch {
    primary: PathBuf,
    fallback: Option<PathBuf>,
    lane: Lane,
}

trait LauncherOps {
    fn effective_uid(&self) -> u32;
    fn effective_gid(&self) -> u32;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn path_is_dir(&self, path: &Path) -> bool;
    fn path_is_executable_file(&self, path: &Path) -> bool;
    fn chown_recursive(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()>;
    fn set_umask(&self, mask: u32) -> io::Result<()>;
    fn drop_privileges(&self, uid: u32, gid: u32) -> io::Result<()>;
    fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()>;
    fn warn(&self, message: &str);

    fn is_root(&self) -> bool {
        self.effective_uid() == 0
    }
}

struct RealLauncherOps;

impl LauncherOps for RealLauncherOps {
    fn effective_uid(&self) -> u32 {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() }
    }

    fn effective_gid(&self) -> u32 {
        // SAFETY: getegid has no preconditions.
        unsafe { libc::getegid() }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn path_is_executable_file(&self, path: &Path) -> bool {
        match fs::metadata(path) {
            Ok(metadata) => metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }

    fn chown_recursive(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
        chown_recursive(path, uid, gid)
    }

    fn set_umask(&self, mask: u32) -> io::Result<()> {
        // SAFETY: umask has no additional preconditions beyond a plain integer mode.
        unsafe {
            libc::umask(mask as libc::mode_t);
        }
        Ok(())
    }

    fn drop_privileges(&self, uid: u32, gid: u32) -> io::Result<()> {
        // SAFETY: setgid/setuid are process-global but valid here before exec.
        unsafe {
            if libc::setgid(gid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()> {
        let mut command = Command::new(program);
        command.args(args.iter());
        Err(command.exec())
    }

    fn warn(&self, message: &str) {
        eprintln!("warning: {message}");
    }
}

fn run_with_ops<O: LauncherOps>(ops: &O, config: &LaunchConfig) -> Result<(), String> {
    let runtime = resolve_runtime_launch(ops, config)?;

    apply_umask(ops, config);
    if ops.is_root() {
        let (uid, gid) = requested_identity(ops, config);
        repair_ownership(ops, config, uid, gid);
        if let Err(error) = ops.drop_privileges(uid, gid) {
            ops.warn(&format!(
                "failed to drop privileges to {uid}:{gid}: {error}; continuing with current credentials"
            ));
        }
    }

    let args = config.launch_args();
    match ops.exec(&runtime.primary, &args) {
        Ok(()) => Ok(()),
        Err(primary_error) => {
            if let Some(fallback) = runtime.fallback {
                ops.warn(&format!(
                    "failed to launch optimized payload '{}': {primary_error}; retrying portable payload",
                    runtime.primary.display()
                ));
                match ops.exec(&fallback, &args) {
                    Ok(()) => Ok(()),
                    Err(fallback_error) => Err(format!(
                        "failed to launch '{}' ({primary_error}) and fallback '{}' ({fallback_error})",
                        runtime.primary.display(),
                        fallback.display()
                    )),
                }
            } else {
                Err(format!(
                    "failed to launch '{}': {primary_error}",
                    runtime.primary.display()
                ))
            }
        }
    }
}

fn resolve_runtime_launch<O: LauncherOps>(
    ops: &O,
    config: &LaunchConfig,
) -> Result<RuntimeLaunch, String> {
    let arch = detect_arch(config.container_arch_override.as_deref());
    let lane = determine_lane(ops, arch, &config.cpuinfo_path);
    let portable = config.payload_root.join(PORTABLE_PAYLOAD_NAME);
    let optimized = match lane {
        Lane::Haswell => Some(config.payload_root.join(HASWELL_PAYLOAD_NAME)),
        Lane::CortexA76 => Some(config.payload_root.join(CORTEX_A76_PAYLOAD_NAME)),
        Lane::Portable => None,
    };

    let portable_ok = ops.path_is_executable_file(&portable);
    let optimized_ok = optimized
        .as_ref()
        .is_some_and(|path| ops.path_is_executable_file(path));

    match (portable_ok, optimized_ok, optimized) {
        (_, true, Some(optimized_path)) => Ok(RuntimeLaunch {
            primary: optimized_path,
            fallback: portable_ok.then_some(portable),
            lane,
        }),
        (true, _, _) => Ok(RuntimeLaunch {
            primary: portable,
            fallback: None,
            lane: Lane::Portable,
        }),
        _ => Err(format!(
            "no executable scryer payload found under {}",
            config.payload_root.display()
        )),
    }
}

fn detect_arch(override_value: Option<&str>) -> Arch {
    let machine = override_value.unwrap_or(match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    });

    match machine.to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => Arch::Amd64,
        "aarch64" | "arm64" => Arch::Arm64,
        _ => Arch::Unknown,
    }
}

fn determine_lane<O: LauncherOps>(ops: &O, arch: Arch, cpuinfo_path: &Path) -> Lane {
    let Ok(contents) = ops.read_to_string(cpuinfo_path) else {
        return Lane::Portable;
    };
    let Some(features) = normalized_features(&contents, arch) else {
        return Lane::Portable;
    };

    match arch {
        Arch::Amd64 if feature_set_has_all(&features, X86_REQUIRED_FEATURES) => Lane::Haswell,
        Arch::Arm64 if feature_set_has_all(&features, ARM_REQUIRED_FEATURES) => Lane::CortexA76,
        _ => Lane::Portable,
    }
}

fn normalized_features(contents: &str, arch: Arch) -> Option<HashSet<&'static str>> {
    if matches!(arch, Arch::Unknown) {
        return None;
    }

    let mut feature_sets = Vec::new();
    for line in contents.lines() {
        let Some((key, values)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key != "flags" && key != "Features" {
            continue;
        }

        let mut line_features = HashSet::new();
        for token in values.split_whitespace() {
            let normalized = match arch {
                Arch::Amd64 => normalize_x86_feature(token),
                Arch::Arm64 => normalize_arm_feature(token),
                Arch::Unknown => None,
            };
            if let Some(feature) = normalized {
                line_features.insert(feature);
            }
        }

        if !line_features.is_empty() {
            feature_sets.push(line_features);
        }
    }

    let mut features = feature_sets.into_iter();
    let mut common = features.next()?;
    for feature_set in features {
        common.retain(|feature| feature_set.contains(feature));
        if common.is_empty() {
            return None;
        }
    }
    Some(common)
}

fn normalize_x86_feature(token: &str) -> Option<&'static str> {
    match token.trim().to_ascii_lowercase().as_str() {
        "avx" | "avx1.0" => Some("avx"),
        "avx2" | "avx2.0" => Some("avx2"),
        "bmi1" => Some("bmi1"),
        "bmi2" => Some("bmi2"),
        "f16c" => Some("f16c"),
        "fma" => Some("fma"),
        "abm" | "lzcnt" => Some("lzcnt"),
        "movbe" => Some("movbe"),
        "pclmul" | "pclmulqdq" => Some("pclmulqdq"),
        "popcnt" => Some("popcnt"),
        "rdrand" => Some("rdrand"),
        "sse3" => Some("sse3"),
        "sse4_1" | "sse4.1" => Some("sse4.1"),
        "sse4_2" | "sse4.2" => Some("sse4.2"),
        "ssse3" => Some("ssse3"),
        "osxsave" | "xsave" => Some("xsave"),
        "xsaveopt" => Some("xsaveopt"),
        _ => None,
    }
}

fn normalize_arm_feature(token: &str) -> Option<&'static str> {
    match token.trim().to_ascii_lowercase().as_str() {
        "aes" => Some("aes"),
        "crc" | "crc32" => Some("crc32"),
        "asimd" | "neon" => Some("neon"),
        "fphp" | "asimdhp" | "fp16" => Some("fp16"),
        "atomics" | "lse" => Some("lse"),
        "asimdrdm" | "rdm" => Some("rdm"),
        "asimddp" | "dotprod" => Some("dotprod"),
        "sha2" => Some("sha2"),
        _ => None,
    }
}

fn feature_set_has_all(features: &HashSet<&'static str>, required: &[&'static str]) -> bool {
    required.iter().all(|feature| features.contains(feature))
}

fn resolved_db_path(raw: Option<String>) -> PathBuf {
    let Some(raw) = raw else {
        return PathBuf::from(DEFAULT_DB_PATH);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathBuf::from(DEFAULT_DB_PATH);
    }

    let stripped = trimmed.strip_prefix("sqlite://").unwrap_or(trimmed);
    let without_query = stripped.split('?').next().unwrap_or(stripped).trim();
    if without_query.is_empty() {
        PathBuf::from(DEFAULT_DB_PATH)
    } else {
        let candidate = PathBuf::from(without_query);
        if candidate.is_absolute() {
            candidate
        } else {
            PathBuf::from(DEFAULT_DB_PATH)
        }
    }
}

fn apply_umask<O: LauncherOps>(ops: &O, config: &LaunchConfig) {
    let Some(raw) = config.umask.as_deref() else {
        return;
    };
    match parse_octal_mode(raw) {
        Ok(Some(mask)) => {
            if let Err(error) = ops.set_umask(mask) {
                ops.warn(&format!(
                    "failed to apply UMASK '{}': {error}; continuing with the process default",
                    raw
                ));
            }
        }
        Ok(None) => {}
        Err(error) => ops.warn(&format!(
            "invalid UMASK '{}': {error}; continuing with the process default",
            raw
        )),
    }
}

fn parse_octal_mode(raw: &str) -> Result<Option<u32>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let digits = trimmed.strip_prefix("0o").unwrap_or(trimmed);
    if digits.is_empty() || !digits.chars().all(|ch| matches!(ch, '0'..='7')) {
        return Err("expected an octal mode such as 022".into());
    }
    u32::from_str_radix(digits, 8)
        .map(Some)
        .map_err(|error| format!("failed to parse octal mode: {error}"))
}

fn requested_identity<O: LauncherOps>(ops: &O, config: &LaunchConfig) -> (u32, u32) {
    if !ops.is_root() {
        return (ops.effective_uid(), ops.effective_gid());
    }

    let uid = parse_requested_id(config.requested_uid.as_deref(), LAUNCHER_UID_DEFAULT)
        .unwrap_or_else(|error| {
            ops.warn(&format!(
                "invalid PUID '{}': {error}; falling back to {LAUNCHER_UID_DEFAULT}",
                config.requested_uid.as_deref().unwrap_or("")
            ));
            LAUNCHER_UID_DEFAULT
        });
    let gid = parse_requested_id(config.requested_gid.as_deref(), LAUNCHER_GID_DEFAULT)
        .unwrap_or_else(|error| {
            ops.warn(&format!(
                "invalid PGID '{}': {error}; falling back to {LAUNCHER_GID_DEFAULT}",
                config.requested_gid.as_deref().unwrap_or("")
            ));
            LAUNCHER_GID_DEFAULT
        });
    (uid, gid)
}

fn parse_requested_id(raw: Option<&str>, default: u32) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    trimmed
        .parse::<u32>()
        .map_err(|error| format!("failed to parse numeric id: {error}"))
}

fn repair_ownership<O: LauncherOps>(ops: &O, config: &LaunchConfig, uid: u32, gid: u32) {
    if let Err(error) = ops.create_dir_all(Path::new(CONFIG_DIR)) {
        ops.warn(&format!(
            "failed to create {CONFIG_DIR} before ownership repair: {error}"
        ));
    }

    if let Err(error) = ops.chown_recursive(Path::new(CONFIG_DIR), uid, gid) {
        ops.warn(&format!(
            "failed to re-own {CONFIG_DIR} to {uid}:{gid}: {error}"
        ));
    }

    let db_dir = config
        .db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(CONFIG_DIR));
    if db_dir != Path::new(CONFIG_DIR) && ops.path_is_dir(&db_dir) {
        if let Err(error) = ops.chown_recursive(&db_dir, uid, gid) {
            ops.warn(&format!(
                "failed to re-own {} to {uid}:{gid}: {error}",
                db_dir.display()
            ));
        }
    }
}

fn chown_recursive(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    chown_one(path, uid, gid)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let entry_metadata = entry.metadata()?;
            if entry_metadata.is_dir() {
                chown_recursive(&entry_path, uid, gid)?;
            } else {
                chown_one(&entry_path, uid, gid)?;
            }
        }
    }
    Ok(())
}

fn chown_one(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL byte"))?;
    // SAFETY: libc::lchown expects a valid NUL-terminated path pointer and plain numeric ids.
    let result = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Clone)]
    struct MockEntry {
        is_dir: bool,
        executable: bool,
        contents: Option<String>,
    }

    impl MockEntry {
        fn directory() -> Self {
            Self {
                is_dir: true,
                executable: false,
                contents: None,
            }
        }

        fn executable_file() -> Self {
            Self {
                is_dir: false,
                executable: true,
                contents: None,
            }
        }

        fn text_file(contents: &str) -> Self {
            Self {
                is_dir: false,
                executable: false,
                contents: Some(contents.to_string()),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum MockExecResult {
        Success,
        Failure,
    }

    struct MockLauncherOps {
        uid: u32,
        gid: u32,
        entries: RefCell<HashMap<PathBuf, MockEntry>>,
        read_failures: RefCell<HashSet<PathBuf>>,
        warnings: RefCell<Vec<String>>,
        exec_calls: RefCell<Vec<(PathBuf, Vec<OsString>)>>,
        exec_results: RefCell<Vec<MockExecResult>>,
        fail_create_dir: bool,
        fail_chown: bool,
        fail_umask: bool,
        fail_drop: bool,
    }

    impl Default for MockLauncherOps {
        fn default() -> Self {
            Self {
                uid: 0,
                gid: 0,
                entries: RefCell::new(HashMap::new()),
                read_failures: RefCell::new(HashSet::new()),
                warnings: RefCell::new(Vec::new()),
                exec_calls: RefCell::new(Vec::new()),
                exec_results: RefCell::new(Vec::new()),
                fail_create_dir: false,
                fail_chown: false,
                fail_umask: false,
                fail_drop: false,
            }
        }
    }

    impl MockLauncherOps {
        fn insert_entry<P: AsRef<Path>>(&self, path: P, entry: MockEntry) {
            self.entries
                .borrow_mut()
                .insert(path.as_ref().to_path_buf(), entry);
        }

        fn add_read_failure<P: AsRef<Path>>(&self, path: P) {
            self.read_failures
                .borrow_mut()
                .insert(path.as_ref().to_path_buf());
        }

        fn push_exec_results(&self, results: &[MockExecResult]) {
            self.exec_results
                .borrow_mut()
                .extend(results.iter().copied());
        }

        fn warnings(&self) -> Vec<String> {
            self.warnings.borrow().clone()
        }

        fn exec_paths(&self) -> Vec<PathBuf> {
            self.exec_calls
                .borrow()
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }
    }

    impl LauncherOps for MockLauncherOps {
        fn effective_uid(&self) -> u32 {
            self.uid
        }

        fn effective_gid(&self) -> u32 {
            self.gid
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            if self.read_failures.borrow().contains(path) {
                return Err(io::Error::other("read failed"));
            }
            self.entries
                .borrow()
                .get(path)
                .and_then(|entry| entry.contents.clone())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.fail_create_dir {
                return Err(io::Error::other("create dir failed"));
            }
            self.insert_entry(path, MockEntry::directory());
            Ok(())
        }

        fn path_is_dir(&self, path: &Path) -> bool {
            self.entries
                .borrow()
                .get(path)
                .is_some_and(|entry| entry.is_dir)
        }

        fn path_is_executable_file(&self, path: &Path) -> bool {
            self.entries
                .borrow()
                .get(path)
                .is_some_and(|entry| !entry.is_dir && entry.executable)
        }

        fn chown_recursive(&self, _path: &Path, _uid: u32, _gid: u32) -> io::Result<()> {
            if self.fail_chown {
                Err(io::Error::other("chown failed"))
            } else {
                Ok(())
            }
        }

        fn set_umask(&self, _mask: u32) -> io::Result<()> {
            if self.fail_umask {
                Err(io::Error::other("umask failed"))
            } else {
                Ok(())
            }
        }

        fn drop_privileges(&self, _uid: u32, _gid: u32) -> io::Result<()> {
            if self.fail_drop {
                Err(io::Error::other("drop failed"))
            } else {
                Ok(())
            }
        }

        fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()> {
            self.exec_calls
                .borrow_mut()
                .push((program.to_path_buf(), args.to_vec()));
            let next_result = {
                let mut results = self.exec_results.borrow_mut();
                if results.is_empty() {
                    None
                } else {
                    Some(results.remove(0))
                }
            };
            match next_result.unwrap_or(MockExecResult::Success) {
                MockExecResult::Success => Ok(()),
                MockExecResult::Failure => Err(io::Error::other("exec failed")),
            }
        }

        fn warn(&self, message: &str) {
            self.warnings.borrow_mut().push(message.to_string());
        }
    }

    fn base_config() -> LaunchConfig {
        LaunchConfig {
            args: vec![OsString::from("--version")],
            cpuinfo_path: PathBuf::from("/cpuinfo"),
            payload_root: PathBuf::from("/payloads"),
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            container_arch_override: Some("amd64".into()),
            requested_uid: Some("1000".into()),
            requested_gid: Some("1000".into()),
            umask: None,
        }
    }

    #[test]
    fn amd64_should_select_optimized_when_haswell_features_are_present() {
        let features = normalized_features(
            include_str!("../tests/fixtures/amd64-haswell.cpuinfo"),
            Arch::Amd64,
        )
        .expect("features should parse");
        assert!(feature_set_has_all(&features, X86_REQUIRED_FEATURES));
    }

    #[test]
    fn amd64_should_fallback_to_portable_when_required_features_are_missing() {
        let ops = MockLauncherOps::default();
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/amd64-portable.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let config = base_config();
        let runtime = resolve_runtime_launch(&ops, &config).expect("portable runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-portable"));
        assert_eq!(runtime.lane, Lane::Portable);
    }

    #[test]
    fn arm64_should_select_optimized_when_cortex_a76_features_are_present() {
        let ops = MockLauncherOps::default();
        let mut config = base_config();
        config.container_arch_override = Some("arm64".into());
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/arm64-cortex-a76.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        ops.insert_entry("/payloads/scryer-cortex-a76", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &config).expect("arm64 optimized runtime");
        assert_eq!(
            runtime.primary,
            PathBuf::from("/payloads/scryer-cortex-a76")
        );
        assert_eq!(
            runtime.fallback,
            Some(PathBuf::from("/payloads/scryer-portable"))
        );
    }

    #[test]
    fn arm64_should_fallback_to_portable_when_required_features_are_missing() {
        let ops = MockLauncherOps::default();
        let mut config = base_config();
        config.container_arch_override = Some("arm64".into());
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/arm64-portable.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &config).expect("portable runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-portable"));
        assert_eq!(runtime.lane, Lane::Portable);
    }

    #[test]
    fn unknown_arch_should_fallback_to_portable() {
        let ops = MockLauncherOps::default();
        let mut config = base_config();
        config.container_arch_override = Some("mips64".into());
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &config).expect("portable runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-portable"));
        assert_eq!(runtime.lane, Lane::Portable);
    }

    #[test]
    fn unreadable_cpuinfo_should_fallback_to_portable() {
        let ops = MockLauncherOps::default();
        ops.add_read_failure("/cpuinfo");
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &base_config()).expect("portable runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-portable"));
    }

    #[test]
    fn malformed_cpuinfo_should_fallback_to_portable() {
        let features = normalized_features(
            include_str!("../tests/fixtures/malformed.cpuinfo"),
            Arch::Amd64,
        );
        assert!(features.is_none());
    }

    #[test]
    fn heterogeneous_cpuinfo_should_require_common_features_before_selecting_optimized() {
        let features = normalized_features(
            "\
processor   : 0\n\
flags       : avx avx2 bmi1 bmi2 f16c fma abm movbe pclmulqdq popcnt rdrand sse3 sse4_1 sse4_2 ssse3 xsave xsaveopt\n\
\n\
processor   : 1\n\
flags       : avx avx2 bmi1 bmi2 f16c fma movbe pclmulqdq popcnt rdrand sse3 sse4_1 sse4_2 ssse3 xsave xsaveopt\n",
            Arch::Amd64,
        )
        .expect("common features should still parse");
        assert!(!feature_set_has_all(&features, X86_REQUIRED_FEATURES));
    }

    #[test]
    fn relative_db_paths_should_fallback_to_the_default_config_database() {
        assert_eq!(
            resolved_db_path(Some("./scryer.db".into())),
            PathBuf::from(DEFAULT_DB_PATH)
        );
        assert_eq!(
            resolved_db_path(Some("sqlite://relative/scryer.db?mode=rwc".into())),
            PathBuf::from(DEFAULT_DB_PATH)
        );
    }

    #[test]
    fn missing_optimized_payload_should_fallback_to_portable() {
        let ops = MockLauncherOps::default();
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/amd64-haswell.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &base_config()).expect("portable runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-portable"));
    }

    #[test]
    fn missing_portable_payload_with_valid_optimized_payload_should_launch_optimized() {
        let ops = MockLauncherOps::default();
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/amd64-haswell.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-haswell", MockEntry::executable_file());
        let runtime = resolve_runtime_launch(&ops, &base_config()).expect("optimized runtime");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer-haswell"));
        assert_eq!(runtime.fallback, None);
    }

    #[test]
    fn invalid_umask_should_still_attempt_to_launch() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let mut config = base_config();
        config.umask = Some("not-octal".into());
        run_with_ops(&ops, &config).expect("launch should still proceed");
        assert_eq!(
            ops.exec_paths(),
            vec![PathBuf::from("/payloads/scryer-portable")]
        );
        assert!(
            ops.warnings()
                .iter()
                .any(|warning| warning.contains("invalid UMASK"))
        );
    }

    #[test]
    fn failed_ownership_repair_should_still_attempt_to_launch() {
        let ops = MockLauncherOps {
            fail_chown: true,
            ..Default::default()
        };
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        run_with_ops(&ops, &base_config()).expect("launch should still proceed");
        assert_eq!(
            ops.exec_paths(),
            vec![PathBuf::from("/payloads/scryer-portable")]
        );
        assert!(
            ops.warnings()
                .iter()
                .any(|warning| warning.contains("failed to re-own"))
        );
    }

    #[test]
    fn failed_privilege_drop_should_still_attempt_to_launch() {
        let ops = MockLauncherOps {
            fail_drop: true,
            ..Default::default()
        };
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        run_with_ops(&ops, &base_config()).expect("launch should still proceed");
        assert_eq!(
            ops.exec_paths(),
            vec![PathBuf::from("/payloads/scryer-portable")]
        );
        assert!(
            ops.warnings()
                .iter()
                .any(|warning| warning.contains("failed to drop privileges"))
        );
    }

    #[test]
    fn failed_optimized_exec_should_retry_portable() {
        let ops = MockLauncherOps::default();
        ops.insert_entry(
            "/cpuinfo",
            MockEntry::text_file(include_str!("../tests/fixtures/amd64-haswell.cpuinfo")),
        );
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        ops.insert_entry("/payloads/scryer-haswell", MockEntry::executable_file());
        ops.push_exec_results(&[MockExecResult::Failure, MockExecResult::Success]);
        run_with_ops(&ops, &base_config()).expect("portable fallback should succeed");
        assert_eq!(
            ops.exec_paths(),
            vec![
                PathBuf::from("/payloads/scryer-haswell"),
                PathBuf::from("/payloads/scryer-portable"),
            ]
        );
    }

    #[test]
    fn launcher_should_prefix_data_dir_before_user_args() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer-portable", MockEntry::executable_file());
        let mut config = base_config();
        config.args = vec![OsString::from("--version")];
        run_with_ops(&ops, &config).expect("launch should proceed");
        let args = &ops.exec_calls.borrow()[0].1;
        assert_eq!(
            args,
            &vec![
                OsString::from("--data-dir"),
                OsString::from("/config"),
                OsString::from("--version"),
            ]
        );
    }
}
