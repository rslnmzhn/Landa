import {
  ChevronRight,
  File,
  Folder,
  FolderPlus,
  FilePlus,
  Trash2,
} from "lucide-react";
import { formatBytes, formatEta, formatSpeed } from "../../core/api";
import type { LocalShareRoot, TransferProgressInfo } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import { GlassButton, GlassCard } from "../../components/glass/GlassPrimitives";

export function LocalSharesSection({
  t,
  localShares,
  onAddLocal,
  onRemoveLocal,
  onPreviewLocalPath,
}: {
  t: TranslationDict;
  localShares: LocalShareRoot[];
  onAddLocal: (directory: boolean) => void;
  onRemoveLocal: (id: string) => void;
  onPreviewLocalPath: (path: string) => void;
}) {
  return (
    <GlassCard className="p-5 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
          {t.shares.myShares}
        </h2>
        <div className="flex items-center gap-2">
          <GlassButton size="sm" onClick={() => onAddLocal(true)}>
            <FolderPlus className="h-4 w-4" aria-hidden="true" />
            <span>{t.shares.addFolder}</span>
          </GlassButton>
          <GlassButton size="sm" onClick={() => onAddLocal(false)}>
            <FilePlus className="h-4 w-4" aria-hidden="true" />
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
          {localShares.map((root) => {
            const rowBody = (
              <>
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
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                    {root.name}
                  </div>
                  <div className="break-all font-mono text-xs text-slate-700/65 dark:text-white/50">
                    {root.path}
                  </div>
                </div>
              </>
            );

            return (
              <div
                key={root.id}
                className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2.5"
              >
                {root.isDir ? (
                  <div className="flex min-h-[44px] items-center gap-3 min-w-0 flex-1 text-left">
                    {rowBody}
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => onPreviewLocalPath(root.path)}
                    className="flex min-h-[44px] items-center gap-3 min-w-0 flex-1 text-left cursor-pointer"
                  >
                    {rowBody}
                  </button>
                )}
                <GlassButton
                  variant="ghost"
                  size="sm"
                  aria-label={`${t.shares.remove}: ${root.name}`}
                  title={t.shares.remove}
                  onClick={() => onRemoveLocal(root.id)}
                >
                  <Trash2 className="h-4 w-4" aria-hidden="true" />
                </GlassButton>
              </div>
            );
          })}
        </div>
      )}
    </GlassCard>
  );
}

export function ActiveReceiveTransfers({
  t,
  transfers,
}: {
  t: TranslationDict;
  transfers: TransferProgressInfo[];
}) {
  if (transfers.length === 0) return null;
  return (
    <div className="space-y-2">
      {transfers.map((tr) => {
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
            className="rounded-2xl bg-white/20 dark:bg-white/5 p-3 space-y-1.5"
          >
            <div className="flex items-center justify-between gap-2 text-xs">
              <span className="truncate font-medium text-slate-900/90 dark:text-white/90">
                {tr.currentFile || t.shares.downloading}
              </span>
              <span className="shrink-0 font-mono text-slate-700/75 dark:text-white/60">
                {tr.filesDone}/{tr.fileCount} · {pct}%
              </span>
            </div>
            <div
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct}
              aria-label={tr.currentFile || t.shares.downloading}
              className="h-1.5 w-full overflow-hidden rounded-full bg-white/20 dark:bg-white/10"
            >
              <div
                className="h-full rounded-full bg-gradient-to-r from-indigo-400 to-cyan-400 transition-all duration-150"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="flex items-center justify-between font-mono text-[11px] text-slate-700/70 dark:text-white/55">
              <span>
                {formatBytes(tr.bytesTransferred)} / {formatBytes(tr.totalBytes)}
              </span>
              <span>
                {formatSpeed(tr.speedBps)} · {formatEta(tr.etaSecs)}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function ShareBreadcrumbs({
  label,
  rootName,
  relPath,
  onNavigate,
}: {
  label: string;
  rootName: string;
  relPath: string;
  onNavigate: (nextRel: string) => void;
}) {
  const segments = relPath.split("/").filter(Boolean);
  const isAtRoot = segments.length === 0;

  return (
    <nav
      aria-label={label}
      className="rounded-2xl bg-white/20 dark:bg-white/5 px-2.5 py-1 font-mono text-xs min-w-0"
    >
      <ol className="flex flex-wrap items-center gap-1">
        <li className="inline-flex items-center min-w-0">
          {isAtRoot ? (
            <span
              aria-current="page"
              className="inline-flex min-h-[44px] items-center px-2 py-1 font-semibold text-slate-900/95 dark:text-white/95 break-all"
            >
              {rootName}
            </span>
          ) : (
            <button
              type="button"
              onClick={() => onNavigate("")}
              className="inline-flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg px-2.5 py-1 text-left break-all text-slate-700/80 dark:text-white/70 hover:bg-white/25 dark:hover:bg-white/10 hover:text-slate-900 dark:hover:text-white transition cursor-pointer"
            >
              {rootName}
            </button>
          )}
        </li>
        {segments.map((seg, idx) => {
          const targetRel = segments.slice(0, idx + 1).join("/");
          const isCurrent = idx === segments.length - 1;
          return (
            <li
              key={targetRel}
              className="inline-flex items-center gap-1 min-w-0"
            >
              <ChevronRight
                className="h-3.5 w-3.5 shrink-0 text-slate-500/70 dark:text-white/40"
                aria-hidden="true"
              />
              {isCurrent ? (
                <span
                  aria-current="page"
                  className="inline-flex min-h-[44px] items-center px-2 py-1 font-semibold text-slate-900/95 dark:text-white/95 break-all"
                >
                  {seg}
                </span>
              ) : (
                <button
                  type="button"
                  onClick={() => onNavigate(targetRel)}
                  className="inline-flex min-h-[44px] min-w-[44px] items-center justify-center rounded-lg px-2.5 py-1 text-left break-all text-slate-700/80 dark:text-white/70 hover:bg-white/25 dark:hover:bg-white/10 hover:text-slate-900 dark:hover:text-white transition cursor-pointer"
                >
                  {seg}
                </button>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
