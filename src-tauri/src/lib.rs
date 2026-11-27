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
use landa_mod_clipboard::{ClipboardContentKind, ClipboardItem, ClipboardModule};
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
                m.set_open_source_hook(android_bridge::make_open_source_hook(app.handle().clone()));
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

            let android_network_listener = Arc::new(commands::AndroidNetworkListenerState::new());

            #[cfg(target_os = "android")]
            {
                let handle_for_net = app.handle().clone();
                let engine_for_net = Arc::clone(&engine);
                let listener_state_for_net = Arc::clone(&android_network_listener);
                tauri::async_runtime::spawn(async move {
                    for (attempt, delay_ms) in [0u64, 500, 1500].into_iter().enumerate() {
                        if delay_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        }
                        match register_android_network_listener_once(
                            &handle_for_net,
                            &engine_for_net,
                            &listener_state_for_net,
                        )
                        .await
                        {
                            Ok(()) => break,
                            Err(err) => {
                                tracing::warn!(
                                    "failed to register Android network listener (attempt {}): {err}",
                                    attempt + 1
                                );
                            }
                        }
                    }
                });

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
                            let is_active =
                                info.status == "waiting_approval" || info.status == "transferring";
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
                        #[cfg(feature = "mod-clipboard")]
                        landa_core::CoreEvent::ModuleEvent {
                            module_id,
                            event_name,
                            data,
                        } if module_id == "clipboard" => {
                            let settings = engine_for_events.get_settings();
                            #[cfg(target_os = "android")]
                            let (is_android, is_foreground, should_notify_os) = {
                                let fg = if let Some(bridge) = handle
                                    .try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                                {
                                    bridge.is_app_in_foreground_async().await
                                } else {
                                    false
                                };
                                (true, fg, !fg)
                            };
                            #[cfg(not(any(target_os = "android", target_os = "ios")))]
                            let (is_android, is_foreground, should_notify_os) = {
                                let notify = should_notify_desktop(&handle);
                                (false, !notify, notify)
                            };
                            #[cfg(target_os = "ios")]
                            let (is_android, is_foreground, should_notify_os) = (false, true, false);

                            let ctx = ClipboardReceiveContext {
                                auto_copy_clipboard: settings.auto_copy_clipboard,
                                is_android,
                                is_foreground,
                                should_notify_os,
                                lang: settings.language,
                            };
                            let sink = TauriClipboardReceiveSink { handle: &handle };
                            let _ = process_clipboard_module_event(event_name, data, &ctx, &sink)
                                .await;
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

            let preview_server =
                tauri::async_runtime::block_on(preview::PreviewServer::start(app.handle().clone()))
                    .map_err(|e| {
                        Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>
                    })?;
            app.manage(preview_server);

            let app_for_sweep = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                preview::sweep_old_preview_cache(&app_for_sweep).await;
                updater::cleanup_updates_cache(&app_for_sweep).await;
            });

            app.manage(AppState {
                engine,
                android_network_listener,
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
            commands::pick_download_dir_android,
            commands::shares_remove_local,
            commands::shares_browse_remote_roots,
            commands::shares_browse_remote_dir,
            commands::shares_download_remote_file,
            commands::shares_download_remote_folder,
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
            commands::get_cache_size,
            commands::clear_cache,
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

#[cfg(target_os = "android")]
pub(crate) async fn register_android_network_listener_once(
    app: &tauri::AppHandle,
    engine: &Arc<LandaEngine>,
    listener_state: &Arc<commands::AndroidNetworkListenerState>,
) -> Result<(), String> {
    let Some(bridge) = app.try_state::<android_bridge::AndroidBridge<tauri::Wry>>() else {
        let err = "Android bridge unavailable".to_string();
        listener_state.record_result(&Err(err.clone()));
        return Err(err);
    };
    let engine_for_net = Arc::clone(engine);
    let res = bridge
        .register_network_listener_async(move || {
            let engine_for_scan = Arc::clone(&engine_for_net);
            tauri::async_runtime::spawn_blocking(move || {
                engine_for_scan.trigger_scan();
            });
        })
        .await;
    listener_state.record_result(&res);
    res
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

#[cfg(feature = "mod-clipboard")]
struct ClipboardReceiveContext {
    auto_copy_clipboard: bool,
    is_android: bool,
    is_foreground: bool,
    should_notify_os: bool,
    lang: String,
}

#[cfg(feature = "mod-clipboard")]
fn format_clipboard_notification_body(is_image: bool, lang: &str) -> &'static str {
    if is_image {
        if lang == "en" {
            "Clipboard image received"
        } else {
            "Получено изображение в буфер"
        }
    } else if lang == "en" {
        "Clipboard text received"
    } else {
        "Получен текст в буфер"
    }
}

#[cfg(feature = "mod-clipboard")]
trait ClipboardReceiveSink: Sync {
    fn write_system_text<'a>(
        &'a self,
        text: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;

    fn notify_received<'a>(
        &'a self,
        peer_name: &'a str,
        is_image: bool,
        body: &'a str,
        lang: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;
}

#[cfg(feature = "mod-clipboard")]
struct TauriClipboardReceiveSink<'a> {
    handle: &'a tauri::AppHandle,
}

#[cfg(feature = "mod-clipboard")]
impl ClipboardReceiveSink for TauriClipboardReceiveSink<'_> {
    fn write_system_text<'a>(
        &'a self,
        text: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(target_os = "android")]
            {
                let bridge = self
                    .handle
                    .try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                    .ok_or_else(|| "Android bridge unavailable".to_string())?;
                bridge.write_system_clipboard_text_async(text).await
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let state = self
                    .handle
                    .try_state::<AppState>()
                    .ok_or_else(|| "AppState unavailable".to_string())?;
                commands::write_desktop_clipboard_text(&state.desktop_clipboard, text)
            }
            #[cfg(target_os = "ios")]
            {
                let _ = text;
                Ok(())
            }
        })
    }

    fn notify_received<'a>(
        &'a self,
        peer_name: &'a str,
        is_image: bool,
        body: &'a str,
        lang: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(target_os = "android")]
            {
                let _ = body;
                let bridge = self
                    .handle
                    .try_state::<android_bridge::AndroidBridge<tauri::Wry>>()
                    .ok_or_else(|| "Android bridge unavailable".to_string())?;
                bridge
                    .show_clipboard_received_notification_async(peer_name, is_image, lang)
                    .await
                    .map(|_| ())
            }

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let _ = (is_image, lang);
                self.handle
                    .notification()
                    .builder()
                    .title(format!("Landa · {peer_name}"))
                    .body(body)
                    .extra("target", "clipboard")
                    .show()
                    .map_err(|e| e.to_string())
            }

            #[cfg(target_os = "ios")]
            {
                let _ = (peer_name, is_image, body, lang);
                Ok(())
            }
        })
    }
}

#[cfg(feature = "mod-clipboard")]
async fn process_clipboard_module_event(
    event_name: &str,
    data: &serde_json::Value,
    ctx: &ClipboardReceiveContext,
    sink: &dyn ClipboardReceiveSink,
) -> Result<(), String> {
    if event_name != "received" {
        return Ok(());
    }
    let Ok(item) = serde_json::from_value::<ClipboardItem>(data.clone()) else {
        return Ok(());
    };
    if item.source_peer_id.is_none() {
        return Ok(());
    }

    let peer_name = item.source_peer_name.as_deref().unwrap_or("Peer");
    let is_image = item.kind == ClipboardContentKind::Image;

    if !is_image && ctx.auto_copy_clipboard {
        if ctx.is_android && !ctx.is_foreground {
            tracing::debug!("skipping background system clipboard write on Android");
        } else if let Err(err) = sink.write_system_text(&item.text).await {
            tracing::warn!("auto-copy incoming clipboard text failed: {err}");
        }
    }

    if ctx.should_notify_os {
        let body = format_clipboard_notification_body(is_image, &ctx.lang);
        if let Err(err) = sink.notify_received(peer_name, is_image, body, &ctx.lang).await {
            tracing::warn!(
                peer = peer_name,
                is_image,
                "failed to show incoming clipboard notification: {err}"
            );
            return Err(err);
        }
    }

    Ok(())
}

#[cfg(all(test, feature = "mod-clipboard"))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeClipboardSink {
        copied_texts: Mutex<Vec<String>>,
        notifications: Mutex<Vec<(String, bool, String, String)>>,
        notify_error: Option<String>,
    }

    impl ClipboardReceiveSink for FakeClipboardSink {
        fn write_system_text<'a>(
            &'a self,
            text: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>
        {
            Box::pin(async move {
                self.copied_texts.lock().unwrap().push(text.to_string());
                Ok(())
            })
        }

        fn notify_received<'a>(
            &'a self,
            peer_name: &'a str,
            is_image: bool,
            body: &'a str,
            lang: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>
        {
            Box::pin(async move {
                if let Some(err) = &self.notify_error {
                    return Err(err.clone());
                }
                self.notifications.lock().unwrap().push((
                    peer_name.to_string(),
                    is_image,
                    body.to_string(),
                    lang.to_string(),
                ));
                Ok(())
            })
        }
    }

    fn make_item(kind: ClipboardContentKind, text: &str, source_peer_id: Option<&str>) -> serde_json::Value {
        serde_json::to_value(ClipboardItem {
            id: "item-1".into(),
            kind,
            text: text.into(),
            image_path: None,
            image_width: if kind == ClipboardContentKind::Image { Some(10) } else { None },
            image_height: if kind == ClipboardContentKind::Image { Some(10) } else { None },
            size_bytes: text.len() as u64,
            content_hash: "hash".into(),
            source_peer_id: source_peer_id.map(str::to_string),
            source_peer_name: source_peer_id.map(|_| "Alice".to_string()),
            created_at: 1000,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn clipboard_receive_copies_text_when_auto_copy_enabled_and_notifies_when_unfocused() {
        let sink = FakeClipboardSink::default();
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: true,
            is_android: false,
            is_foreground: false,
            should_notify_os: true,
            lang: "ru".into(),
        };
        let data = make_item(ClipboardContentKind::Text, "hello from alice", Some("peer-a"));

        let res = process_clipboard_module_event("received", &data, &ctx, &sink).await;
        assert_eq!(res, Ok(()));

        assert_eq!(
            *sink.copied_texts.lock().unwrap(),
            vec!["hello from alice".to_string()]
        );
        let notifs = sink.notifications.lock().unwrap();
        assert_eq!(
            *notifs,
            vec![(
                "Alice".to_string(),
                false,
                "Получен текст в буфер".to_string(),
                "ru".to_string()
            )]
        );
        assert!(!notifs[0].2.contains("hello from alice"));
    }

    #[tokio::test]
    async fn clipboard_receive_respects_disabled_auto_copy_setting_and_still_notifies() {
        let sink = FakeClipboardSink::default();
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: false,
            is_android: false,
            is_foreground: false,
            should_notify_os: true,
            lang: "en".into(),
        };
        let data = make_item(ClipboardContentKind::Text, "secret", Some("peer-a"));

        let res = process_clipboard_module_event("received", &data, &ctx, &sink).await;
        assert_eq!(res, Ok(()));

        assert!(sink.copied_texts.lock().unwrap().is_empty());
        let notifs = sink.notifications.lock().unwrap();
        assert_eq!(
            *notifs,
            vec![(
                "Alice".to_string(),
                false,
                "Clipboard text received".to_string(),
                "en".to_string()
            )]
        );
        assert!(!notifs[0].2.contains("secret"));
    }

    #[tokio::test]
    async fn clipboard_receive_skips_android_background_clipboard_write_and_notifies() {
        let sink = FakeClipboardSink::default();
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: true,
            is_android: true,
            is_foreground: false,
            should_notify_os: true,
            lang: "ru".into(),
        };
        let data = make_item(ClipboardContentKind::Text, "bg text", Some("peer-a"));

        let res = process_clipboard_module_event("received", &data, &ctx, &sink).await;
        assert_eq!(res, Ok(()));

        assert!(sink.copied_texts.lock().unwrap().is_empty());
        let notifs = sink.notifications.lock().unwrap();
        assert_eq!(
            *notifs,
            vec![(
                "Alice".to_string(),
                false,
                "Получен текст в буфер".to_string(),
                "ru".to_string()
            )]
        );
        assert!(!notifs[0].2.contains("bg text"));
    }

    #[tokio::test]
    async fn clipboard_receive_image_does_not_write_text_clipboard_and_notifies() {
        let sink = FakeClipboardSink::default();
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: true,
            is_android: true,
            is_foreground: false,
            should_notify_os: true,
            lang: "en".into(),
        };
        let data = make_item(ClipboardContentKind::Image, "PNG 10×10", Some("peer-a"));

        let res = process_clipboard_module_event("received", &data, &ctx, &sink).await;
        assert_eq!(res, Ok(()));

        assert!(sink.copied_texts.lock().unwrap().is_empty());
        assert_eq!(
            *sink.notifications.lock().unwrap(),
            vec![(
                "Alice".to_string(),
                true,
                "Clipboard image received".to_string(),
                "en".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn clipboard_local_updated_event_does_not_auto_copy_or_notify() {
        let sink = FakeClipboardSink::default();
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: true,
            is_android: false,
            is_foreground: false,
            should_notify_os: true,
            lang: "ru".into(),
        };
        let data = make_item(ClipboardContentKind::Text, "local", None);

        let res = process_clipboard_module_event("updated", &data, &ctx, &sink).await;
        assert_eq!(res, Ok(()));

        assert!(sink.copied_texts.lock().unwrap().is_empty());
        assert!(sink.notifications.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn clipboard_receive_returns_error_when_notification_fails_without_leaking_text() {
        let sink = FakeClipboardSink {
            notify_error: Some("Failed to show clipboard notification: SecurityException".into()),
            ..Default::default()
        };
        let ctx = ClipboardReceiveContext {
            auto_copy_clipboard: true,
            is_android: true,
            is_foreground: false,
            should_notify_os: true,
            lang: "ru".into(),
        };
        let data = make_item(ClipboardContentKind::Text, "otp-994411", Some("peer-a"));

        let res = process_clipboard_module_event("received", &data, &ctx, &sink).await;

        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("SecurityException"));
        assert!(!err.contains("otp-994411"));
        assert!(sink.copied_texts.lock().unwrap().is_empty());
        assert!(sink.notifications.lock().unwrap().is_empty());
    }
}
