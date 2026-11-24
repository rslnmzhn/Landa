import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  ArrowDownLeft,
  ArrowUpRight,
  ChevronDown,
  File as FileIcon,
  FolderOpen,
  Trash2,
  X,
} from "lucide-react";
import { api, formatBytes, formatEta, formatSpeed } from "../../core/api";
import type {
  TransferHistoryRecord,
  TransferProgressInfo,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
} from "../../components/glass/GlassPrimitives";
import { summarizeTransferPaths } from "./activityPaths";

interface ActivityViewProps {
  t: TranslationDict;
  activeTransfers: TransferProgressInfo[];
  history: TransferHistoryRecord[];
  onPreviewPath: (pathOrUri: string) => void;
  onError: (msg: string) => void;
}

export function ActivityView({
  t,
  activeTransfers,
  history,
  onPreviewPath,
  onError,
}: ActivityViewProps) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [resolvedNames, setResolvedNames] = useState<Record<string, string>>({});
  const resolvedNamesRef = useRef<Record<string, string>>({});
  const inFlightUrisRef = useRef<Set<string>>(new Set());
  const mountedRef = useRef(true);

  const isAndroid = navigator.userAgent.toLowerCase().includes("android");

  useEffect(() => {
    resolvedNamesRef.current = resolvedNames;
  }, [resolvedNames]);

  const pathSummaries = useMemo(() => {
    const map = new Map<string, ReturnType<typeof summarizeTransferPaths>>();
    for (const item of history) {
      map.set(
        item.id,
        summarizeTransferPaths(
          item.savedPaths,
          resolvedNames,
          item.relativePaths
        )
      );
    }
    return map;
  }, [history, resolvedNames]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const resolveUris = useCallback((paths: string[]) => {
    const unresolved: string[] = [];
    for (const p of paths) {
      if (
        p.startsWith("content://") &&
        !resolvedNamesRef.current[p] &&
        !inFlightUrisRef.current.has(p)
      ) {
        inFlightUrisRef.current.add(p);
        unresolved.push(p);
      }
    }
    if (unresolved.length === 0) return;

    api
      .resolvePreviewNames(unresolved)
      .then((names) => {
        if (!mountedRef.current || !names || Object.keys(names).length === 0) {
          return;
        }
        setResolvedNames((prev) => ({ ...prev, ...names }));
      })
      .catch(() => {})
      .finally(() => {
        for (const p of unresolved) {
          inFlightUrisRef.current.delete(p);
        }
      });
  }, []);

  useEffect(() => {
    const allPaths: string[] = [];
    for (const item of history) {
      for (const p of item.savedPaths) {
        allPaths.push(p);
      }
    }
    resolveUris(allPaths);
  }, [history, resolveUris]);

  const handleOpenSaved = async (paths: string[]) => {
    if (paths.length === 0) return;
    try {
      if (paths[0].startsWith("content://") || isAndroid) {
        await api.openFileExternal(paths[0]);
      } else if (paths.length === 1) {
        await revealItemInDir(paths[0]);
      } else {
        await openPath(paths[0]);
      }
    } catch (e) {
      onError(String(e));
    }
  };

  const toggleExpand = (item: TransferHistoryRecord) => {
    const nextExpanded = expandedId === item.id ? null : item.id;
    setExpandedId(nextExpanded);
    if (nextExpanded) {
      resolveUris(item.savedPaths);
    }
  };

  const statusLabel = (status: string) => {
    switch (status) {
      case "completed":
        return t.activity.completed;
      case "failed":
        return t.activity.failed;
      case "cancelled":
        return t.activity.cancelled;
      case "rejected":
        return t.activity.rejected;
      default:
        return status;
    }
  };

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
          {t.activity.title}
        </h1>
        {history.length > 0 && (
          <GlassButton
            size="sm"
            variant="ghost"
            onClick={() => api.clearHistory().catch((e) => onError(String(e)))}
          >
            <Trash2 className="h-4 w-4" aria-hidden="true" />
            <span>{t.activity.clear}</span>
          </GlassButton>
        )}
      </div>

      {/* Active Transfers */}
      {activeTransfers.length > 0 && (
        <GlassCard elevated className="p-5 space-y-3">
          <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
            {t.activity.active}
          </h2>
          <div className="space-y-3">
            {activeTransfers.map((tr) => {
              const pct =
                tr.totalBytes > 0
                  ? Math.min(
                      100,
                      Math.round((tr.bytesTransferred / tr.totalBytes) * 100)
                    )
                  : 0;
              return (
                <div
                  key={tr.transferId}
                  className="rounded-2xl bg-white/20 dark:bg-white/5 p-4 space-y-2.5"
                >
                  <div className="flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2 font-medium text-slate-900/90 dark:text-white/90">
                        {tr.direction === "send" ? (
                          <ArrowUpRight
                            className="h-4 w-4 text-indigo-400"
                            aria-hidden="true"
                          />
                        ) : (
                          <ArrowDownLeft
                            className="h-4 w-4 text-emerald-400"
                            aria-hidden="true"
                          />
                        )}
                        <span className="truncate">{tr.peerName}</span>
                      </div>
                      <div className="truncate text-xs text-slate-700/70 dark:text-white/55">
                        {tr.status === "waiting_approval"
                          ? t.activity.waiting
                          : tr.currentFile}
                      </div>
                    </div>
                    <GlassButton
                      size="sm"
                      variant="danger"
                      aria-label={`${t.activity.cancel}: ${tr.peerName}`}
                      title={t.activity.cancel}
                      onClick={() => api.cancelTransfer(tr.transferId)}
                    >
                      <X className="h-4 w-4" aria-hidden="true" />
                    </GlassButton>
                  </div>

                  <div className="h-2 w-full overflow-hidden rounded-full bg-white/20 dark:bg-white/10">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-indigo-400 to-cyan-400 transition-all duration-150"
                      style={{ width: `${pct}%` }}
                    />
                  </div>

                  <div className="flex items-center justify-between font-mono text-xs text-slate-700/75 dark:text-white/60">
                    <span>
                      {pct}% · {formatBytes(tr.bytesTransferred)} /{" "}
                      {formatBytes(tr.totalBytes)}
                    </span>
                    <span>
                      {formatSpeed(tr.speedBps)} · {formatEta(tr.etaSecs)}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        </GlassCard>
      )}

      {/* History */}
      <GlassCard className="p-5 space-y-3">
        {activeTransfers.length > 0 && (
          <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
            {t.activity.history}
          </h2>
        )}
        {history.length === 0 ? (
          <p className="py-8 text-center text-sm text-slate-700/65 dark:text-white/50">
            {t.activity.empty}
          </p>
        ) : (
          <div className="space-y-2">
            {history.map((item) => {
              const canPreview =
                item.direction === "receive" &&
                item.status === "completed" &&
                item.savedPaths.length > 0;
              const isMulti = item.savedPaths.length > 1;
              const canExpand = isMulti;
              const canClickRow = canExpand || canPreview;
              const isExpanded = expandedId === item.id;
              const pathSummary =
                pathSummaries.get(item.id) ??
                summarizeTransferPaths(
                  item.savedPaths,
                  resolvedNames,
                  item.relativePaths
                );
              const singleItem = !isMulti ? pathSummary.items[0] : undefined;
              const singlePrimaryLabel =
                singleItem &&
                singleItem.relativePath !== singleItem.fileName &&
                singleItem.displayPath !== singleItem.relativePath
                  ? singleItem.relativePath
                  : singleItem?.fileName;

              const rowSummary = (
                <>
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white/25 dark:bg-white/10">
                    {item.direction === "send" ? (
                      <ArrowUpRight
                        className="h-5 w-5 text-indigo-500 dark:text-indigo-300"
                        aria-hidden="true"
                      />
                    ) : (
                      <ArrowDownLeft
                        className="h-5 w-5 text-emerald-500 dark:text-emerald-300"
                        aria-hidden="true"
                      />
                    )}
                  </div>
                  <div className="min-w-0 flex-1 space-y-0.5">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                        {item.peerName}
                      </span>
                      <GlassBadge
                        tone={
                          item.status === "completed"
                            ? "success"
                            : item.status === "failed"
                            ? "warning"
                            : "neutral"
                        }
                      >
                        {statusLabel(item.status)}
                      </GlassBadge>
                    </div>
                    <div className="font-mono text-xs text-slate-700/65 dark:text-white/50">
                      {item.fileCount} {t.activity.filesUnit} ·{" "}
                      {formatBytes(item.totalBytes)}
                    </div>
                    {singleItem && (
                      <div className="space-y-0.5 pt-0.5">
                        <div className="truncate text-xs font-medium text-slate-900/85 dark:text-white/85">
                          {singlePrimaryLabel}
                        </div>
                        {singleItem.displayPath !== singlePrimaryLabel && (
                          <div className="break-all font-mono text-[11px] text-slate-700/65 dark:text-white/50">
                            {singleItem.displayPath}
                          </div>
                        )}
                      </div>
                    )}
                    {isMulti && pathSummary.commonDir && (
                      <div className="break-all font-mono text-[11px] text-slate-700/65 dark:text-white/50 pt-0.5">
                        {pathSummary.commonDir}
                      </div>
                    )}
                  </div>
                </>
              );

              return (
                <div
                  key={item.id}
                  className="rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3 space-y-2"
                >
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    {canClickRow ? (
                      <button
                        type="button"
                        aria-expanded={canExpand ? isExpanded : undefined}
                        onClick={() => {
                          if (canExpand) {
                            toggleExpand(item);
                          } else if (canPreview) {
                            onPreviewPath(item.savedPaths[0]);
                          }
                        }}
                        className="flex min-h-[44px] flex-1 items-center gap-3 min-w-0 text-left cursor-pointer"
                      >
                        {rowSummary}
                      </button>
                    ) : (
                      <div className="flex min-h-[44px] flex-1 items-center gap-3 min-w-0 text-left">
                        {rowSummary}
                      </div>
                    )}

                    {(canExpand || canPreview) && (
                      <div className="flex items-center gap-1.5 shrink-0">
                        {canExpand && (
                          <GlassButton
                            size="sm"
                            variant="glass"
                            onClick={() => toggleExpand(item)}
                            aria-expanded={isExpanded}
                            aria-label={
                              isExpanded
                                ? `${t.activity.collapseFiles}: ${item.peerName}`
                                : `${t.activity.expandFiles}: ${item.peerName}`
                            }
                            className="px-3"
                          >
                            <ChevronDown
                              aria-hidden="true"
                              className={`h-4 w-4 transition-transform duration-200 ${
                                isExpanded ? "rotate-180" : ""
                              }`}
                            />
                          </GlassButton>
                        )}
                        {canPreview && (
                          <GlassButton
                            size="sm"
                            variant="glass"
                            onClick={() => handleOpenSaved(item.savedPaths)}
                            aria-label={t.activity.openFolder}
                            title={t.activity.openFolder}
                          >
                            <FolderOpen className="h-4 w-4" aria-hidden="true" />
                            <span className="hidden sm:inline">
                              {t.activity.openFolder}
                            </span>
                          </GlassButton>
                        )}
                      </div>
                    )}
                  </div>

                  {canExpand && isExpanded && (
                    <div className="space-y-1 border-t border-white/15 pt-2">
                      {pathSummary.items.map((entry, idx) => {
                        const content = (
                          <>
                            <FileIcon
                              className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-700/70 dark:text-white/60"
                              aria-hidden="true"
                            />
                            <div className="min-w-0 flex-1 space-y-0.5">
                              <div className="break-all font-mono text-slate-900/90 dark:text-white/85">
                                {entry.relativePath}
                              </div>
                              {entry.displayPath !== entry.relativePath && (
                                <div className="break-all font-mono text-[11px] text-slate-700/65 dark:text-white/50">
                                  {entry.displayPath}
                                </div>
                              )}
                            </div>
                          </>
                        );

                        return canPreview ? (
                          <button
                            key={`${entry.rawPath}-${idx}`}
                            type="button"
                            onClick={() => onPreviewPath(entry.rawPath)}
                            className="flex min-h-[44px] w-full items-start gap-2 rounded-xl bg-white/15 dark:bg-white/5 px-3 py-2 text-left text-xs transition hover:bg-white/25 dark:hover:bg-white/10"
                          >
                            {content}
                          </button>
                        ) : (
                          <div
                            key={`${entry.rawPath}-${idx}`}
                            className="flex min-h-[44px] w-full items-start gap-2 rounded-xl bg-white/15 dark:bg-white/5 px-3 py-2 text-left text-xs"
                          >
                            {content}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </GlassCard>
    </div>
  );
}
