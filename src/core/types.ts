export type DeviceOs =
  | "windows"
  | "linux"
  | "macos"
  | "android"
  | "ios"
  | "unknown";

export type DeviceCategory = "desktop" | "phone" | "unknown";

export interface LocalIdentityInfo {
  peerId: string;
  publicKeyHex: string;
  deviceName: string;
  quicPort: number;
}

export interface PeerInfo {
  peerId: string;
  publicKeyHex: string;
  sasCode: string;
  deviceName: string;
  aliasName: string | null;
  displayName: string;
  os: DeviceOs;
  category: DeviceCategory;
  ip: string | null;
  quicPort: number | null;
  isTrusted: boolean;
  isOnline: boolean;
  lastSeenAt: number;
}

export interface FriendRequestInfo {
  peerId: string;
  deviceName: string;
  os: DeviceOs;
  category: DeviceCategory;
  sasCode: string;
  direction: "incoming" | "outgoing";
  createdAt: number;
}

export interface IncomingTransferOfferInfo {
  transferId: string;
  peerId: string;
  peerName: string;
  fileCount: number;
  totalBytes: number;
  fileNames: string[];
}

export interface TransferProgressInfo {
  transferId: string;
  direction: "send" | "receive";
  peerId: string;
  peerName: string;
  currentFile: string;
  filesDone: number;
  fileCount: number;
  bytesTransferred: number;
  totalBytes: number;
  speedBps: number;
  etaSecs: number | null;
  status: "waiting_approval" | "transferring" | "completed" | "cancelled" | "rejected" | "failed";
  error: string | null;
}

export interface TransferHistoryRecord {
  id: string;
  direction: "send" | "receive";
  peerId: string;
  peerName: string;
  fileCount: number;
  totalBytes: number;
  savedPaths: string[];
  status: string;
  errorMessage: string | null;
  createdAt: number;
}

export interface AppSettings {
  deviceName: string;
  downloadDir: string;
  autoAcceptFromFriends: boolean;
  minimizeToTray: boolean;
  leftHandedMode: boolean;
  theme: "dark" | "light" | "system";
  language: "ru" | "en";
  discoveryIntervalSecs: number;
  enabledModules: Record<string, boolean>;
}

export interface ModuleStatus {
  id: string;
  enabled: boolean;
  available: boolean;
}

export interface ManualTarget {
  ip: string;
  label: string | null;
}

export interface LocalShareRoot {
  id: string;
  name: string;
  path: string;
  isDir: boolean;
}

export interface ShareRootInfo {
  id: string;
  name: string;
  isDir: boolean;
}

export interface ShareEntry {
  name: string;
  relPath: string;
  isDir: boolean;
  sizeBytes: number;
  modifiedMs: number;
}

export interface ClipboardItem {
  id: string;
  text: string;
  contentHash: string;
  sourcePeerId: string | null;
  sourcePeerName: string | null;
  createdAt: number;
}

export type CoreEvent =
  | { type: "peersChanged" }
  | { type: "friendRequestsChanged" }
  | { type: "incomingTransferOffer"; payload: IncomingTransferOfferInfo }
  | { type: "transferProgress"; payload: TransferProgressInfo }
  | { type: "historyChanged" }
  | {
      type: "moduleEvent";
      payload: { moduleId: string; eventName: string; data: unknown };
    };
