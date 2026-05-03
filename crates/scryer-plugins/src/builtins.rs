#[derive(Clone, Copy, Debug)]
pub struct BuiltinPluginAsset {
    pub wasm: &'static [u8],
    pub descriptor_json: &'static str,
    pub description: &'static str,
}

/// Built-in NZBGeek indexer plugin asset pair.
pub const NZBGEEK: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm: include_bytes!("../builtins/nzbgeek_indexer.wasm"),
    descriptor_json: include_str!("../builtins/nzbgeek_indexer.descriptor.json"),
    description: include_str!("../builtins/nzbgeek_indexer.description.txt"),
};

/// Built-in generic Newznab indexer plugin asset pair.
pub const NEWZNAB: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm: include_bytes!("../builtins/newznab_indexer.wasm"),
    descriptor_json: include_str!("../builtins/newznab_indexer.descriptor.json"),
    description: include_str!("../builtins/newznab_indexer.description.txt"),
};

/// Built-in AnimeTosho indexer plugin asset pair.
pub const ANIMETOSHO: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm: include_bytes!("../builtins/animetosho_indexer.wasm"),
    descriptor_json: include_str!("../builtins/animetosho_indexer.descriptor.json"),
    description: include_str!("../builtins/animetosho_indexer.description.txt"),
};

/// Built-in Torznab indexer plugin asset pair.
pub const TORZNAB: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm: include_bytes!("../builtins/torznab_indexer.wasm"),
    descriptor_json: include_str!("../builtins/torznab_indexer.descriptor.json"),
    description: include_str!("../builtins/torznab_indexer.description.txt"),
};

pub const INDEXER_BUILTINS: &[BuiltinPluginAsset] = &[NZBGEEK, NEWZNAB, ANIMETOSHO, TORZNAB];
pub const SUBTITLE_BUILTINS: &[BuiltinPluginAsset] = &[];
pub const DOWNLOAD_CLIENT_BUILTINS: &[BuiltinPluginAsset] = &[];
pub const NOTIFICATION_BUILTINS: &[BuiltinPluginAsset] = &[];

pub fn builtin_description_for_provider(provider_type: &str) -> Option<&'static str> {
    let key = provider_type.trim().to_ascii_lowercase();
    INDEXER_BUILTINS
        .iter()
        .chain(SUBTITLE_BUILTINS.iter())
        .chain(DOWNLOAD_CLIENT_BUILTINS.iter())
        .chain(NOTIFICATION_BUILTINS.iter())
        .find_map(|asset| {
            let descriptor: serde_json::Value = serde_json::from_str(asset.descriptor_json).ok()?;
            let provider = descriptor.get("provider")?;
            let actual = provider.get("provider_type")?.as_str()?;
            (actual.eq_ignore_ascii_case(&key)).then_some(asset.description.trim())
        })
}
