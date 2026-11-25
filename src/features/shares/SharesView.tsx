import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Download,
  File,
  Folder,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type {
  LocalIdentityInfo,
  LocalShareRoot,
  PeerInfo,
  ShareEntry,
  ShareRootInfo,
  TransferProgressInfo,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassButton,
  GlassCard,
  GlassSegmented,
} from "../../components/glass/GlassPrimitives";
import {
  ActiveReceiveTransfers,
  LocalSharesSection,
  ShareBreadcrumbs,
} from "./SharesSections";

const THUMB_EXTS = new Set(["png", "jpg", "jpeg", "webp", "gif"]);
const thumbMemoryCache = new Map<string, string>();

function isThumbImage(name: string): boolean {
  const dot = name.lastIndexOf(".");
  if (dot < 0) return false;
  return THUMB_EXTS.has(name.slice(dot + 1).toLowerCase());
}

function LazyShareThumbnail({
  peerId,
  rootId,
  entry,
  sizeClass,
  iconClass,
}: {
  peerId: string;
  rootId: string;
  entry: ShareEntry;
  sizeClass: string;
  iconClass: string;
}) {
  const cacheKey = `${peerId}:${rootId}:${entry.relPath}:${entry.modifiedMs}`;
  const [dataUrl, setDataUrl] = useState<string | null>(
    () => thumbMemoryCache.get(cacheKey) ?? null
  );
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canThumb = !entry.isDir && isThumbImage(entry.name);

  useEffect(() => {
    if (!canThumb) {
      setDataUrl(null);
      return;
    }
    const cached = thumbMemoryCache.get(cacheKey);
    if (cached) {
      setDataUrl(cached);
      return;
    }
    setDataUrl(null);
    const el = containerRef.current;
    if (!el) return;

    let cancelled = false;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        observer.disconnect();
        api
          .sharesGetRemoteThumbnail(peerId, rootId, entry.relPath, 160)
          .then((res) => {
            if (cancelled) return;
            if (thumbMemoryCache.size >= 300) {
              const firstKey = thumbMemoryCache.keys().next().value;
              if (firstKey) thumbMemoryCache.delete(firstKey);
            }
            thumbMemoryCache.set(cacheKey, res.dataUrl);
            setDataUrl(res.dataUrl);
          })
          .catch(() => {});
      },
      { rootMargin: "120px" }
    );

    observer.observe(el);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [canThumb, cacheKey, peerId, rootId, entry.relPath]);

  return (
    <div
      ref={containerRef}
      className={`relative flex shrink-0 items-center justify-center overflow-hidden bg-white/25 dark:bg-white/10 border border-white/20 dark:border-white/10 ${sizeClass}`}
    >
      {dataUrl ? (
        <img
          src={dataUrl}
          alt={entry.name}
          className="h-full w-full object-cover transition-opacity duration-200"
        />
      ) : entry.isDir ? (
        <Folder
          className={`${iconClass} text-indigo-500 dark:text-indigo-300`}
          aria-hidden="true"
        />
      ) : (
        <File
          className={`${iconClass} text-slate-600 dark:text-white/60`}
          aria-hidden="true"
        />
      )}
    </div>
  );
}

type FailedBrowseTarget =
  | { kind: "roots"; peerId: string }
  | { kind: "dir"; peerId: string; root: ShareRootInfo; relPath: string };

interface SharesViewProps {
  t: TranslationDict;
  identity: LocalIdentityInfo | null;
  peers: PeerInfo[];
  enabled: boolean;
  activeTransfers?: TransferProgressInfo[];
  downloadingKey?: string | null;
  onDownloadingKeyChange?: Dispatch<SetStateAction<string | null>>;
  onPreviewRemoteFile: (peerId: string, rootId: string, relPath: string) => void;
  onPreviewLocalPath: (path: string) => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function SharesView({
  t,
  identity,
  peers,
  enabled,
  activeTransfers = [],
  downloadingKey: controlledDownloadingKey,
  onDownloadingKeyChange,
  onPreviewRemoteFile,
  onPreviewLocalPath,
  onError,
  onNotice,
}: SharesViewProps) {
  const [localShares, setLocalShares] = useState<LocalShareRoot[]>([]);
  const [selectedPeerId, setSelectedPeerId] = useState<string>("");
  const [remoteRoots, setRemoteRoots] = useState<ShareRootInfo[]>([]);
  const [activeRoot, setActiveRoot] = useState<ShareRootInfo | null>(null);
  const [relPath, setRelPath] = useState<string>("");
  const [entries, setEntries] = useState<ShareEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<"list" | "grid">("list");
  const [localDownloadingKey, setLocalDownloadingKey] = useState<string | null>(
    null
  );

  const downloadingKey =
    controlledDownloadingKey !== undefined
      ? controlledDownloadingKey
      : localDownloadingKey;
  const setDownloadingKey = onDownloadingKeyChange ?? setLocalDownloadingKey;

  const downloadingRef = useRef(false);
  const browseReqIdRef = useRef(0);
  const failedTargetRef = useRef<FailedBrowseTarget | null>(null);
  const prevSelectedPeerRef = useRef<{ peerId: string; isOnline: boolean } | null>(
    null
  );
  const activeRootRef = useRef<ShareRootInfo | null>(null);
  activeRootRef.current = activeRoot;
  const relPathRef = useRef<string>("");
  relPathRef.current = relPath;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const isAndroid =
    identity?.os === "android" ||
    navigator.userAgent.toLowerCase().includes("android");

  const trustedPeers = peers.filter((p) => p.isTrusted);
  const trustedOnlinePeers = trustedPeers.filter((p) => p.isOnline);
  const selectedPeer =
    trustedPeers.find((p) => p.peerId === selectedPeerId) ?? null;
  const selectorPeers = trustedPeers.filter(
    (p) => p.isOnline || p.peerId === selectedPeerId
  );

  const loadRemoteRoots = useCallback(async (peerId: string) => {
    const reqId = ++browseReqIdRef.current;
    failedTargetRef.current = null;
    setLoading(true);
    setRemoteError(null);
    let failedDirTarget: FailedBrowseTarget | null = null;
    try {
      const roots = await api.sharesBrowseRemoteRoots(peerId);
      if (browseReqIdRef.current !== reqId) return;
      setRemoteRoots(roots);

      const currentRoot = activeRootRef.current;
      if (currentRoot) {
        const stillExists = roots.find((r) => r.id === currentRoot.id) ?? null;
        if (!stillExists) {
          setActiveRoot(null);
          setRelPath("");
          setEntries([]);
        } else {
          const targetRel = relPathRef.current;
          failedDirTarget = {
            kind: "dir",
            peerId,
            root: stillExists,
            relPath: targetRel,
          };
          const nextEntries = await api.sharesBrowseRemoteDir(
            peerId,
            stillExists.id,
            targetRel
          );
          if (browseReqIdRef.current !== reqId) return;
          setActiveRoot(stillExists);
          setEntries(nextEntries);
        }
      }
    } catch (e) {
      if (browseReqIdRef.current !== reqId) return;
      const msg = String(e);
      failedTargetRef.current = failedDirTarget ?? { kind: "roots", peerId };
      setRemoteError(msg);
      onErrorRef.current(msg);
    } finally {
      if (browseReqIdRef.current === reqId) {
        setLoading(false);
      }
    }
  }, []);

  const openDirTarget = useCallback(
    async (peerId: string, root: ShareRootInfo, nextRel: string) => {
      const reqId = ++browseReqIdRef.current;
      failedTargetRef.current = null;
      setLoading(true);
      setRemoteError(null);
      try {
        const nextEntries = await api.sharesBrowseRemoteDir(
          peerId,
          root.id,
          nextRel
        );
        if (browseReqIdRef.current === reqId) {
          setActiveRoot(root);
          setRelPath(nextRel);
          setEntries(nextEntries);
        }
      } catch (e) {
        if (browseReqIdRef.current === reqId) {
          const msg = String(e);
          failedTargetRef.current = {
            kind: "dir",
            peerId,
            root,
            relPath: nextRel,
          };
          setRemoteError(msg);
          onErrorRef.current(msg);
        }
      } finally {
        if (browseReqIdRef.current === reqId) {
          setLoading(false);
        }
      }
    },
    []
  );

  useEffect(() => {
    if (!selectedPeerId) {
      prevSelectedPeerRef.current = null;
      return;
    }
    if (!selectedPeer) {
      browseReqIdRef.current++;
      failedTargetRef.current = null;
      prevSelectedPeerRef.current = null;
      setSelectedPeerId("");
      setRemoteRoots([]);
      setActiveRoot(null);
      setRelPath("");
      setEntries([]);
      setRemoteError(null);
      setLoading(false);
      return;
    }

    const prev = prevSelectedPeerRef.current;
    prevSelectedPeerRef.current = {
      peerId: selectedPeer.peerId,
      isOnline: selectedPeer.isOnline,
    };

    if (!selectedPeer.isOnline) {
      browseReqIdRef.current++;
      setLoading(false);
      return;
    }

    if (prev && prev.peerId === selectedPeer.peerId && !prev.isOnline) {
      void loadRemoteRoots(selectedPeer.peerId);
    }
  }, [selectedPeer, selectedPeerId, loadRemoteRoots]);

  const loadLocal = useCallback(async () => {
    try {
      setLocalShares(await api.sharesListLocal());
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }, []);

  const handleRemoveLocal = async (id: string) => {
    try {
      await api.sharesRemoveLocal(id);
      await loadLocal();
    } catch (e) {
      onError(String(e));
    }
  };

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    api
      .sharesListLocal()
      .then((list) => {
        if (!cancelled) setLocalShares(list);
      })
      .catch((e) => {
        if (!cancelled) onErrorRef.current(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  const handleAddLocal = async (directory: boolean) => {
    try {
      if (isAndroid) {
        const added = directory
          ? await api.sharesPickFolderAndroid()
          : await api.sharesPickFileAndroid();
        if (!added) return;
        await loadLocal();
        return;
      }
      const selected = await open({ directory, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      await api.sharesAddLocal(selected, null);
      await loadLocal();
    } catch (e) {
      onError(String(e));
    }
  };

  const handleSelectPeer = async (peerId: string) => {
    browseReqIdRef.current++;
    failedTargetRef.current = null;
    activeRootRef.current = null;
    relPathRef.current = "";
    setSelectedPeerId(peerId);
    setRemoteRoots([]);
    setActiveRoot(null);
    setRelPath("");
    setEntries([]);
    setRemoteError(null);
    if (!peerId) {
      prevSelectedPeerRef.current = null;
      setLoading(false);
      return;
    }
    const target = trustedPeers.find((p) => p.peerId === peerId);
    prevSelectedPeerRef.current = {
      peerId,
      isOnline: Boolean(target?.isOnline),
    };
    if (!target?.isOnline) {
      setLoading(false);
      return;
    }
    await loadRemoteRoots(peerId);
  };

  const handleOpenRoot = async (root: ShareRootInfo) => {
    if (!selectedPeerId) return;
    await openDirTarget(selectedPeerId, root, "");
  };

  const handleOpenDir = async (nextRel: string) => {
    if (!activeRoot || !selectedPeerId) return;
    await openDirTarget(selectedPeerId, activeRoot, nextRel);
  };

  const handleBack = () => {
    if (!activeRoot) return;
    if (!relPath) {
      browseReqIdRef.current++;
      failedTargetRef.current = null;
      setActiveRoot(null);
      setEntries([]);
      setRemoteError(null);
      setLoading(false);
      return;
    }
    const parts = relPath.split("/").filter(Boolean);
    parts.pop();
    handleOpenDir(parts.join("/"));
  };

  const retryFailedTarget = async () => {
    if (!selectedPeerId || !selectedPeer?.isOnline) return;
    const target = failedTargetRef.current;
    if (target && target.peerId === selectedPeerId) {
      if (target.kind === "dir") {
        await openDirTarget(target.peerId, target.root, target.relPath);
      } else {
        await loadRemoteRoots(target.peerId);
      }
      return;
    }
    if (activeRoot) {
      await openDirTarget(selectedPeerId, activeRoot, relPath);
    } else {
      await loadRemoteRoots(selectedPeerId);
    }
  };

  const handleRefresh = async () => {
    setScanError(null);
    const scanPromise = api.triggerScan().catch((e) => {
      const msg = String(e);
      setScanError(msg);
      onErrorRef.current(msg);
    });
    if (selectedPeerId && selectedPeer?.isOnline) {
      await retryFailedTarget();
    }
    await scanPromise;
  };

  const runDownload = async (key: string, task: () => Promise<string>) => {
    if (downloadingRef.current || downloadingKey !== null) return;
    downloadingRef.current = true;
    setDownloadingKey(key);
    try {
      const notice = await task();
      onNotice(notice);
    } catch (e) {
      onError(String(e));
    } finally {
      downloadingRef.current = false;
      setDownloadingKey((prev) => (prev === key ? null : prev));
    }
  };

  const handleDownloadFolder = (
    peerId: string,
    rootId: string,
    targetRelPath: string,
    key: string
  ) =>
    runDownload(key, async () => {
      const res = await api.sharesDownloadRemoteFolder(
        peerId,
        rootId,
        targetRelPath
      );
      return `${t.shares.downloadedTo}: ${res.destination} (${res.fileCount} ${t.activity.filesUnit})`;
    });

  const handleDownloadRoot = async (root: ShareRootInfo) => {
    if (!selectedPeerId) return;
    const key = `root:${selectedPeerId}:${root.id}`;
    if (root.isDir) {
      await handleDownloadFolder(selectedPeerId, root.id, "", key);
      return;
    }
    await runDownload(key, async () => {
      const saved = await api.sharesDownloadRemoteFile(
        selectedPeerId,
        root.id,
        ""
      );
      return `${t.shares.downloadedTo}: ${saved}`;
    });
  };

  const handleDownload = async (entry: ShareEntry) => {
    if (!activeRoot || !selectedPeerId) return;
    const key = `entry:${selectedPeerId}:${activeRoot.id}:${entry.relPath || entry.name}`;
    if (entry.isDir) {
      await handleDownloadFolder(
        selectedPeerId,
        activeRoot.id,
        entry.relPath,
        key
      );
      return;
    }
    await runDownload(key, async () => {
      const saved = await api.sharesDownloadRemoteFile(
        selectedPeerId,
        activeRoot.id,
        entry.relPath
      );
      return `${t.shares.downloadedTo}: ${saved}`;
    });
  };

  const activeReceiveTransfers = activeTransfers.filter(
    (tr) => tr.direction === "receive" && tr.status === "transferring"
  );

  if (!enabled) {
    return (
      <GlassCard className="p-8 text-center text-sm text-slate-700/75 dark:text-white/60">
        {t.shares.disabledNotice}
      </GlassCard>
    );
  }

  return (
    <div className="space-y-5">
      <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
        {t.shares.title}
      </h1>

      {/* My Shared Folders */}
      <LocalSharesSection
        t={t}
        localShares={localShares}
        onAddLocal={handleAddLocal}
        onRemoveLocal={handleRemoveLocal}
        onPreviewLocalPath={onPreviewLocalPath}
      />

      {/* Friends' Shares */}
      <GlassCard className="p-5 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
            {t.shares.friendShares}
          </h2>
          <div className="flex flex-wrap items-center gap-2">
            {selectorPeers.length > 0 && (
              <GlassSegmented
                options={selectorPeers.map((p) => ({
                  id: p.peerId,
                  label: p.displayName,
                }))}
                value={selectedPeerId}
                onChange={handleSelectPeer}
              />
            )}
            <GlassButton
              size="sm"
              variant="glass"
              onClick={handleRefresh}
              aria-label={t.shares.refresh}
              title={t.shares.refresh}
              className="min-w-[44px] shrink-0"
            >
              <RefreshCw
                className={`h-4 w-4 ${loading ? "animate-spin" : ""}`}
                aria-hidden="true"
              />
            </GlassButton>
          </div>
        </div>

        <ActiveReceiveTransfers t={t} transfers={activeReceiveTransfers} />

        {scanError && (
          <div className="rounded-2xl border border-rose-400/30 bg-rose-500/10 px-4 py-2.5">
            <div className="text-xs font-medium text-rose-900 dark:text-rose-200">
              {t.shares.scanError}
            </div>
            <div className="break-all font-mono text-xs text-rose-800/80 dark:text-rose-200/75">
              {scanError}
            </div>
          </div>
        )}

        {trustedPeers.length === 0 ? (
          <p className="text-sm text-slate-700/65 dark:text-white/50">
            {t.shares.noFriends}
          </p>
        ) : !selectedPeer ? (
          <p className="text-sm text-slate-700/65 dark:text-white/50">
            {trustedOnlinePeers.length === 0
              ? t.shares.noOnlineFriends
              : t.shares.selectFriend}
          </p>
        ) : !selectedPeer.isOnline ? (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3">
            <p className="text-sm text-slate-700/75 dark:text-white/65">
              {t.shares.peerOffline}
            </p>
            <GlassButton size="sm" variant="glass" onClick={handleRefresh}>
              <RefreshCw className="h-4 w-4" aria-hidden="true" />
              <span>{t.shares.refresh}</span>
            </GlassButton>
          </div>
        ) : loading ? (
          <div className="py-6 text-center text-sm text-slate-700/65 dark:text-white/50">
            ...
          </div>
        ) : remoteError ? (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-rose-400/30 bg-rose-500/10 px-4 py-3">
            <div className="min-w-0 flex-1 space-y-0.5">
              <div className="text-sm font-medium text-rose-900 dark:text-rose-200">
                {t.shares.loadError}
              </div>
              <div className="break-all font-mono text-xs text-rose-800/80 dark:text-rose-200/75">
                {remoteError}
              </div>
            </div>
            <GlassButton size="sm" variant="glass" onClick={retryFailedTarget}>
              <RefreshCw className="h-4 w-4" aria-hidden="true" />
              <span>{t.shares.retry}</span>
            </GlassButton>
          </div>
        ) : !activeRoot ? (
          remoteRoots.length === 0 ? (
            <p className="text-sm text-slate-700/65 dark:text-white/50">
              {t.shares.noRemoteShares}
            </p>
          ) : (
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {remoteRoots.map((root) => {
                const rootKey = `root:${selectedPeerId}:${root.id}`;
                const isDownloading = downloadingKey === rootKey;
                const downloadLabel = root.isDir
                  ? t.shares.downloadFolder
                  : t.shares.download;
                return (
                  <div
                    key={root.id}
                    className="flex min-h-[52px] items-center justify-between gap-2 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2 transition hover:bg-white/30 dark:hover:bg-white/10"
                  >
                    <button
                      type="button"
                      onClick={() => handleOpenRoot(root)}
                      className="flex min-h-[44px] min-w-0 flex-1 items-center gap-3 text-left"
                    >
                      {root.isDir ? (
                        <Folder
                          className="h-5 w-5 shrink-0 text-indigo-500 dark:text-indigo-300"
                          aria-hidden="true"
                        />
                      ) : (
                        <File
                          className="h-5 w-5 shrink-0 text-sky-500 dark:text-sky-300"
                          aria-hidden="true"
                        />
                      )}
                      <span className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                        {root.name}
                      </span>
                    </button>
                    <GlassButton
                      size="sm"
                      variant="glass"
                      disabled={downloadingKey !== null}
                      onClick={() => handleDownloadRoot(root)}
                      aria-label={`${downloadLabel}: ${root.name}`}
                      title={downloadLabel}
                      className="min-w-[44px] shrink-0"
                    >
                      {isDownloading ? (
                        <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                      ) : (
                        <Download className="h-4 w-4" aria-hidden="true" />
                      )}
                    </GlassButton>
                  </div>
                );
              })}
            </div>
          )
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2 min-w-0 flex-1">
                <GlassButton size="sm" variant="ghost" onClick={handleBack}>
                  <ArrowLeft className="h-4 w-4" aria-hidden="true" />
                  <span>{t.shares.back}</span>
                </GlassButton>
                <ShareBreadcrumbs
                  label={t.shares.breadcrumbsLabel}
                  rootName={activeRoot.name}
                  relPath={relPath}
                  onNavigate={handleOpenDir}
                />
              </div>

              <GlassSegmented
                options={[
                  { id: "list", label: t.shares.viewList },
                  { id: "grid", label: t.shares.viewGrid },
                ]}
                value={viewMode}
                onChange={(v) => setViewMode(v as "list" | "grid")}
              />
            </div>

            {entries.length === 0 ? (
              <p className="text-sm text-slate-700/65 dark:text-white/50">
                {t.shares.emptyDir}
              </p>
            ) : viewMode === "grid" ? (
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4">
                {entries.map((entry) => {
                  const entryKey = `entry:${selectedPeerId}:${activeRoot.id}:${entry.relPath || entry.name}`;
                  const isDownloading = downloadingKey === entryKey;
                  const downloadLabel = entry.isDir
                    ? t.shares.downloadFolder
                    : t.shares.download;
                  return (
                    <div
                      key={entry.relPath || entry.name}
                      className="flex flex-col justify-between gap-2 rounded-2xl bg-white/20 dark:bg-white/5 p-2.5 transition hover:bg-white/25 dark:hover:bg-white/10"
                    >
                      <button
                        type="button"
                        onClick={() => {
                          if (entry.isDir) {
                            handleOpenDir(entry.relPath);
                          } else {
                            onPreviewRemoteFile(
                              selectedPeerId,
                              activeRoot.id,
                              entry.relPath
                            );
                          }
                        }}
                        className="flex flex-col gap-2 text-left"
                      >
                        <LazyShareThumbnail
                          peerId={selectedPeerId}
                          rootId={activeRoot.id}
                          entry={entry}
                          sizeClass="aspect-square w-full rounded-xl"
                          iconClass="h-8 w-8"
                        />
                        <div className="min-w-0 px-0.5">
                          <div className="truncate text-xs font-medium text-slate-900/90 dark:text-white/90">
                            {entry.name}
                          </div>
                          {!entry.isDir && (
                            <div className="font-mono text-[11px] text-slate-700/65 dark:text-white/45">
                              {formatBytes(entry.sizeBytes)}
                            </div>
                          )}
                        </div>
                      </button>
                      <div className="flex items-center justify-end gap-1">
                        <GlassButton
                          size="sm"
                          variant="glass"
                          disabled={downloadingKey !== null}
                          onClick={() => handleDownload(entry)}
                          aria-label={`${downloadLabel}: ${entry.name}`}
                          title={downloadLabel}
                          className="min-w-[44px] px-2.5"
                        >
                          {isDownloading ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                          ) : (
                            <Download className="h-3.5 w-3.5" aria-hidden="true" />
                          )}
                        </GlassButton>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="space-y-1.5">
                {entries.map((entry) => {
                  const entryKey = `entry:${selectedPeerId}:${activeRoot.id}:${entry.relPath || entry.name}`;
                  const isDownloading = downloadingKey === entryKey;
                  const downloadLabel = entry.isDir
                    ? t.shares.downloadFolder
                    : t.shares.download;
                  return (
                    <div
                      key={entry.relPath || entry.name}
                      className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2.5"
                    >
                      <button
                        type="button"
                        onClick={() => {
                          if (entry.isDir) {
                            handleOpenDir(entry.relPath);
                          } else {
                            onPreviewRemoteFile(
                              selectedPeerId,
                              activeRoot.id,
                              entry.relPath
                            );
                          }
                        }}
                        className="flex min-h-[44px] min-w-0 flex-1 items-center gap-3 text-left"
                      >
                        <LazyShareThumbnail
                          peerId={selectedPeerId}
                          rootId={activeRoot.id}
                          entry={entry}
                          sizeClass="h-10 w-10 rounded-xl"
                          iconClass="h-5 w-5"
                        />
                        <div className="min-w-0">
                          <div className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                            {entry.name}
                          </div>
                          {!entry.isDir && (
                            <div className="font-mono text-xs text-slate-700/65 dark:text-white/45">
                              {formatBytes(entry.sizeBytes)}
                            </div>
                          )}
                        </div>
                      </button>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <GlassButton
                          size="sm"
                          variant="glass"
                          disabled={downloadingKey !== null}
                          onClick={() => handleDownload(entry)}
                          aria-label={`${downloadLabel}: ${entry.name}`}
                          title={downloadLabel}
                          className="min-w-[44px]"
                        >
                          {isDownloading ? (
                            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                          ) : (
                            <Download className="h-4 w-4" aria-hidden="true" />
                          )}
                        </GlassButton>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}
      </GlassCard>
    </div>
  );
}
