import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowDownToLine,
  Check,
  Copy,
  Edit3,
  FileUp,
  FolderUp,
  Laptop,
  QrCode,
  RefreshCw,
  ScanLine,
  ShieldCheck,
  Smartphone,
  Upload,
  UserMinus,
  UserPlus,
  Wifi,
  X,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type {
  FriendRequestInfo,
  LocalIdentityInfo,
  PeerInfo,
  QrConnectInfo,
  UpdateCheckResult,
  UpdateProgressEvent,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassInput,
  GlassModal,
} from "../../components/glass/GlassPrimitives";

interface DevicesViewProps {
  t: TranslationDict;
  identity: LocalIdentityInfo | null;
  peers: PeerInfo[];
  friendRequests: FriendRequestInfo[];
  leftHanded: boolean;
  updateInfo: UpdateCheckResult | null;
  updateProgress: UpdateProgressEvent | null;
  onUpdateInfoChange: (next: UpdateCheckResult | null) => void;
  onError: (msg: string) => void;
  onNotice?: (msg: string) => void;
}

function pathBasename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function DevicesView({
  t,
  identity,
  peers,
  friendRequests,
  leftHanded,
  updateInfo,
  updateProgress,
  onUpdateInfoChange,
  onError,
  onNotice,
}: DevicesViewProps) {
  const [scanning, setScanning] = useState(false);
  const [renameTarget, setRenameTarget] = useState<PeerInfo | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [downloadedPath, setDownloadedPath] = useState<string | null>(null);
  const [updateHint, setUpdateHint] = useState<string | null>(null);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  const [hoveredPeerId, setHoveredPeerId] = useState<string | null>(null);
  const [droppedPaths, setDroppedPaths] = useState<string[]>([]);
  const [qrModalOpen, setQrModalOpen] = useState(false);
  const [qrInfo, setQrInfo] = useState<QrConnectInfo | null>(null);
  const [qrLoading, setQrLoading] = useState(false);
  const [qrUriDraft, setQrUriDraft] = useState("");
  const [qrConnecting, setQrConnecting] = useState(false);
  const [qrConnectedPeer, setQrConnectedPeer] = useState<PeerInfo | null>(null);

  const peersRef = useRef(peers);
  peersRef.current = peers;
  const tRef = useRef(t);
  tRef.current = t;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const onNoticeRef = useRef(onNotice);
  onNoticeRef.current = onNotice;

  const isAndroid =
    identity?.os === "android" ||
    navigator.userAgent.toLowerCase().includes("android");

  const sendPathsToPeer = async (peerId: string, paths: string[]) => {
    if (paths.length === 0) return;
    const peer = peersRef.current.find((p) => p.peerId === peerId);
    if (peer) {
      onNoticeRef.current?.(`${tRef.current.devices.dropSending}: ${peer.displayName}`);
    }
    try {
      await api.sendFiles(peerId, paths);
    } catch (e) {
      onErrorRef.current(String(e));
    }
  };

  useEffect(() => {
    if (isAndroid) return;

    const findOnlinePeerAtPosition = (position: { x: number; y: number }): string | null => {
      const dpr = window.devicePixelRatio || 1;
      const x = position.x / dpr;
      const y = position.y / dpr;
      const el = document.elementFromPoint(x, y);
      const card = el?.closest<HTMLElement>("[data-peer-id]");
      const peerId = card?.dataset.peerId ?? null;
      if (!peerId) return null;
      const peer = peersRef.current.find((p) => p.peerId === peerId && p.isOnline);
      return peer ? peer.peerId : null;
    };

    let disposed = false;
    let unlisten: (() => void) | undefined;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setIsDraggingFiles(true);
          setHoveredPeerId(findOnlinePeerAtPosition(payload.position));
        } else if (payload.type === "leave") {
          setIsDraggingFiles(false);
          setHoveredPeerId(null);
        } else if (payload.type === "drop") {
          setIsDraggingFiles(false);
          setHoveredPeerId(null);
          if (payload.paths.length === 0) return;

          const targetPeerId = findOnlinePeerAtPosition(payload.position);
          if (targetPeerId) {
            void sendPathsToPeer(targetPeerId, payload.paths);
            return;
          }

          const onlinePeers = peersRef.current.filter((p) => p.isOnline);
          if (onlinePeers.length === 0) {
            onErrorRef.current(tRef.current.devices.dropNoOnline);
            return;
          }
          setDroppedPaths(payload.paths);
        }
      })
      .then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [isAndroid]);

  const handleScan = async () => {
    setScanning(true);
    try {
      await api.triggerScan();
    } catch (e) {
      onError(String(e));
    } finally {
      setTimeout(() => setScanning(false), 600);
    }
  };

  const handleSendFiles = async (peerId: string, directory: boolean) => {
    try {
      if (directory && isAndroid) {
        await api.sendFolderAndroid(peerId);
        return;
      }
      const selected = await open({
        multiple: !directory,
        directory,
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      if (paths.length === 0) return;
      await api.sendFiles(peerId, paths);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleSaveRename = async () => {
    if (!renameTarget) return;
    try {
      await api.renamePeer(
        renameTarget.peerId,
        aliasDraft.trim() ? aliasDraft.trim() : null
      );
      setRenameTarget(null);
    } catch (e) {
      onError(String(e));
    }
  };

  const runUpdateCheck = async () => {
    setCheckingUpdate(true);
    setUpdateHint(null);
    setDownloadedPath(null);
    try {
      const res = await api.checkForUpdates();
      onUpdateInfoChange(res);
    } catch (e) {
      onError(String(e));
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleOpenUpdateModal = () => {
    setUpdateModalOpen(true);
    if (isAndroid && downloadedPath) {
      setUpdateHint(t.update.savedToDownloadsHint.replace("{path}", downloadedPath));
    } else {
      setUpdateHint(null);
    }
    if (!updateInfo) {
      void runUpdateCheck();
    }
  };

  const handleDownloadAndInstall = async () => {
    if (!updateInfo?.asset) return;
    setInstallingUpdate(true);
    if (!isAndroid) {
      setUpdateHint(null);
    }
    try {
      if (isAndroid) {
        if (!downloadedPath) {
          const savedPath = await api.downloadUpdate(updateInfo.asset);
          setDownloadedPath(savedPath);
          setUpdateHint(t.update.savedToDownloadsHint.replace("{path}", savedPath));
        } else {
          await api.installUpdate(downloadedPath);
        }
        return;
      }
      const path = downloadedPath ?? (await api.downloadUpdate(updateInfo.asset));
      setDownloadedPath(path);
      await api.installUpdate(path);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("INSTALL_PERMISSION_REQUIRED")) {
        setUpdateHint(t.update.permissionHint);
      } else {
        onError(msg);
      }
    } finally {
      setInstallingUpdate(false);
    }
  };

  const handleOpenQrModal = async () => {
    setQrModalOpen(true);
    setQrLoading(true);
    try {
      const info = await api.getQrConnectInfo();
      setQrInfo(info);
    } catch (e) {
      onError(String(e));
    } finally {
      setQrLoading(false);
    }
  };

  const handleScanQrAndroid = async () => {
    try {
      const peer = await api.scanQrAndConnectAndroid();
      if (!peer) return;
      setQrConnectedPeer(peer);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleConnectQrUri = async () => {
    const trimmed = qrUriDraft.trim();
    if (!trimmed) return;
    setQrConnecting(true);
    try {
      const peer = await api.connectViaQrUri(trimmed);
      setQrUriDraft("");
      setQrModalOpen(false);
      setQrConnectedPeer(peer);
    } catch (e) {
      onError(String(e));
    } finally {
      setQrConnecting(false);
    }
  };

  const isDownloading =
    installingUpdate &&
    (updateProgress?.stage === "downloading" ||
      updateProgress?.stage === "verifying");
  const downloadPct =
    updateProgress && updateProgress.totalBytes > 0
      ? Math.min(
          100,
          Math.round(
            (updateProgress.downloadedBytes / updateProgress.totalBytes) * 100
          )
        )
      : 0;

  return (
    <div className="space-y-5">
      {/* Minimal Header */}
      <div
        className={`flex items-center justify-between gap-3 ${
          leftHanded ? "flex-row-reverse" : ""
        }`}
      >
        <div className="flex items-center gap-3 min-w-0">
          <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
            {t.devices.title}
          </h1>
          {identity && (
            <GlassBadge tone="neutral">
              <span className="h-2 w-2 rounded-full bg-emerald-400" />
              <span className="truncate max-w-[140px] text-xs">
                {identity.deviceName}
              </span>
            </GlassBadge>
          )}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {isAndroid && (
            <GlassButton
              onClick={handleScanQrAndroid}
              size="sm"
              title={t.devices.scanQr}
              aria-label={t.devices.scanQr}
              className="px-3"
            >
              <ScanLine className="h-4 w-4" />
            </GlassButton>
          )}

          <GlassButton
            onClick={handleOpenQrModal}
            size="sm"
            title={t.devices.showQr}
            aria-label={t.devices.showQr}
            className="px-3"
          >
            <QrCode className="h-4 w-4" />
          </GlassButton>

          <GlassButton
            onClick={handleOpenUpdateModal}
            size="sm"
            title={t.update.tooltip}
            aria-label={t.update.tooltip}
            className="relative px-3"
          >
            <ArrowDownToLine className="h-4 w-4" />
            {updateInfo?.hasUpdate && (
              <span className="absolute right-2 top-2 h-2 w-2 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.85)]" />
            )}
          </GlassButton>

          <GlassButton
            onClick={handleScan}
            size="sm"
            title={t.devices.scan}
            aria-label={t.devices.scan}
          >
            <RefreshCw
              className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`}
            />
            <span className="hidden sm:inline">{t.devices.scan}</span>
          </GlassButton>
        </div>
      </div>

      {/* Pending Friend Requests */}
      {friendRequests.length > 0 && (
        <GlassCard elevated className="p-4 space-y-2.5">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
            {t.devices.requestsTitle}
          </div>
          <div className="space-y-2">
            {friendRequests.map((req) => (
              <div
                key={req.peerId}
                className="flex flex-wrap items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3"
              >
                <div className="min-w-0">
                  <div className="font-medium text-slate-900/90 dark:text-white/90">
                    {req.deviceName}
                  </div>
                  <div className="flex items-center gap-2 text-xs text-slate-700/70 dark:text-white/55">
                    <span>
                      {req.direction === "incoming"
                        ? t.devices.incomingReq
                        : t.devices.outgoingReq}
                    </span>
                    <span>·</span>
                    <span className="font-mono">
                      {t.devices.sasLabel} {req.sasCode}
                    </span>
                  </div>
                </div>
                {req.direction === "incoming" && (
                  <div className="flex items-center gap-2">
                    <GlassButton
                      variant="primary"
                      size="sm"
                      onClick={() =>
                        api
                          .respondFriendRequest(req.peerId, true)
                          .catch((e) => onError(String(e)))
                      }
                    >
                      <Check className="h-4 w-4" />
                      <span>{t.devices.accept}</span>
                    </GlassButton>
                    <GlassButton
                      variant="danger"
                      size="sm"
                      onClick={() =>
                        api
                          .respondFriendRequest(req.peerId, false)
                          .catch((e) => onError(String(e)))
                      }
                    >
                      <X className="h-4 w-4" />
                    </GlassButton>
                  </div>
                )}
              </div>
            ))}
          </div>
        </GlassCard>
      )}

      {/* Device Grid */}
      {peers.length === 0 ? (
        <GlassCard className="flex flex-col items-center justify-center px-6 py-16 text-center">
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-indigo-500/15 border border-indigo-400/25">
            <Wifi className="h-6 w-6 text-indigo-600 dark:text-indigo-300" />
          </div>
          <h3 className="text-base font-medium text-slate-900/90 dark:text-white/90">
            {t.devices.emptyTitle}
          </h3>
          <p className="mt-1 max-w-sm text-xs text-slate-700/65 dark:text-white/50">
            {t.devices.emptyDesc}
          </p>
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {peers.map((peer) => {
            const canDrop = isDraggingFiles && peer.isOnline;
            const isDropHovered = canDrop && hoveredPeerId === peer.peerId;
            return (
              <GlassCard
                key={peer.peerId}
                data-peer-id={peer.peerId}
                className={`relative overflow-hidden flex flex-col justify-between gap-4 p-5 transition-all duration-200 ${
                  isDropHovered
                    ? "ring-2 ring-indigo-400/90 bg-indigo-500/15 dark:bg-indigo-400/15 shadow-[0_0_36px_rgba(99,102,241,0.35)] scale-[1.01]"
                    : canDrop
                      ? "ring-1 ring-indigo-400/50 bg-white/20 dark:bg-white/[0.09]"
                      : "hover:bg-white/20 dark:hover:bg-white/[0.1]"
                }`}
              >
                {canDrop && (
                  <div
                    aria-hidden="true"
                    className={`pointer-events-none absolute inset-0 z-20 flex items-center justify-center rounded-3xl transition-all duration-200 ${
                      isDropHovered
                        ? "bg-indigo-500/20 dark:bg-indigo-500/25 backdrop-blur-md opacity-100"
                        : "bg-indigo-500/5 backdrop-blur-[2px] opacity-90"
                    }`}
                  >
                    <div
                      className={`inline-flex items-center gap-2 rounded-full border px-4 py-2 text-xs font-semibold shadow-lg transition-transform duration-200 ${
                        isDropHovered
                          ? "scale-105 border-indigo-300/70 bg-indigo-500/85 text-white"
                          : "border-white/40 dark:border-white/20 bg-white/70 dark:bg-slate-900/70 text-slate-900 dark:text-white/90"
                      }`}
                    >
                      <Upload className="h-3.5 w-3.5" />
                      <span>{t.devices.dropHint}</span>
                    </div>
                  </div>
                )}
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-center gap-3.5 min-w-0">
                    <div className="relative flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-white/35 dark:bg-white/10 border border-white/30 dark:border-white/15">
                      {peer.category === "phone" ? (
                        <Smartphone className="h-6 w-6 text-slate-800 dark:text-white/90" />
                      ) : (
                        <Laptop className="h-6 w-6 text-slate-800 dark:text-white/90" />
                      )}
                      <span
                        className={`absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full border-2 border-white/80 dark:border-[#0b0f1e] ${
                          peer.isOnline ? "bg-emerald-400" : "bg-slate-400"
                        }`}
                      />
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-1.5">
                        <h3 className="truncate text-base font-semibold text-slate-900/95 dark:text-white/95">
                          {peer.displayName}
                        </h3>
                        <button
                          type="button"
                          title={t.devices.rename}
                          onClick={() => {
                            setRenameTarget(peer);
                            setAliasDraft(peer.aliasName ?? peer.deviceName);
                          }}
                          className="rounded-full p-1.5 text-slate-600/65 dark:text-white/40 hover:bg-white/20 dark:hover:bg-white/10 hover:text-slate-900 dark:hover:text-white"
                        >
                          <Edit3 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                      <div className="mt-0.5 flex items-center gap-2 text-xs text-slate-700/65 dark:text-white/50 font-mono">
                        <span>{peer.ip ?? "—"}</span>
                        <span>·</span>
                        <span className="uppercase">{peer.os}</span>
                      </div>
                    </div>
                  </div>

                  {peer.isTrusted && (
                    <GlassBadge tone="success">
                      <ShieldCheck className="h-3.5 w-3.5" />
                      <span>{t.devices.friend}</span>
                    </GlassBadge>
                  )}
                </div>

                <div className="flex items-center gap-2">
                  <GlassButton
                    variant="primary"
                    size="sm"
                    disabled={!peer.isOnline}
                    onClick={() => handleSendFiles(peer.peerId, false)}
                    className="flex-1"
                  >
                    <FileUp className="h-4 w-4" />
                    <span>{t.devices.sendFiles}</span>
                  </GlassButton>
                  <GlassButton
                    variant="glass"
                    size="sm"
                    disabled={!peer.isOnline}
                    title={t.devices.sendFolder}
                    onClick={() => handleSendFiles(peer.peerId, true)}
                  >
                    <FolderUp className="h-4 w-4" />
                  </GlassButton>
                  {peer.isTrusted ? (
                    <GlassButton
                      variant="ghost"
                      size="sm"
                      title={t.devices.unfriend}
                      onClick={() =>
                        api.unfriend(peer.peerId).catch((e) => onError(String(e)))
                      }
                    >
                      <UserMinus className="h-4 w-4" />
                    </GlassButton>
                  ) : (
                    <GlassButton
                      variant="glass"
                      size="sm"
                      disabled={!peer.isOnline}
                      title={t.devices.addFriend}
                      onClick={() =>
                        api
                          .sendFriendRequest(peer.peerId)
                          .catch((e) => onError(String(e)))
                      }
                    >
                      <UserPlus className="h-4 w-4" />
                    </GlassButton>
                  )}
                </div>
              </GlassCard>
            );
          })}
        </div>
      )}

      {/* Rename Modal */}
      <GlassModal
        open={renameTarget !== null}
        onClose={() => setRenameTarget(null)}
        title={t.devices.renamePrompt}
      >
        <div className="space-y-4">
          <GlassInput
            value={aliasDraft}
            onChange={(e) => setAliasDraft(e.target.value)}
            autoFocus
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSaveRename();
            }}
          />
          <div className="flex justify-end gap-2">
            <GlassButton variant="ghost" onClick={() => setRenameTarget(null)}>
              {t.devices.decline}
            </GlassButton>
            <GlassButton variant="primary" onClick={handleSaveRename}>
              {t.settings.save}
            </GlassButton>
          </div>
        </div>
      </GlassModal>

      {/* App Update Modal */}
      <GlassModal
        open={updateModalOpen}
        onClose={() => {
          if (!installingUpdate) setUpdateModalOpen(false);
        }}
        title={t.update.title}
      >
        <div className="space-y-4">
          {checkingUpdate ? (
            <div className="flex items-center gap-2.5 py-2 text-sm text-slate-700/80 dark:text-white/70">
              <RefreshCw className="h-4 w-4 animate-spin" />
              <span>{t.update.checking}</span>
            </div>
          ) : updateInfo ? (
            <>
              <div className="flex items-center justify-between rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3 text-sm">
                <div>
                  <div className="text-xs text-slate-700/65 dark:text-white/50">
                    {t.update.current}
                  </div>
                  <div className="font-mono font-medium">
                    v{updateInfo.currentVersion}
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-xs text-slate-700/65 dark:text-white/50">
                    {t.update.latest}
                  </div>
                  <div className="font-mono font-semibold text-indigo-600 dark:text-indigo-300">
                    v{updateInfo.latestVersion}
                  </div>
                </div>
              </div>

              {!updateInfo.hasUpdate ? (
                <div className="text-sm text-emerald-700 dark:text-emerald-300">
                  {t.update.upToDate}
                </div>
              ) : (
                <>
                  {updateInfo.releaseNotes && (
                    <div className="max-h-36 overflow-y-auto whitespace-pre-wrap rounded-2xl bg-white/20 dark:bg-white/5 p-3 font-mono text-xs text-slate-800/85 dark:text-white/75">
                      {updateInfo.releaseNotes}
                    </div>
                  )}

                  {updateInfo.asset ? (
                    <div className="flex items-center justify-between text-xs font-mono text-slate-700/70 dark:text-white/55">
                      <span className="truncate">{updateInfo.asset.fileName}</span>
                      <span className="shrink-0 ml-2">
                        {formatBytes(updateInfo.asset.size)}
                      </span>
                    </div>
                  ) : (
                    <div className="text-xs text-amber-700 dark:text-amber-300">
                      {t.update.noAsset}
                    </div>
                  )}

                  {isDownloading && updateProgress && (
                    <div className="space-y-1.5">
                      <div className="h-2 w-full overflow-hidden rounded-full bg-white/20 dark:bg-white/10">
                        <div
                          className="h-full rounded-full bg-gradient-to-r from-indigo-400 to-cyan-400 transition-all duration-150"
                          style={{ width: `${downloadPct}%` }}
                        />
                      </div>
                      <div className="flex justify-between font-mono text-xs text-slate-700/70 dark:text-white/55">
                        <span>
                          {updateProgress.stage === "verifying"
                            ? t.update.verifying
                            : t.update.downloading}
                        </span>
                        <span>{downloadPct}%</span>
                      </div>
                    </div>
                  )}

                  {updateHint && (
                    <div
                      className={`rounded-2xl border px-3 py-2 text-xs ${
                        isAndroid && downloadedPath
                          ? "bg-emerald-400/15 border-emerald-400/30 text-emerald-900 dark:text-emerald-200"
                          : "bg-amber-400/15 border-amber-400/30 text-amber-900 dark:text-amber-200"
                      }`}
                    >
                      {updateHint}
                    </div>
                  )}
                </>
              )}
            </>
          ) : null}

          <div className="flex justify-end gap-2">
            <GlassButton
              variant="ghost"
              disabled={installingUpdate}
              onClick={() => setUpdateModalOpen(false)}
            >
              {t.update.close}
            </GlassButton>
            {!updateInfo?.hasUpdate && (
              <GlassButton
                variant="glass"
                disabled={checkingUpdate}
                onClick={runUpdateCheck}
              >
                {t.update.checkAgain}
              </GlassButton>
            )}
            {updateInfo?.hasUpdate && updateInfo.asset && (
              <GlassButton
                variant="primary"
                disabled={installingUpdate || checkingUpdate}
                onClick={handleDownloadAndInstall}
              >
                {isAndroid
                  ? downloadedPath
                    ? t.update.openDownloads
                    : t.update.downloadApk
                  : downloadedPath
                  ? t.update.installNow
                  : t.update.downloadAndInstall}
              </GlassButton>
            )}
          </div>
        </div>
      </GlassModal>

      {/* Drag & Drop Quick Send Modal */}
      <GlassModal
        open={droppedPaths.length > 0}
        onClose={() => setDroppedPaths([])}
        title={t.devices.dropModalTitle}
      >
        {(() => {
          const onlinePeers = peers.filter((p) => p.isOnline);
          const singlePeer = onlinePeers.length === 1 ? onlinePeers[0] : null;
          return (
            <div className="space-y-4">
              <div className="text-xs font-mono text-slate-700/75 dark:text-white/60">
                {t.devices.dropFilesCount}: {droppedPaths.length}
              </div>

              <div className="max-h-28 space-y-1 overflow-y-auto rounded-2xl bg-white/20 dark:bg-white/5 p-3 font-mono text-xs">
                {droppedPaths.map((p) => (
                  <div key={p} className="truncate">
                    {pathBasename(p)}
                  </div>
                ))}
              </div>

              {onlinePeers.length === 0 ? (
                <div className="rounded-2xl bg-white/20 dark:bg-white/5 p-3 text-xs text-slate-700/75 dark:text-white/60">
                  {t.devices.dropNoOnline}
                </div>
              ) : singlePeer ? (
                <div className="space-y-3">
                  <div className="text-sm text-slate-800/85 dark:text-white/80">
                    {t.devices.dropConfirmSingle}:
                  </div>
                  <div className="flex items-center justify-between rounded-2xl bg-white/25 dark:bg-white/10 px-4 py-3">
                    <div className="flex items-center gap-3 min-w-0">
                      {singlePeer.category === "phone" ? (
                        <Smartphone className="h-5 w-5 shrink-0 text-slate-800 dark:text-white/90" />
                      ) : (
                        <Laptop className="h-5 w-5 shrink-0 text-slate-800 dark:text-white/90" />
                      )}
                      <span className="truncate font-medium text-slate-900/95 dark:text-white/95">
                        {singlePeer.displayName}
                      </span>
                    </div>
                    <span className="font-mono text-xs text-slate-700/70 dark:text-white/55">
                      {singlePeer.ip ?? ""}
                    </span>
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="text-sm text-slate-800/85 dark:text-white/80">
                    {t.devices.dropPickDevice}
                  </div>
                  <div className="max-h-52 space-y-2 overflow-y-auto">
                    {onlinePeers.map((peer) => (
                      <button
                        key={peer.peerId}
                        type="button"
                        onClick={() => {
                          const paths = droppedPaths;
                          setDroppedPaths([]);
                          void sendPathsToPeer(peer.peerId, paths);
                        }}
                        className="flex min-h-[44px] w-full items-center justify-between rounded-2xl bg-white/25 dark:bg-white/10 px-4 py-3 text-left transition hover:bg-white/35 dark:hover:bg-white/15"
                      >
                        <div className="flex items-center gap-3 min-w-0">
                          {peer.category === "phone" ? (
                            <Smartphone className="h-5 w-5 shrink-0 text-slate-800 dark:text-white/90" />
                          ) : (
                            <Laptop className="h-5 w-5 shrink-0 text-slate-800 dark:text-white/90" />
                          )}
                          <span className="truncate font-medium text-slate-900/95 dark:text-white/95">
                            {peer.displayName}
                          </span>
                        </div>
                        <span className="font-mono text-xs text-slate-700/70 dark:text-white/55">
                          {peer.ip ?? ""}
                        </span>
                      </button>
                    ))}
                  </div>
                </div>
              )}

              <div className="flex justify-end gap-2">
                <GlassButton
                  variant="ghost"
                  onClick={() => setDroppedPaths([])}
                >
                  {t.devices.dropCancel}
                </GlassButton>
                {singlePeer && (
                  <GlassButton
                    variant="primary"
                    onClick={() => {
                      const paths = droppedPaths;
                      setDroppedPaths([]);
                      void sendPathsToPeer(singlePeer.peerId, paths);
                    }}
                  >
                    <FileUp className="h-4 w-4" />
                    <span>{t.devices.dropSendAction}</span>
                  </GlassButton>
                )}
              </div>
            </div>
          );
        })()}
      </GlassModal>

      {/* QR Quick Connect Modal */}
      <GlassModal
        open={qrModalOpen}
        onClose={() => setQrModalOpen(false)}
        title={t.devices.qrTitle}
      >
        <div className="space-y-4">
          {qrLoading ? (
            <div className="flex items-center justify-center py-10">
              <RefreshCw className="h-6 w-6 animate-spin text-indigo-500 dark:text-indigo-300" />
            </div>
          ) : qrInfo ? (
            <>
              <p className="text-xs text-slate-700/75 dark:text-white/60">
                {t.devices.qrHint}
              </p>
              <div className="flex justify-center">
                <div className="rounded-3xl bg-white p-4 shadow-lg">
                  <img
                    src={`data:image/svg+xml;utf8,${encodeURIComponent(qrInfo.svg)}`}
                    alt="Landa QR"
                    className="h-52 w-52"
                  />
                </div>
              </div>
              <div className="flex items-center justify-between gap-2 rounded-2xl bg-white/20 dark:bg-white/5 px-3.5 py-2.5">
                <span className="font-mono text-xs text-slate-800/85 dark:text-white/80">
                  {qrInfo.ip}:{qrInfo.port}
                </span>
                <GlassButton
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(qrInfo.uri);
                      onNotice?.(t.devices.qrCopied);
                    } catch (e) {
                      onError(String(e));
                    }
                  }}
                >
                  <Copy className="h-3.5 w-3.5" />
                  <span>{t.devices.qrCopyLink}</span>
                </GlassButton>
              </div>
            </>
          ) : null}

          <div className="flex gap-2 pt-1">
            <GlassInput
              value={qrUriDraft}
              onChange={(e) => setQrUriDraft(e.target.value)}
              placeholder={t.devices.qrPastePlaceholder}
              onKeyDown={(e) => {
                if (e.key === "Enter") void handleConnectQrUri();
              }}
            />
            <GlassButton
              variant="primary"
              disabled={!qrUriDraft.trim() || qrConnecting}
              onClick={handleConnectQrUri}
              className="shrink-0"
            >
              {t.devices.qrConnectBtn}
            </GlassButton>
          </div>

          <div className="flex justify-end">
            <GlassButton variant="ghost" onClick={() => setQrModalOpen(false)}>
              {t.update.close}
            </GlassButton>
          </div>
        </div>
      </GlassModal>

      {/* QR Connected Confirmation Modal */}
      <GlassModal
        open={qrConnectedPeer !== null}
        onClose={() => setQrConnectedPeer(null)}
        title={t.devices.qrConnectedTitle}
      >
        {qrConnectedPeer && (
          <div className="space-y-4">
            <div className="rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3 space-y-1">
              <div className="font-semibold text-slate-900/95 dark:text-white/95">
                {qrConnectedPeer.displayName}
              </div>
              <div className="flex items-center gap-2 font-mono text-xs text-slate-700/70 dark:text-white/55">
                <span>{qrConnectedPeer.ip ?? "—"}</span>
                <span>·</span>
                <span>
                  {t.devices.sasLabel}: {qrConnectedPeer.sasCode}
                </span>
              </div>
            </div>

            {!qrConnectedPeer.isTrusted && (
              <p className="text-sm text-slate-800/85 dark:text-white/80">
                {t.devices.qrConnectedDesc}
              </p>
            )}

            <div className="flex justify-end gap-2">
              <GlassButton
                variant="ghost"
                onClick={() => setQrConnectedPeer(null)}
              >
                {t.devices.qrDone}
              </GlassButton>
              {!qrConnectedPeer.isTrusted && (
                <GlassButton
                  variant="primary"
                  onClick={async () => {
                    const peerId = qrConnectedPeer.peerId;
                    setQrConnectedPeer(null);
                    try {
                      await api.sendFriendRequest(peerId);
                    } catch (e) {
                      onError(String(e));
                    }
                  }}
                >
                  <UserPlus className="h-4 w-4" />
                  <span>{t.devices.addFriend}</span>
                </GlassButton>
              )}
            </div>
          </div>
        )}
      </GlassModal>
    </div>
  );
}
