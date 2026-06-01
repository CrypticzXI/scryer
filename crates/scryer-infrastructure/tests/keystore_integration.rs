#[test]
fn platform_keystore_is_disabled_for_integration_test_binaries() {
    let original = std::env::var("SCRYER_DISABLE_PLATFORM_KEYSTORE").ok();
    unsafe { std::env::remove_var("SCRYER_DISABLE_PLATFORM_KEYSTORE") };

    assert!(scryer_infrastructure::keystore::platform_keystores(None).is_empty());

    match original {
        Some(value) => unsafe { std::env::set_var("SCRYER_DISABLE_PLATFORM_KEYSTORE", value) },
        None => unsafe { std::env::remove_var("SCRYER_DISABLE_PLATFORM_KEYSTORE") },
    }
}
