import { useEffect, useMemo, useRef, useState } from "react";
import {
  ClipboardPaste,
  Copy,
  Image as ImageIcon,
  RefreshCw,
  Send,
  Trash2,
  X,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import { processClipboardImagePaste } from "./clipboardPaste.ts";
import type {
  ClipboardItem,
  PeerInfo,
  PreviewDescriptor,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassInput,
  GlassSegmented,
} from "../../components/glass/GlassPrimitives";

const imagePreviewCache = new Map<string, PreviewDescriptor>();

function ClipboardImageThumb({
  item,
  peerId,
  onOpenPreview,
}: {
  item: ClipboardItem;
  peerId: string | null;
  onOpenPreview: (desc: PreviewDescriptor) => void;
}) {
  const cacheKey = `${peerId ?? "local"}:${item.id}:${item.contentHash}`;
  const [desc, setDesc] = useState<PreviewDescriptor | null>(
    () => imagePreviewCache.get(cacheKey) ?? null
  );
  const containerRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const cached = imagePreviewCache.get(cacheKey);
    if (cached) {
      setDesc(cached);
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
          .clipboardGetImagePreview(item.id, peerId)
          .then((res) => {
            if (cancelled) return;
            if (imagePreviewCache.size >= 100) {
              const firstKey = imagePreviewCache.keys().next().value;
              if (firstKey) imagePreviewCache.delete(firstKey);
            }
            imagePreviewCache.set(cacheKey, res);
            setDesc(res);
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
  }, [cacheKey, item.id, peerId]);

  return (
    <button
      ref={containerRef}
      type="button"
      onClick={() => {
        if (desc) {
          onOpenPreview(desc);
        } else {
          api
            .clipboardGetImagePreview(item.id, peerId)
            .then((res) => {
              imagePreviewCache.set(cacheKey, res);
              setDesc(res);
              onOpenPreview(res);
            })
            .catch(() => {});
        }
      }}
      className="group relative flex h-24 w-36 shrink-0 items-center justify-center overflow-hidden rounded-2xl border border-white/25 dark:border-white/15 bg-white/25 dark:bg-white/10 transition hover:brightness-105"
    >
      {desc?.url ? (
        <img
          src={desc.url}
          alt={item.text || "PNG"}
          className="h-full w-full object-cover transition-transform duration-200 group-hover:scale-[1.03]"
        />
      ) : (
        <ImageIcon className="h-6 w-6 text-slate-600/70 dark:text-white/50" />
      )}
    </button>
  );
}

interface ClipboardViewProps {
  t: TranslationDict;
  peers: PeerInfo[];
  enabled: boolean;
  refreshTick: number;
  onPreviewDescriptor: (desc: PreviewDescriptor) => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function ClipboardView({
  t,
  peers,
  enabled,
  refreshTick,
  onPreviewDescriptor,
  onError,
  onNotice,
}: ClipboardViewProps) {
  const [draft, setDraft] = useState("");
  const [pendingImage, setPendingImage] = useState<ClipboardItem | null>(null);
  const [pasting, setPasting] = useState(false);
  const [targetId, setTargetId] = useState<string>("local");
  const [localItems, setLocalItems] = useState<ClipboardItem[]>([]);
  const [remoteItems, setRemoteItems] = useState<ClipboardItem[]>([]);

  const trustedOnlinePeers = useMemo(
    () => peers.filter((p) => p.isTrusted && p.isOnline),
    [peers]
  );

  const loadLocal = async () => {
    try {
      setLocalItems(await api.clipboardListLocal());
    } catch (e) {
      onError(String(e));
    }
  };

  const loadRemote = async (peerId: string) => {
    try {
      setRemoteItems(await api.clipboardFetchRemote(peerId));
    } catch (e) {
      onError(String(e));
    }
  };

  useEffect(() => {
    if (!enabled) return;
    if (targetId === "local") {
      loadLocal();
    } else if (trustedOnlinePeers.some((p) => p.peerId === targetId)) {
      loadRemote(targetId);
    } else {
      setTargetId("local");
    }
  }, [enabled, refreshTick, targetId, trustedOnlinePeers]);

  const handleSubmit = async () => {
    if (pendingImage && targetId !== "local") {
      try {
        await api.clipboardSendImageToPeer(targetId, pendingImage.id);
        setPendingImage(null);
        onNotice(t.clipboard.sent);
      } catch (e) {
        onError(String(e));
      }
      return;
    }

    if (!draft.trim()) return;
    try {
      if (targetId === "local") {
        await api.clipboardPushLocalText(draft);
        setDraft("");
        await loadLocal();
      } else {
        await api.clipboardSendToPeer(targetId, draft);
        await api.clipboardPushLocalText(draft);
        setDraft("");
        onNotice(t.clipboard.sent);
      }
    } catch (e) {
      onError(String(e));
    }
  };

  const handlePasteFromSystem = async () => {
    setPasting(true);
    try {
      const res = await api.clipboardPasteFromSystem();
      if (res.kind === "empty") {
        onNotice(t.clipboard.clipboardEmpty);
        return;
      }
      if (res.kind === "text") {
        setPendingImage(null);
        setDraft(res.text);
        return;
      }
      if (res.kind === "image") {
        if (targetId === "local") {
          await loadLocal();
        } else {
          setPendingImage(res.item);
        }
      }
    } catch (e) {
      onError(String(e));
    } finally {
      setPasting(false);
    }
  };

  const handleInputPaste = async (
    e: React.ClipboardEvent<HTMLInputElement>
  ) => {
    await processClipboardImagePaste(
      e.clipboardData?.items,
      () => e.preventDefault(),
      {
        pushLocalImageBytes: api.clipboardPushLocalImageBytes,
        onImageSaved: async (saved) => {
          if (targetId === "local") {
            await loadLocal();
          } else {
            setPendingImage(saved);
          }
        },
        onError,
        tooLargeMessage: t.clipboard.imageTooLarge,
      }
    );
  };

  const copyItemToSystem = async (item: ClipboardItem) => {
    try {
      await api.clipboardCopyToSystem(
        item.id,
        targetId === "local" ? null : targetId,
        item.kind === "text" ? item.text : null
      );
      onNotice(t.clipboard.copied);
    } catch (e) {
      onError(String(e));
    }
  };

  if (!enabled) {
    return (
      <GlassCard className="p-8 text-center text-sm text-slate-700/75 dark:text-white/60">
        {t.clipboard.disabledNotice}
      </GlassCard>
    );
  }

  const targetOptions = [
    { id: "local", label: t.clipboard.localTab },
    ...trustedOnlinePeers.map((p) => ({
      id: p.peerId,
      label: p.displayName,
    })),
  ];

  const visibleItems = targetId === "local" ? localItems : remoteItems;
  const canSubmit = Boolean(pendingImage && targetId !== "local") || Boolean(draft.trim());

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
          {t.clipboard.title}
        </h1>
        {targetId !== "local" && (
          <GlassButton
            size="sm"
            variant="glass"
            onClick={() => loadRemote(targetId)}
          >
            <RefreshCw className="h-4 w-4" />
          </GlassButton>
        )}
      </div>

      {/* Single Unified Target Selector */}
      {targetOptions.length > 1 && (
        <div className="overflow-x-auto pb-1">
          <GlassSegmented
            options={targetOptions}
            value={targetId}
            onChange={(next) => {
              setTargetId(next);
              if (next === "local") setPendingImage(null);
            }}
          />
        </div>
      )}

      {/* Compose */}
      <GlassCard className="p-4 space-y-3">
        {pendingImage && targetId !== "local" && (
          <div className="flex items-center justify-between gap-3 rounded-2xl bg-white/25 dark:bg-white/10 p-2.5">
            <div className="flex items-center gap-3 min-w-0">
              <ClipboardImageThumb
                item={pendingImage}
                peerId={null}
                onOpenPreview={onPreviewDescriptor}
              />
              <div className="min-w-0">
                <div className="text-sm font-medium text-slate-900/90 dark:text-white/90">
                  PNG {pendingImage.imageWidth}×{pendingImage.imageHeight}
                </div>
                <div className="font-mono text-xs text-slate-700/65 dark:text-white/50">
                  {formatBytes(pendingImage.sizeBytes)}
                </div>
              </div>
            </div>
            <GlassButton
              size="sm"
              variant="ghost"
              onClick={() => setPendingImage(null)}
            >
              <X className="h-4 w-4" />
            </GlassButton>
          </div>
        )}

        <div className="flex gap-2">
          <GlassInput
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onPaste={handleInputPaste}
            placeholder={t.clipboard.inputPlaceholder}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSubmit();
            }}
          />
          <GlassButton
            variant="glass"
            disabled={pasting}
            title={t.clipboard.pasteFromClipboard}
            aria-label={t.clipboard.pasteFromClipboard}
            onClick={handlePasteFromSystem}
            className="shrink-0 px-3.5"
          >
            <ClipboardPaste className="h-4 w-4" />
            <span className="hidden md:inline">
              {t.clipboard.pasteFromClipboard}
            </span>
          </GlassButton>
          <GlassButton
            variant="primary"
            disabled={!canSubmit}
            onClick={handleSubmit}
            className="shrink-0"
          >
            {targetId === "local" ? (
              <span>{t.clipboard.saveLocal}</span>
            ) : (
              <>
                <Send className="h-4 w-4" />
                <span className="hidden sm:inline">{t.clipboard.send}</span>
              </>
            )}
          </GlassButton>
        </div>
      </GlassCard>

      {/* Items List */}
      <GlassCard className="p-4">
        {visibleItems.length === 0 ? (
          <p className="py-6 text-center text-sm text-slate-700/65 dark:text-white/50">
            {t.clipboard.empty}
          </p>
        ) : (
          <div className="space-y-2">
            {visibleItems.map((item) => (
              <div
                key={item.id}
                className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3"
              >
                <div className="min-w-0 flex-1">
                  {item.kind === "image" ? (
                    <div className="flex items-center gap-3">
                      <ClipboardImageThumb
                        item={item}
                        peerId={targetId === "local" ? null : targetId}
                        onOpenPreview={onPreviewDescriptor}
                      />
                      <div className="min-w-0 space-y-1">
                        <div className="text-sm font-medium text-slate-900/90 dark:text-white/90">
                          {item.imageWidth && item.imageHeight
                            ? `PNG ${item.imageWidth}×${item.imageHeight}`
                            : "PNG"}
                        </div>
                        <div className="font-mono text-xs text-slate-700/65 dark:text-white/50">
                          {formatBytes(item.sizeBytes)}
                        </div>
                        {targetId === "local" && item.sourcePeerName && (
                          <GlassBadge tone="accent">
                            {item.sourcePeerName}
                          </GlassBadge>
                        )}
                      </div>
                    </div>
                  ) : (
                    <>
                      <p className="line-clamp-2 break-all text-sm text-slate-900/90 dark:text-white/90">
                        {item.text}
                      </p>
                      {targetId === "local" && item.sourcePeerName && (
                        <div className="mt-1">
                          <GlassBadge tone="accent">
                            {item.sourcePeerName}
                          </GlassBadge>
                        </div>
                      )}
                    </>
                  )}
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <GlassButton
                    size="sm"
                    variant="ghost"
                    onClick={() => copyItemToSystem(item)}
                  >
                    <Copy className="h-4 w-4" />
                  </GlassButton>
                  {targetId === "local" && (
                    <GlassButton
                      size="sm"
                      variant="ghost"
                      onClick={async () => {
                        await api.clipboardDeleteLocal(item.id);
                        await loadLocal();
                      }}
                    >
                      <Trash2 className="h-4 w-4" />
                    </GlassButton>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </GlassCard>
    </div>
  );
}
