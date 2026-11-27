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
  os: DeviceOs;
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
  savedPaths?: string[];
}

export interface TransferHistoryRecord {
  id: string;
  direction: "send" | "receive";
  peerId: string;
  peerName: string;
  fileCount: number;
  totalBytes: number;
  savedPaths: string[];
  relativePaths?: string[];
  status: string;
  errorMessage: string | null;
  createdAt: number;
}

export interface AppSettings {
  deviceName: string;
  downloadDir: string;
  autoAcceptFromFriends: boolean;
  autoCopyClipboard?: boolean;
  minimizeToTray: boolean;
  leftHandedMode: boolean;
  theme: "dark" | "light" | "system";
  language: "ru" | "en";
  discoveryIntervalSecs: number;
  enabledModules: Record<string, boolean>;
  excludedSubnets?: string[];
}

export interface LanInterfaceInfo {
  name: string;
  ip: string;
  netmask: string;
  broadcast: string;
  subnetCidr: string;
  enabled: boolean;
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

export interface ShareThumbnail {
  mimeType: string;
  dataUrl: string;
}

export interface ShareFolderDownloadResult {
  folderName: string;
  fileCount: number;
  totalBytes: number;
  savedPaths: string[];
  destination: string;
}

export interface VideoLinkStatus {
  running: boolean;
  url: string | null;
  port: number | null;
  fileName: string | null;
  sizeBytes: number | null;
}

export type ClipboardContentKind = "text" | "image";

export interface ClipboardItem {
  id: string;
  kind: ClipboardContentKind;
  text: string;
  imagePath: string | null;
  imageWidth: number | null;
  imageHeight: number | null;
  sizeBytes: number;
  contentHash: string;
  sourcePeerId: string | null;
  sourcePeerName: string | null;
  createdAt: number;
}

export type ClipboardPasteResult =
  | { kind: "empty" }
  | { kind: "text"; text: string }
  | { kind: "image"; item: ClipboardItem };

export interface UpdateAssetInfo {
  platform: string;
  arch: string;
  format: string;
  fileName: string;
  size: number;
  sha256: string;
  downloadUrl: string;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseTag: string;
  releaseNotes: string;
  publishedAt: string | null;
  asset: UpdateAssetInfo | null;
}

export interface UpdateProgressEvent {
  stage: "downloading" | "verifying" | "ready" | "installing" | "failed";
  downloadedBytes: number;
  totalBytes: number;
  filePath: string | null;
  error: string | null;
}

export type PreviewKind = "image" | "video" | "audio" | "text" | "unsupported";

export interface PreviewDescriptor {
  kind: PreviewKind;
  fileName: string;
  mimeType: string;
  sizeBytes: number;
  url: string | null;
  textContent: string | null;
  truncated: boolean;
  sourcePath: string;
}

export interface PendingFocusTarget {
  target: string;
  transferId?: string | null;
  peerId?: string | null;
}

export interface QrConnectInfo {
  uri: string;
  svg: string;
  ip: string;
  port: number;
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
