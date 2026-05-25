use std::collections::HashSet;

pub const PLUGIN_REQUIRED_FEATURE_SIMD128: &str = "simd128";
pub const PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD: &str = "relaxed-simd";

const SIMD128_PROBE_WAT: &str = r#"(module
  (func (export "probe") (result i32)
    (i32x4.extract_lane 0
      (v128.const i32x4 0 0 0 0))))
"#;

const RELAXED_SIMD_PROBE_WAT: &str = r#"(module
  (func (export "probe") (result i32)
    (i8x16.extract_lane_u 0
      (i8x16.relaxed_swizzle
        (v128.const i8x16 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
        (v128.const i8x16 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)))))
"#;

pub fn detect_supported_plugin_required_features() -> HashSet<String> {
    detect_supported_plugin_required_features_with(supports_plugin_module)
}

fn detect_supported_plugin_required_features_with(
    mut supports: impl FnMut(&str) -> bool,
) -> HashSet<String> {
    let mut features = HashSet::new();
    if !supports(SIMD128_PROBE_WAT) {
        return features;
    }

    features.insert(PLUGIN_REQUIRED_FEATURE_SIMD128.to_string());
    if supports(RELAXED_SIMD_PROBE_WAT) {
        features.insert(PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD.to_string());
    }
    features
}

fn supports_plugin_module(wat: &str) -> bool {
    compile_plugin_module(wat).is_ok()
}

fn compile_plugin_module(wat: &str) -> Result<(), extism::Error> {
    extism::PluginBuilder::new(wat)
        .with_cache_disabled()
        .compile()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_simd_probe_degrades_to_baseline() {
        let features = detect_supported_plugin_required_features_with(|_| false);
        assert!(features.is_empty());
    }

    #[test]
    fn relaxed_simd_requires_simd128() {
        let mut calls = 0;
        let features = detect_supported_plugin_required_features_with(|_| {
            calls += 1;
            calls == 2
        });
        assert!(features.is_empty());
    }

    #[test]
    fn simd128_can_be_reported_without_relaxed_simd() {
        let mut calls = 0;
        let features = detect_supported_plugin_required_features_with(|_| {
            calls += 1;
            calls == 1
        });
        assert_eq!(
            features,
            HashSet::from([PLUGIN_REQUIRED_FEATURE_SIMD128.to_string()])
        );
    }

    #[test]
    fn reports_simd128_and_relaxed_simd_when_both_probes_pass() {
        let features = detect_supported_plugin_required_features_with(|_| true);
        assert_eq!(
            features,
            HashSet::from([
                PLUGIN_REQUIRED_FEATURE_SIMD128.to_string(),
                PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD.to_string(),
            ])
        );
    }

    #[test]
    fn extism_runtime_accepts_probe_modules() {
        compile_plugin_module(SIMD128_PROBE_WAT).expect("simd128 probe should compile");
        compile_plugin_module(RELAXED_SIMD_PROBE_WAT).expect("relaxed-simd probe should compile");
    }

    #[test]
    fn detected_features_use_catalog_feature_tokens() {
        let features = detect_supported_plugin_required_features();
        for feature in &features {
            assert!(
                matches!(
                    feature.as_str(),
                    PLUGIN_REQUIRED_FEATURE_SIMD128 | PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD
                ),
                "unexpected feature token {feature}"
            );
        }
        if features.contains(PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD) {
            assert!(features.contains(PLUGIN_REQUIRED_FEATURE_SIMD128));
        }
    }
}
