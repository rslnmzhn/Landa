import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Copy,
  ExternalLink,
  File as FileIcon,
  Music,
  RefreshCw,
  X,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type { PreviewDescriptor } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import { GlassBadge, GlassButton } from "../../components/glass/GlassPrimitives";
import {
  DOUBLE_TAP_DELAY_MS,
  MIN_IMAGE_SCALE,
  TAP_SLOP_PX,
  getPinchMetrics,
  isDoubleTapCandidate,
  panByDelta,
  toggleDoubleTapZoom,
  zoomAroundPoint,
  type Point,
  type Size,
  type TapRecord,
  type TransformState,
} from "./imageGestures.ts";

interface FilePreviewModalProps {
  t: TranslationDict;
  preview: PreviewDescriptor | null;
  loading: boolean;
  onClose: () => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

const INITIAL_TRANSFORM: TransformState = {
  scale: MIN_IMAGE_SCALE,
  offset: { x: 0, y: 0 },
};

export function FilePreviewModal({
  t,
  preview,
  loading,
  onClose,
  onError,
  onNotice,
}: FilePreviewModalProps) {
  const [transform, setTransform] = useState<TransformState>(INITIAL_TRANSFORM);
  const [animateTransform, setAnimateTransform] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true);

  const dialogRef = useRef<HTMLDivElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const transformRef = useRef<TransformState>(INITIAL_TRANSFORM);
  const pointersRef = useRef<Map<number, Point>>(new Map());
  const gestureRef = useRef<{
    startPoint: Point;
    lastPoint: Point;
    moved: boolean;
    wasPinching: boolean;
    isSecondTap: boolean;
    prevPinch: { distance: number; midpoint: Point } | null;
  }>({
    startPoint: { x: 0, y: 0 },
    lastPoint: { x: 0, y: 0 },
    moved: false,
    wasPinching: false,
    isSecondTap: false,
    prevPinch: null,
  });
  const lastTapRef = useRef<TapRecord | null>(null);
  const singleTapTimerRef = useRef<number | null>(null);
  const pendingTapToggleRef = useRef(false);

  const isOpen = Boolean(preview || loading);

  const commitTransform = useCallback((next: TransformState) => {
    transformRef.current = next;
    setTransform(next);
  }, []);

  const clearPendingTap = useCallback(() => {
    if (singleTapTimerRef.current !== null) {
      window.clearTimeout(singleTapTimerRef.current);
      singleTapTimerRef.current = null;
    }
  }, []);

  const flushPendingTapToggle = useCallback(() => {
    clearPendingTap();
    if (pendingTapToggleRef.current) {
      pendingTapToggleRef.current = false;
      setControlsVisible((visible) => !visible);
    }
  }, [clearPendingTap]);

  const cancelPendingTapToggle = useCallback(() => {
    clearPendingTap();
    pendingTapToggleRef.current = false;
  }, [clearPendingTap]);

  const getViewportMetrics = useCallback((): {
    viewport: Size;
    center: Point;
    content?: Size;
  } => {
    const rect = viewportRef.current?.getBoundingClientRect();
    const viewport: Size = rect
      ? { width: rect.width, height: rect.height }
      : { width: window.innerWidth, height: window.innerHeight };
    const center: Point = rect
      ? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
      : { x: viewport.width / 2, y: viewport.height / 2 };
    const img = imgRef.current;
    const content: Size | undefined =
      img && img.offsetWidth > 0 && img.offsetHeight > 0
        ? { width: img.offsetWidth, height: img.offsetHeight }
        : undefined;
    return { viewport, center, content };
  }, []);

  useEffect(() => {
    cancelPendingTapToggle();
    lastTapRef.current = null;
    pointersRef.current.clear();
    setAnimateTransform(false);
    commitTransform(INITIAL_TRANSFORM);
    setControlsVisible(true);
  }, [preview?.sourcePath, preview?.url, cancelPendingTapToggle, commitTransform]);

  useEffect(() => {
    return () => cancelPendingTapToggle();
  }, [cancelPendingTapToggle]);

  useEffect(() => {
    if (!isOpen) return;
    const prevOverflow = document.body.style.overflow;
    const prevFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    document.body.style.overflow = "hidden";
    dialogRef.current?.focus({ preventScroll: true });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      if (
        preview?.kind === "image" &&
        (e.key === "Enter" || e.key === " ") &&
        (e.target === dialogRef.current || e.target === viewportRef.current)
      ) {
        e.preventDefault();
        setControlsVisible((visible) => !visible);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = prevOverflow;
      window.removeEventListener("keydown", onKey);
      if (prevFocused && prevFocused.isConnected) {
        prevFocused.focus({ preventScroll: true });
      }
    };
  }, [isOpen, preview?.kind, onClose]);

  useEffect(() => {
    const el = viewportRef.current;
    if (loading || !el || preview?.kind !== "image" || !preview.url) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const { viewport, center, content } = getViewportMetrics();
      const factor = Math.exp(-e.deltaY * 0.002);
      const focal: Point = {
        x: e.clientX - center.x,
        y: e.clientY - center.y,
      };
      setAnimateTransform(false);
      commitTransform(
        zoomAroundPoint(
          transformRef.current,
          transformRef.current.scale * factor,
          focal,
          focal,
          viewport,
          content
        )
      );
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [loading, preview?.kind, preview?.url, getViewportMetrics, commitTransform]);

  if (!isOpen) return null;

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

  const releaseCaptureSafely = (
    target: HTMLDivElement,
    pointerId: number
  ) => {
    try {
      if (target.hasPointerCapture(pointerId)) {
        target.releasePointerCapture(pointerId);
      }
    } catch {
      // Ignore synthetic pointer events
    }
  };

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.pointerType === "mouse") {
      if (e.button !== 0) return;
      // Only one mouse pointer can be active; clear any stale entry if capture was lost.
      pointersRef.current.clear();
    }

    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // Ignore synthetic pointer events without active capture support
    }

    const point: Point = { x: e.clientX, y: e.clientY };
    pointersRef.current.set(e.pointerId, point);
    setAnimateTransform(false);

    if (pointersRef.current.size === 1) {
      const isSecondTap = isDoubleTapCandidate(lastTapRef.current, {
        time: Date.now(),
        point,
      });
      if (isSecondTap) {
        clearPendingTap();
      }
      gestureRef.current = {
        startPoint: point,
        lastPoint: point,
        moved: false,
        wasPinching: false,
        isSecondTap,
        prevPinch: null,
      };
    } else if (pointersRef.current.size >= 2) {
      flushPendingTapToggle();
      lastTapRef.current = null;
      const [p1, p2] = Array.from(pointersRef.current.values());
      gestureRef.current.wasPinching = true;
      gestureRef.current.moved = true;
      gestureRef.current.isSecondTap = false;
      gestureRef.current.prevPinch = getPinchMetrics(p1, p2);
    }
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointersRef.current.has(e.pointerId)) return;
    const point: Point = { x: e.clientX, y: e.clientY };
    pointersRef.current.set(e.pointerId, point);

    if (pointersRef.current.size >= 2) {
      const [p1, p2] = Array.from(pointersRef.current.values());
      const currPinch = getPinchMetrics(p1, p2);
      const prevPinch = gestureRef.current.prevPinch;
      if (prevPinch && prevPinch.distance > 0 && currPinch.distance > 0) {
        const { viewport, center, content } = getViewportMetrics();
        const nextScale =
          transformRef.current.scale * (currPinch.distance / prevPinch.distance);
        const prevFocal: Point = {
          x: prevPinch.midpoint.x - center.x,
          y: prevPinch.midpoint.y - center.y,
        };
        const nextFocal: Point = {
          x: currPinch.midpoint.x - center.x,
          y: currPinch.midpoint.y - center.y,
        };
        commitTransform(
          zoomAroundPoint(
            transformRef.current,
            nextScale,
            prevFocal,
            nextFocal,
            viewport,
            content
          )
        );
      }
      gestureRef.current.prevPinch = currPinch;
      return;
    }

    if (gestureRef.current.wasPinching) return;

    const distFromStart = Math.hypot(
      point.x - gestureRef.current.startPoint.x,
      point.y - gestureRef.current.startPoint.y
    );
    if (distFromStart > TAP_SLOP_PX) {
      gestureRef.current.moved = true;
      if (gestureRef.current.isSecondTap) {
        gestureRef.current.isSecondTap = false;
        flushPendingTapToggle();
      }
      lastTapRef.current = null;
    }

    const delta: Point = {
      x: point.x - gestureRef.current.lastPoint.x,
      y: point.y - gestureRef.current.lastPoint.y,
    };
    gestureRef.current.lastPoint = point;

    if (transformRef.current.scale > MIN_IMAGE_SCALE) {
      const { viewport, content } = getViewportMetrics();
      commitTransform(
        panByDelta(transformRef.current, delta, viewport, content)
      );
    }
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!pointersRef.current.has(e.pointerId)) return;
    pointersRef.current.delete(e.pointerId);
    releaseCaptureSafely(e.currentTarget, e.pointerId);

    if (pointersRef.current.size === 1) {
      const [remaining] = Array.from(pointersRef.current.values());
      gestureRef.current.lastPoint = remaining;
      gestureRef.current.prevPinch = null;
      gestureRef.current.wasPinching = false;
      return;
    }

    if (pointersRef.current.size > 0) return;

    if (gestureRef.current.moved || gestureRef.current.wasPinching) {
      lastTapRef.current = null;
      return;
    }

    const tap: TapRecord = {
      time: Date.now(),
      point: { x: e.clientX, y: e.clientY },
    };

    if (
      gestureRef.current.isSecondTap ||
      isDoubleTapCandidate(lastTapRef.current, tap)
    ) {
      cancelPendingTapToggle();
      lastTapRef.current = null;
      const { viewport, center, content } = getViewportMetrics();
      const tapFromCenter: Point = {
        x: tap.point.x - center.x,
        y: tap.point.y - center.y,
      };
      setAnimateTransform(true);
      commitTransform(
        toggleDoubleTapZoom(
          transformRef.current,
          tapFromCenter,
          viewport,
          content
        )
      );
      return;
    }

    flushPendingTapToggle();

    lastTapRef.current = tap;
    pendingTapToggleRef.current = true;
    singleTapTimerRef.current = window.setTimeout(() => {
      singleTapTimerRef.current = null;
      pendingTapToggleRef.current = false;
      lastTapRef.current = null;
      setControlsVisible((visible) => !visible);
    }, DOUBLE_TAP_DELAY_MS);
  };

  const handlePointerCancel = (e: React.PointerEvent<HTMLDivElement>) => {
    pointersRef.current.delete(e.pointerId);
    releaseCaptureSafely(e.currentTarget, e.pointerId);
    if (pointersRef.current.size < 2) {
      gestureRef.current.prevPinch = null;
    }
    if (pointersRef.current.size === 1) {
      const [remaining] = Array.from(pointersRef.current.values());
      gestureRef.current.lastPoint = remaining;
      gestureRef.current.wasPinching = false;
      return;
    }
    if (pointersRef.current.size === 0 && gestureRef.current.isSecondTap) {
      gestureRef.current.isSecondTap = false;
      flushPendingTapToggle();
      lastTapRef.current = null;
    }
  };

  const modal = (
    <div
      ref={dialogRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label={preview ? preview.fileName : t.preview.loading}
      className="fixed inset-0 w-screen h-screen z-50 overflow-hidden bg-black text-white select-none outline-none"
    >
      {/* Floating Top Overlay Bar */}
      <div
        aria-hidden={!controlsVisible}
        className={`safe-top absolute inset-x-0 top-0 z-20 flex items-center justify-between gap-3 bg-gradient-to-b from-black/80 via-black/50 to-transparent px-4 pb-6 sm:px-6 transition-all duration-200 ${
          controlsVisible
            ? "opacity-100 translate-y-0 pointer-events-auto visible"
            : "opacity-0 -translate-y-2 pointer-events-none invisible"
        }`}
      >
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-white drop-shadow-sm">
            {preview ? preview.fileName : t.preview.loading}
          </div>
          {preview && (
            <div className="font-mono text-xs text-white/70">
              {formatBytes(preview.sizeBytes)}
            </div>
          )}
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {preview?.kind === "text" && preview.textContent !== null && (
            <GlassButton
              size="sm"
              variant="glass"
              onClick={handleCopyText}
              title={t.preview.copyText}
              aria-label={t.preview.copyText}
              className="!bg-white/15 !text-white !border-white/25 hover:!bg-white/25"
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
              aria-label={t.preview.openExternal}
              className="!bg-white/15 !text-white !border-white/25 hover:!bg-white/25"
            >
              <ExternalLink className="h-4 w-4" />
              <span className="hidden sm:inline">{t.preview.openExternal}</span>
            </GlassButton>
          )}

          <GlassButton
            size="sm"
            variant="ghost"
            onClick={onClose}
            className="px-2.5 !text-white hover:!bg-white/15"
            aria-label={t.update.close}
          >
            <X className="h-5 w-5" />
          </GlassButton>
        </div>
      </div>

      {/* Fullscreen Media Viewport */}
      <div className="h-full w-full">
        {loading ? (
          <div className="flex h-full w-full flex-col items-center justify-center gap-3 text-sm text-white/75">
            <RefreshCw className="h-7 w-7 animate-spin text-indigo-400" />
            <span>{t.preview.loading}</span>
          </div>
        ) : preview?.kind === "image" && preview.url ? (
          <div
            ref={viewportRef}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
            onPointerCancel={handlePointerCancel}
            onLostPointerCapture={handlePointerCancel}
            className={`flex h-full w-full items-center justify-center overflow-hidden touch-none ${
              transform.scale > MIN_IMAGE_SCALE
                ? "cursor-grab active:cursor-grabbing"
                : "cursor-default"
            }`}
          >
            <img
              ref={imgRef}
              src={preview.url}
              alt={preview.fileName}
              draggable={false}
              style={{
                transform: `translate3d(${transform.offset.x}px, ${transform.offset.y}px, 0) scale(${transform.scale})`,
                transition: animateTransform
                  ? "transform 220ms cubic-bezier(0.22, 1, 0.36, 1)"
                  : "none",
              }}
              className="max-h-full max-w-full object-contain will-change-transform"
            />
          </div>
        ) : preview?.kind === "video" && preview.url ? (
          <div className="flex h-full w-full items-center justify-center bg-black">
            <video
              src={preview.url}
              controls
              playsInline
              preload="metadata"
              className="h-full w-full object-contain outline-none"
            />
          </div>
        ) : preview?.kind === "audio" && preview.url ? (
          <div className="flex h-full w-full flex-col items-center justify-center gap-5 px-6">
            <div className="flex h-24 w-24 items-center justify-center rounded-3xl bg-indigo-500/20 border border-indigo-400/30">
              <Music className="h-10 w-10 text-indigo-300" />
            </div>
            <div className="text-center">
              <div className="text-base font-medium text-white/95">
                {preview.fileName}
              </div>
              <div className="mt-0.5 font-mono text-xs text-white/55">
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
          <div className="safe-top safe-bottom h-full w-full overflow-auto bg-slate-950">
            <div className="mx-auto max-w-4xl px-4 pb-8 pt-16 sm:px-8">
              {preview.truncated && (
                <div className="mb-3">
                  <GlassBadge tone="warning">{t.preview.truncated}</GlassBadge>
                </div>
              )}
              <pre className="font-mono text-xs leading-relaxed text-white/90 whitespace-pre-wrap break-words select-text">
                {preview.textContent ?? ""}
              </pre>
            </div>
          </div>
        ) : (
          <div className="flex h-full w-full flex-col items-center justify-center gap-4 px-6 text-center">
            <div className="flex h-16 w-16 items-center justify-center rounded-3xl bg-white/10 border border-white/15">
              <FileIcon className="h-7 w-7 text-white/75" />
            </div>
            <div className="space-y-1">
              <div className="text-sm font-medium text-white/90">
                {preview?.fileName}
              </div>
              <div className="text-xs text-white/60">
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
  );

  if (typeof document !== "undefined" && document.body) {
    return createPortal(modal, document.body);
  }
  return modal;
}
