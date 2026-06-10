use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineArch {
    Amd64,
    Arm64,
    Unknown,
}

impl MachineArch {
    pub fn from_machine(machine: &str) -> Self {
        match machine.trim().to_ascii_lowercase().as_str() {
            "x86_64" | "amd64" => Self::Amd64,
            "aarch64" | "arm64" => Self::Arm64,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryLane {
    Portable,
    Haswell,
    Arm64Optimized,
}

impl BinaryLane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Haswell => "haswell",
            Self::Arm64Optimized => "arm64_optimized",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "portable" => Some(Self::Portable),
            "haswell" => Some(Self::Haswell),
            "arm64_optimized" => Some(Self::Arm64Optimized),
            _ => None,
        }
    }

    pub const fn binary_class(self) -> BinaryClass {
        match self {
            Self::Portable => BinaryClass::Portable,
            Self::Haswell | Self::Arm64Optimized => BinaryClass::Optimized,
        }
    }
}

impl fmt::Display for BinaryLane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryClass {
    Portable,
    Optimized,
}

impl BinaryClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Optimized => "optimized",
        }
    }
}

impl fmt::Display for BinaryClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub const X86_REQUIRED_FEATURES: &[&str] = &[
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

pub const ARM_REQUIRED_FEATURES: &[&str] = &[
    "aes", "crc32", "dotprod", "fp16", "lse", "neon", "rdm", "sha2",
];

pub fn normalize_feature_token(arch: MachineArch, token: &str) -> Option<&'static str> {
    match arch {
        MachineArch::Amd64 => normalize_x86_feature(token),
        MachineArch::Arm64 => normalize_arm_feature(token),
        MachineArch::Unknown => None,
    }
}

pub fn lane_from_canonical_features(
    arch: MachineArch,
    features: &HashSet<&'static str>,
) -> BinaryLane {
    match arch {
        MachineArch::Amd64 if feature_set_has_all(features, X86_REQUIRED_FEATURES) => {
            BinaryLane::Haswell
        }
        MachineArch::Arm64 if feature_set_has_all(features, ARM_REQUIRED_FEATURES) => {
            BinaryLane::Arm64Optimized
        }
        _ => BinaryLane::Portable,
    }
}

pub fn determine_build_lane(_target_arch: &str, _target_features: &str) -> BinaryLane {
    BinaryLane::Portable
}

pub fn validate_build_lane_assertion(
    asserted: Option<&str>,
    derived: BinaryLane,
) -> Result<BinaryLane, String> {
    let Some(raw) = asserted.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(derived);
    };
    let asserted_lane = BinaryLane::parse(raw).ok_or_else(|| {
        format!(
            "invalid SCRYER_BUILD_LANE '{raw}'; expected one of portable, haswell, arm64_optimized"
        )
    })?;
    if asserted_lane != derived {
        return Err(format!(
            "SCRYER_BUILD_LANE asserted '{asserted_lane}', but target arch/features derive '{derived}'"
        ));
    }
    Ok(derived)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_lane_is_portable_without_required_x86_features() {
        let lane = determine_build_lane("x86_64", "sse2,ssse3,popcnt");
        assert_eq!(lane, BinaryLane::Portable);
    }

    #[test]
    fn build_lane_derives_portable_even_with_required_x86_features() {
        let lane = determine_build_lane(
            "x86_64",
            "avx,avx2,bmi1,bmi2,f16c,fma,lzcnt,movbe,pclmulqdq,popcnt,rdrand,sse3,sse4.1,sse4.2,ssse3,xsave,xsaveopt",
        );
        assert_eq!(lane, BinaryLane::Portable);
        assert_eq!(lane.binary_class(), BinaryClass::Portable);
    }

    #[test]
    fn build_lane_derives_portable_even_with_required_arm_features() {
        let lane = determine_build_lane("aarch64", "aes,crc,dotprod,fp16,lse,neon,rdm,sha2");
        assert_eq!(lane, BinaryLane::Portable);
    }

    #[test]
    fn validate_build_lane_assertion_accepts_matching_portable_lane() {
        let lane =
            validate_build_lane_assertion(Some("portable"), BinaryLane::Portable).expect("match");
        assert_eq!(lane, BinaryLane::Portable);
    }

    #[test]
    fn validate_build_lane_assertion_rejects_invalid_value() {
        let error = validate_build_lane_assertion(Some("banana"), BinaryLane::Portable)
            .expect_err("invalid assertion");
        assert!(error.contains("invalid SCRYER_BUILD_LANE"));
    }

    #[test]
    fn validate_build_lane_assertion_rejects_mismatch() {
        let haswell_error = validate_build_lane_assertion(Some("haswell"), BinaryLane::Portable)
            .expect_err("haswell mismatch");
        assert!(haswell_error.contains("asserted 'haswell'"));
        assert!(haswell_error.contains("derive 'portable'"));

        let arm_error =
            validate_build_lane_assertion(Some("arm64_optimized"), BinaryLane::Portable)
                .expect_err("arm64 mismatch");
        assert!(arm_error.contains("asserted 'arm64_optimized'"));
        assert!(arm_error.contains("derive 'portable'"));
    }
}
