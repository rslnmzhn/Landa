use std::sync::Arc;
use std::time::Duration;

use landa_core::{CoreEvent, LandaConfig, LandaEngine};
use landa_mod_clipboard::{ClipboardContentKind, ClipboardItem, ClipboardModule, MAX_IMAGE_BYTES};
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

    let clip_a = ClipboardModule::new(
        Arc::clone(engine_a.storage()),
        engine_a.events().clone(),
        engine_a.app_data_dir().join("clipboard_images"),
    );
    let clip_b = ClipboardModule::new(
        Arc::clone(engine_b.storage()),
        engine_b.events().clone(),
        engine_b.app_data_dir().join("clipboard_images"),
    );
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
    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let mut rx_b = engine_b.subscribe_events();

    clip_a
        .send_text_to_peer(&engine_a, &info_b.peer_id, "https://landa.dev".into())
        .await
        .unwrap();

    let text_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(CoreEvent::ModuleEvent {
                module_id,
                event_name,
                data,
            }) = rx_b.recv().await
            {
                if module_id == "clipboard" && event_name == "received" {
                    return serde_json::from_value::<ClipboardItem>(data).unwrap();
                }
            }
        }
    })
    .await
    .expect("expected clipboard received event for incoming text");
    assert_eq!(text_event.kind, ClipboardContentKind::Text);
    assert_eq!(text_event.text, "https://landa.dev");
    assert_eq!(text_event.source_peer_id.as_deref(), Some(info_a.peer_id.as_str()));

    let remote_items = clip_a
        .fetch_remote_items(&engine_a, &info_b.peer_id)
        .await
        .unwrap();
    assert_eq!(remote_items.len(), 1);
    assert_eq!(remote_items[0].text, "https://landa.dev");

    // Push PNG image over QUIC and fetch it back
    let png_1x1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    clip_a
        .send_image_bytes_to_peer(&engine_a, &info_b.peer_id, png_1x1)
        .await
        .unwrap();

    let img_event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(CoreEvent::ModuleEvent {
                module_id,
                event_name,
                data,
            }) = rx_b.recv().await
            {
                if module_id == "clipboard" && event_name == "received" {
                    return serde_json::from_value::<ClipboardItem>(data).unwrap();
                }
            }
        }
    })
    .await
    .expect("expected clipboard received event for incoming image");
    assert_eq!(img_event.kind, ClipboardContentKind::Image);
    assert_eq!(img_event.source_peer_id.as_deref(), Some(info_a.peer_id.as_str()));

    let remote_after_img = clip_a
        .fetch_remote_items(&engine_a, &info_b.peer_id)
        .await
        .unwrap();
    assert_eq!(remote_after_img.len(), 2);
    let img_item = &remote_after_img[0];
    assert_eq!(img_item.kind, ClipboardContentKind::Image);
    assert_eq!(img_item.image_width, Some(1));
    assert_eq!(img_item.image_height, Some(1));
    assert!(img_item.image_path.is_none()); // local path must not leak over network

    let fetched_png = clip_a
        .fetch_remote_image(&engine_a, &info_b.peer_id, &img_item.id)
        .await
        .unwrap();
    assert_eq!(fetched_png, png_1x1);

    // Local push emits "updated" rather than "received"
    clip_b.push_local_text("local-only".into(), None, None).unwrap();
    let local_event_name = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(CoreEvent::ModuleEvent {
                module_id,
                event_name,
                ..
            }) = rx_b.recv().await
            {
                if module_id == "clipboard" {
                    return event_name;
                }
            }
        }
    })
    .await
    .expect("expected clipboard updated event for local push");
    assert_eq!(local_event_name, "updated");

    // Local image file exists on B and is deleted when item is deleted
    let b_local_img = clip_b.get_local_item(&img_item.id).unwrap();
    let b_img_path = std::path::PathBuf::from(b_local_img.image_path.unwrap());
    assert!(b_img_path.is_file());
    clip_b.delete_local_item(&img_item.id);
    assert!(!b_img_path.exists());

    // Oversized image is rejected
    let oversized = vec![0u8; MAX_IMAGE_BYTES + 1];
    assert!(clip_a.push_local_image(&oversized, None, None).is_err());

    // Disabling module blocks incoming requests
    engine_b.set_module_enabled("clipboard", false).unwrap();
    assert!(clip_a
        .fetch_remote_items(&engine_a, &info_b.peer_id)
        .await
        .is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn clipboard_rgba_image_and_text_roundtrip_between_friends() {
    let dir_a = temp_dir("rgba-a");
    let dir_b = temp_dir("rgba-b");

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("downloads"),
        default_device_name: "Sender".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "Receiver".into(),
    })
    .unwrap();

    let clip_a = ClipboardModule::new(
        Arc::clone(engine_a.storage()),
        engine_a.events().clone(),
        engine_a.app_data_dir().join("clipboard_images"),
    );
    let clip_b = ClipboardModule::new(
        Arc::clone(engine_b.storage()),
        engine_b.events().clone(),
        engine_b.app_data_dir().join("clipboard_images"),
    );
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
            "Receiver",
            DeviceOs::Windows,
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
            "Sender",
            DeviceOs::Android,
            DeviceCategory::Phone,
            Some("127.0.0.1"),
            Some(info_a.quic_port),
        )
        .unwrap();

    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Push 2x2 RGBA image locally (desktop paste path) and send its encoded PNG to B
    let rgba_2x2: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let local_img = clip_a
        .push_local_rgba_image(2, 2, &rgba_2x2)
        .expect("RGBA image should encode to PNG");
    assert_eq!(local_img.kind, ClipboardContentKind::Image);
    assert_eq!(local_img.image_width, Some(2));
    assert_eq!(local_img.image_height, Some(2));

    let png_bytes = std::fs::read(local_img.image_path.as_ref().unwrap()).unwrap();
    clip_a
        .send_image_bytes_to_peer(&engine_a, &info_b.peer_id, &png_bytes)
        .await
        .unwrap();
    clip_a
        .send_text_to_peer(&engine_a, &info_b.peer_id, "hello from sender".into())
        .await
        .unwrap();

    let b_items = clip_b.list_local_items();
    assert_eq!(b_items.len(), 2);
    assert_eq!(b_items[0].kind, ClipboardContentKind::Text);
    assert_eq!(b_items[0].text, "hello from sender");
    assert_eq!(b_items[0].source_peer_name.as_deref(), Some("Sender"));
    assert_eq!(b_items[1].kind, ClipboardContentKind::Image);
    assert_eq!(b_items[1].image_width, Some(2));
    assert_eq!(b_items[1].image_height, Some(2));

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
