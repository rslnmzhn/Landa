use std::path::PathBuf;
use std::sync::Arc;

use landa_core::{
    AppSettings, FriendRequestInfo, LandaEngine, LocalIdentityInfo, ManualTarget, ModuleStatus,
    PeerInfo, TransferHistoryRecord,
};
#[cfg(feature = "mod-clipboard")]
use landa_mod_clipboard::{ClipboardItem, ClipboardModule};
#[cfg(feature = "mod-shares")]
use landa_mod_shares::{LocalShareRoot, ShareEntry, ShareRootInfo, SharesModule};
use tauri::State;

pub struct AppState {
    pub engine: Arc<LandaEngine>,
    #[cfg(feature = "mod-shares")]
    pub shares: Arc<SharesModule>,
    #[cfg(feature = "mod-clipboard")]
    pub clipboard: Arc<ClipboardModule>,
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
    state: State<'_, AppState>,
    peer_id: String,
    accept: bool,
) -> Result<(), String> {
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
pub fn respond_transfer_offer(
    state: State<'_, AppState>,
    transfer_id: String,
    accept: bool,
) -> Result<(), String> {
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
pub fn set_module_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Vec<ModuleStatus>, String> {
    state
        .engine
        .set_module_enabled(&id, enabled)
        .map_err(|e| e.to_string())
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

// --- Platform / Android Bridge ---

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
pub fn has_all_files_access(app: tauri::AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return bridge.has_all_files_access();
        }
    }
    let _ = app;
    Ok(true)
}

#[tauri::command]
pub fn request_all_files_access(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return bridge.request_all_files_access();
        }
    }
    let _ = app;
    Ok(())
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
