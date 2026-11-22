use std::sync::Arc;
use std::time::Duration;

use landa_core::{LandaConfig, LandaEngine};
use landa_mod_shares::SharesModule;
use landa_proto::{DeviceCategory, DeviceOs, PeerId};
use uuid::Uuid;

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("landa-shares-{label}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex_decode_32(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(chunk).unwrap();
        out[i] = u8::from_str_radix(s, 16).unwrap();
    }
    out
}

#[tokio::test]
async fn shares_browse_download_and_reject_traversal() {
    let dir_a = temp_dir("a");
    let dir_b = temp_dir("b");
    let shared_dir = dir_b.join("shared_docs");
    std::fs::create_dir_all(shared_dir.join("nested")).unwrap();
    std::fs::write(shared_dir.join("nested/note.txt"), b"secret-share-content").unwrap();
    std::fs::write(dir_b.join("outside.txt"), b"must-not-escape").unwrap();

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("downloads"),
        default_device_name: "A".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "B".into(),
    })
    .unwrap();

    let shares_a = SharesModule::new(Arc::clone(engine_a.storage()));
    let shares_b = SharesModule::new(Arc::clone(engine_b.storage()));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b.add_local_share(shared_dir.clone(), Some("Docs".into())).unwrap();

    engine_a.start_transport_loop();
    engine_b.start_transport_loop();

    let info_a = engine_a.local_identity_info();
    let info_b = engine_b.local_identity_info();
    let peer_a = PeerId::from_hex(&info_a.peer_id).unwrap();
    let peer_b = PeerId::from_hex(&info_b.peer_id).unwrap();

    engine_a
        .storage()
        .upsert_peer(
            &peer_b,
            &hex_decode_32(&info_b.public_key_hex),
            "B",
            DeviceOs::Linux,
            DeviceCategory::Desktop,
            Some("127.0.0.1"),
            Some(info_b.quic_port),
        )
        .unwrap();
    engine_b
        .storage()
        .upsert_peer(
            &peer_a,
            &hex_decode_32(&info_a.public_key_hex),
            "A",
            DeviceOs::Windows,
            DeviceCategory::Desktop,
            Some("127.0.0.1"),
            Some(info_a.quic_port),
        )
        .unwrap();

    // Pair as friends
    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b.respond_friend_request(&info_a.peer_id, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let roots = shares_a.browse_remote_roots(&engine_a, &info_b.peer_id).await.unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].name, "Docs");

    let entries = shares_a
        .browse_remote_dir(&engine_a, &info_b.peer_id, &root.id, "nested")
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "note.txt");

    // Traversal attempt must fail
    let traversal = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root.id, "../outside.txt")
        .await;
    assert!(traversal.is_err());

    // Valid download succeeds with BLAKE3 verification
    let saved = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root.id, "nested/note.txt")
        .await
        .unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), b"secret-share-content");

    // Lazy thumbnail generation over QUIC (1x1 PNG)
    let png_1x1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(shared_dir.join("nested/pixel.png"), png_1x1).unwrap();
    let thumb = shares_a
        .get_remote_thumbnail(&engine_a, &info_b.peer_id, &root.id, "nested/pixel.png", 160)
        .await
        .unwrap();
    assert!(thumb.data_url.starts_with("data:image/"));

    let thumb_traversal = shares_a
        .get_remote_thumbnail(&engine_a, &info_b.peer_id, &root.id, "../outside.txt", 160)
        .await;
    assert!(thumb_traversal.is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
