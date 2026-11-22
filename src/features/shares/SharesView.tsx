import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Download,
  Eye,
  File,
  FilePlus,
  Folder,
  FolderPlus,
  Trash2,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type {
  LocalShareRoot,
  PeerInfo,
  ShareEntry,
  ShareRootInfo,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassSegmented,
} from "../../components/glass/GlassPrimitives";

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
    if (!canThumb) return;
    const cached = thumbMemoryCache.get(cacheKey);
    if (cached) {
      setDataUrl(cached);
      return;
    }
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
        <Folder className={`${iconClass} text-indigo-500 dark:text-indigo-300`} />
      ) : (
        <File className={`${iconClass} text-slate-600 dark:text-white/60`} />
      )}
    </div>
  );
}

interface SharesViewProps {
  t: TranslationDict;
  peers: PeerInfo[];
  enabled: boolean;
  onPreviewRemoteFile: (peerId: string, rootId: string, relPath: string) => void;
  onPreviewLocalPath: (path: string) => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function SharesView({
  t,
  peers,
  enabled,
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

  const trustedOnlinePeers = peers.filter((p) => p.isTrusted && p.isOnline);

  const loadLocal = async () => {
    try {
      setLocalShares(await api.sharesListLocal());
    } catch (e) {
      onError(String(e));
    }
  };

  useEffect(() => {
    if (enabled) {
      loadLocal();
    }
  }, [enabled]);

  const handleAddLocal = async (directory: boolean) => {
    try {
      const hasAccess = await api.hasAllFilesAccess().catch(() => true);
      if (!hasAccess) {
        await api.requestAllFilesAccess();
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
    setSelectedPeerId(peerId);
    setActiveRoot(null);
    setRelPath("");
    setEntries([]);
    if (!peerId) {
      setRemoteRoots([]);
      return;
    }
    setLoading(true);
    try {
      setRemoteRoots(await api.sharesBrowseRemoteRoots(peerId));
    } catch (e) {
      onError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleOpenRoot = async (root: ShareRootInfo) => {
    setActiveRoot(root);
    setRelPath("");
    setLoading(true);
    try {
      setEntries(await api.sharesBrowseRemoteDir(selectedPeerId, root.id, ""));
    } catch (e) {
      onError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleOpenDir = async (nextRel: string) => {
    if (!activeRoot) return;
    setRelPath(nextRel);
    setLoading(true);
    try {
      setEntries(
        await api.sharesBrowseRemoteDir(selectedPeerId, activeRoot.id, nextRel)
      );
    } catch (e) {
      onError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleBack = () => {
    if (!activeRoot) return;
    if (!relPath) {
      setActiveRoot(null);
      setEntries([]);
      return;
    }
    const parts = relPath.split("/").filter(Boolean);
    parts.pop();
    handleOpenDir(parts.join("/"));
  };

  const handleDownload = async (entry: ShareEntry) => {
    if (!activeRoot) return;
    try {
      const saved = await api.sharesDownloadRemoteFile(
        selectedPeerId,
        activeRoot.id,
        entry.relPath
      );
      onNotice(`${t.shares.downloadedTo}: ${saved}`);
    } catch (e) {
      onError(String(e));
    }
  };

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
      <GlassCard className="p-5 space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
            {t.shares.myShares}
          </h2>
          <div className="flex items-center gap-2">
            <GlassButton size="sm" onClick={() => handleAddLocal(true)}>
              <FolderPlus className="h-4 w-4" />
              <span>{t.shares.addFolder}</span>
            </GlassButton>
            <GlassButton size="sm" onClick={() => handleAddLocal(false)}>
              <FilePlus className="h-4 w-4" />
              <span>{t.shares.addFile}</span>
            </GlassButton>
          </div>
        </div>

        {localShares.length === 0 ? (
          <p className="text-sm text-slate-700/65 dark:text-white/50">
            {t.shares.noLocalShares}
          </p>
        ) : (
          <div className="space-y-2">
            {localShares.map((root) => (
              <div
                key={root.id}
                className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2.5"
              >
                <button
                  type="button"
                  disabled={root.isDir}
                  onClick={() => !root.isDir && onPreviewLocalPath(root.path)}
                  className={`flex items-center gap-3 min-w-0 flex-1 text-left ${
                    !root.isDir ? "cursor-pointer" : "cursor-default"
                  }`}
                >
                  {root.isDir ? (
                    <Folder className="h-5 w-5 shrink-0 text-indigo-500 dark:text-indigo-300" />
                  ) : (
                    <File className="h-5 w-5 shrink-0 text-sky-500 dark:text-sky-300" />
                  )}
                  <span className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                    {root.name}
                  </span>
                </button>
                <GlassButton
                  variant="ghost"
                  size="sm"
                  onClick={async () => {
                    await api.sharesRemoveLocal(root.id);
                    await loadLocal();
                  }}
                >
                  <Trash2 className="h-4 w-4" />
                </GlassButton>
              </div>
            ))}
          </div>
        )}
      </GlassCard>

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
            {remoteRoots.map((root) => (
              <button
                key={root.id}
                type="button"
                onClick={() => handleOpenRoot(root)}
                className="flex min-h-[52px] items-center gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3 text-left transition hover:bg-white/30 dark:hover:bg-white/10"
              >
                {root.isDir ? (
                  <Folder className="h-5 w-5 text-indigo-500 dark:text-indigo-300" />
                ) : (
                  <File className="h-5 w-5 text-sky-500 dark:text-sky-300" />
                )}
                <span className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                  {root.name}
                </span>
              </button>
            ))}
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <GlassButton size="sm" variant="ghost" onClick={handleBack}>
                  <ArrowLeft className="h-4 w-4" />
                  <span>{t.shares.back}</span>
                </GlassButton>
                <GlassBadge>
                  <span className="font-mono text-xs truncate max-w-[180px] sm:max-w-xs">
                    {activeRoot.name}
                    {relPath ? `/${relPath}` : ""}
                  </span>
                </GlassBadge>
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
                {entries.map((entry) => (
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
                    {!entry.isDir && (
                      <div className="flex items-center justify-end gap-1">
                        <GlassButton
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            onPreviewRemoteFile(
                              selectedPeerId,
                              activeRoot.id,
                              entry.relPath
                            )
                          }
                          className="px-2.5"
                        >
                          <Eye className="h-3.5 w-3.5" />
                        </GlassButton>
                        <GlassButton
                          size="sm"
                          variant="glass"
                          onClick={() => handleDownload(entry)}
                          className="px-2.5"
                        >
                          <Download className="h-3.5 w-3.5" />
                        </GlassButton>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            ) : (
              <div className="space-y-1.5">
                {entries.map((entry) => (
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
                    {!entry.isDir && (
                      <div className="flex items-center gap-1.5 shrink-0">
                        <GlassButton
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            onPreviewRemoteFile(
                              selectedPeerId,
                              activeRoot.id,
                              entry.relPath
                            )
                          }
                          className="px-3"
                        >
                          <Eye className="h-4 w-4" />
                        </GlassButton>
                        <GlassButton
                          size="sm"
                          variant="glass"
                          onClick={() => handleDownload(entry)}
                        >
                          <Download className="h-4 w-4" />
                        </GlassButton>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </GlassCard>
    </div>
  );
}
