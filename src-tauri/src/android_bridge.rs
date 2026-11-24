use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use landa_core::{
    FriendRequestInfo, IncomingTransferOfferInfo, LanInterface, LanInterfaceHook, OpenSourceHook,
    OutgoingFileItem, ReceivePublishHook, SocketBindHook, TransferProgressInfo,
};
use landa_proto::path_policy::sanitize_relative_path;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishArgs {
    source_path: String,
    relative_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishResponse {
    published_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedFilesResponse {
    paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceNameResponse {
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BindSocketArgs {
    fd: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanInterfaceDto {
    name: String,
    ip: String,
    netmask: String,
    broadcast: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanInterfacesResponse {
    interfaces: Vec<LanInterfaceDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferCompletedNotifArgs {
    transfer_id: String,
    peer_name: String,
    file_count: usize,
    saved_paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferFailedNotifArgs {
    transfer_id: String,
    peer_name: String,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallApkArgs {
    apk_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PickShareResponse {
    cancelled: bool,
    uri: Option<String>,
    name: Option<String>,
    #[serde(default)]
    is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SafShareArgs {
    root_uri: String,
    is_dir: bool,
    rel_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SafShareTreeArgs {
    root_uri: String,
    is_dir: bool,
    rel_path: String,
    max_depth: usize,
    max_files: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SafShareEntryDto {
    name: String,
    rel_path: String,
    is_dir: bool,
    size_bytes: u64,
    modified_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSafShareDirResponse {
    entries: Vec<SafShareEntryDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenSafShareFileResponse {
    fd: i32,
    file_name: String,
    size_bytes: u64,
    modified_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFolderItem {
    pub uri: String,
    pub relative_path: String,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PickFolderResponse {
    cancelled: bool,
    items: Vec<PickedFolderItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenContentUriArgs {
    uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenContentUriResponse {
    fd: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentUriMeta {
    pub name: String,
    pub size_bytes: u64,
    pub mime_type: String,
    #[serde(default)]
    pub display_path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveContentUriDisplayNamesArgs {
    uris: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveContentUriDisplayNamesResponse {
    names: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveContentUriResponse {
    uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFocusTarget {
    pub target: String,
    pub transfer_id: Option<String>,
    pub peer_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingFocusResponse {
    focus: Option<PendingFocusTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForegroundStateResponse {
    in_foreground: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferForegroundArgs {
    active_count: usize,
    direction: String,
    peer_name: String,
    current_file: String,
    files_done: usize,
    file_count: usize,
    percent: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferOfferNotifArgs {
    transfer_id: String,
    peer_name: String,
    file_count: usize,
    total_bytes: u64,
    file_names: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelTransferOfferNotifArgs {
    transfer_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FriendRequestNotifArgs {
    peer_id: String,
    device_name: String,
    sas_code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelFriendRequestNotifArgs {
    peer_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemClipboardReadResponse {
    pub kind: String,
    pub text: Option<String>,
    pub temp_path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteClipboardTextArgs {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteClipboardImageArgs {
    image_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanQrResponse {
    cancelled: bool,
    uri: Option<String>,
}

pub struct AndroidBridge<R: Runtime> {
    handle: PluginHandle<R>,
}

impl<R: Runtime> AndroidBridge<R> {
    pub async fn get_system_device_name_async(&self) -> Option<String> {
        self.handle
            .run_mobile_plugin_async::<DeviceNameResponse>("getSystemDeviceName", ())
            .await
            .ok()
            .map(|r| r.name)
            .filter(|n| !n.trim().is_empty())
    }

    pub async fn ensure_startup_permissions_async(&self) -> Result<(), String> {
        self.handle
            .run_mobile_plugin_async::<serde_json::Value>("ensureStartupPermissions", ())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn bind_socket_to_lan(&self, fd: i32) {
        let _ = self
            .handle
            .run_mobile_plugin::<serde_json::Value>("bindSocketToLan", BindSocketArgs { fd });
    }

    pub fn get_lan_interfaces(&self) -> Vec<LanInterface> {
        let Ok(res) = self
            .handle
            .run_mobile_plugin::<LanInterfacesResponse>("getLanInterfaces", ())
        else {
            return Vec::new();
        };
        res.interfaces
            .into_iter()
            .filter_map(|dto| {
                let ip = dto.ip.parse::<Ipv4Addr>().ok()?;
                let netmask = dto.netmask.parse::<Ipv4Addr>().ok()?;
                let broadcast = dto.broadcast.parse::<Ipv4Addr>().ok()?;
                Some(LanInterface {
                    name: dto.name,
                    ip,
                    netmask,
                    broadcast,
                })
            })
            .collect()
    }

    pub async fn show_transfer_completed_async(&self, info: &TransferProgressInfo) {
        let args = TransferCompletedNotifArgs {
            transfer_id: info.transfer_id.clone(),
            peer_name: info.peer_name.clone(),
            file_count: info.file_count,
            saved_paths: info.saved_paths.clone(),
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("showTransferCompleted", args)
            .await;
    }

    pub async fn is_app_in_foreground_async(&self) -> bool {
        self.handle
            .run_mobile_plugin_async::<ForegroundStateResponse>("isAppInForeground", ())
            .await
            .map(|r| r.in_foreground)
            .unwrap_or(true)
    }

    pub async fn update_transfer_foreground_async(
        &self,
        active_count: usize,
        info: &TransferProgressInfo,
    ) {
        let percent = if info.total_bytes > 0 {
            ((info.bytes_transferred.saturating_mul(100)) / info.total_bytes).min(100) as u32
        } else {
            0
        };
        let args = TransferForegroundArgs {
            active_count,
            direction: info.direction.clone(),
            peer_name: info.peer_name.clone(),
            current_file: info.current_file.clone(),
            files_done: info.files_done,
            file_count: info.file_count,
            percent,
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("updateTransferForeground", args)
            .await;
    }

    pub async fn stop_transfer_foreground_async(&self) {
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("stopTransferForeground", ())
            .await;
    }

    pub async fn show_transfer_offer_notification_async(&self, offer: &IncomingTransferOfferInfo) {
        let args = TransferOfferNotifArgs {
            transfer_id: offer.transfer_id.clone(),
            peer_name: offer.peer_name.clone(),
            file_count: offer.file_count,
            total_bytes: offer.total_bytes,
            file_names: offer.file_names.clone(),
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("showTransferOfferNotification", args)
            .await;
    }

    pub async fn cancel_transfer_offer_notification_async(&self, transfer_id: &str) {
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "cancelTransferOfferNotification",
                CancelTransferOfferNotifArgs {
                    transfer_id: transfer_id.to_string(),
                },
            )
            .await;
    }

    pub async fn show_friend_request_notification_async(&self, req: &FriendRequestInfo) {
        let args = FriendRequestNotifArgs {
            peer_id: req.peer_id.clone(),
            device_name: req.device_name.clone(),
            sas_code: req.sas_code.clone(),
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("showFriendRequestNotification", args)
            .await;
    }

    pub async fn cancel_friend_request_notification_async(&self, peer_id: &str) {
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "cancelFriendRequestNotification",
                CancelFriendRequestNotifArgs {
                    peer_id: peer_id.to_string(),
                },
            )
            .await;
    }

    pub fn take_pending_focus_target(&self) -> Result<Option<PendingFocusTarget>, String> {
        self.handle
            .run_mobile_plugin::<PendingFocusResponse>("takePendingFocusTarget", ())
            .map(|r| r.focus)
            .map_err(|e| e.to_string())
    }

    pub async fn read_system_clipboard_async(&self) -> Result<SystemClipboardReadResponse, String> {
        self.handle
            .run_mobile_plugin_async::<SystemClipboardReadResponse>("readSystemClipboard", ())
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn write_system_clipboard_text_async(&self, text: &str) -> Result<(), String> {
        self.handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "writeSystemClipboardText",
                WriteClipboardTextArgs {
                    text: text.to_string(),
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn write_system_clipboard_image_async(&self, image_path: &str) -> Result<(), String> {
        self.handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "writeSystemClipboardImage",
                WriteClipboardImageArgs {
                    image_path: image_path.to_string(),
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn scan_qr_code_async(&self) -> Result<Option<String>, String> {
        let res = self
            .handle
            .run_mobile_plugin_async::<ScanQrResponse>("scanQrCode", ())
            .await
            .map_err(|e| e.to_string())?;
        if res.cancelled {
            Ok(None)
        } else {
            Ok(res.uri.filter(|u| !u.trim().is_empty()))
        }
    }

    pub async fn show_transfer_failed_async(&self, info: &TransferProgressInfo) {
        let args = TransferFailedNotifArgs {
            transfer_id: info.transfer_id.clone(),
            peer_name: info.peer_name.clone(),
            error: info.error.clone(),
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("showTransferFailed", args)
            .await;
    }

    pub fn publish_to_downloads(&self, source: &Path, relative: &Path) -> Option<String> {
        let args = PublishArgs {
            source_path: source.to_string_lossy().to_string(),
            relative_path: relative.to_string_lossy().replace('\\', "/"),
        };
        match self
            .handle
            .run_mobile_plugin::<PublishResponse>("publishToDownloads", args)
        {
            Ok(res) => Some(res.published_path),
            Err(err) => {
                tracing::warn!("Android MediaStore publish failed: {err}");
                None
            }
        }
    }

    pub fn take_pending_shared_files(&self) -> Result<Vec<String>, String> {
        self.handle
            .run_mobile_plugin::<SharedFilesResponse>("takePendingSharedFiles", ())
            .map(|r| r.paths)
            .map_err(|e| e.to_string())
    }

    pub async fn pick_share_folder_async(&self) -> Result<Option<(String, String, bool)>, String> {
        let res = self
            .handle
            .run_mobile_plugin_async::<PickShareResponse>("pickShareFolder", ())
            .await
            .map_err(|e| e.to_string())?;
        if res.cancelled {
            return Ok(None);
        }
        let uri = res
            .uri
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| "Missing folder URI".to_string())?;
        let name = res
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Folder".to_string());
        Ok(Some((uri, name, true)))
    }

    pub async fn pick_share_file_async(&self) -> Result<Option<(String, String, bool)>, String> {
        let res = self
            .handle
            .run_mobile_plugin_async::<PickShareResponse>("pickShareFile", ())
            .await
            .map_err(|e| e.to_string())?;
        if res.cancelled {
            return Ok(None);
        }
        let uri = res
            .uri
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| "Missing file URI".to_string())?;
        let name = res
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "File".to_string());
        Ok(Some((uri, name, res.is_dir)))
    }

    #[cfg(feature = "mod-shares")]
    fn list_saf_share_dir(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<Vec<landa_mod_shares::ShareEntry>, String> {
        let res = self
            .handle
            .run_mobile_plugin::<ListSafShareDirResponse>(
                "listSafShareDir",
                SafShareArgs {
                    root_uri: root_uri.to_string(),
                    is_dir,
                    rel_path: rel_path.to_string(),
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(res
            .entries
            .into_iter()
            .map(|e| landa_mod_shares::ShareEntry {
                name: e.name,
                rel_path: e.rel_path,
                is_dir: e.is_dir,
                size_bytes: e.size_bytes,
                modified_ms: e.modified_ms,
            })
            .collect())
    }

    #[cfg(feature = "mod-shares")]
    fn list_saf_share_tree(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
        max_depth: usize,
        max_files: usize,
    ) -> Result<Vec<landa_mod_shares::ShareEntry>, String> {
        let res = self
            .handle
            .run_mobile_plugin::<ListSafShareDirResponse>(
                "listSafShareTree",
                SafShareTreeArgs {
                    root_uri: root_uri.to_string(),
                    is_dir,
                    rel_path: rel_path.to_string(),
                    max_depth,
                    max_files,
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(res
            .entries
            .into_iter()
            .map(|e| landa_mod_shares::ShareEntry {
                name: e.name,
                rel_path: e.rel_path,
                is_dir: e.is_dir,
                size_bytes: e.size_bytes,
                modified_ms: e.modified_ms,
            })
            .collect())
    }

    #[cfg(feature = "mod-shares")]
    fn open_saf_share_file(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<landa_mod_shares::ShareOpenedFile, String> {
        let res = self
            .handle
            .run_mobile_plugin::<OpenSafShareFileResponse>(
                "openSafShareFile",
                SafShareArgs {
                    root_uri: root_uri.to_string(),
                    is_dir,
                    rel_path: rel_path.to_string(),
                },
            )
            .map_err(|e| e.to_string())?;
        if res.fd < 0 {
            return Err("Invalid file descriptor".into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            // SAFETY: `detachFd()` in Kotlin transfers ownership of a valid open fd to Rust.
            let file = unsafe { std::fs::File::from_raw_fd(res.fd) };
            Ok(landa_mod_shares::ShareOpenedFile {
                file,
                file_name: res.file_name,
                size_bytes: res.size_bytes,
                modified_ms: res.modified_ms,
            })
        }
        #[cfg(not(unix))]
        {
            Err("SAF file descriptors are only supported on Unix/Android".into())
        }
    }

    pub async fn install_apk_update_async(&self, apk_path: &str) -> Result<(), String> {
        self.handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "installApkUpdate",
                InstallApkArgs {
                    apk_path: apk_path.to_string(),
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn pick_folder_for_send_async(
        &self,
    ) -> Result<Option<Vec<OutgoingFileItem>>, String> {
        let res = self
            .handle
            .run_mobile_plugin_async::<PickFolderResponse>("pickFolderForSend", ())
            .await
            .map_err(|e| e.to_string())?;
        if res.cancelled {
            return Ok(None);
        }
        let items = res
            .items
            .into_iter()
            .map(|item| {
                let clean_rel = sanitize_relative_path(&item.relative_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                OutgoingFileItem {
                    source_path: PathBuf::from(&item.uri),
                    source_uri: Some(item.uri),
                    relative_path: clean_rel,
                    size_bytes: item.size_bytes,
                }
            })
            .collect();
        Ok(Some(items))
    }

    pub fn open_content_uri_file(&self, uri: &str) -> Option<std::fs::File> {
        let res = self
            .handle
            .run_mobile_plugin::<OpenContentUriResponse>(
                "openContentUriFd",
                OpenContentUriArgs {
                    uri: uri.to_string(),
                },
            )
            .ok()?;
        if res.fd < 0 {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            // SAFETY: `detachFd()` in Kotlin transfers ownership of a valid open fd to Rust.
            Some(unsafe { std::fs::File::from_raw_fd(res.fd) })
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    pub async fn open_content_uri_file_async(&self, uri: &str) -> Option<std::fs::File> {
        let res = self
            .handle
            .run_mobile_plugin_async::<OpenContentUriResponse>(
                "openContentUriFd",
                OpenContentUriArgs {
                    uri: uri.to_string(),
                },
            )
            .await
            .ok()?;
        if res.fd < 0 {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            Some(unsafe { std::fs::File::from_raw_fd(res.fd) })
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    pub async fn get_content_uri_meta_async(&self, uri: &str) -> Result<ContentUriMeta, String> {
        self.handle
            .run_mobile_plugin_async::<ContentUriMeta>(
                "getContentUriMeta",
                OpenContentUriArgs {
                    uri: uri.to_string(),
                },
            )
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn resolve_content_uri_display_names_async(
        &self,
        uris: Vec<String>,
    ) -> Result<std::collections::HashMap<String, String>, String> {
        let res = self
            .handle
            .run_mobile_plugin_async::<ResolveContentUriDisplayNamesResponse>(
                "resolveContentUriDisplayNames",
                ResolveContentUriDisplayNamesArgs { uris },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.names)
    }

    pub async fn resolve_content_uri_for_path_async(&self, path: &str) -> Option<String> {
        self.handle
            .run_mobile_plugin_async::<ResolveContentUriResponse>(
                "resolveContentUriForPath",
                OpenContentUriArgs {
                    uri: path.to_string(),
                },
            )
            .await
            .ok()
            .and_then(|r| r.uri)
            .filter(|u| u.starts_with("content://"))
    }

    pub async fn open_file_external_async(&self, path_or_uri: &str) -> Result<(), String> {
        self.handle
            .run_mobile_plugin_async::<serde_json::Value>(
                "openFileExternal",
                OpenContentUriArgs {
                    uri: path_or_uri.to_string(),
                },
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("landa-android")
        .setup(|app, api| {
            let handle = api.register_android_plugin("dev.landa.app", "LandaAndroidPlugin")?;
            app.manage(AndroidBridge { handle });
            Ok(())
        })
        .build()
}

pub fn make_publish_hook<R: Runtime>(app: tauri::AppHandle<R>) -> ReceivePublishHook {
    Arc::new(move |source: &Path, relative: &Path| {
        app.try_state::<AndroidBridge<R>>()
            .and_then(|bridge| bridge.publish_to_downloads(source, relative))
    })
}

pub fn make_lan_interface_hook<R: Runtime>(app: tauri::AppHandle<R>) -> LanInterfaceHook {
    Arc::new(move || {
        app.try_state::<AndroidBridge<R>>()
            .map(|bridge| bridge.get_lan_interfaces())
            .unwrap_or_default()
    })
}

pub fn make_socket_bind_hook<R: Runtime>(app: tauri::AppHandle<R>) -> SocketBindHook {
    Arc::new(move |fd: i32| {
        if let Some(bridge) = app.try_state::<AndroidBridge<R>>() {
            bridge.bind_socket_to_lan(fd);
        }
    })
}

pub fn make_open_source_hook<R: Runtime>(app: tauri::AppHandle<R>) -> OpenSourceHook {
    Arc::new(move |uri: &str| {
        app.try_state::<AndroidBridge<R>>()
            .and_then(|bridge| bridge.open_content_uri_file(uri))
    })
}

#[cfg(feature = "mod-shares")]
struct AndroidShareContentProvider<R: Runtime> {
    app: tauri::AppHandle<R>,
}

#[cfg(feature = "mod-shares")]
impl<R: Runtime> landa_mod_shares::ShareContentProvider for AndroidShareContentProvider<R> {
    fn list_dir(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<Vec<landa_mod_shares::ShareEntry>, landa_core::CoreError> {
        let bridge = self.app.try_state::<AndroidBridge<R>>().ok_or_else(|| {
            landa_core::CoreError::InvalidInput("Android bridge unavailable".into())
        })?;
        bridge
            .list_saf_share_dir(root_uri, is_dir, rel_path)
            .map_err(landa_core::CoreError::InvalidInput)
    }

    fn open_file(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<landa_mod_shares::ShareOpenedFile, landa_core::CoreError> {
        let bridge = self.app.try_state::<AndroidBridge<R>>().ok_or_else(|| {
            landa_core::CoreError::InvalidInput("Android bridge unavailable".into())
        })?;
        bridge
            .open_saf_share_file(root_uri, is_dir, rel_path)
            .map_err(landa_core::CoreError::InvalidInput)
    }

    fn list_tree(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
        max_depth: usize,
        max_files: usize,
    ) -> Result<Vec<landa_mod_shares::ShareEntry>, landa_core::CoreError> {
        let bridge = self.app.try_state::<AndroidBridge<R>>().ok_or_else(|| {
            landa_core::CoreError::InvalidInput("Android bridge unavailable".into())
        })?;
        bridge
            .list_saf_share_tree(root_uri, is_dir, rel_path, max_depth, max_files)
            .map_err(landa_core::CoreError::InvalidInput)
    }
}

#[cfg(feature = "mod-shares")]
pub fn make_share_content_provider<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> Arc<dyn landa_mod_shares::ShareContentProvider> {
    Arc::new(AndroidShareContentProvider { app })
}
