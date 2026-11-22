use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;

use landa_core::{
    LanInterface, LanInterfaceHook, ReceivePublishHook, SocketBindHook, TransferProgressInfo,
};
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
struct AccessResponse {
    granted: bool,
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
struct TransferProgressNotifArgs {
    transfer_id: String,
    peer_name: String,
    current_file: String,
    files_done: usize,
    file_count: usize,
    percent: u32,
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

    pub async fn show_transfer_progress_async(&self, info: &TransferProgressInfo) {
        let percent = if info.total_bytes > 0 {
            ((info.bytes_transferred.saturating_mul(100)) / info.total_bytes).min(100) as u32
        } else {
            0
        };
        let args = TransferProgressNotifArgs {
            transfer_id: info.transfer_id.clone(),
            peer_name: info.peer_name.clone(),
            current_file: info.current_file.clone(),
            files_done: info.files_done,
            file_count: info.file_count,
            percent,
        };
        let _ = self
            .handle
            .run_mobile_plugin_async::<serde_json::Value>("showTransferProgress", args)
            .await;
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

    pub fn has_all_files_access(&self) -> Result<bool, String> {
        self.handle
            .run_mobile_plugin::<AccessResponse>("hasAllFilesAccess", ())
            .map(|r| r.granted)
            .map_err(|e| e.to_string())
    }

    pub fn request_all_files_access(&self) -> Result<(), String> {
        self.handle
            .run_mobile_plugin::<serde_json::Value>("requestAllFilesAccess", ())
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
