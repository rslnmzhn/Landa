import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  ArrowDownLeft,
  ArrowUpRight,
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

interface ActivityViewProps {
  t: TranslationDict;
  activeTransfers: TransferProgressInfo[];
  history: TransferHistoryRecord[];
  onError: (msg: string) => void;
}

export function ActivityView({
  t,
  activeTransfers,
  history,
  onError,
}: ActivityViewProps) {
  const handleOpenSaved = async (paths: string[]) => {
    if (paths.length === 0) return;
    try {
      if (paths.length === 1) {
        await revealItemInDir(paths[0]);
      } else {
        await openPath(paths[0]);
      }
    } catch (e) {
      onError(String(e));
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
            <Trash2 className="h-4 w-4" />
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
                          <ArrowUpRight className="h-4 w-4 text-indigo-400" />
                        ) : (
                          <ArrowDownLeft className="h-4 w-4 text-emerald-400" />
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
                      onClick={() => api.cancelTransfer(tr.transferId)}
                    >
                      <X className="h-4 w-4" />
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
            {history.map((item) => (
              <div
                key={item.id}
                className="flex flex-wrap items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-white/25 dark:bg-white/10">
                    {item.direction === "send" ? (
                      <ArrowUpRight className="h-5 w-5 text-indigo-500 dark:text-indigo-300" />
                    ) : (
                      <ArrowDownLeft className="h-5 w-5 text-emerald-500 dark:text-emerald-300" />
                    )}
                  </div>
                  <div className="min-w-0">
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
                  </div>
                </div>

                {item.direction === "receive" &&
                  item.status === "completed" &&
                  item.savedPaths.length > 0 && (
                    <GlassButton
                      size="sm"
                      variant="glass"
                      onClick={() => handleOpenSaved(item.savedPaths)}
                    >
                      <FolderOpen className="h-4 w-4" />
                      <span className="hidden sm:inline">
                        {t.activity.openFolder}
                      </span>
                    </GlassButton>
                  )}
              </div>
            ))}
          </div>
        )}
      </GlassCard>
    </div>
  );
}
