use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use landa_core::{
    AppSettings, FriendRequestInfo, LanInterfaceStatus, LandaEngine, LocalIdentityInfo,
    ManualTarget, ModuleStatus, PeerInfo, TransferHistoryRecord,
};
#[cfg(feature = "mod-clipboard")]
use landa_mod_clipboard::{ClipboardContentKind, ClipboardItem, ClipboardModule};
#[cfg(feature = "mod-shares")]
use landa_mod_shares::{
    LocalShareRoot, ShareEntry, ShareFolderDownloadResult, ShareRootInfo, ShareThumbnail,
    SharesModule,
};
#[cfg(feature = "mod-videolink")]
use landa_mod_videolink::{VideoLinkModule, VideoLinkStatus, VideoSource};
use tauri::State;

#[derive(Default)]
pub struct AndroidNetworkListenerState {
    registered: AtomicBool,
    startup_permissions_fallback_claimed: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl AndroidNetworkListenerState {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    pub fn record_result(&self, res: &Result<(), String>) {
        match res {
            Ok(()) => {
                self.registered.store(true, Ordering::Release);
                if let Ok(mut guard) = self.last_error.lock() {
                    *guard = None;
                }
            }
            Err(err) => {
                self.registered.store(false, Ordering::Release);
                if let Ok(mut guard) = self.last_error.lock() {
                    *guard = Some(err.clone());
                }
            }
        }
    }

    pub fn is_registered(&self) -> bool {
        self.registered.load(Ordering::Acquire)
    }

    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    pub fn try_claim_startup_permissions_fallback(&self) -> bool {
        !self
            .startup_permissions_fallback_claimed
            .swap(true, Ordering::AcqRel)
    }

    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    pub fn release_startup_permissions_fallback(&self) {
        self.startup_permissions_fallback_claimed
            .store(false, Ordering::Release);
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|g| g.clone())
    }

    pub fn diagnostics_line(&self) -> String {
        match self.last_error() {
            Some(err) => format!(
                "Android Network Listener: registered={}, error={err}",
                self.is_registered()
            ),
            None => format!(
                "Android Network Listener: registered={}",
                self.is_registered()
            ),
        }
    }
}

#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub trait StartupScanTrigger: Send + Sync {
    fn trigger_scan(&self) -> Result<(), String>;
}

impl StartupScanTrigger for LandaEngine {
    fn trigger_scan(&self) -> Result<(), String> {
        LandaEngine::trigger_scan(self);
        Ok(())
    }
}

#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
pub async fn ensure_startup_permissions_with_deps<F, Fut, R, RFut>(
    request_permissions: F,
    re_register_listener: R,
    listener_state: &AndroidNetworkListenerState,
    scan_trigger: &dyn StartupScanTrigger,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
    R: FnOnce() -> RFut,
    RFut: std::future::Future<Output = Result<(), String>>,
{
    let listener_notified = request_permissions().await?;
    if !listener_notified {
        if !listener_state.is_registered() {
            let res = re_register_listener().await;
            listener_state.record_result(&res);
            if let Err(err) = res {
                tracing::warn!(
                    "failed to re-register Android network listener after startup permissions: {err}"
                );
            }
        }
        if listener_state.try_claim_startup_permissions_fallback() {
            if let Err(first_err) = scan_trigger.trigger_scan() {
                tracing::warn!(
                    "startup permissions fallback trigger_scan failed, retrying once: {first_err}"
                );
                if let Err(retry_err) = scan_trigger.trigger_scan() {
                    tracing::error!(
                        "startup permissions fallback trigger_scan retry failed: {retry_err}"
                    );
                    listener_state.release_startup_permissions_fallback();
                    return Err(retry_err);
                }
            }
        }
    }
    Ok(())
}

pub struct AppState {
    pub engine: Arc<LandaEngine>,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub android_network_listener: Arc<AndroidNetworkListenerState>,
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
pub fn trigger_scan(app: tauri::AppHandle, state: State<'_, AppState>) {
    #[cfg(target_os = "android")]
    {
        if !state.android_network_listener.is_registered() {
            let app_clone = app.clone();
            let engine_clone = Arc::clone(&state.engine);
            let listener_state = Arc::clone(&state.android_network_listener);
            tauri::async_runtime::spawn(async move {
                if let Err(err) = crate::register_android_network_listener_once(
                    &app_clone,
                    &engine_clone,
                    &listener_state,
                )
                .await
                {
                    tracing::warn!("retry register Android network listener failed: {err}");
                }
            });
        }
    }
    let _ = app;
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
pub fn forget_peer(state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    state
        .engine
        .forget_peer(&peer_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_friend_requests(state: State<'_, AppState>) -> Result<Vec<FriendRequestInfo>, String> {
    state
        .engine
        .list_friend_requests()
        .map_err(|e| e.to_string())
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
            bridge
                .cancel_friend_request_notification_async(&peer_id)
                .await;
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
    state
        .engine
        .list_manual_targets()
        .map_err(|e| e.to_string())
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
    #[cfg(target_os = "android")]
    let listener_diag = Some(state.android_network_listener.as_ref());
    #[cfg(not(target_os = "android"))]
    let listener_diag: Option<&AndroidNetworkListenerState> = None;
    let report = build_diagnostics_report_with_listener(&state.engine, &log_tail, listener_diag);
    write_text_to_system_clipboard(&app, &state, &report).await?;
    Ok(report)
}

pub fn build_diagnostics_report_with_listener(
    engine: &LandaEngine,
    log_tail: &str,
    android_listener: Option<&AndroidNetworkListenerState>,
) -> String {
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

    if let Some(listener) = android_listener {
        let _ = writeln!(out, "\n{}", listener.diagnostics_line());
    }

    let _ = writeln!(out, "\nRecent Logs (landa.log):");
    if log_tail.trim().is_empty() {
        let _ = writeln!(out, "  (empty)");
    } else {
        let _ = writeln!(out, "{log_tail}");
    }

    out
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn with_clipboard_retry<C, T, F, N>(
    clipboard: &Mutex<Option<C>>,
    mut create: N,
    mut op: F,
) -> Result<T, String>
where
    N: FnMut() -> Result<C, arboard::Error>,
    F: FnMut(&mut C) -> Result<T, arboard::Error>,
{
    let mut guard = clipboard
        .lock()
        .map_err(|_| "Clipboard lock poisoned".to_string())?;

    let mut last_err: Option<String> = None;
    for attempt in 0..3 {
        if guard.is_none() {
            match create() {
                Ok(cb) => *guard = Some(cb),
                Err(e) => {
                    last_err = Some(format!("System clipboard unavailable: {e}"));
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                    continue;
                }
            }
        }

        let cb = guard.as_mut().expect("clipboard initialized");
        match op(cb) {
            Ok(val) => return Ok(val),
            Err(arboard::Error::ContentNotAvailable) => {
                return Err("Clipboard content not available".to_string());
            }
            Err(e) => {
                last_err = Some(e.to_string());
                // Drop potentially stale X11/Wayland/Win32 handle and recreate on next attempt
                *guard = None;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| "System clipboard unavailable".to_string()))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn write_desktop_clipboard_text(
    clipboard: &Mutex<Option<arboard::Clipboard>>,
    text: &str,
) -> Result<(), String> {
    with_clipboard_retry(clipboard, arboard::Clipboard::new, |cb| {
        cb.set_text(text.to_string())
    })
    .map_err(|e| format!("Failed to copy text: {e}"))
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
        write_desktop_clipboard_text(&state.desktop_clipboard, text)
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
    state
        .shares
        .remove_local_share(&id)
        .map_err(|e| e.to_string())
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
pub async fn shares_browse_remote_roots(
    _peer_id: String,
) -> Result<Vec<serde_json::Value>, String> {
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
pub async fn shares_download_remote_folder(
    state: State<'_, AppState>,
    peer_id: String,
    root_id: String,
    rel_path: String,
) -> Result<ShareFolderDownloadResult, String> {
    state
        .shares
        .download_remote_folder(&state.engine, &peer_id, &root_id, &rel_path)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "mod-shares"))]
#[tauri::command]
pub async fn shares_download_remote_folder(
    _peer_id: String,
    _root_id: String,
    _rel_path: String,
) -> Result<serde_json::Value, String> {
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
            crate::preview::MAX_PREVIEW_CACHE_BYTES,
        )
        .await
        .map_err(|e| e.to_string())?;
    crate::preview::touch_cache_file(std::path::Path::new(&cached_path)).await;
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
        enum DesktopClipboardRead {
            Image(arboard::ImageData<'static>),
            Text(String),
            Empty,
        }

        let read = with_clipboard_retry(&state.desktop_clipboard, arboard::Clipboard::new, |cb| {
            match cb.get_image() {
                Ok(img) => return Ok(DesktopClipboardRead::Image(img)),
                Err(arboard::Error::ContentNotAvailable | arboard::Error::ConversionFailure) => {}
                Err(e) => return Err(e),
            }
            match cb.get_text() {
                Ok(text) if !text.trim().is_empty() => Ok(DesktopClipboardRead::Text(text)),
                Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(DesktopClipboardRead::Empty),
                Err(e) => Err(e),
            }
        })?;

        match read {
            DesktopClipboardRead::Image(img) => {
                let width = u32::try_from(img.width)
                    .map_err(|_| "Clipboard image width overflow".to_string())?;
                let height = u32::try_from(img.height)
                    .map_err(|_| "Clipboard image height overflow".to_string())?;
                let item = state
                    .clipboard
                    .push_local_rgba_image(width, height, img.bytes.as_ref())
                    .map_err(|e| e.to_string())?;
                Ok(ClipboardPasteResult::Image { item })
            }
            DesktopClipboardRead::Text(text) => Ok(ClipboardPasteResult::Text { text }),
            DesktopClipboardRead::Empty => Ok(ClipboardPasteResult::Empty),
        }
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

        with_clipboard_retry(&state.desktop_clipboard, arboard::Clipboard::new, |cb| {
            cb.set_image(arboard::ImageData {
                width: rgba.0,
                height: rgba.1,
                bytes: std::borrow::Cow::Borrowed(rgba.2.as_slice()),
            })
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
        crate::preview::touch_cache_file(&cached_path).await;
        return Ok(cached_path.to_string_lossy().to_string());
    }

    let bytes = state
        .clipboard
        .fetch_remote_image(&state.engine, peer_id, item_id)
        .await
        .map_err(|e| e.to_string())?;
    let _reservation = state
        .engine
        .preview_quota()
        .reserve(
            &cache_dir,
            bytes.len() as u64,
            crate::preview::MAX_PREVIEW_CACHE_BYTES,
            std::slice::from_ref(&cached_path),
        )
        .map_err(|e| format!("Preview cache quota exceeded: {e}"))?;
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
pub async fn ensure_startup_permissions(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            return ensure_startup_permissions_with_deps(
                || async {
                    bridge
                        .ensure_startup_permissions_async()
                        .await
                        .map(|outcome| outcome.listener_notified)
                },
                || {
                    crate::register_android_network_listener_once(
                        &app,
                        &state.engine,
                        &state.android_network_listener,
                    )
                },
                &state.android_network_listener,
                state.engine.as_ref(),
            )
            .await;
        }
    }
    let _ = (app, state);
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

// --- Cache Management ---

const CACHE_SUBDIRS: &[&str] = &[
    "preview_cache",
    "updates",
    "share_intent",
    "clipboard_tmp",
    "clipboard_share",
];

fn resolve_cache_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    app.path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| format!("Failed to resolve cache directory: {e}"))
}

fn dir_size_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            total = total.saturating_add(meta.len());
        } else if meta.is_dir() {
            total = total.saturating_add(dir_size_bytes(&entry.path()));
        }
    }
    total
}

fn compute_app_cache_size(cache_root: &std::path::Path) -> u64 {
    let mut total = 0u64;
    for sub in CACHE_SUBDIRS {
        total = total.saturating_add(dir_size_bytes(&cache_root.join(sub)));
    }
    if let Ok(entries) = std::fs::read_dir(cache_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("saf_share_") && name_str.ends_with(".bin") {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total = total.saturating_add(meta.len());
                    }
                }
            }
        }
    }
    total
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearCacheResult {
    pub remaining_bytes: u64,
    pub deleted_files: u64,
    pub skipped_files: u64,
    pub failed_files: u64,
}

fn is_protected_path(
    path: &std::path::Path,
    protected: &std::collections::HashSet<std::path::PathBuf>,
    protected_canon: &std::collections::HashSet<std::path::PathBuf>,
) -> bool {
    if protected.contains(path) {
        return true;
    }
    if let Ok(canon) = path.canonicalize() {
        if protected_canon.contains(&canon) {
            return true;
        }
    }
    false
}

fn clear_app_cache_dir(
    cache_root: &std::path::Path,
    protected: &std::collections::HashSet<std::path::PathBuf>,
) -> ClearCacheResult {
    let protected_canon: std::collections::HashSet<std::path::PathBuf> = protected
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    let mut deleted_files = 0u64;
    let mut skipped_files = 0u64;
    let mut failed_files = 0u64;

    for sub in CACHE_SUBDIRS {
        let dir = cache_root.join(sub);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_protected_path(&path, protected, &protected_canon) {
                    skipped_files += 1;
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        match std::fs::remove_dir_all(&path) {
                            Ok(()) => deleted_files += 1,
                            Err(_) => failed_files += 1,
                        }
                    } else {
                        match std::fs::remove_file(&path) {
                            Ok(()) => deleted_files += 1,
                            Err(_) => failed_files += 1,
                        }
                    }
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(cache_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("saf_share_") && name_str.ends_with(".bin") {
                let path = entry.path();
                if is_protected_path(&path, protected, &protected_canon) {
                    skipped_files += 1;
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => deleted_files += 1,
                    Err(_) => failed_files += 1,
                }
            }
        }
    }
    ClearCacheResult {
        remaining_bytes: compute_app_cache_size(cache_root),
        deleted_files,
        skipped_files,
        failed_files,
    }
}

#[tauri::command]
pub async fn get_cache_size(app: tauri::AppHandle) -> Result<u64, String> {
    let cache_root = resolve_cache_root(&app)?;
    tokio::task::spawn_blocking(move || compute_app_cache_size(&cache_root))
        .await
        .map_err(|e| format!("Failed to compute cache size: {e}"))
}

#[tauri::command]
pub async fn clear_cache(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ClearCacheResult, String> {
    let cache_root = resolve_cache_root(&app)?;
    let mut protected = state.engine.preview_quota().active_paths_snapshot();
    protected.extend(crate::updater::active_update_paths_snapshot());

    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            if let Ok(pending) = bridge.peek_pending_shared_files() {
                for p in pending {
                    protected.insert(std::path::PathBuf::from(p));
                }
            }
        }
    }

    tokio::task::spawn_blocking(move || clear_app_cache_dir(&cache_root, &protected))
        .await
        .map_err(|e| format!("Failed to clear cache: {e}"))
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
pub async fn open_file_external(app: tauri::AppHandle, path_or_uri: String) -> Result<(), String> {
    crate::preview::open_file_external(&app, &path_or_uri).await
}

#[tauri::command]
pub async fn resolve_preview_names(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut out = std::collections::HashMap::new();
    #[cfg(target_os = "android")]
    {
        use std::collections::HashSet;
        use tauri::Manager;
        if let Some(bridge) = app.try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>() {
            let mut seen = HashSet::new();
            let unique_uris: Vec<String> = paths
                .into_iter()
                .filter(|p| p.starts_with("content://") && seen.insert(p.clone()))
                .collect();
            if !unique_uris.is_empty() {
                if let Ok(resolved) = bridge
                    .resolve_content_uri_display_names_async(unique_uris)
                    .await
                {
                    out.extend(resolved);
                }
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, paths);
    }
    Ok(out)
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

        let listener_state = AndroidNetworkListenerState::new();
        listener_state.record_result(&Err("plugin not initialized".into()));
        assert!(!listener_state.is_registered());
        assert_eq!(
            listener_state.last_error().as_deref(),
            Some("plugin not initialized")
        );

        let report = build_diagnostics_report_with_listener(
            &engine,
            "2026-11-23 INFO test log line",
            Some(&listener_state),
        );
        assert!(report.contains("Device Name: DiagNode"));
        assert!(report.contains("192.168.56.0/24"));
        assert!(report.contains("Android Network Listener: registered=false, error=plugin not initialized"));
        assert!(report.contains("2026-11-23 INFO test log line"));

        listener_state.record_result(&Ok(()));
        assert!(listener_state.is_registered());
        assert_eq!(listener_state.last_error(), None);
        let recovered_report =
            build_diagnostics_report_with_listener(&engine, "", Some(&listener_state));
        assert!(recovered_report.contains("Android Network Listener: registered=true"));
        assert!(!recovered_report.contains("error="));

        engine.shutdown();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    struct FakeScanTrigger {
        calls: std::sync::atomic::AtomicUsize,
        fail_first_n: std::sync::atomic::AtomicUsize,
    }

    impl FakeScanTrigger {
        fn new(fail_first_n: usize) -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                fail_first_n: std::sync::atomic::AtomicUsize::new(fail_first_n),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }
    }

    impl StartupScanTrigger for FakeScanTrigger {
        fn trigger_scan(&self) -> Result<(), String> {
            let idx = self.calls.fetch_add(1, Ordering::AcqRel);
            if idx < self.fail_first_n.load(Ordering::Acquire) {
                Err(format!("simulated scan failure #{idx}"))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn ensure_startup_permissions_runs_single_scan_across_two_calls_when_listener_not_notified_and_reregister_fails() {
        let listener_state = AndroidNetworkListenerState::new();
        let trigger = FakeScanTrigger::new(0);

        ensure_startup_permissions_with_deps(
            || async { Ok(false) },
            || async { Err("plugin unavailable".to_string()) },
            &listener_state,
            &trigger,
        )
        .await
        .expect("first call should succeed via fallback scan");

        ensure_startup_permissions_with_deps(
            || async { Ok(false) },
            || async { Err("plugin unavailable".to_string()) },
            &listener_state,
            &trigger,
        )
        .await
        .expect("second call should not trigger duplicate fallback scan");

        assert_eq!(trigger.call_count(), 1);
        assert!(!listener_state.is_registered());
        assert_eq!(
            listener_state.last_error().as_deref(),
            Some("plugin unavailable")
        );
    }

    #[tokio::test]
    async fn ensure_startup_permissions_skips_fallback_scan_when_listener_already_notified() {
        let listener_state = AndroidNetworkListenerState::new();
        let trigger = FakeScanTrigger::new(0);

        ensure_startup_permissions_with_deps(
            || async { Ok(true) },
            || async { Ok(()) },
            &listener_state,
            &trigger,
        )
        .await
        .expect("call with notified listener should succeed");

        assert_eq!(trigger.call_count(), 0);
    }

    #[tokio::test]
    async fn ensure_startup_permissions_retries_failed_trigger_scan_once_and_releases_claim_on_persistent_failure() {
        // Transient failure on first attempt -> retried once and succeeds.
        let transient_state = AndroidNetworkListenerState::new();
        let transient_trigger = FakeScanTrigger::new(1);
        ensure_startup_permissions_with_deps(
            || async { Ok(false) },
            || async { Err("no listener".to_string()) },
            &transient_state,
            &transient_trigger,
        )
        .await
        .expect("transient trigger_scan failure should recover on retry");
        assert_eq!(transient_trigger.call_count(), 2);

        // Persistent failure (both initial and retry fail) -> returns Err and releases claim so next call can recover.
        let persistent_state = AndroidNetworkListenerState::new();
        let persistent_trigger = FakeScanTrigger::new(2);
        let err = ensure_startup_permissions_with_deps(
            || async { Ok(false) },
            || async { Err("no listener".to_string()) },
            &persistent_state,
            &persistent_trigger,
        )
        .await
        .expect_err("persistent trigger_scan failure must not be silently swallowed");
        assert!(err.contains("simulated scan failure"));
        assert_eq!(persistent_trigger.call_count(), 2);

        ensure_startup_permissions_with_deps(
            || async { Ok(false) },
            || async { Err("no listener".to_string()) },
            &persistent_state,
            &persistent_trigger,
        )
        .await
        .expect("released claim should allow subsequent recovery call to scan");
        assert_eq!(persistent_trigger.call_count(), 3);
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn desktop_clipboard_retry_recreates_instance_on_transient_error() {
        let slot: Mutex<Option<usize>> = Mutex::new(Some(0));
        let mut created = 0usize;
        let mut ops = 0usize;

        let res = with_clipboard_retry(
            &slot,
            || {
                created += 1;
                Ok(created)
            },
            |instance| {
                ops += 1;
                if *instance == 0 {
                    Err(arboard::Error::ClipboardOccupied)
                } else {
                    Ok(*instance)
                }
            },
        )
        .expect("should recover after recreating clipboard instance");

        assert_eq!(res, 1);
        assert_eq!(created, 1);
        assert_eq!(ops, 2);

        // ContentNotAvailable must not recreate or retry
        let err = with_clipboard_retry(
            &slot,
            || {
                created += 1;
                Ok(created)
            },
            |_| Err::<(), _>(arboard::Error::ContentNotAvailable),
        )
        .expect_err("ContentNotAvailable should return immediately");
        assert!(err.contains("not available"));
        assert_eq!(created, 1);
    }

    #[test]
    fn clear_app_cache_dir_skips_protected_files_and_reports_counts() {
        let root = std::env::temp_dir().join(format!("landa-clear-cache-{}", uuid::Uuid::new_v4()));
        let preview_dir = root.join("preview_cache");
        let updates_dir = root.join("updates");
        let share_dir = root.join("share_intent");
        std::fs::create_dir_all(&preview_dir).unwrap();
        std::fs::create_dir_all(&updates_dir).unwrap();
        std::fs::create_dir_all(&share_dir).unwrap();

        let stale_preview = preview_dir.join("old.bin");
        let active_preview = preview_dir.join(".preview-part-active");
        let active_apk = updates_dir.join("landa-v0.5.1.apk");
        let queued_share = share_dir.join("queued.jpg");

        std::fs::write(&stale_preview, vec![1u8; 10]).unwrap();
        std::fs::write(&active_preview, vec![2u8; 20]).unwrap();
        std::fs::write(&active_apk, vec![3u8; 30]).unwrap();
        std::fs::write(&queued_share, vec![4u8; 40]).unwrap();

        let mut protected = std::collections::HashSet::new();
        protected.insert(active_preview.clone());
        protected.insert(active_apk.clone());
        protected.insert(queued_share.clone());

        let res = clear_app_cache_dir(&root, &protected);

        assert_eq!(res.deleted_files, 1);
        assert_eq!(res.skipped_files, 3);
        assert_eq!(res.failed_files, 0);
        assert_eq!(res.remaining_bytes, 90);
        assert!(!stale_preview.exists());
        assert!(active_preview.exists());
        assert!(active_apk.exists());
        assert!(queued_share.exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
