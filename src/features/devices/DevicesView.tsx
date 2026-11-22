import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowDownToLine,
  Check,
  Edit3,
  FileUp,
  FolderUp,
  Laptop,
  RefreshCw,
  ShieldCheck,
  Smartphone,
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
}: DevicesViewProps) {
  const [scanning, setScanning] = useState(false);
  const [renameTarget, setRenameTarget] = useState<PeerInfo | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [downloadedPath, setDownloadedPath] = useState<string | null>(null);
  const [updateHint, setUpdateHint] = useState<string | null>(null);

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
      const isAndroid =
        identity?.os === "android" ||
        navigator.userAgent.toLowerCase().includes("android");
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
    setUpdateHint(null);
    if (!updateInfo) {
      void runUpdateCheck();
    }
  };

  const handleDownloadAndInstall = async () => {
    if (!updateInfo?.asset) return;
    setInstallingUpdate(true);
    setUpdateHint(null);
    try {
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
          {peers.map((peer) => (
            <GlassCard
              key={peer.peerId}
              className="flex flex-col justify-between gap-4 p-5 hover:bg-white/20 dark:hover:bg-white/[0.1]"
            >
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
          ))}
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
                    <div className="rounded-2xl bg-amber-400/15 border border-amber-400/30 px-3 py-2 text-xs text-amber-900 dark:text-amber-200">
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
                {downloadedPath
                  ? t.update.installNow
                  : t.update.downloadAndInstall}
              </GlassButton>
            )}
          </div>
        </div>
      </GlassModal>
    </div>
  );
}
