use std::sync::Arc;
use std::time::Duration;

use landa_core::{LandaConfig, LandaEngine};
use landa_mod_clipboard::ClipboardModule;
use landa_proto::{DeviceCategory, DeviceOs, PeerId};
use uuid::Uuid;

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("landa-clip-{label}-{}", Uuid::new_v4()));
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
async fn clipboard_requires_trust_and_syncs_between_friends() {
    let dir_a = temp_dir("a");
    let dir_b = temp_dir("b");

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

    let clip_a = ClipboardModule::new(Arc::clone(engine_a.storage()), engine_a.events().clone());
    let clip_b = ClipboardModule::new(Arc::clone(engine_b.storage()), engine_b.events().clone());
    engine_a.register_module(clip_a.clone());
    engine_b.register_module(clip_b.clone());

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

    // Untrusted peer cannot push or fetch clipboard
    assert!(clip_a
        .send_text_to_peer(&engine_a, &info_b.peer_id, "secret".into())
        .await
        .is_err());

    // Pair as friends
    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b.respond_friend_request(&info_a.peer_id, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    clip_a
        .send_text_to_peer(&engine_a, &info_b.peer_id, "https://landa.dev".into())
        .await
        .unwrap();

    let remote_items = clip_a.fetch_remote_items(&engine_a, &info_b.peer_id).await.unwrap();
    assert_eq!(remote_items.len(), 1);
    assert_eq!(remote_items[0].text, "https://landa.dev");

    // Disabling module blocks incoming requests
    engine_b.set_module_enabled("clipboard", false).unwrap();
    assert!(clip_a.fetch_remote_items(&engine_a, &info_b.peer_id).await.is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
