import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
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
import { api } from "../../core/api";
import type {
  FriendRequestInfo,
  LocalIdentityInfo,
  PeerInfo,
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
  onError: (msg: string) => void;
}

export function DevicesView({
  t,
  identity,
  peers,
  friendRequests,
  leftHanded,
  onError,
}: DevicesViewProps) {
  const [scanning, setScanning] = useState(false);
  const [renameTarget, setRenameTarget] = useState<PeerInfo | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");

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

  return (
    <div className="space-y-5">
      {/* Header */}
      <div
        className={`flex flex-wrap items-center justify-between gap-3 ${
          leftHanded ? "flex-row-reverse" : ""
        }`}
      >
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
            {t.devices.title}
          </h1>
          <p className="text-sm text-slate-700/70 dark:text-white/60">
            {t.devices.subtitle}
          </p>
        </div>

        <div className="flex items-center gap-2">
          {identity && (
            <GlassBadge tone="neutral">
              <span className="h-2 w-2 rounded-full bg-emerald-400" />
              <span className="font-mono text-xs">
                {identity.deviceName} · {identity.peerId.slice(0, 8)}
              </span>
            </GlassBadge>
          )}
          <GlassButton onClick={handleScan} size="sm">
            <RefreshCw
              className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`}
            />
            <span>{t.devices.scan}</span>
          </GlassButton>
        </div>
      </div>

      {/* Pending Friend Requests */}
      {friendRequests.length > 0 && (
        <GlassCard elevated className="p-4 space-y-3">
          <div className="text-xs font-semibold uppercase tracking-wider text-slate-700/70 dark:text-white/55">
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
                      {t.devices.sasLabel}: {req.sasCode}
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
        <GlassCard className="flex flex-col items-center justify-center px-6 py-14 text-center">
          <div className="mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-indigo-500/20 border border-indigo-400/30">
            <Wifi className="h-7 w-7 text-indigo-600 dark:text-indigo-300" />
          </div>
          <h3 className="text-lg font-medium text-slate-900/90 dark:text-white/90">
            {t.devices.emptyTitle}
          </h3>
          <p className="mt-1 max-w-md text-sm text-slate-700/70 dark:text-white/55">
            {t.devices.emptyDesc}
          </p>
          <GlassButton className="mt-5" variant="primary" onClick={handleScan}>
            <RefreshCw className="h-4 w-4" />
            <span>{t.devices.scan}</span>
          </GlassButton>
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {peers.map((peer) => (
            <GlassCard
              key={peer.peerId}
              className="flex flex-col justify-between gap-4 p-5 hover:bg-white/20 dark:hover:bg-white/[0.11]"
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
                    <div className="flex items-center gap-2">
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
                        className="rounded-full p-1.5 text-slate-600/70 dark:text-white/45 hover:bg-white/20 dark:hover:bg-white/10 hover:text-slate-900 dark:hover:text-white"
                      >
                        <Edit3 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                    <div className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-slate-700/70 dark:text-white/55 font-mono">
                      <span>{peer.ip ?? "—"}</span>
                      <span>·</span>
                      <span className="uppercase">{peer.os}</span>
                      <span>·</span>
                      <span>SAS {peer.sasCode}</span>
                    </div>
                  </div>
                </div>

                <div className="flex flex-col items-end gap-1.5 shrink-0">
                  {peer.isTrusted && (
                    <GlassBadge tone="success">
                      <ShieldCheck className="h-3.5 w-3.5" />
                      <span>{t.devices.friend}</span>
                    </GlassBadge>
                  )}
                  <GlassBadge tone={peer.isOnline ? "accent" : "neutral"}>
                    {peer.isOnline ? t.devices.online : t.devices.offline}
                  </GlassBadge>
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2 pt-1">
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
                  onClick={() => handleSendFiles(peer.peerId, true)}
                >
                  <FolderUp className="h-4 w-4" />
                  <span>{t.devices.sendFolder}</span>
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
    </div>
  );
}
