import { useEffect, useState } from "react";
import {
  Copy,
  ExternalLink,
  File as FileIcon,
  Music,
  RefreshCw,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type { PreviewDescriptor } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import { GlassBadge, GlassButton } from "../../components/glass/GlassPrimitives";

interface FilePreviewModalProps {
  t: TranslationDict;
  preview: PreviewDescriptor | null;
  loading: boolean;
  onClose: () => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function FilePreviewModal({
  t,
  preview,
  loading,
  onClose,
  onError,
  onNotice,
}: FilePreviewModalProps) {
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [dragStart, setDragStart] = useState({ x: 0, y: 0 });

  useEffect(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, [preview?.sourcePath, preview?.url]);

  useEffect(() => {
    if (!preview && !loading) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview, loading, onClose]);

  if (!preview && !loading) return null;

  const handleOpenExternal = async () => {
    if (!preview) return;
    try {
      await api.openFileExternal(preview.sourcePath);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleCopyText = async () => {
    if (!preview?.textContent) return;
    try {
      await navigator.clipboard.writeText(preview.textContent);
      onNotice(t.preview.copied);
    } catch (e) {
      onError(String(e));
    }
  };

  const resetZoom = () => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-3 sm:p-6 backdrop-blur-md animate-glass-modal">
      <div className="fixed inset-0" onClick={onClose} aria-hidden="true" />

      <div className="liquid-glass-elevated relative z-10 flex max-h-[90vh] w-full max-w-4xl flex-col overflow-hidden rounded-3xl">
        {/* Header */}
        <div className="flex items-center justify-between gap-3 border-b border-white/15 px-4 py-3 sm:px-5">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-slate-900/95 dark:text-white/95">
              {preview ? preview.fileName : t.preview.loading}
            </div>
            {preview && (
              <div className="font-mono text-xs text-slate-700/65 dark:text-white/50">
                {formatBytes(preview.sizeBytes)}
              </div>
            )}
          </div>

          <div className="flex items-center gap-1.5 shrink-0">
            {preview?.kind === "image" && (
              <>
                <GlassButton
                  size="sm"
                  variant="ghost"
                  onClick={() => setScale((s) => Math.max(0.5, +(s - 0.25).toFixed(2)))}
                  className="px-2.5"
                >
                  <ZoomOut className="h-4 w-4" />
                </GlassButton>
                <button
                  type="button"
                  onClick={resetZoom}
                  title={t.preview.zoomReset}
                  className="min-h-[44px] rounded-full px-2 font-mono text-xs text-slate-700/80 dark:text-white/75 hover:bg-white/15"
                >
                  {Math.round(scale * 100)}%
                </button>
                <GlassButton
                  size="sm"
                  variant="ghost"
                  onClick={() => setScale((s) => Math.min(5, +(s + 0.25).toFixed(2)))}
                  className="px-2.5"
                >
                  <ZoomIn className="h-4 w-4" />
                </GlassButton>
              </>
            )}

            {preview?.kind === "text" && preview.textContent !== null && (
              <GlassButton
                size="sm"
                variant="glass"
                onClick={handleCopyText}
                title={t.preview.copyText}
              >
                <Copy className="h-4 w-4" />
                <span className="hidden sm:inline">{t.preview.copyText}</span>
              </GlassButton>
            )}

            {preview && (
              <GlassButton
                size="sm"
                variant="glass"
                onClick={handleOpenExternal}
                title={t.preview.openExternal}
              >
                <ExternalLink className="h-4 w-4" />
                <span className="hidden sm:inline">{t.preview.openExternal}</span>
              </GlassButton>
            )}

            <GlassButton
              size="sm"
              variant="ghost"
              onClick={onClose}
              className="px-2.5"
              aria-label="Close"
            >
              <X className="h-4 w-4" />
            </GlassButton>
          </div>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-auto p-4 sm:p-5">
          {loading ? (
            <div className="flex min-h-[240px] flex-col items-center justify-center gap-3 text-sm text-slate-700/75 dark:text-white/65">
              <RefreshCw className="h-6 w-6 animate-spin text-indigo-400" />
              <span>{t.preview.loading}</span>
            </div>
          ) : preview?.kind === "image" && preview.url ? (
            <div
              className="flex h-[65vh] w-full items-center justify-center overflow-hidden rounded-2xl bg-black/20 select-none touch-none"
              onWheel={(e) => {
                e.preventDefault();
                const delta = e.deltaY < 0 ? 0.2 : -0.2;
                setScale((s) => Math.min(5, Math.max(0.5, +(s + delta).toFixed(2))));
              }}
              onDoubleClick={() => {
                if (scale > 1) {
                  resetZoom();
                } else {
                  setScale(2);
                }
              }}
              onPointerDown={(e) => {
                if (scale <= 1) return;
                setDragging(true);
                setDragStart({ x: e.clientX - offset.x, y: e.clientY - offset.y });
              }}
              onPointerMove={(e) => {
                if (!dragging) return;
                setOffset({ x: e.clientX - dragStart.x, y: e.clientY - dragStart.y });
              }}
              onPointerUp={() => setDragging(false)}
              onPointerCancel={() => setDragging(false)}
            >
              <img
                src={preview.url}
                alt={preview.fileName}
                draggable={false}
                style={{
                  transform: `translate3d(${offset.x}px, ${offset.y}px, 0) scale(${scale})`,
                  transition: dragging ? "none" : "transform 150ms ease-out",
                }}
                className="max-h-full max-w-full object-contain"
              />
            </div>
          ) : preview?.kind === "video" && preview.url ? (
            <div className="flex items-center justify-center rounded-2xl bg-black/40 overflow-hidden">
              <video
                src={preview.url}
                controls
                playsInline
                preload="metadata"
                className="max-h-[70vh] w-full rounded-2xl outline-none"
              />
            </div>
          ) : preview?.kind === "audio" && preview.url ? (
            <div className="flex flex-col items-center justify-center gap-5 py-10">
              <div className="flex h-20 w-20 items-center justify-center rounded-3xl bg-indigo-500/20 border border-indigo-400/30">
                <Music className="h-9 w-9 text-indigo-500 dark:text-indigo-300" />
              </div>
              <div className="text-center">
                <div className="text-base font-medium text-slate-900/95 dark:text-white/95">
                  {preview.fileName}
                </div>
                <div className="mt-0.5 font-mono text-xs text-slate-700/65 dark:text-white/50">
                  {formatBytes(preview.sizeBytes)}
                </div>
              </div>
              <audio
                src={preview.url}
                controls
                preload="metadata"
                className="w-full max-w-md"
              />
            </div>
          ) : preview?.kind === "text" ? (
            <div className="space-y-2">
              {preview.truncated && (
                <GlassBadge tone="warning">{t.preview.truncated}</GlassBadge>
              )}
              <pre className="max-h-[65vh] overflow-auto rounded-2xl bg-white/25 dark:bg-black/30 p-4 font-mono text-xs leading-relaxed text-slate-900/90 dark:text-white/85 whitespace-pre-wrap break-words">
                {preview.textContent ?? ""}
              </pre>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
              <div className="flex h-16 w-16 items-center justify-center rounded-3xl bg-white/25 dark:bg-white/10 border border-white/25 dark:border-white/15">
                <FileIcon className="h-7 w-7 text-slate-700 dark:text-white/75" />
              </div>
              <div className="space-y-1">
                <div className="text-sm font-medium text-slate-900/90 dark:text-white/90">
                  {preview?.fileName}
                </div>
                <div className="text-xs text-slate-700/65 dark:text-white/55">
                  {t.preview.unsupported}
                </div>
              </div>
              <GlassButton variant="primary" onClick={handleOpenExternal}>
                <ExternalLink className="h-4 w-4" />
                <span>{t.preview.openExternal}</span>
              </GlassButton>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
