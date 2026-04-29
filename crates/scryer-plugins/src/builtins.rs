/// Built-in NZBGeek indexer WASM plugin. Handles NZBGeek-specific metadata
/// (thumbs, subtitles, password) and the Newznab protocol.
pub const NZBGEEK_WASM: &[u8] = include_bytes!("../builtins/nzbgeek_indexer.wasm");

/// Built-in generic Newznab indexer WASM plugin. Handles the standard Newznab
/// protocol for compatible indexers.
pub const NEWZNAB_WASM: &[u8] = include_bytes!("../builtins/newznab_indexer.wasm");

/// Built-in DogNZB indexer WASM plugin. Handles DogNZB-specific metadata and
/// the Newznab protocol.
pub const DOGNZB_WASM: &[u8] = include_bytes!("../builtins/dognzb_indexer.wasm");

/// Built-in AnimeTosho indexer WASM plugin. Searches via AniDB ID + freetext
/// against the AnimeTosho JSON API.
pub const ANIMETOSHO_WASM: &[u8] = include_bytes!("../builtins/animetosho_indexer.wasm");

/// Built-in Torznab indexer WASM plugin. Handles the Torznab protocol for
/// Jackett, Prowlarr, and other compatible torrent indexer proxies.
pub const TORZNAB_WASM: &[u8] = include_bytes!("../builtins/torznab_indexer.wasm");

/// Built-in Jimaku subtitle-provider WASM plugin.
pub const JIMAKU_WASM: &[u8] = include_bytes!("../builtins/jimaku_subtitle_provider.wasm");
