use chrono::Utc;
use eda_model::{format_from_filename, is_eda_asset, AssetRecord, ManifestRecord};

#[test]
fn detects_common_and_archive_formats() {
    assert_eq!(format_from_filename("part.BXL"), "bxl");
    assert_eq!(format_from_filename("model.step"), "step");
    assert_eq!(format_from_filename("cad.zip"), "zip");
    assert!(is_eda_asset("library.zip"));
}

#[test]
fn manifest_is_jsonl_serializable() {
    let a = AssetRecord {
        manufacturer: "Texas Instruments".into(),
        mpn: "X".into(),
        part_url: "https://ti.com/product/X".into(),
        asset_url: "https://ti.com/X.bxl".into(),
        discovery_url: None,
        filename: "X.bxl".into(),
        format: "bxl".into(),
        content_type: Some("application/octet-stream".into()),
        source_package: None,
        request_url: None,
    };
    let m = ManifestRecord {
        manufacturer: a.manufacturer,
        mpn: a.mpn,
        part_url: a.part_url,
        asset_url: a.asset_url,
        discovery_url: a.discovery_url,
        filename: a.filename,
        format: a.format,
        sha256: "00".repeat(32),
        size: 2,
        retrieved_at: Utc::now(),
        http_etag: None,
        http_last_modified: None,
        content_type: a.content_type,
        source_package: None,
        request_url: None,
        final_download_url: None,
        content_disposition_filename: None,
    };
    let line = serde_json::to_string(&m).unwrap();
    assert!(line.contains("\"sha256\""));
    assert_eq!(
        serde_json::from_str::<ManifestRecord>(&line).unwrap().size,
        2
    );
}
