import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ClipboardItem,
  ClipboardPasteResult,
  CoreEvent,
  FriendRequestInfo,
  LanInterfaceInfo,
  LocalIdentityInfo,
  LocalShareRoot,
  ManualTarget,
  ModuleStatus,
  PeerInfo,
  PendingFocusTarget,
  PreviewDescriptor,
  QrConnectInfo,
  ShareEntry,
  ShareFolderDownloadResult,
  ShareRootInfo,
  ShareThumbnail,
  TransferHistoryRecord,
  UpdateAssetInfo,
  UpdateCheckResult,
  UpdateProgressEvent,
  VideoLinkStatus,
} from "./types";

export const api = {
  getLocalIdentity: () => invoke<LocalIdentityInfo>("get_local_identity"),
  listPeers: () => invoke<PeerInfo[]>("list_peers"),
  triggerScan: () => invoke<void>("trigger_scan"),
  renamePeer: (peerId: string, alias: string | null) =>
    invoke<void>("rename_peer", { peerId, alias }),
  forgetPeer: (peerId: string) => invoke<void>("forget_peer", { peerId }),

  listFriendRequests: () => invoke<FriendRequestInfo[]>("list_friend_requests"),
  sendFriendRequest: (peerId: string) =>
    invoke<void>("send_friend_request", { peerId }),
  respondFriendRequest: (peerId: string, accept: boolean) =>
    invoke<void>("respond_friend_request", { peerId, accept }),
  unfriend: (peerId: string) => invoke<void>("unfriend", { peerId }),

  sendFiles: (peerId: string, paths: string[]) =>
    invoke<TransferHistoryRecord>("send_files", { peerId, paths }),
  sendFolderAndroid: (peerId: string) =>
    invoke<TransferHistoryRecord | null>("send_folder_android", { peerId }),
  respondTransferOffer: (transferId: string, accept: boolean) =>
    invoke<void>("respond_transfer_offer", { transferId, accept }),
  cancelTransfer: (transferId: string) =>
    invoke<boolean>("cancel_transfer", { transferId }),

  listHistory: (limit = 100) =>
    invoke<TransferHistoryRecord[]>("list_history", { limit }),
  clearHistory: () => invoke<void>("clear_history"),

  getSettings: () => invoke<AppSettings>("get_settings"),
  updateSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_settings", { settings }),

  listModules: () => invoke<ModuleStatus[]>("list_modules"),
  setModuleEnabled: (id: string, enabled: boolean) =>
    invoke<ModuleStatus[]>("set_module_enabled", { id, enabled }),

  listManualTargets: () => invoke<ManualTarget[]>("list_manual_targets"),
  addManualTarget: (ip: string, label: string | null) =>
    invoke<void>("add_manual_target", { ip, label }),
  removeManualTarget: (ip: string) =>
    invoke<void>("remove_manual_target", { ip }),

  listLanInterfaces: () => invoke<LanInterfaceInfo[]>("list_lan_interfaces"),
  openLogs: () => invoke<void>("open_logs"),
  copyDiagnostics: () => invoke<string>("copy_diagnostics"),

  // Shares module
  sharesListLocal: () => invoke<LocalShareRoot[]>("shares_list_local"),
  sharesAddLocal: (path: string, name: string | null) =>
    invoke<LocalShareRoot>("shares_add_local", { path, name }),
  sharesPickFolderAndroid: () =>
    invoke<LocalShareRoot | null>("shares_pick_folder_android"),
  sharesPickFileAndroid: () =>
    invoke<LocalShareRoot | null>("shares_pick_file_android"),
  sharesRemoveLocal: (id: string) =>
    invoke<void>("shares_remove_local", { id }),
  sharesBrowseRemoteRoots: (peerId: string) =>
    invoke<ShareRootInfo[]>("shares_browse_remote_roots", { peerId }),
  sharesBrowseRemoteDir: (peerId: string, rootId: string, relPath: string) =>
    invoke<ShareEntry[]>("shares_browse_remote_dir", {
      peerId,
      rootId,
      relPath,
    }),
  sharesDownloadRemoteFile: (peerId: string, rootId: string, relPath: string) =>
    invoke<string>("shares_download_remote_file", {
      peerId,
      rootId,
      relPath,
    }),
  sharesDownloadRemoteFolder: (
    peerId: string,
    rootId: string,
    relPath: string
  ) =>
    invoke<ShareFolderDownloadResult>("shares_download_remote_folder", {
      peerId,
      rootId,
      relPath,
    }),
  sharesPreviewRemoteFile: (peerId: string, rootId: string, relPath: string) =>
    invoke<PreviewDescriptor>("shares_preview_remote_file", {
      peerId,
      rootId,
      relPath,
    }),
  sharesGetRemoteThumbnail: (
    peerId: string,
    rootId: string,
    relPath: string,
    maxSize = 160
  ) =>
    invoke<ShareThumbnail>("shares_get_remote_thumbnail", {
      peerId,
      rootId,
      relPath,
      maxSize,
    }),

  // Clipboard module
  clipboardListLocal: () => invoke<ClipboardItem[]>("clipboard_list_local"),
  clipboardPushLocalText: (text: string) =>
    invoke<ClipboardItem>("clipboard_push_local_text", { text }),
  clipboardPushLocalImageBytes: (bytes: number[]) =>
    invoke<ClipboardItem>("clipboard_push_local_image_bytes", { bytes }),
  clipboardDeleteLocal: (id: string) =>
    invoke<void>("clipboard_delete_local", { id }),
  clipboardSendToPeer: (peerId: string, text: string) =>
    invoke<void>("clipboard_send_to_peer", { peerId, text }),
  clipboardSendImageToPeer: (peerId: string, itemId: string) =>
    invoke<void>("clipboard_send_image_to_peer", { peerId, itemId }),
  clipboardFetchRemote: (peerId: string) =>
    invoke<ClipboardItem[]>("clipboard_fetch_remote", { peerId }),
  clipboardPasteFromSystem: () =>
    invoke<ClipboardPasteResult>("clipboard_paste_from_system"),
  clipboardCopyToSystem: (
    itemId: string,
    peerId: string | null = null,
    text: string | null = null
  ) => invoke<void>("clipboard_copy_to_system", { itemId, peerId, text }),
  clipboardGetImagePreview: (itemId: string, peerId: string | null = null) =>
    invoke<PreviewDescriptor>("clipboard_get_image_preview", {
      itemId,
      peerId,
    }),

  // Platform / Android bridge
  takePendingSharedFiles: () => invoke<string[]>("take_pending_shared_files"),
  takePendingFocusTarget: () =>
    invoke<PendingFocusTarget | null>("take_pending_focus_target"),
  ensureStartupPermissions: () => invoke<void>("ensure_startup_permissions"),

  // App Auto-Update
  checkForUpdates: () => invoke<UpdateCheckResult>("check_for_updates"),
  downloadUpdate: (asset: UpdateAssetInfo) =>
    invoke<string>("download_update", { asset }),
  installUpdate: (filePath: string) =>
    invoke<void>("install_update", { filePath }),

  // In-App File Preview
  prepareFilePreview: (pathOrUri: string) =>
    invoke<PreviewDescriptor>("prepare_file_preview", { pathOrUri }),
  openFileExternal: (pathOrUri: string) =>
    invoke<void>("open_file_external", { pathOrUri }),
  resolvePreviewNames: (paths: string[]) =>
    invoke<Record<string, string>>("resolve_preview_names", { paths }),

  // QR Quick Connect
  getQrConnectInfo: () => invoke<QrConnectInfo>("get_qr_connect_info"),
  connectViaQrUri: (uri: string) =>
    invoke<PeerInfo>("connect_via_qr_uri", { uri }),
  scanQrAndConnectAndroid: () =>
    invoke<PeerInfo | null>("scan_qr_and_connect_android"),

  // Video Link module
  videolinkStatus: () => invoke<VideoLinkStatus>("videolink_status"),
  videolinkStart: (pathOrUri: string, password: string) =>
    invoke<VideoLinkStatus>("videolink_start", { pathOrUri, password }),
  videolinkStop: () => invoke<VideoLinkStatus>("videolink_stop"),

  onCoreEvent: (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
    listen<CoreEvent>("landa://event", (ev) => handler(ev.payload)),
  onIdentityUpdated: (handler: () => void): Promise<UnlistenFn> =>
    listen<void>("landa://identity-updated", () => handler()),
  onUpdateProgress: (
    handler: (event: UpdateProgressEvent) => void
  ): Promise<UnlistenFn> =>
    listen<UpdateProgressEvent>("landa://update-progress", (ev) =>
      handler(ev.payload)
    ),
  onFocusTarget: (
    handler: (target: PendingFocusTarget) => void
  ): Promise<UnlistenFn> =>
    listen<PendingFocusTarget>("landa://focus-target", (ev) =>
      handler(ev.payload)
    ),
};

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let val = bytes / 1024;
  let idx = 0;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx++;
  }
  return `${val.toFixed(val >= 100 ? 0 : 1)} ${units[idx]}`;
}

export function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function formatEta(secs: number | null): string {
  if (secs === null || secs < 0) return "--:--";
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}
