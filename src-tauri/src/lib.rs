#[cfg(target_os = "android")]
mod android_bridge;
mod commands;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;

use std::sync::Arc;

use commands::AppState;
use landa_core::{LandaConfig, LandaEngine};
#[cfg(feature = "mod-clipboard")]
use landa_mod_clipboard::ClipboardModule;
#[cfg(feature = "mod-shares")]
use landa_mod_shares::SharesModule;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(target_os = "android")]
    {
        builder = builder.plugin(android_bridge::init());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::show_main_window(app);
        }));
    }

    builder
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("landa"));
            let default_download_dir = app
                .path()
                .download_dir()
                .map(|d| d.join("Landa"))
                .unwrap_or_else(|_| app_data_dir.join("Downloads"));

            let default_device_name = resolve_default_device_name();
            let engine = tauri::async_runtime::block_on(async move {
                LandaEngine::new(LandaConfig {
                    app_data_dir,
                    default_download_dir,
                    default_device_name,
                })
            })
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

            #[cfg(feature = "mod-shares")]
            let shares = {
                let m = SharesModule::new(Arc::clone(engine.storage()));
                engine.register_module(m.clone());
                m
            };

            #[cfg(feature = "mod-clipboard")]
            let clipboard = {
                let m = ClipboardModule::new(
                    Arc::clone(engine.storage()),
                    engine.events().clone(),
                );
                engine.register_module(m.clone());
                m
            };

            // Start background discovery & QUIC accept loops inside Tauri's async runtime
            let engine_for_loops = Arc::clone(&engine);
            tauri::async_runtime::spawn(async move {
                engine_for_loops.start_loops();
            });

            // Forward CoreEvents to the WebView
            let mut rx = engine.subscribe_events();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let _ = handle.emit("landa://event", &event);
                }
            });

            #[cfg(target_os = "android")]
            {
                engine.set_receive_publish_hook(android_bridge::make_publish_hook(
                    app.handle().clone(),
                ));
            }

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                desktop::setup_tray_and_window(app.handle(), Arc::clone(&engine))?;
            }

            app.manage(AppState {
                engine,
                #[cfg(feature = "mod-shares")]
                shares,
                #[cfg(feature = "mod-clipboard")]
                clipboard,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_local_identity,
            commands::list_peers,
            commands::trigger_scan,
            commands::rename_peer,
            commands::list_friend_requests,
            commands::send_friend_request,
            commands::respond_friend_request,
            commands::unfriend,
            commands::send_files,
            commands::respond_transfer_offer,
            commands::cancel_transfer,
            commands::list_history,
            commands::clear_history,
            commands::get_settings,
            commands::update_settings,
            commands::list_modules,
            commands::set_module_enabled,
            commands::list_manual_targets,
            commands::add_manual_target,
            commands::remove_manual_target,
            commands::shares_list_local,
            commands::shares_add_local,
            commands::shares_remove_local,
            commands::shares_browse_remote_roots,
            commands::shares_browse_remote_dir,
            commands::shares_download_remote_file,
            commands::clipboard_list_local,
            commands::clipboard_push_local_text,
            commands::clipboard_delete_local,
            commands::clipboard_send_to_peer,
            commands::clipboard_fetch_remote,
            commands::take_pending_shared_files,
            commands::has_all_files_access,
            commands::request_all_files_access,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Landa");
}

fn resolve_default_device_name() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() && trimmed != "localhost" {
                return trimmed.to_string();
            }
        }
    }
    #[cfg(target_os = "android")]
    {
        "Android Device".to_string()
    }
    #[cfg(not(target_os = "android"))]
    {
        "Landa Device".to_string()
    }
}
