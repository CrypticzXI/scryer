use std::{fs, path::PathBuf};

use serde_json::json;

const DICTIONARY_BYTES: usize = 8 * 1024;

fn main() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/corpora/domain_event_payload_sanitized.jsonl");
    let mut samples = scryer_infrastructure_sql::sanitized_corpus::load_sanitized_jsonl(&corpus)
        .expect("sanitized production-shape corpus should pass audit");
    samples.extend(synthetic_samples());
    let refs = samples.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let dictionary = zstd::dict::from_samples(&refs, DICTIONARY_BYTES)
        .expect("domain event corpus should train a dictionary");
    assert_eq!(dictionary.len(), DICTIONARY_BYTES);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/domain_event_payload_v1.dict");
    fs::write(&output, dictionary).expect("dictionary should be writable");
    println!("wrote {}", output.display());
}

fn synthetic_samples() -> Vec<Vec<u8>> {
    let event_types = [
        "release_grabbed",
        "import_completed",
        "import_rejected",
        "media_file_deleted",
        "media_file_upgraded",
        "download_failed",
        "job_run_started",
        "job_run_completed",
        "library_scan_started",
        "library_scan_completed",
        "seeding_started",
        "seeding_completed",
    ];
    (0..4096)
        .map(|index| {
            let client_type = ["usenet", "torrent", "weaver", "qbittorrent"][index % 4];
            let quality = ["WEBDL-1080p", "Bluray-1080p", "HDTV-720p", "WEBDL-2160p"][index % 4];
            let items = (0..(index % 6))
                .map(|item| {
                    let state = ["queued", "downloading", "completed", "failed"][item % 4];
                    json!({
                        "id": format!("<item-id:{}>", item),
                        "state": state,
                        "progress": (index + item) % 101,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::to_vec(&json!({
                "type": event_types[index % event_types.len()],
                "data": {
                    "title_id": format!("<title-id:{}>", index % 4),
                    "download_id": format!("<download-id:{}>", index % 6),
                    "status": if index % 3 == 0 { "completed" } else { "failed" },
                    "reason": if index % 5 == 0 { "upgrade_cleanup" } else { "<message:medium>" },
                    "source_path": "<absolute-path:long>",
                    "destination_path": "<absolute-path:long>",
                    "episode_ids": ["<episode-id:medium>"],
                    "collection_id": format!("<collection-id:{}>", index % 11),
                    "client_id": format!("<client-id:{}>", index % 13),
                    "client_type": client_type,
                    "quality": quality,
                    "source_title": format!("<release-title:{}>", index % 17),
                    "source_provider": format!("<provider-name:{}>", index % 19),
                    "correlation_id": format!("<correlation-id:{}>", index % 23),
                    "items": items,
                    "size_bytes": (index % 8) * 1024,
                    "successful": index % 3 == 0,
                }
            }))
            .expect("synthetic event should serialize")
        })
        .collect()
}
