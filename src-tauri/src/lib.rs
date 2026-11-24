#[cfg(target_os = "android")]
mod android_bridge;
mod commands;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;
mod logging;
mod preview;
mod updater;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use commands::AppState;
use landa_core::{LandaConfig, LandaEngine};
#[cfg(feature = "mod-clipboard")]
use landa_mod_clipboard::ClipboardModule;
#[cfg(feature = "mod-shares")]
use landa_mod_shares::SharesModule;
#[cfg(feature = "mod-videolink")]
use landa_mod_videolink::VideoLinkModule;
use tauri::{Emitter, Manager};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_notification::NotificationExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init_tracing();

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
            if let Err(err) = logging::attach_log_dir(&app_data_dir) {
                tracing::warn!("failed to attach rotating log file: {err}");
            }
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
                #[cfg(target_os = "android")]
                m.set_content_provider(android_bridge::make_share_content_provider(
                    app.handle().clone(),
                ));
                engine.register_module(m.clone());
                m
            };

            #[cfg(feature = "mod-clipboard")]
            let clipboard = {
                let m = ClipboardModule::new(
                    Arc::clone(engine.storage()),
                    engine.events().clone(),
                    engine.app_data_dir().join("clipboard_images"),
                );
                engine.register_module(m.clone());
                m
            };

            #[cfg(feature = "mod-videolink")]
            let videolink = {
                let m = VideoLinkModule::new();
                #[cfg(target_os = "android")]
                m.set_open_source_hook(android_bridge::make_open_source_hook(
                    app.handle().clone(),
                ));
                engine.register_module(m.clone());
                m
            };

            #[cfg(target_os = "android")]
            {
                landa_core::set_lan_interface_hook(android_bridge::make_lan_interface_hook(
                    app.handle().clone(),
                ));
                landa_core::set_socket_bind_hook(android_bridge::make_socket_bind_hook(
                    app.handle().clone(),
                ));
                engine.set_receive_publish_hook(android_bridge::make_publish_hook(
                    app.handle().clone(),
                ));
                engine.set_open_source_hook(android_bridge::make_open_source_hook(
                    app.handle().clone(),
                ));
            }

            // Start background discovery & QUIC accept loops inside Tauri's async runtime
            let engine_for_loops = Arc::clone(&engine);
            tauri::async_runtime::spawn(async move {
                engine_for_loops.start_loops();
                engine_for_loops.trigger_scan();
            });

            #[cfg(target_os = "android")]
            {
                let handle_for_name = app.handle().clone();
                let engine_for_name = Arc::clone(&engine);
                tauri::async_runtime::spawn(async move {
                    if let Some(bridge) =
                        handle_for_name.try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                    {
                        if let Some(real_name) = bridge.get_system_device_name_async().await {
                            engine_for_name.apply_system_device_name_if_placeholder(&real_name);
                            let _ = handle_for_name.emit("landa://identity-updated", ());
                        }
                    }
                });
            }

            // Forward CoreEvents to the WebView (and Android/Desktop notifications)
            let mut rx = engine.subscribe_events();
            let handle = app.handle().clone();
            let engine_for_events = Arc::clone(&engine);
            let mut known_incoming_requests: HashSet<String> = engine
                .list_friend_requests()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.direction == "incoming")
                .map(|r| r.peer_id)
                .collect();
            let mut active_transfers: HashMap<String, landa_core::TransferProgressInfo> =
                HashMap::new();

            tauri::async_runtime::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    match &event {
                        landa_core::CoreEvent::TransferProgress(info) => {
                            let is_active = info.status == "waiting_approval"
                                || info.status == "transferring";
                            if is_active {
                                active_transfers.insert(info.transfer_id.clone(), info.clone());
                            } else {
                                active_transfers.remove(&info.transfer_id);
                            }

                            #[cfg(target_os = "android")]
                            if let Some(bridge) =
                                handle.try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                            {
                                if is_active {
                                    bridge
                                        .update_transfer_foreground_async(
                                            active_transfers.len(),
                                            info,
                                        )
                                        .await;
                                } else {
                                    bridge
                                        .cancel_transfer_offer_notification_async(&info.transfer_id)
                                        .await;
                                    if active_transfers.is_empty() {
                                        bridge.stop_transfer_foreground_async().await;
                                    } else if let Some(next_active) =
                                        active_transfers.values().next()
                                    {
                                        bridge
                                            .update_transfer_foreground_async(
                                                active_transfers.len(),
                                                next_active,
                                            )
                                            .await;
                                    }

                                    if info.direction == "receive" {
                                        match info.status.as_str() {
                                            "completed" => {
                                                bridge.show_transfer_completed_async(info).await;
                                            }
                                            "failed" | "cancelled" => {
                                                bridge.show_transfer_failed_async(info).await;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                        landa_core::CoreEvent::IncomingTransferOffer(offer) => {
                            #[cfg(target_os = "android")]
                            if let Some(bridge) =
                                handle.try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                            {
                                if !bridge.is_app_in_foreground_async().await {
                                    bridge.show_transfer_offer_notification_async(offer).await;
                                }
                            }

                            #[cfg(not(any(target_os = "android", target_os = "ios")))]
                            if should_notify_desktop(&handle) {
                                let lang = engine_for_events.get_settings().language;
                                let size_str = format_bytes_human(offer.total_bytes);
                                let body = if lang == "en" {
                                    format!(
                                        "Incoming transfer: {} file(s) ({})",
                                        offer.file_count, size_str
                                    )
                                } else {
                                    format!(
                                        "Входящая передача: {} ф. ({})",
                                        offer.file_count, size_str
                                    )
                                };
                                let _ = handle
                                    .notification()
                                    .builder()
                                    .title(format!("Landa · {}", offer.peer_name))
                                    .body(body)
                                    .extra("target", "transfer_offer")
                                    .extra("transferId", offer.transfer_id.clone())
                                    .show();
                            }
                        }
                        landa_core::CoreEvent::FriendRequestsChanged => {
                            let current_incoming = engine_for_events
                                .list_friend_requests()
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|r| r.direction == "incoming")
                                .collect::<Vec<_>>();
                            let current_ids: HashSet<String> =
                                current_incoming.iter().map(|r| r.peer_id.clone()).collect();

                            for req in &current_incoming {
                                if !known_incoming_requests.contains(&req.peer_id) {
                                    #[cfg(target_os = "android")]
                                    if let Some(bridge) = handle
                                        .try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                                    {
                                        if !bridge.is_app_in_foreground_async().await {
                                            bridge
                                                .show_friend_request_notification_async(req)
                                                .await;
                                        }
                                    }

                                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                                    if should_notify_desktop(&handle) {
                                        let lang = engine_for_events.get_settings().language;
                                        let body = if lang == "en" {
                                            format!("Friend request · Code {}", req.sas_code)
                                        } else {
                                            format!("Запрос в друзья · Код {}", req.sas_code)
                                        };
                                        let _ = handle
                                            .notification()
                                            .builder()
                                            .title(format!("Landa · {}", req.device_name))
                                            .body(body)
                                            .extra("target", "friend_requests")
                                            .extra("peerId", req.peer_id.clone())
                                            .show();
                                    }
                                }
                            }

                            #[cfg(target_os = "android")]
                            if let Some(bridge) =
                                handle.try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                            {
                                for old_id in &known_incoming_requests {
                                    if !current_ids.contains(old_id) {
                                        bridge
                                            .cancel_friend_request_notification_async(old_id)
                                            .await;
                                    }
                                }
                            }

                            known_incoming_requests = current_ids;
                        }
                        _ => {}
                    }

                    let _ = handle.emit("landa://event", &event);
                }
            });

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                desktop::setup_tray_and_window(app.handle(), Arc::clone(&engine))?;
                let app_for_notif = app.handle().clone();
                let _ = app.notification().on_action(move |performed| {
                    desktop::show_main_window(&app_for_notif);
                    if let Some(notif) = performed.notification() {
                        let extra = notif.extra();
                        if let Some(target) = extra.get("target").and_then(|v| v.as_str()) {
                            let payload = serde_json::json!({
                                "target": target,
                                "transferId": extra.get("transferId").and_then(|v| v.as_str()),
                                "peerId": extra.get("peerId").and_then(|v| v.as_str()),
                            });
                            let _ = app_for_notif.emit("landa://focus-target", payload);
                        }
                    }
                });
            }

            let preview_server = tauri::async_runtime::block_on(preview::PreviewServer::start(
                app.handle().clone(),
            ))
            .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>)?;
            app.manage(preview_server);

            let app_for_sweep = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                preview::sweep_old_preview_cache(&app_for_sweep).await;
            });

            app.manage(AppState {
                engine,
                #[cfg(feature = "mod-shares")]
                shares,
                #[cfg(feature = "mod-clipboard")]
                clipboard,
                #[cfg(feature = "mod-videolink")]
                videolink,
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                desktop_clipboard: std::sync::Mutex::new(None),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_local_identity,
            commands::list_peers,
            commands::trigger_scan,
            commands::rename_peer,
            commands::forget_peer,
            commands::list_friend_requests,
            commands::send_friend_request,
            commands::respond_friend_request,
            commands::unfriend,
            commands::send_files,
            commands::send_folder_android,
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
            commands::list_lan_interfaces,
            commands::open_logs,
            commands::copy_diagnostics,
            commands::shares_list_local,
            commands::shares_add_local,
            commands::shares_pick_folder_android,
            commands::shares_pick_file_android,
            commands::shares_remove_local,
            commands::shares_browse_remote_roots,
            commands::shares_browse_remote_dir,
            commands::shares_download_remote_file,
            commands::shares_preview_remote_file,
            commands::shares_get_remote_thumbnail,
            commands::clipboard_list_local,
            commands::clipboard_push_local_text,
            commands::clipboard_push_local_image_bytes,
            commands::clipboard_delete_local,
            commands::clipboard_send_to_peer,
            commands::clipboard_send_image_to_peer,
            commands::clipboard_fetch_remote,
            commands::clipboard_paste_from_system,
            commands::clipboard_copy_to_system,
            commands::clipboard_get_image_preview,
            commands::take_pending_shared_files,
            commands::take_pending_focus_target,
            commands::ensure_startup_permissions,
            commands::check_for_updates,
            commands::download_update,
            commands::install_update,
            commands::prepare_file_preview,
            commands::open_file_external,
            commands::resolve_preview_names,
            commands::get_qr_connect_info,
            commands::connect_via_qr_uri,
            commands::scan_qr_and_connect_android,
            commands::videolink_status,
            commands::videolink_start,
            commands::videolink_stop,
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn should_notify_desktop(app: &tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return true;
    };
    let visible = window.is_visible().unwrap_or(true);
    let minimized = window.is_minimized().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(true);
    !visible || minimized || !focused
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn format_bytes_human(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut val = bytes as f64 / 1024.0;
    let mut idx = 0;
    while val >= 1024.0 && idx < UNITS.len() - 1 {
        val /= 1024.0;
        idx += 1;
    }
    if val >= 100.0 {
        format!("{:.0} {}", val, UNITS[idx])
    } else {
        format!("{:.1} {}", val, UNITS[idx])
    }
}
