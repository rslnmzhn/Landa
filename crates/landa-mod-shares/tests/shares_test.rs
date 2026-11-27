use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use landa_core::{CoreError, CoreEvent, LandaConfig, LandaEngine, LandaModule, PeerContext};
use landa_mod_shares::{
    ShareContentProvider, ShareEntry, ShareFolderFileMeta, ShareOpenedFile, ShareResponse,
    SharesModule,
};
use landa_proto::wire::{write_frame, StreamKind};
use landa_proto::{DeviceCategory, DeviceOs, PeerId};
use quinn::{RecvStream, SendStream};
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

    let root = shares_b
        .add_local_share(shared_dir.clone(), Some("Docs".into()))
        .unwrap();

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
    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let roots = shares_a
        .browse_remote_roots(&engine_a, &info_b.peer_id)
        .await
        .unwrap();
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
        .get_remote_thumbnail(
            &engine_a,
            &info_b.peer_id,
            &root.id,
            "nested/pixel.png",
            160,
        )
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

struct MockSafProvider {
    tree_uri: String,
    backing_dir: PathBuf,
}

impl ShareContentProvider for MockSafProvider {
    fn list_dir(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        if root_uri != self.tree_uri || !is_dir {
            return Err(CoreError::NotFound("saf root".into()));
        }
        let target = if rel_path.is_empty() {
            self.backing_dir.clone()
        } else {
            self.backing_dir.join(rel_path)
        };
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&target)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let meta = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let next_rel = if rel_path.is_empty() {
                name.clone()
            } else {
                format!("{rel_path}/{name}")
            };
            out.push(ShareEntry {
                name,
                rel_path: next_rel,
                is_dir: ft.is_dir(),
                size_bytes: if ft.is_dir() { 0 } else { meta.len() },
                modified_ms: 12345,
            });
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    fn open_file(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        if root_uri != self.tree_uri || !is_dir {
            return Err(CoreError::NotFound("saf root".into()));
        }
        let target = self.backing_dir.join(rel_path);
        let meta = std::fs::metadata(&target)?;
        let file = std::fs::File::open(&target)?;
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.bin")
            .to_string();
        Ok(ShareOpenedFile {
            file,
            file_name,
            size_bytes: meta.len(),
            modified_ms: 12345,
        })
    }
}

#[tokio::test]
async fn saf_content_uri_share_persists_across_restart_and_streams_files() {
    let dir_a = temp_dir("saf-a");
    let dir_b = temp_dir("saf-b");
    let saf_backing = dir_b.join("saf_tree");
    std::fs::create_dir_all(saf_backing.join("photos")).unwrap();
    std::fs::write(
        saf_backing.join("photos/hello.txt"),
        b"saf-persisted-payload",
    )
    .unwrap();
    let png_1x1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(saf_backing.join("photos/pixel.png"), png_1x1).unwrap();

    let tree_uri =
        "content://com.android.externalstorage.documents/tree/primary%3ADocuments".to_string();

    // First session: add SAF share URI and shut down
    let root_id = {
        let engine_b1 = LandaEngine::new_loopback(LandaConfig {
            app_data_dir: dir_b.clone(),
            default_download_dir: dir_b.join("downloads"),
            default_device_name: "B".into(),
        })
        .unwrap();
        let shares_b1 = SharesModule::new(Arc::clone(engine_b1.storage()));
        let root = shares_b1
            .add_local_share_uri(tree_uri.clone(), "Documents".into(), true)
            .unwrap();
        assert_eq!(root.path, tree_uri);
        engine_b1.shutdown();
        root.id
    };

    // Second session (simulated app restart): re-open same storage and attach SAF provider
    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("downloads"),
        default_device_name: "A".into(),
    })
    .unwrap();
    let engine_b2 = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "B".into(),
    })
    .unwrap();

    let shares_a = SharesModule::new(Arc::clone(engine_a.storage()));
    let shares_b2 = SharesModule::new(Arc::clone(engine_b2.storage()));
    shares_b2.set_content_provider(Arc::new(MockSafProvider {
        tree_uri: tree_uri.clone(),
        backing_dir: saf_backing.clone(),
    }));
    engine_a.register_module(shares_a.clone());
    engine_b2.register_module(shares_b2.clone());

    // Persisted share root must still be present after restart
    let persisted = shares_b2.list_local_shares().unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, root_id);
    assert_eq!(persisted[0].path, tree_uri);

    engine_a.start_transport_loop();
    engine_b2.start_transport_loop();

    let info_a = engine_a.local_identity_info();
    let info_b = engine_b2.local_identity_info();
    let peer_a = PeerId::from_hex(&info_a.peer_id).unwrap();
    let peer_b = PeerId::from_hex(&info_b.peer_id).unwrap();

    engine_a
        .storage()
        .upsert_peer(
            &peer_b,
            &hex_decode_32(&info_b.public_key_hex),
            "B",
            DeviceOs::Android,
            DeviceCategory::Phone,
            Some("127.0.0.1"),
            Some(info_b.quic_port),
        )
        .unwrap();
    engine_b2
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

    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b2
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let roots = shares_a
        .browse_remote_roots(&engine_a, &info_b.peer_id)
        .await
        .unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].name, "Documents");

    let entries = shares_a
        .browse_remote_dir(&engine_a, &info_b.peer_id, &root_id, "photos")
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);

    let saved = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root_id, "photos/hello.txt")
        .await
        .unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), b"saf-persisted-payload");

    let thumb = shares_a
        .get_remote_thumbnail(
            &engine_a,
            &info_b.peer_id,
            &root_id,
            "photos/pixel.png",
            160,
        )
        .await
        .unwrap();
    assert!(thumb.data_url.starts_with("data:image/"));

    let traversal = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root_id, "../secret.txt")
        .await;
    assert!(traversal.is_err());

    engine_a.shutdown();
    engine_b2.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn shares_download_nested_folder_and_reject_traversal() {
    let dir_a = temp_dir("folder-a");
    let dir_b = temp_dir("folder-b");
    let shared_dir = dir_b.join("shared_docs");
    std::fs::create_dir_all(shared_dir.join("album/2026")).unwrap();
    std::fs::create_dir_all(shared_dir.join("album/empty_sub")).unwrap();
    std::fs::create_dir_all(shared_dir.join("album/nested_empty/leaf_empty")).unwrap();
    std::fs::write(shared_dir.join("root_note.txt"), b"root-note-bytes").unwrap();
    std::fs::write(shared_dir.join("album/cover.txt"), b"album-cover-bytes").unwrap();
    std::fs::write(
        shared_dir.join("album/2026/photo.bin"),
        b"nested-photo-payload",
    )
    .unwrap();
    let empty_root_dir = dir_b.join("empty_root");
    std::fs::create_dir_all(&empty_root_dir).unwrap();
    std::fs::create_dir_all(dir_b.join("outside_dir")).unwrap();
    std::fs::write(dir_b.join("outside_dir/secret.txt"), b"must-not-escape").unwrap();

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

    let root = shares_b
        .add_local_share(shared_dir.clone(), Some("Docs".into()))
        .unwrap();
    let empty_root = shares_b
        .add_local_share(empty_root_dir.clone(), Some("EmptyRoot".into()))
        .unwrap();

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

    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Traversal attempts must fail
    assert!(shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "../outside_dir")
        .await
        .is_err());
    assert!(shares_a
        .download_remote_folder(
            &engine_a,
            &info_b.peer_id,
            &root.id,
            "album/../../outside_dir"
        )
        .await
        .is_err());
    assert!(shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "/etc")
        .await
        .is_err());
    // Downloading a file path as a folder must fail
    assert!(shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "root_note.txt")
        .await
        .is_err());

    let mut events_rx = engine_a.subscribe_events();

    // Download nested subfolder "album" (including empty subdirectories)
    let sub_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "album")
        .await
        .unwrap();
    assert_eq!(sub_res.folder_name, "album");
    assert_eq!(sub_res.file_count, 2);
    assert_eq!(
        std::fs::read(dir_a.join("downloads/album/cover.txt")).unwrap(),
        b"album-cover-bytes"
    );
    assert_eq!(
        std::fs::read(dir_a.join("downloads/album/2026/photo.bin")).unwrap(),
        b"nested-photo-payload"
    );
    assert!(
        dir_a.join("downloads/album/empty_sub").is_dir(),
        "empty subdirectory must be created"
    );
    assert!(
        dir_a
            .join("downloads/album/nested_empty/leaf_empty")
            .is_dir(),
        "nested empty subdirectory must be created"
    );

    // Verify progress and history events were emitted and history was recorded
    let mut saw_progress = false;
    let mut saw_completed = false;
    let mut saw_history = false;
    while let Ok(ev) = events_rx.try_recv() {
        match ev {
            CoreEvent::TransferProgress(p) if p.status == "transferring" => saw_progress = true,
            CoreEvent::TransferProgress(p) if p.status == "completed" => saw_completed = true,
            CoreEvent::HistoryChanged => saw_history = true,
            _ => {}
        }
    }
    assert!(saw_progress, "expected transferring progress event");
    assert!(saw_completed, "expected completed progress event");
    assert!(saw_history, "expected history changed event");

    let history = engine_a.list_history(10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].file_count, 2);
    assert_eq!(history[0].status, "completed");
    assert_eq!(
        history[0].relative_paths,
        vec![
            "album/2026/photo.bin".to_string(),
            "album/cover.txt".to_string(),
        ]
    );

    // Download entire share root ("")
    let root_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .unwrap();
    assert_eq!(root_res.folder_name, "Docs");
    assert_eq!(root_res.file_count, 3);
    assert_eq!(
        std::fs::read(dir_a.join("downloads/Docs/root_note.txt")).unwrap(),
        b"root-note-bytes"
    );
    assert_eq!(
        std::fs::read(dir_a.join("downloads/Docs/album/cover.txt")).unwrap(),
        b"album-cover-bytes"
    );
    assert_eq!(
        std::fs::read(dir_a.join("downloads/Docs/album/2026/photo.bin")).unwrap(),
        b"nested-photo-payload"
    );
    assert!(dir_a.join("downloads/Docs/album/empty_sub").is_dir());
    assert!(dir_a
        .join("downloads/Docs/album/nested_empty/leaf_empty")
        .is_dir());

    // Download completely empty share root ("")
    let empty_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &empty_root.id, "")
        .await
        .unwrap();
    assert_eq!(empty_res.folder_name, "EmptyRoot");
    assert_eq!(empty_res.file_count, 0);
    assert_eq!(empty_res.total_bytes, 0);
    assert!(
        dir_a.join("downloads/EmptyRoot").is_dir(),
        "empty root folder must be created in download_dir"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct MaliciousSafProvider;

impl ShareContentProvider for MaliciousSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![ShareEntry {
            name: "escape.txt".into(),
            rel_path: "../escape.txt".into(),
            is_dir: false,
            size_bytes: 5,
            modified_ms: 0,
        }])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        Err(CoreError::InvalidInput("should not open".into()))
    }
}

#[tokio::test]
async fn saf_content_uri_folder_download_and_traversal_rejection() {
    let dir_a = temp_dir("saf-folder-a");
    let dir_b = temp_dir("saf-folder-b");
    let saf_backing = dir_b.join("saf_tree");
    std::fs::create_dir_all(saf_backing.join("photos/summer")).unwrap();
    std::fs::create_dir_all(saf_backing.join("photos/empty_album")).unwrap();
    std::fs::write(saf_backing.join("photos/hello.txt"), b"saf-hello").unwrap();
    std::fs::write(saf_backing.join("photos/summer/beach.txt"), b"saf-beach").unwrap();

    let tree_uri =
        "content://com.android.externalstorage.documents/tree/primary%3ADocuments".to_string();

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
    shares_b.set_content_provider(Arc::new(MockSafProvider {
        tree_uri: tree_uri.clone(),
        backing_dir: saf_backing.clone(),
    }));
    let root = shares_b
        .add_local_share_uri(tree_uri.clone(), "Documents".into(), true)
        .unwrap();
    let bad_root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3ABad".into(),
            "Bad".into(),
            true,
        )
        .unwrap();

    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());
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
            DeviceOs::Android,
            DeviceCategory::Phone,
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

    engine_a.send_friend_request(&info_b.peer_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    engine_b
        .respond_friend_request(&info_a.peer_id, true)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Download nested SAF folder
    let res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "photos")
        .await
        .unwrap();
    assert_eq!(res.folder_name, "photos");
    assert_eq!(res.file_count, 2);
    assert_eq!(
        std::fs::read(dir_a.join("downloads/photos/hello.txt")).unwrap(),
        b"saf-hello"
    );
    assert_eq!(
        std::fs::read(dir_a.join("downloads/photos/summer/beach.txt")).unwrap(),
        b"saf-beach"
    );
    assert!(
        dir_a.join("downloads/photos/empty_album").is_dir(),
        "empty SAF subdirectory must be created"
    );

    // Traversal attempt on SAF share must fail
    assert!(shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "../secret")
        .await
        .is_err());

    // Malicious provider returning `../escape.txt` must be rejected by validate_relative_path
    shares_b.set_content_provider(Arc::new(MaliciousSafProvider));
    assert!(shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &bad_root.id, "")
        .await
        .is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn shares_folder_download_uses_publish_hook_and_enforces_depth_limit() {
    let dir_a = temp_dir("hook-a");
    let dir_b = temp_dir("hook-b");
    let shared_dir = dir_b.join("shared_media");
    std::fs::create_dir_all(shared_dir.join("trip/day1")).unwrap();
    std::fs::create_dir_all(shared_dir.join("trip/empty_notes")).unwrap();
    std::fs::create_dir_all(shared_dir.join("empty_only")).unwrap();
    std::fs::write(shared_dir.join("trip/info.txt"), b"trip-info").unwrap();
    std::fs::write(shared_dir.join("trip/day1/pic.jpg"), b"trip-pic").unwrap();

    // Create a directory deeper than MAX_TREE_DEPTH (32)
    let mut deep_dir = shared_dir.join("too_deep");
    for i in 0..34 {
        deep_dir = deep_dir.join(format!("d{i}"));
    }
    std::fs::create_dir_all(&deep_dir).unwrap();
    std::fs::write(deep_dir.join("leaf.txt"), b"too-deep").unwrap();

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

    let published_rels = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let published_rels_clone = Arc::clone(&published_rels);
    engine_a.set_receive_publish_hook(Arc::new(move |source, relative| {
        let rel_str = relative.to_string_lossy().replace('\\', "/");
        assert!(
            source.exists(),
            "staged temp file must exist before publish"
        );
        published_rels_clone.lock().unwrap().push(rel_str.clone());
        Some(format!("content://media/external/downloads/{rel_str}"))
    }));

    let shares_a = SharesModule::new(Arc::clone(engine_a.storage()));
    let shares_b = SharesModule::new(Arc::clone(engine_b.storage()));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share(shared_dir.clone(), Some("Media".into()))
        .unwrap();

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

    let res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "trip")
        .await
        .unwrap();
    assert_eq!(res.destination, "Downloads/Landa/trip");
    assert_eq!(res.file_count, 2);
    assert_eq!(
        *published_rels.lock().unwrap(),
        vec!["trip/day1/pic.jpg".to_string(), "trip/info.txt".to_string(),]
    );
    assert_eq!(
        res.saved_paths,
        vec![
            "content://media/external/downloads/trip/day1/pic.jpg".to_string(),
            "content://media/external/downloads/trip/info.txt".to_string(),
        ]
    );
    let hook_history = engine_a.list_history(10).unwrap();
    assert_eq!(
        hook_history[0].relative_paths,
        vec!["trip/day1/pic.jpg".to_string(), "trip/info.txt".to_string(),]
    );

    // Exceeding MAX_TREE_DEPTH must fail cleanly
    let deep_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "too_deep")
        .await;
    assert!(deep_res.is_err());

    // Empty folder with zero files on Android MediaStore (publish_hook active) must fail explicitly
    let empty_hook_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "empty_only")
        .await;
    assert!(
        empty_hook_res.is_err(),
        "MediaStore cannot represent an empty folder with zero files"
    );

    // If publish_hook fails (returns None), folder download must fail explicitly instead of falling back
    engine_a.set_receive_publish_hook(Arc::new(|_, _| None));
    let hook_fail = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "trip")
        .await;
    assert!(hook_fail.is_err());

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct DuplicateSafProvider;

impl ShareContentProvider for DuplicateSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![
            ShareEntry {
                name: "dup.txt".into(),
                rel_path: "dup.txt".into(),
                is_dir: false,
                size_bytes: 3,
                modified_ms: 1,
            },
            ShareEntry {
                name: "dup.txt".into(),
                rel_path: "dup.txt".into(),
                is_dir: false,
                size_bytes: 3,
                modified_ms: 1,
            },
        ])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        Err(CoreError::NotFound("unused".into()))
    }
}

#[tokio::test]
async fn shares_folder_download_rejects_duplicate_paths_from_provider() {
    let dir_a = temp_dir("dup-a");
    let dir_b = temp_dir("dup-b");

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
    shares_b.set_content_provider(Arc::new(DuplicateSafProvider));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3ADup".into(),
            "Dup".into(),
            true,
        )
        .unwrap();

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
            DeviceOs::Android,
            DeviceCategory::Phone,
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
            DeviceOs::Linux,
            DeviceCategory::Desktop,
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

    let dup_res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await;
    assert!(
        dup_res.is_err(),
        "duplicate rel_path entries must be rejected"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct MaliciousDeepManifestModule {
    send_deep_dir: bool,
}

#[async_trait]
impl LandaModule for MaliciousDeepManifestModule {
    fn id(&self) -> &'static str {
        "shares"
    }

    fn stream_kinds(&self) -> &'static [StreamKind] {
        &[StreamKind::Shares]
    }

    fn is_enabled(&self) -> bool {
        true
    }

    fn set_enabled(&self, _enabled: bool) {}

    async fn handle_stream(
        &self,
        _peer: PeerContext,
        _kind: StreamKind,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Result<(), CoreError> {
        let mut discard = [0u8; 1024];
        let _ = recv.read(&mut discard).await;

        let deep_dir = (0..33)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        let manifest = if self.send_deep_dir {
            ShareResponse::FolderManifest {
                folder_name: "DeepDir".into(),
                dirs: vec![deep_dir],
                files: Vec::new(),
                total_bytes: 0,
            }
        } else {
            ShareResponse::FolderManifest {
                folder_name: "DeepFile".into(),
                dirs: Vec::new(),
                files: vec![ShareFolderFileMeta {
                    rel_path: format!("{deep_dir}/leaf.txt"),
                    size_bytes: 4,
                }],
                total_bytes: 4,
            }
        };
        write_frame(&mut send, &manifest).await?;
        let _ = send.finish();
        Ok(())
    }
}

#[tokio::test]
async fn shares_folder_download_rejects_manifest_exceeding_depth_limit_without_writing_files() {
    for send_deep_dir in [false, true] {
        let dir_a = temp_dir("deep-rx-a");
        let dir_b = temp_dir("deep-rx-b");
        let download_dir = dir_a.join("downloads");
        std::fs::create_dir_all(&download_dir).unwrap();

        let engine_a = LandaEngine::new_loopback(LandaConfig {
            app_data_dir: dir_a.clone(),
            default_download_dir: download_dir.clone(),
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
        engine_a.register_module(shares_a.clone());
        engine_b.register_module(Arc::new(MaliciousDeepManifestModule { send_deep_dir }));

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
                DeviceOs::Linux,
                DeviceCategory::Desktop,
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

        let res = shares_a
            .download_remote_folder(&engine_a, &info_b.peer_id, "fake-root", "")
            .await;
        assert!(
            res.is_err(),
            "receiver must reject manifest exceeding MAX_TREE_DEPTH (send_deep_dir={send_deep_dir})"
        );
        assert_eq!(
            std::fs::read_dir(&download_dir).unwrap().count(),
            0,
            "no files or directories may be created in download_dir on rejected manifest"
        );
        let temp_root = dir_a.join("incoming_temp");
        if temp_root.exists() {
            assert_eq!(
                std::fs::read_dir(&temp_root).unwrap().count(),
                0,
                "no temp files may be created on rejected manifest"
            );
        }

        engine_a.shutdown();
        engine_b.shutdown();
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }
}

struct SizeMismatchSafProvider {
    backing_file: PathBuf,
}

impl ShareContentProvider for SizeMismatchSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![ShareEntry {
            name: "shot:1.png".into(),
            rel_path: "shot:1.png".into(),
            is_dir: false,
            size_bytes: 5,
            modified_ms: 1,
        }])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        assert_eq!(rel_path, "shot:1.png");
        let file = std::fs::File::open(&self.backing_file)?;
        let size_bytes = file.metadata()?.len();
        Ok(ShareOpenedFile {
            file,
            file_name: "shot:1.png".into(),
            size_bytes,
            modified_ms: 1,
        })
    }
}

#[tokio::test]
async fn shares_folder_download_reports_size_mismatch_to_receiver() {
    let dir_a = temp_dir("mismatch-a");
    let dir_b = temp_dir("mismatch-b");
    std::fs::create_dir_all(&dir_b).unwrap();
    let backing_file = dir_b.join("actual.bin");
    std::fs::write(&backing_file, b"twelve-bytes").unwrap();

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
    shares_b.set_content_provider(Arc::new(SizeMismatchSafProvider {
        backing_file: backing_file.clone(),
    }));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3AMismatch".into(),
            "Mismatch".into(),
            true,
        )
        .unwrap();

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
            DeviceOs::Android,
            DeviceCategory::Phone,
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
            DeviceOs::Linux,
            DeviceCategory::Desktop,
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

    let err = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .expect_err("receiver must receive size mismatch error");
    let msg = err.to_string();
    assert!(msg.contains("shot_1.png"), "unexpected error: {msg}");
    assert!(msg.contains("5"), "unexpected error: {msg}");
    assert!(msg.contains("12"), "unexpected error: {msg}");

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct SingleOpenSafProvider {
    checked_file: PathBuf,
    stale_file: PathBuf,
    open_calls: std::sync::atomic::AtomicUsize,
}

impl ShareContentProvider for SingleOpenSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![ShareEntry {
            name: "shot:1.png".into(),
            rel_path: "shot:1.png".into(),
            is_dir: false,
            size_bytes: 5,
            modified_ms: 1,
        }])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        assert_eq!(rel_path, "shot:1.png");
        let call_idx = self
            .open_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = if call_idx == 0 {
            &self.checked_file
        } else {
            &self.stale_file
        };
        let file = std::fs::File::open(path)?;
        let size_bytes = file.metadata()?.len();
        Ok(ShareOpenedFile {
            file,
            file_name: "shot:1.png".into(),
            size_bytes,
            modified_ms: 1,
        })
    }
}

#[tokio::test]
async fn shares_folder_download_streams_checked_handle_without_reopening() {
    let dir_a = temp_dir("single-open-a");
    let dir_b = temp_dir("single-open-b");
    std::fs::create_dir_all(&dir_b).unwrap();
    let checked_file = dir_b.join("checked.bin");
    let stale_file = dir_b.join("stale.bin");
    std::fs::write(&checked_file, b"hello").unwrap();
    std::fs::write(&stale_file, b"twelve-bytes").unwrap();

    let provider = Arc::new(SingleOpenSafProvider {
        checked_file,
        stale_file,
        open_calls: std::sync::atomic::AtomicUsize::new(0),
    });

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
    shares_b.set_content_provider(provider.clone());
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3ASingleOpen".into(),
            "SingleOpen".into(),
            true,
        )
        .unwrap();

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
            DeviceOs::Android,
            DeviceCategory::Phone,
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
            DeviceOs::Linux,
            DeviceCategory::Desktop,
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

    let res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .expect("folder download should stream from the checked handle");
    assert_eq!(
        provider
            .open_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "provider.open_file must be called only once per file"
    );
    assert_eq!(
        std::fs::read(PathBuf::from(&res.destination).join("shot_1.png")).unwrap(),
        b"hello"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct SequentialBudgetSafProvider {
    backing_file: PathBuf,
    file_count: usize,
}

impl ShareContentProvider for SequentialBudgetSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok((0..self.file_count)
            .map(|i| ShareEntry {
                name: format!("item_{i:02}.txt"),
                rel_path: format!("item_{i:02}.txt"),
                is_dir: false,
                size_bytes: 7,
                modified_ms: 1,
            })
            .collect())
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Exclusive read access: fails if any previous source handle is still open.
            opts.share_mode(0);
        }
        let file = opts.open(&self.backing_file)?;
        let size_bytes = file.metadata()?.len();
        Ok(ShareOpenedFile {
            file,
            file_name: "item.txt".into(),
            size_bytes,
            modified_ms: 1,
        })
    }
}

#[tokio::test]
async fn shares_folder_download_closes_each_source_before_opening_next() {
    let dir_a = temp_dir("budget-a");
    let dir_b = temp_dir("budget-b");
    std::fs::create_dir_all(&dir_b).unwrap();
    let backing_file = dir_b.join("shared.txt");
    std::fs::write(&backing_file, b"payload").unwrap();

    let file_count = 12usize;
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
    shares_b.set_content_provider(Arc::new(SequentialBudgetSafProvider {
        backing_file,
        file_count,
    }));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3ABudget".into(),
            "Budget".into(),
            true,
        )
        .unwrap();

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
            DeviceOs::Android,
            DeviceCategory::Phone,
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
            DeviceOs::Linux,
            DeviceCategory::Desktop,
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

    let res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .expect("folder download must close each source handle before opening the next");
    assert_eq!(res.file_count, file_count);
    assert_eq!(
        std::fs::read(PathBuf::from(&res.destination).join("item_11.txt")).unwrap(),
        b"payload"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct TrailingBytesSafProvider {
    backing_file: PathBuf,
}

impl ShareContentProvider for TrailingBytesSafProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![ShareEntry {
            name: "shot:1.png".into(),
            rel_path: "shot:1.png".into(),
            is_dir: false,
            size_bytes: 5,
            modified_ms: 1,
        }])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        assert_eq!(rel_path, "shot:1.png");
        let file = std::fs::File::open(&self.backing_file)?;
        Ok(ShareOpenedFile {
            file,
            file_name: "shot:1.png".into(),
            // Both manifest and open_file claim 5 bytes, while the descriptor yields 12 bytes.
            size_bytes: 5,
            modified_ms: 1,
        })
    }
}

#[tokio::test]
async fn shares_folder_download_rejects_descriptor_with_trailing_bytes() {
    let dir_a = temp_dir("trailing-a");
    let dir_b = temp_dir("trailing-b");
    std::fs::create_dir_all(&dir_b).unwrap();
    let backing_file = dir_b.join("grown.bin");
    std::fs::write(&backing_file, b"twelve-bytes").unwrap();

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
    shares_b.set_content_provider(Arc::new(TrailingBytesSafProvider { backing_file }));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3ATrailing".into(),
            "Trailing".into(),
            true,
        )
        .unwrap();

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
            DeviceOs::Android,
            DeviceCategory::Phone,
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
            DeviceOs::Linux,
            DeviceCategory::Desktop,
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

    let err = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .expect_err("receiver must reject descriptor with trailing bytes beyond declared size");
    let msg = err.to_string();
    assert!(msg.contains("shot_1.png"), "unexpected error: {msg}");
    assert!(msg.contains("5"), "unexpected error: {msg}");
    assert!(msg.contains("12"), "unexpected error: {msg}");

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

struct AndroidSafMockProvider {
    backing_dir: PathBuf,
}

impl ShareContentProvider for AndroidSafMockProvider {
    fn list_dir(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(Vec::new())
    }

    fn list_tree(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        _rel_path: &str,
        _max_depth: usize,
        _max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        Ok(vec![
            ShareEntry {
                name: "empty_root_sub".into(),
                rel_path: "empty_root_sub".into(),
                is_dir: true,
                size_bytes: 0,
                modified_ms: 1,
            },
            ShareEntry {
                name: "DCIM:2026".into(),
                rel_path: "DCIM:2026".into(),
                is_dir: true,
                size_bytes: 0,
                modified_ms: 1,
            },
            ShareEntry {
                name: "Camera".into(),
                rel_path: "DCIM:2026/Camera".into(),
                is_dir: true,
                size_bytes: 0,
                modified_ms: 1,
            },
            ShareEntry {
                name: "empty_nested".into(),
                rel_path: "DCIM:2026/empty_nested".into(),
                is_dir: true,
                size_bytes: 0,
                modified_ms: 1,
            },
            ShareEntry {
                name: "shot:12:30.png".into(),
                rel_path: "shot:12:30.png".into(),
                is_dir: false,
                size_bytes: 18,
                modified_ms: 1,
            },
            ShareEntry {
                name: "clip:01.mp4".into(),
                rel_path: "DCIM:2026/Camera/clip:01.mp4".into(),
                is_dir: false,
                size_bytes: 24,
                modified_ms: 1,
            },
        ])
    }

    fn open_file(
        &self,
        _root_uri: &str,
        _is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError> {
        let disk_name = match rel_path {
            "shot:12:30.png" => "shot.png",
            "DCIM:2026/Camera/clip:01.mp4" => "clip.mp4",
            other => return Err(CoreError::NotFound(format!("unexpected rel_path: {other}"))),
        };
        let file = std::fs::File::open(self.backing_dir.join(disk_name))?;
        let size_bytes = file.metadata()?.len();
        Ok(ShareOpenedFile {
            file,
            file_name: rel_path.rsplit('/').next().unwrap_or(rel_path).into(),
            size_bytes,
            modified_ms: 1,
        })
    }
}

#[tokio::test]
async fn saf_android_folder_download_with_colons_empty_dirs_and_blake3() {
    let dir_a = temp_dir("saf-android-a");
    let dir_b = temp_dir("saf-android-b");
    let backing_dir = dir_b.join("saf_backing");
    std::fs::create_dir_all(&backing_dir).unwrap();

    let shot_bytes = b"png-shot-12-30-ok!";
    let clip_bytes = b"mp4-clip-01-nested-data!";
    assert_eq!(shot_bytes.len(), 18);
    assert_eq!(clip_bytes.len(), 24);
    std::fs::write(backing_dir.join("shot.png"), shot_bytes).unwrap();
    std::fs::write(backing_dir.join("clip.mp4"), clip_bytes).unwrap();

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("downloads"),
        default_device_name: "PC".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "Xiaomi".into(),
    })
    .unwrap();

    let shares_a = SharesModule::new(Arc::clone(engine_a.storage()));
    let shares_b = SharesModule::new(Arc::clone(engine_b.storage()));
    shares_b.set_content_provider(Arc::new(AndroidSafMockProvider { backing_dir }));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share_uri(
            "content://com.android.externalstorage.documents/tree/primary%3APictures".into(),
            "Pictures".into(),
            true,
        )
        .unwrap();

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
            "Xiaomi",
            DeviceOs::Android,
            DeviceCategory::Phone,
            Some("127.0.0.1"),
            Some(info_b.quic_port),
        )
        .unwrap();
    engine_b
        .storage()
        .upsert_peer(
            &peer_a,
            &hex_decode_32(&info_a.public_key_hex),
            "PC",
            DeviceOs::Windows,
            DeviceCategory::Desktop,
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

    let res = shares_a
        .download_remote_folder(&engine_a, &info_b.peer_id, &root.id, "")
        .await
        .expect("Android SAF folder with colons and empty subdirs must download cleanly");
    assert_eq!(res.folder_name, "Pictures");
    assert_eq!(res.file_count, 2);
    assert_eq!(res.total_bytes, 42);

    let dest = PathBuf::from(&res.destination);
    assert!(dest.join("empty_root_sub").is_dir());
    assert!(dest.join("DCIM_2026/empty_nested").is_dir());

    let downloaded_shot = std::fs::read(dest.join("shot_12_30.png")).unwrap();
    let downloaded_clip = std::fs::read(dest.join("DCIM_2026/Camera/clip_01.mp4")).unwrap();
    assert_eq!(
        blake3::hash(&downloaded_shot),
        blake3::hash(shot_bytes),
        "BLAKE3 mismatch for shot_12_30.png"
    );
    assert_eq!(
        blake3::hash(&downloaded_clip),
        blake3::hash(clip_bytes),
        "BLAKE3 mismatch for clip_01.mp4"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn shares_single_file_download_uses_publish_hook_and_cleans_temp() {
    let dir_a = temp_dir("single-hook-a");
    let dir_b = temp_dir("single-hook-b");
    let shared_dir = dir_b.join("shared");
    std::fs::create_dir_all(&shared_dir).unwrap();
    let payload = b"single-file-mediastore-payload";
    std::fs::write(shared_dir.join("report.pdf"), payload).unwrap();

    let engine_a = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_a.clone(),
        default_download_dir: dir_a.join("private_downloads"),
        default_device_name: "A".into(),
    })
    .unwrap();
    let engine_b = LandaEngine::new_loopback(LandaConfig {
        app_data_dir: dir_b.clone(),
        default_download_dir: dir_b.join("downloads"),
        default_device_name: "B".into(),
    })
    .unwrap();

    let published_calls = Arc::new(std::sync::Mutex::new(Vec::<(PathBuf, String, Vec<u8>)>::new()));
    let published_calls_clone = Arc::clone(&published_calls);
    let expected_temp_dir = dir_a.join("incoming_temp");
    engine_a.set_receive_publish_hook(Arc::new(move |source, relative| {
        assert!(
            source.starts_with(&expected_temp_dir),
            "single file must be staged in incoming_temp, got {}",
            source.display()
        );
        let bytes = std::fs::read(source).expect("staged temp file must exist during publish");
        let rel_str = relative.to_string_lossy().replace('\\', "/");
        published_calls_clone
            .lock()
            .unwrap()
            .push((source.to_path_buf(), rel_str.clone(), bytes));
        Some(format!("content://media/external/downloads/{rel_str}"))
    }));

    let shares_a = SharesModule::new(Arc::clone(engine_a.storage()));
    let shares_b = SharesModule::new(Arc::clone(engine_b.storage()));
    engine_a.register_module(shares_a.clone());
    engine_b.register_module(shares_b.clone());

    let root = shares_b
        .add_local_share(shared_dir.clone(), Some("Docs".into()))
        .unwrap();

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

    let saved = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root.id, "report.pdf")
        .await
        .unwrap();
    assert_eq!(saved, "content://media/external/downloads/report.pdf");

    let calls = published_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "report.pdf");
    assert_eq!(calls[0].2, payload);
    assert!(
        !calls[0].0.exists(),
        "staged temp file must be deleted immediately after publish"
    );
    drop(calls);

    assert!(
        !dir_a.join("private_downloads/report.pdf").exists(),
        "file must not be written to private download_dir when publish hook is active"
    );

    let history = engine_a.list_history(10).unwrap();
    assert_eq!(
        history[0].saved_paths,
        vec!["content://media/external/downloads/report.pdf".to_string()]
    );
    assert_eq!(history[0].relative_paths, vec!["report.pdf".to_string()]);

    // If publish hook fails (returns None), download must fail and clean up temp file.
    engine_a.set_receive_publish_hook(Arc::new(|_, _| None));
    let err = shares_a
        .download_remote_file(&engine_a, &info_b.peer_id, &root.id, "report.pdf")
        .await
        .expect_err("publish hook failure must fail single-file download");
    assert!(err.to_string().contains("failed to publish"));
    let remaining_temp: Vec<_> = std::fs::read_dir(dir_a.join("incoming_temp"))
        .unwrap()
        .collect();
    assert!(
        remaining_temp.is_empty(),
        "incoming_temp must be empty after failed publish"
    );

    engine_a.shutdown();
    engine_b.shutdown();
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
