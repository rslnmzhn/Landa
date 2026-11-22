use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::Mutex;

use landa_core::{
    AppSettings, FriendRequestInfo, LanInterfaceStatus, LandaEngine, LocalIdentityInfo,
    ManualTarget, ModuleStatus, PeerInfo, TransferHistoryRecord,
};
#[cfg(feature = "mod-clipboard")]
use landa_mod_clipboard::{ClipboardContentKind, ClipboardItem, ClipboardModule};
#[cfg(feature = "mod-shares")]
use landa_mod_shares::{LocalShareRoot, ShareEntry, ShareRootInfo, ShareThumbnail, SharesModule};
#[cfg(feature = "mod-videolink")]
use landa_mod_videolink::{VideoLinkModule, VideoLinkStatus, VideoSource};
use tauri::State;

pub struct AppState {
    pub engine: Arc<LandaEngine>,
    #[cfg(feature = "mod-shares")]
    pub shares: Arc<SharesModule>,
    #[cfg(feature = "mod-clipboard")]
    pub clipboard: Arc<ClipboardModule>,
    #[cfg(feature = "mod-videolink")]
    pub videolink: Arc<VideoLinkModule>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub desktop_clipboard: Mutex<Option<arboard::Clipboard>>,
}

#[tauri::command]
pub fn get_local_identity(state: State<'_, AppState>) -> LocalIdentityInfo {
    state.engine.local_identity_info()
}

#[tauri::command]
pub fn list_peers(state: State<'_, AppState>) -> Result<Vec<PeerInfo>, String> {
    state.engine.list_peers().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn trigger_scan(state: State<'_, AppState>) {
    state.engine.trigger_scan();
}

#[tauri::command]
pub fn rename_peer(
    state: State<'_, AppState>,
    peer_id: String,
    alias: Option<String>,
) -> Result<(), String> {
    state
        .engine
        .rename_peer(&peer_id, alias.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_friend_requests(state: State<'_, AppState>) -> Result<Vec<FriendRequestInfo>, String> {
    state.engine.list_friend_requests().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_friend_request(
    state: State<'_, AppState>,
    peer_id: String,
) -> Result<(), String> {
    state
        .engine
        .send_friend_request(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn respond_friend_request(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
    accept: bool,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            bridge.cancel_friend_request_notification_async(&peer_id).await;
        }
    }
    let _ = app;
    state
        .engine
        .respond_friend_request(&peer_id, accept)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unfriend(state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    state
        .engine
        .unfriend(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_files(
    state: State<'_, AppState>,
    peer_id: String,
    paths: Vec<String>,
) -> Result<TransferHistoryRecord, String> {
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    state
        .engine
        .send_files(&peer_id, path_bufs)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_folder_android(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    peer_id: String,
) -> Result<Option<TransferHistoryRecord>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let Some(items) = bridge.pick_folder_for_send_async().await? else {
            return Ok(None);
        };
        if items.is_empty() {
            return Err("Selected folder is empty".into());
        }
        let record = state
            .engine
            .send_items(&peer_id, items)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Some(record))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, state, peer_id);
        Err("send_folder_android is only supported on Android".into())
    }
}

#[tauri::command]
pub async fn respond_transfer_offer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    transfer_id: String,
    accept: bool,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            bridge
                .cancel_transfer_offer_notification_async(&transfer_id)
                .await;
        }
    }
    let _ = app;
    state
        .engine
        .respond_transfer_offer(&transfer_id, accept)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_transfer(state: State<'_, AppState>, transfer_id: String) -> bool {
    state.engine.cancel_transfer(&transfer_id)
}

#[tauri::command]
pub fn list_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<TransferHistoryRecord>, String> {
    state
        .engine
        .list_history(limit.unwrap_or(100))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.clear_history().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.engine.get_settings()
}

#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    state
        .engine
        .update_settings(settings)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_modules(state: State<'_, AppState>) -> Vec<ModuleStatus> {
    state.engine.list_modules()
}

#[tauri::command]
pub async fn set_module_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Vec<ModuleStatus>, String> {
    let statuses = state
        .engine
        .set_module_enabled(&id, enabled)
        .map_err(|e| e.to_string())?;
    #[cfg(feature = "mod-videolink")]
    if id == "videolink" && !enabled {
        state.videolink.stop().await;
    }
    Ok(statuses)
}

#[tauri::command]
pub fn list_manual_targets(state: State<'_, AppState>) -> Result<Vec<ManualTarget>, String> {
    state.engine.list_manual_targets().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_manual_target(
    state: State<'_, AppState>,
    ip: String,
    label: Option<String>,
) -> Result<(), String> {
    state
        .engine
        .add_manual_target(&ip, label.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_manual_target(state: State<'_, AppState>, ip: String) -> Result<(), String> {
    state
        .engine
        .remove_manual_target(&ip)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_lan_interfaces(state: State<'_, AppState>) -> Vec<LanInterfaceStatus> {
    state.engine.list_lan_interfaces()
}

#[tauri::command]
pub async fn open_logs(app: tauri::AppHandle) -> Result<(), String> {
    let log_path = crate::logging::ensure_log_file()?;
    crate::preview::open_file_external(&app, &log_path.to_string_lossy()).await
}

#[tauri::command]
pub async fn copy_diagnostics(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let log_tail = crate::logging::read_log_tail(200);
    let report = build_diagnostics_report(&state.engine, &log_tail);
    write_text_to_system_clipboard(&app, &state, &report).await?;
    Ok(report)
}

pub fn build_diagnostics_report(engine: &LandaEngine, log_tail: &str) -> String {
    use std::fmt::Write;

    let identity = engine.local_identity_info();
    let settings = engine.get_settings();
    let interfaces = engine.list_lan_interfaces();
    let manual_targets = engine.list_manual_targets().unwrap_or_default();
    let modules = engine.list_modules();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "Landa v{} ({}/{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(out, "Device Name: {}", identity.device_name);
    let _ = writeln!(out, "Peer ID: {}", identity.peer_id);
    let _ = writeln!(out, "QUIC Port: {}", identity.quic_port);
    let _ = writeln!(out, "App Data Dir: {}", engine.app_data_dir().display());
    let _ = writeln!(out, "Download Dir: {}", settings.download_dir);

    let _ = writeln!(out, "\nLAN Interfaces ({}):", interfaces.len());
    if interfaces.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for iface in &interfaces {
            let state_label = if iface.enabled { "enabled" } else { "excluded" };
            let _ = writeln!(
                out,
                "  - {} | ip={} | subnet={} | broadcast={} | {}",
                iface.name, iface.ip, iface.subnet_cidr, iface.broadcast, state_label
            );
        }
    }

    let _ = writeln!(
        out,
        "\nExcluded Subnets ({}):",
        settings.excluded_subnets.len()
    );
    if settings.excluded_subnets.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for cidr in &settings.excluded_subnets {
            let _ = writeln!(out, "  - {cidr}");
        }
    }

    let _ = writeln!(out, "\nManual Targets ({}):", manual_targets.len());
    if manual_targets.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for target in &manual_targets {
            let _ = writeln!(out, "  - {}", target.ip);
        }
    }

    let _ = writeln!(out, "\nModules:");
    for m in &modules {
        let _ = writeln!(
            out,
            "  - {}: enabled={}, available={}",
            m.id, m.enabled, m.available
        );
    }

    let _ = writeln!(out, "\nRecent Logs (landa.log):");
    if log_tail.trim().is_empty() {
        let _ = writeln!(out, "  (empty)");
    } else {
        let _ = writeln!(out, "{log_tail}");
    }

    out
}

async fn write_text_to_system_clipboard(
    app: &tauri::AppHandle,
    state: &AppState,
    text: &str,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let _ = state;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        bridge.write_system_clipboard_text_async(text).await
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        let mut guard = state
            .desktop_clipboard
            .lock()
            .map_err(|_| "Clipboard lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
        let cb = guard
            .as_mut()
            .ok_or_else(|| "System clipboard unavailable".to_string())?;
        cb.set_text(text.to_string())
            .map_err(|e| format!("Failed to copy text: {e}"))
    }
}

// --- Optional Module: Shares ---

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub fn shares_list_local(state: State<'_, AppState>) -> Result<Vec<LocalShareRoot>, String> {
    state.shares.list_local_shares().map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub fn shares_list_local() -> Result<Vec<serde_json::Value>, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub fn shares_add_local(
    state: State<'_, AppState>,
    path: String,
    name: Option<String>,
) -> Result<LocalShareRoot, String> {
    state
        .shares
        .add_local_share(PathBuf::from(path), name)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub fn shares_add_local(_path: String, _name: Option<String>) -> Result<serde_json::Value, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_pick_folder_android(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<LocalShareRoot>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let Some((uri, name, is_dir)) = bridge.pick_share_folder_async().await? else {
            return Ok(None);
        };
        let root = state
            .shares
            .add_local_share_uri(uri, name, is_dir)
            .map_err(|e| e.to_string())?;
        Ok(Some(root))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, state);
        Err("shares_pick_folder_android is only supported on Android".into())
    }
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_pick_folder_android() -> Result<Option<serde_json::Value>, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_pick_file_android(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<LocalShareRoot>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let Some((uri, name, is_dir)) = bridge.pick_share_file_async().await? else {
            return Ok(None);
        };
        let root = state
            .shares
            .add_local_share_uri(uri, name, is_dir)
            .map_err(|e| e.to_string())?;
        Ok(Some(root))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, state);
        Err("shares_pick_file_android is only supported on Android".into())
    }
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_pick_file_android() -> Result<Option<serde_json::Value>, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub fn shares_remove_local(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.shares.remove_local_share(&id).map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub fn shares_remove_local(_id: String) -> Result<(), String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_browse_remote_roots(
    state: State<'_, AppState>,
    peer_id: String,
) -> Result<Vec<ShareRootInfo>, String> {
    state
        .shares
        .browse_remote_roots(&state.engine, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_browse_remote_roots(_peer_id: String) -> Result<Vec<serde_json::Value>, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_browse_remote_dir(
    state: State<'_, AppState>,
    peer_id: String,
    root_id: String,
    rel_path: String,
) -> Result<Vec<ShareEntry>, String> {
    state
        .shares
        .browse_remote_dir(&state.engine, &peer_id, &root_id, &rel_path)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_browse_remote_dir(
    _peer_id: String,
    _root_id: String,
    _rel_path: String,
) -> Result<Vec<serde_json::Value>, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_download_remote_file(
    state: State<'_, AppState>,
    peer_id: String,
    root_id: String,
    rel_path: String,
) -> Result<String, String> {
    state
        .shares
        .download_remote_file(&state.engine, &peer_id, &root_id, &rel_path)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_download_remote_file(
    _peer_id: String,
    _root_id: String,
    _rel_path: String,
) -> Result<String, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_preview_remote_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    server: State<'_, Arc<crate::preview::PreviewServer>>,
    peer_id: String,
    root_id: String,
    rel_path: String,
) -> Result<crate::preview::PreviewDescriptor, String> {
    let cache_dir = crate::preview::resolve_preview_cache_dir(&app)?;
    let cached_path = state
        .shares
        .preview_remote_file(
            &state.engine,
            &peer_id,
            &root_id,
            &rel_path,
            &cache_dir,
            crate::preview::MAX_REMOTE_PREVIEW_BYTES,
        )
        .await
        .map_err(|e| e.to_string())?;
    crate::preview::prepare_file_preview(&app, &state, &server, &cached_path).await
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_preview_remote_file(
    _peer_id: String,
    _root_id: String,
    _rel_path: String,
) -> Result<crate::preview::PreviewDescriptor, String> {
    Err("shares module is not compiled".into())
}

#[cfg(feature = "mod-shares")]
#[tauri::command]
pub async fn shares_get_remote_thumbnail(
    state: State<'_, AppState>,
    peer_id: String,
    root_id: String,
    rel_path: String,
    max_size: Option<u32>,
) -> Result<ShareThumbnail, String> {
    state
        .shares
        .get_remote_thumbnail(
            &state.engine,
            &peer_id,
            &root_id,
            &rel_path,
            max_size.unwrap_or(160),
        )
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_get_remote_thumbnail(
    _peer_id: String,
    _root_id: String,
    _rel_path: String,
    _max_size: Option<u32>,
) -> Result<serde_json::Value, String> {
    Err("shares module is not compiled".into())
}

// --- Optional Module: Clipboard ---

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub fn clipboard_list_local(state: State<'_, AppState>) -> Vec<ClipboardItem> {
    state.clipboard.list_local_items()
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub fn clipboard_list_local() -> Vec<serde_json::Value> {
    Vec::new()
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub fn clipboard_push_local_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<ClipboardItem, String> {
    state
        .clipboard
        .push_local_text(text, None, None)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub fn clipboard_push_local_text(_text: String) -> Result<serde_json::Value, String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub fn clipboard_delete_local(state: State<'_, AppState>, id: String) {
    state.clipboard.delete_local_item(&id);
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub fn clipboard_delete_local(_id: String) {}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_send_to_peer(
    state: State<'_, AppState>,
    peer_id: String,
    text: String,
) -> Result<(), String> {
    state
        .clipboard
        .send_text_to_peer(&state.engine, &peer_id, text)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_send_to_peer(_peer_id: String, _text: String) -> Result<(), String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_fetch_remote(
    state: State<'_, AppState>,
    peer_id: String,
) -> Result<Vec<ClipboardItem>, String> {
    state
        .clipboard
        .fetch_remote_items(&state.engine, &peer_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_fetch_remote(_peer_id: String) -> Result<Vec<serde_json::Value>, String> {
    Err("clipboard module is not compiled".into())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClipboardPasteResult {
    Empty,
    Text {
        text: String,
    },
    #[cfg(feature = "mod-clipboard")]
    Image {
        item: ClipboardItem,
    },
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub fn clipboard_push_local_image_bytes(
    state: State<'_, AppState>,
    bytes: Vec<u8>,
) -> Result<ClipboardItem, String> {
    state
        .clipboard
        .push_local_image(&bytes, None, None)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub fn clipboard_push_local_image_bytes(_bytes: Vec<u8>) -> Result<serde_json::Value, String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_send_image_to_peer(
    state: State<'_, AppState>,
    peer_id: String,
    item_id: String,
) -> Result<(), String> {
    state
        .clipboard
        .send_local_image_to_peer(&state.engine, &peer_id, &item_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_send_image_to_peer(
    _peer_id: String,
    _item_id: String,
) -> Result<(), String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_paste_from_system(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ClipboardPasteResult, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let res = bridge.read_system_clipboard_async().await?;
        match res.kind.as_str() {
            "image" => {
                let temp_path = res
                    .temp_path
                    .ok_or_else(|| "Missing clipboard temp image path".to_string())?;
                let bytes = tokio::fs::read(&temp_path)
                    .await
                    .map_err(|e| format!("Failed to read clipboard image: {e}"))?;
                let _ = tokio::fs::remove_file(&temp_path).await;
                let item = state
                    .clipboard
                    .push_local_image(&bytes, None, None)
                    .map_err(|e| e.to_string())?;
                Ok(ClipboardPasteResult::Image { item })
            }
            "text" => {
                let text = res.text.unwrap_or_default();
                if text.trim().is_empty() {
                    Ok(ClipboardPasteResult::Empty)
                } else {
                    Ok(ClipboardPasteResult::Text { text })
                }
            }
            _ => Ok(ClipboardPasteResult::Empty),
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        let mut guard = state
            .desktop_clipboard
            .lock()
            .map_err(|_| "Clipboard lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
        let Some(cb) = guard.as_mut() else {
            return Err("System clipboard unavailable".into());
        };

        if let Ok(img) = cb.get_image() {
            let width = u32::try_from(img.width)
                .map_err(|_| "Clipboard image width overflow".to_string())?;
            let height = u32::try_from(img.height)
                .map_err(|_| "Clipboard image height overflow".to_string())?;
            let item = state
                .clipboard
                .push_local_rgba_image(width, height, img.bytes.as_ref())
                .map_err(|e| e.to_string())?;
            return Ok(ClipboardPasteResult::Image { item });
        }

        if let Ok(text) = cb.get_text() {
            if !text.trim().is_empty() {
                return Ok(ClipboardPasteResult::Text { text });
            }
        }

        Ok(ClipboardPasteResult::Empty)
    }
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_paste_from_system() -> Result<ClipboardPasteResult, String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_copy_to_system(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    item_id: String,
    peer_id: Option<String>,
    text: Option<String>,
) -> Result<(), String> {
    let (is_image, text_val, image_path) = if let Some(ref pid) = peer_id {
        if let Some(t) = text.filter(|s| !s.is_empty()) {
            (false, Some(t), None)
        } else {
            let cached = ensure_remote_clipboard_image_cached(&app, &state, pid, &item_id).await?;
            (true, None, Some(cached))
        }
    } else {
        let item = state
            .clipboard
            .get_local_item(&item_id)
            .ok_or_else(|| format!("Clipboard item {item_id} not found"))?;
        match item.kind {
            ClipboardContentKind::Text => (false, Some(item.text), None),
            ClipboardContentKind::Image => (true, None, item.image_path),
        }
    };

    if !is_image {
        let text_to_copy = text_val.unwrap_or_default();
        return write_text_to_system_clipboard(&app, &state, &text_to_copy).await;
    }

    let path = image_path.ok_or_else(|| "Clipboard image path missing".to_string())?;
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        bridge.write_system_clipboard_image_async(&path).await
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = app;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("Failed to read clipboard image: {e}"))?;
        let rgba = tokio::task::spawn_blocking(move || {
            let img = image::load_from_memory(&bytes)
                .map_err(|e| format!("Failed to decode clipboard image: {e}"))?;
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            Ok::<_, String>((w as usize, h as usize, rgba.into_raw()))
        })
        .await
        .map_err(|e| format!("Image decode task failed: {e}"))??;

        let mut guard = state
            .desktop_clipboard
            .lock()
            .map_err(|_| "Clipboard lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
        let cb = guard
            .as_mut()
            .ok_or_else(|| "System clipboard unavailable".to_string())?;
        cb.set_image(arboard::ImageData {
            width: rgba.0,
            height: rgba.1,
            bytes: std::borrow::Cow::Owned(rgba.2),
        })
        .map_err(|e| format!("Failed to copy image: {e}"))
    }
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_copy_to_system(
    _item_id: String,
    _peer_id: Option<String>,
    _text: Option<String>,
) -> Result<(), String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
#[tauri::command]
pub async fn clipboard_get_image_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    server: State<'_, Arc<crate::preview::PreviewServer>>,
    item_id: String,
    peer_id: Option<String>,
) -> Result<crate::preview::PreviewDescriptor, String> {
    let path = if let Some(pid) = peer_id {
        ensure_remote_clipboard_image_cached(&app, &state, &pid, &item_id).await?
    } else {
        let item = state
            .clipboard
            .get_local_item(&item_id)
            .ok_or_else(|| format!("Clipboard item {item_id} not found"))?;
        item.image_path
            .ok_or_else(|| "Clipboard item has no image".to_string())?
    };
    crate::preview::prepare_file_preview(&app, &state, &server, &path).await
}

#[cfg(not(feature = "mod-clipboard"))]
#[tauri::command]
pub async fn clipboard_get_image_preview(
    _item_id: String,
    _peer_id: Option<String>,
) -> Result<crate::preview::PreviewDescriptor, String> {
    Err("clipboard module is not compiled".into())
}

#[cfg(feature = "mod-clipboard")]
async fn ensure_remote_clipboard_image_cached(
    app: &tauri::AppHandle,
    state: &AppState,
    peer_id: &str,
    item_id: &str,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let cache_dir = crate::preview::resolve_preview_cache_dir(app)?;
    tokio::fs::create_dir_all(&cache_dir)
        .await
        .map_err(|e| format!("Failed to create preview cache dir: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(peer_id.as_bytes());
    hasher.update(b":");
    hasher.update(item_id.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let cached_path = cache_dir.join(format!("clip_{hex}.png"));

    if tokio::fs::try_exists(&cached_path).await.unwrap_or(false) {
        return Ok(cached_path.to_string_lossy().to_string());
    }

    let bytes = state
        .clipboard
        .fetch_remote_image(&state.engine, peer_id, item_id)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&cached_path, &bytes)
        .await
        .map_err(|e| format!("Failed to cache remote clipboard image: {e}"))?;
    Ok(cached_path.to_string_lossy().to_string())
}

// --- Platform / Android Bridge ---

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFocusTargetDto {
    pub target: String,
    pub transfer_id: Option<String>,
    pub peer_id: Option<String>,
}

#[tauri::command]
pub fn take_pending_focus_target(
    app: tauri::AppHandle,
) -> Result<Option<PendingFocusTargetDto>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return bridge.take_pending_focus_target().map(|opt| {
                opt.map(|f| PendingFocusTargetDto {
                    target: f.target,
                    transfer_id: f.transfer_id,
                    peer_id: f.peer_id,
                })
            });
        }
    }
    let _ = app;
    Ok(None)
}

#[tauri::command]
pub fn take_pending_shared_files(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return bridge.take_pending_shared_files();
        }
    }
    let _ = app;
    Ok(Vec::new())
}

#[tauri::command]
pub async fn ensure_startup_permissions(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return bridge.ensure_startup_permissions_async().await;
        }
    }
    let _ = app;
    Ok(())
}

// --- App Auto-Update ---

#[tauri::command]
pub async fn check_for_updates() -> Result<crate::updater::UpdateCheckResult, String> {
    crate::updater::check_for_updates().await
}

#[tauri::command]
pub async fn download_update(
    app: tauri::AppHandle,
    asset: crate::updater::UpdateAssetInfo,
) -> Result<String, String> {
    crate::updater::download_and_verify_update(&app, asset).await
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    crate::updater::install_update(&app, &file_path).await
}

// --- In-App File Preview ---

#[tauri::command]
pub async fn prepare_file_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    server: State<'_, Arc<crate::preview::PreviewServer>>,
    path_or_uri: String,
) -> Result<crate::preview::PreviewDescriptor, String> {
    crate::preview::prepare_file_preview(&app, &state, &server, &path_or_uri).await
}

// --- QR Quick Connect ---

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrConnectInfo {
    pub uri: String,
    pub svg: String,
    pub ip: String,
    pub port: u16,
}

#[tauri::command]
pub fn get_qr_connect_info(state: State<'_, AppState>) -> Result<QrConnectInfo, String> {
    let (uri, ip, port) = state
        .engine
        .local_qr_connect_uri()
        .map_err(|e| e.to_string())?;
    let code = qrcode::QrCode::new(uri.as_bytes())
        .map_err(|e| format!("Failed to generate QR code: {e}"))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(qrcode::render::svg::Color("#0f172a"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(QrConnectInfo { uri, svg, ip, port })
}

#[tauri::command]
pub async fn connect_via_qr_uri(
    state: State<'_, AppState>,
    uri: String,
) -> Result<PeerInfo, String> {
    state
        .engine
        .connect_via_qr_uri(&uri)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn scan_qr_and_connect_android(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<PeerInfo>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let Some(uri) = bridge.scan_qr_code_async().await? else {
            return Ok(None);
        };
        let peer = state
            .engine
            .connect_via_qr_uri(&uri)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Some(peer))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, state);
        Err("QR camera scanner is only supported on Android".into())
    }
}

#[tauri::command]
pub async fn open_file_external(
    app: tauri::AppHandle,
    path_or_uri: String,
) -> Result<(), String> {
    crate::preview::open_file_external(&app, &path_or_uri).await
}

// --- Optional Module: Video Link ---

#[cfg(feature = "mod-videolink")]
#[tauri::command]
pub fn videolink_status(state: State<'_, AppState>) -> VideoLinkStatus {
    state.videolink.status()
}

#[cfg(not(feature = "mod-videolink"))]
#[tauri::command]
pub fn videolink_status() -> serde_json::Value {
    serde_json::json!({
        "running": false,
        "url": null,
        "port": null,
        "fileName": null,
        "sizeBytes": null
    })
}

#[cfg(feature = "mod-videolink")]
#[tauri::command]
pub async fn videolink_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path_or_uri: String,
    password: String,
) -> Result<VideoLinkStatus, String> {
    let trimmed = path_or_uri.trim();
    if trimmed.is_empty() {
        return Err("Video file path is empty".into());
    }

    let source = if trimmed.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            use tauri::Manager;
            let bridge = app
                .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
                .ok_or_else(|| "Android bridge unavailable".to_string())?;
            let meta = bridge.get_content_uri_meta_async(trimmed).await?;
            VideoSource::ContentUri {
                uri: trimmed.to_string(),
                file_name: meta.name,
                size_bytes: meta.size_bytes,
                mime_type: Some(meta.mime_type),
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = app;
            return Err("content:// URIs are only supported on Android".into());
        }
    } else {
        let _ = app;
        VideoSource::File(PathBuf::from(trimmed))
    };

    state
        .videolink
        .start(source, password)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-videolink"))]
#[tauri::command]
pub async fn videolink_start(
    _path_or_uri: String,
    _password: String,
) -> Result<serde_json::Value, String> {
    Err("videolink module is not compiled".into())
}

#[cfg(feature = "mod-videolink")]
#[tauri::command]
pub async fn videolink_stop(state: State<'_, AppState>) -> Result<VideoLinkStatus, String> {
    Ok(state.videolink.stop().await)
}

#[cfg(not(feature = "mod-videolink"))]
#[tauri::command]
pub async fn videolink_stop() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "running": false,
        "url": null,
        "port": null,
        "fileName": null,
        "sizeBytes": null
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use landa_core::LandaConfig;

    #[tokio::test]
    async fn diagnostics_report_includes_identity_subnets_and_logs() {
        let temp_dir = std::env::temp_dir().join(format!("landa-diag-{}", uuid::Uuid::new_v4()));
        let engine = LandaEngine::new_loopback(LandaConfig {
            app_data_dir: temp_dir.clone(),
            default_download_dir: temp_dir.join("dl"),
            default_device_name: "DiagNode".into(),
        })
        .unwrap();

        let mut settings = engine.get_settings();
        settings.excluded_subnets = vec!["192.168.56.12/24".into()];
        let saved = engine.update_settings(settings).unwrap();
        assert_eq!(saved.excluded_subnets, vec!["192.168.56.0/24".to_string()]);

        let report = build_diagnostics_report(&engine, "2026-11-23 INFO test log line");
        assert!(report.contains("Device Name: DiagNode"));
        assert!(report.contains("192.168.56.0/24"));
        assert!(report.contains("2026-11-23 INFO test log line"));

        engine.shutdown();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
