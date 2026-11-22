import { useEffect, useMemo, useState } from "react";
import { Copy, RefreshCw, Send, Trash2 } from "lucide-react";
import { api } from "../../core/api";
import type { ClipboardItem, PeerInfo } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassInput,
  GlassSegmented,
} from "../../components/glass/GlassPrimitives";

interface ClipboardViewProps {
  t: TranslationDict;
  peers: PeerInfo[];
  enabled: boolean;
  refreshTick: number;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function ClipboardView({
  t,
  peers,
  enabled,
  refreshTick,
  onError,
  onNotice,
}: ClipboardViewProps) {
  const [draft, setDraft] = useState("");
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

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
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
            onChange={setTargetId}
          />
        </div>
      )}

      {/* Compose */}
      <GlassCard className="p-4">
        <div className="flex gap-2">
          <GlassInput
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={t.clipboard.inputPlaceholder}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSubmit();
            }}
          />
          <GlassButton
            variant="primary"
            disabled={!draft.trim()}
            onClick={handleSubmit}
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
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <GlassButton
                    size="sm"
                    variant="ghost"
                    onClick={() => copyText(item.text)}
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
