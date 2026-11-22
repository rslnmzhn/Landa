use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use landa_core::{CoreEvent, LandaConfig, LandaEngine, OutgoingFileItem};
use landa_proto::{DeviceCategory, DeviceOs, PeerId};
use uuid::Uuid;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("landa-test-{label}-{}", Uuid::new_v4()));
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
async fn quic_friend_pairing_and_file_transfer_end_to_end() {
    let dir_a = temp_dir("node-a");
    let dir_b = temp_dir("node-b");
    let dl_a = dir_a.join("downloads");
    let dl_b = dir_b.join("downloads");

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dl_a,
        default_device_name: "Node A".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dl_b.clone(),
        default_device_name: "Node B".into(),
    })
    .unwrap();

    engine_a.start_transport_loop();
    engine_b.start_transport_loop();

    let info_a = engine_a.local_identity_info();
    let info_b = engine_b.local_identity_info();
    let peer_id_a = PeerId::from_hex(&info_a.peer_id).unwrap();
    let peer_id_b = PeerId::from_hex(&info_b.peer_id).unwrap();
    let pk_a = hex_decode_32(&info_a.public_key_hex);
    let pk_b = hex_decode_32(&info_b.public_key_hex);

    // Seed peer endpoints (simulating discovery)
    engine_a
        .storage()
        .upsert_peer(
            &peer_id_b,
            &pk_b,
            "Node B",
            DeviceOs::Linux,
            DeviceCategory::Desktop,
            Some("127.0.0.1"),
            Some(info_b.quic_port),
        )
        .unwrap();
    engine_b
        .storage()
        .upsert_peer(
            &peer_id_a,
            &pk_a,
            "Node A",
            DeviceOs::Windows,
            DeviceCategory::Desktop,
            Some("127.0.0.1"),
            Some(info_a.quic_port),
        )
        .unwrap();

    // Untrusted peer cannot open module stream
    let untrusted_res = engine_a
        .open_trusted_module_stream(&info_b.peer_id, landa_proto::wire::StreamKind::Shares)
        .await;
    assert!(untrusted_res.is_err());

    // Friend handshake
    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reqs_b = engine_b.list_friend_requests().unwrap();
    assert_eq!(reqs_b.len(), 1);
    assert_eq!(reqs_b[0].peer_id, info_a.peer_id);

    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let peers_a = engine_a.list_peers().unwrap();
    assert!(peers_a.iter().any(|p| p.peer_id == info_b.peer_id && p.is_trusted));

    // File transfer with explicit offer acceptance
    let src_file = dir_a.join("hello-landa.txt");
    let payload = b"Hello from Rust QUIC Landa v2 with BLAKE3 integrity!";
    std::fs::write(&src_file, payload).unwrap();

    let mut rx_b = engine_b.subscribe_events();
    let engine_b_clone = engine_b.clone();
    let accept_task = tokio::spawn(async move {
        while let Ok(ev) = rx_b.recv().await {
            if let CoreEvent::IncomingTransferOffer(offer) = ev {
                engine_b_clone
                    .respond_transfer_offer(&offer.transfer_id, true)
                    .unwrap();
                break;
            }
        }
    });

    let record = engine_a
        .send_files(&info_b.peer_id, vec![src_file])
        .await
        .unwrap();
    assert_eq!(record.status, "completed");
    assert_eq!(record.file_count, 1);

    accept_task.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let received_file = dl_b.join("hello-landa.txt");
    assert!(received_file.exists());
    assert_eq!(std::fs::read(&received_file).unwrap(), payload);

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn quic_send_items_via_open_source_hook_preserves_folder_tree() {
    let dir_a = temp_dir("saf-a");
    let dir_b = temp_dir("saf-b");
    let dl_a = dir_a.join("downloads");
    let dl_b = dir_b.join("downloads");

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dl_a,
        default_device_name: "Android Node".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dl_b.clone(),
        default_device_name: "Desktop Node".into(),
    })
    .unwrap();

    let backing_file = dir_a.join("backing.bin");
    let payload = b"SAF streamed folder payload 1234567890";
    std::fs::write(&backing_file, payload).unwrap();

    let backing_clone = backing_file.clone();
    engine_a.set_open_source_hook(Arc::new(move |uri: &str| {
        if uri == "content://com.android.externalstorage.documents/tree/primary%3APhotos/document/1" {
            std::fs::File::open(&backing_clone).ok()
        } else {
            None
        }
    }));

    engine_a.start_transport_loop();
    engine_b.start_transport_loop();

    let info_a = engine_a.local_identity_info();
    let info_b = engine_b.local_identity_info();
    let peer_id_a = PeerId::from_hex(&info_a.peer_id).unwrap();
    let peer_id_b = PeerId::from_hex(&info_b.peer_id).unwrap();
    let pk_a = hex_decode_32(&info_a.public_key_hex);
    let pk_b = hex_decode_32(&info_b.public_key_hex);

    engine_a
        .storage()
        .upsert_peer(
            &peer_id_b,
            &pk_b,
            "Desktop Node",
            DeviceOs::Windows,
            DeviceCategory::Desktop,
            Some("127.0.0.1"),
            Some(info_b.quic_port),
        )
        .unwrap();
    engine_b
        .storage()
        .upsert_peer(
            &peer_id_a,
            &pk_a,
            "Android Node",
            DeviceOs::Android,
            DeviceCategory::Phone,
            Some("127.0.0.1"),
            Some(info_a.quic_port),
        )
        .unwrap();

    let mut rx_b = engine_b.subscribe_events();
    let engine_b_clone = engine_b.clone();
    let accept_task = tokio::spawn(async move {
        while let Ok(ev) = rx_b.recv().await {
            if let CoreEvent::IncomingTransferOffer(offer) = ev {
                engine_b_clone
                    .respond_transfer_offer(&offer.transfer_id, true)
                    .unwrap();
                break;
            }
        }
    });

    let items = vec![OutgoingFileItem {
        source_path: PathBuf::from("content://com.android.externalstorage.documents/tree/primary%3APhotos/document/1"),
        source_uri: Some("content://com.android.externalstorage.documents/tree/primary%3APhotos/document/1".into()),
        relative_path: "Photos/2026/pic.bin".into(),
        size_bytes: payload.len() as u64,
    }];

    let record = engine_a.send_items(&info_b.peer_id, items).await.unwrap();
    assert_eq!(record.status, "completed");
    assert_eq!(record.file_count, 1);

    accept_task.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let received_file = dl_b.join("Photos").join("2026").join("pic.bin");
    assert!(received_file.exists());
    assert_eq!(std::fs::read(&received_file).unwrap(), payload);

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn qr_connect_establishes_verified_quic_without_auto_trust() {
    let dir_a = temp_dir("qr-a");
    let dir_b = temp_dir("qr-b");

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("downloads"),
        default_device_name: "QR Scanner".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "QR Host".into(),
    })
    .unwrap();

    engine_a.start_transport_loop();
    engine_b.start_transport_loop();

    let info_b = engine_b.local_identity_info();
    let peer_id_b = PeerId::from_hex(&info_b.peer_id).unwrap();
    let pk_b = hex_decode_32(&info_b.public_key_hex);

    let qr_uri = landa_core::build_qr_connect_uri(
        std::net::Ipv4Addr::LOCALHOST,
        info_b.quic_port,
        peer_id_b,
        &pk_b,
        &info_b.device_name,
    );

    let connected = engine_a.connect_via_qr_uri(&qr_uri).await.unwrap();
    assert_eq!(connected.peer_id, info_b.peer_id);
    assert_eq!(connected.display_name, "QR Host");
    assert!(connected.is_online);
    assert!(!connected.is_trusted);

    // Self-connect via QR must be rejected
    let info_a = engine_a.local_identity_info();
    let self_uri = landa_core::build_qr_connect_uri(
        std::net::Ipv4Addr::LOCALHOST,
        info_a.quic_port,
        PeerId::from_hex(&info_a.peer_id).unwrap(),
        &hex_decode_32(&info_a.public_key_hex),
        &info_a.device_name,
    );
    assert!(engine_a.connect_via_qr_uri(&self_uri).await.is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
