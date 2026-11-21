use std::path::Path;
use std::sync::Arc;

use landa_core::ReceivePublishHook;
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

pub struct AndroidBridge<R: Runtime> {
    handle: PluginHandle<R>,
}

impl<R: Runtime> AndroidBridge<R> {
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
