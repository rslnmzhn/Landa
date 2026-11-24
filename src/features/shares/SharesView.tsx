import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Download,
  File,
  Folder,
  Loader2,
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
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const isAndroid =
    identity?.os === "android" ||
    navigator.userAgent.toLowerCase().includes("android");

  const trustedOnlinePeers = peers.filter((p) => p.isTrusted && p.isOnline);

  useEffect(() => {
    if (
      selectedPeerId &&
      !peers.some((p) => p.peerId === selectedPeerId && p.isTrusted && p.isOnline)
    ) {
      browseReqIdRef.current++;
      setSelectedPeerId("");
      setRemoteRoots([]);
      setActiveRoot(null);
      setRelPath("");
      setEntries([]);
      setLoading(false);
    }
  }, [peers, selectedPeerId]);

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
    const reqId = ++browseReqIdRef.current;
    setSelectedPeerId(peerId);
    setRemoteRoots([]);
    setActiveRoot(null);
    setRelPath("");
    setEntries([]);
    if (!peerId) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const roots = await api.sharesBrowseRemoteRoots(peerId);
      if (browseReqIdRef.current === reqId) {
        setRemoteRoots(roots);
      }
    } catch (e) {
      if (browseReqIdRef.current === reqId) {
        onError(String(e));
      }
    } finally {
      if (browseReqIdRef.current === reqId) {
        setLoading(false);
      }
    }
  };

  const handleOpenRoot = async (root: ShareRootInfo) => {
    if (!selectedPeerId) return;
    const reqId = ++browseReqIdRef.current;
    setLoading(true);
    try {
      const nextEntries = await api.sharesBrowseRemoteDir(
        selectedPeerId,
        root.id,
        ""
      );
      if (browseReqIdRef.current === reqId) {
        setActiveRoot(root);
        setRelPath("");
        setEntries(nextEntries);
      }
    } catch (e) {
      if (browseReqIdRef.current === reqId) {
        onError(String(e));
      }
    } finally {
      if (browseReqIdRef.current === reqId) {
        setLoading(false);
      }
    }
  };

  const handleOpenDir = async (nextRel: string) => {
    if (!activeRoot || !selectedPeerId) return;
    const reqId = ++browseReqIdRef.current;
    setLoading(true);
    try {
      const nextEntries = await api.sharesBrowseRemoteDir(
        selectedPeerId,
        activeRoot.id,
        nextRel
      );
      if (browseReqIdRef.current === reqId) {
        setRelPath(nextRel);
        setEntries(nextEntries);
      }
    } catch (e) {
      if (browseReqIdRef.current === reqId) {
        onError(String(e));
      }
    } finally {
      if (browseReqIdRef.current === reqId) {
        setLoading(false);
      }
    }
  };

  const handleBack = () => {
    if (!activeRoot) return;
    if (!relPath) {
      browseReqIdRef.current++;
      setActiveRoot(null);
      setEntries([]);
      setLoading(false);
      return;
    }
    const parts = relPath.split("/").filter(Boolean);
    parts.pop();
    handleOpenDir(parts.join("/"));
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
          {trustedOnlinePeers.length > 0 && (
            <GlassSegmented
              options={trustedOnlinePeers.map((p) => ({
                id: p.peerId,
                label: p.displayName,
              }))}
              value={selectedPeerId}
              onChange={handleSelectPeer}
            />
          )}
        </div>

        <ActiveReceiveTransfers t={t} transfers={activeReceiveTransfers} />

        {trustedOnlinePeers.length === 0 ? (
          <p className="text-sm text-slate-700/65 dark:text-white/50">
            {t.shares.noFriends}
          </p>
        ) : !selectedPeerId ? (
          <p className="text-sm text-slate-700/65 dark:text-white/50">
            {t.shares.selectFriend}
          </p>
        ) : loading ? (
          <div className="py-6 text-center text-sm text-slate-700/65 dark:text-white/50">
            ...
          </div>
        ) : !activeRoot ? (
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
