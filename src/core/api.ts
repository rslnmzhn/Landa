import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ClipboardItem,
  CoreEvent,
  FriendRequestInfo,
  LocalIdentityInfo,
  LocalShareRoot,
  ManualTarget,
  ModuleStatus,
  PeerInfo,
  ShareEntry,
  ShareRootInfo,
  TransferHistoryRecord,
} from "./types";

export const api = {
  getLocalIdentity: () => invoke<LocalIdentityInfo>("get_local_identity"),
  listPeers: () => invoke<PeerInfo[]>("list_peers"),
  triggerScan: () => invoke<void>("trigger_scan"),
  renamePeer: (peerId: string, alias: string | null) =>
    invoke<void>("rename_peer", { peerId, alias }),

  listFriendRequests: () => invoke<FriendRequestInfo[]>("list_friend_requests"),
  sendFriendRequest: (peerId: string) =>
    invoke<void>("send_friend_request", { peerId }),
  respondFriendRequest: (peerId: string, accept: boolean) =>
    invoke<void>("respond_friend_request", { peerId, accept }),
  unfriend: (peerId: string) => invoke<void>("unfriend", { peerId }),

  sendFiles: (peerId: string, paths: string[]) =>
    invoke<TransferHistoryRecord>("send_files", { peerId, paths }),
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

  // Shares module
  sharesListLocal: () => invoke<LocalShareRoot[]>("shares_list_local"),
  sharesAddLocal: (path: string, name: string | null) =>
    invoke<LocalShareRoot>("shares_add_local", { path, name }),
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

  // Clipboard module
  clipboardListLocal: () => invoke<ClipboardItem[]>("clipboard_list_local"),
  clipboardPushLocalText: (text: string) =>
    invoke<ClipboardItem>("clipboard_push_local_text", { text }),
  clipboardDeleteLocal: (id: string) =>
    invoke<void>("clipboard_delete_local", { id }),
  clipboardSendToPeer: (peerId: string, text: string) =>
    invoke<void>("clipboard_send_to_peer", { peerId, text }),
  clipboardFetchRemote: (peerId: string) =>
    invoke<ClipboardItem[]>("clipboard_fetch_remote", { peerId }),

  // Platform / Android bridge
  takePendingSharedFiles: () => invoke<string[]>("take_pending_shared_files"),
  hasAllFilesAccess: () => invoke<boolean>("has_all_files_access"),
  requestAllFilesAccess: () => invoke<void>("request_all_files_access"),
  ensureStartupPermissions: () => invoke<void>("ensure_startup_permissions"),

  onCoreEvent: (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
    listen<CoreEvent>("landa://event", (ev) => handler(ev.payload)),
  onIdentityUpdated: (handler: () => void): Promise<UnlistenFn> =>
    listen<void>("landa://identity-updated", () => handler()),
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
