import { useEffect, useState } from "react";
import { Copy, RefreshCw, Send, Trash2 } from "lucide-react";
import { api } from "../../core/api";
import type { ClipboardItem, PeerInfo } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassInput,
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
  const [localItems, setLocalItems] = useState<ClipboardItem[]>([]);
  const [selectedPeerId, setSelectedPeerId] = useState("");
  const [remoteItems, setRemoteItems] = useState<ClipboardItem[]>([]);

  const trustedOnlinePeers = peers.filter((p) => p.isTrusted && p.isOnline);

  const loadLocal = async () => {
    try {
      setLocalItems(await api.clipboardListLocal());
    } catch (e) {
      onError(String(e));
    }
  };

  useEffect(() => {
    if (enabled) {
      loadLocal();
    }
  }, [enabled, refreshTick]);

  const handleSaveLocal = async () => {
    if (!draft.trim()) return;
    try {
      await api.clipboardPushLocalText(draft);
      setDraft("");
      await loadLocal();
    } catch (e) {
      onError(String(e));
    }
  };

  const handleSendToPeer = async (peerId: string) => {
    if (!draft.trim()) return;
    try {
      await api.clipboardSendToPeer(peerId, draft);
      await api.clipboardPushLocalText(draft);
      setDraft("");
      await loadLocal();
      onNotice(t.clipboard.copied);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleFetchRemote = async (peerId: string) => {
    setSelectedPeerId(peerId);
    try {
      setRemoteItems(await api.clipboardFetchRemote(peerId));
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

  return (
    <div className="space-y-5">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
          {t.clipboard.title}
        </h1>
        <p className="text-sm text-slate-700/70 dark:text-white/60">
          {t.clipboard.subtitle}
        </p>
      </div>

      {/* Compose / Send */}
      <GlassCard className="p-5 space-y-3">
        <div className="flex flex-col gap-3 sm:flex-row">
          <GlassInput
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={t.clipboard.inputPlaceholder}
          />
          <GlassButton variant="primary" onClick={handleSaveLocal}>
            {t.clipboard.saveLocal}
          </GlassButton>
        </div>

        {trustedOnlinePeers.length > 0 && (
          <div className="flex flex-wrap items-center gap-2 pt-1">
            <span className="text-xs text-slate-700/70 dark:text-white/55">
              {t.clipboard.sendTo}:
            </span>
            {trustedOnlinePeers.map((p) => (
              <GlassButton
                key={p.peerId}
                size="sm"
                disabled={!draft.trim()}
                onClick={() => handleSendToPeer(p.peerId)}
              >
                <Send className="h-3.5 w-3.5" />
                <span>{p.displayName}</span>
              </GlassButton>
            ))}
          </div>
        )}
      </GlassCard>

      {/* Local History */}
      <GlassCard className="p-5 space-y-3">
        <h2 className="text-base font-semibold text-slate-900/90 dark:text-white/90">
          {t.clipboard.localHistory}
        </h2>
        {localItems.length === 0 ? (
          <p className="text-sm text-slate-700/70 dark:text-white/55">
            {t.clipboard.empty}
          </p>
        ) : (
          <div className="space-y-2">
            {localItems.map((item) => (
              <div
                key={item.id}
                className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3"
              >
                <div className="min-w-0 flex-1">
                  <p className="line-clamp-2 break-all text-sm text-slate-900/90 dark:text-white/90">
                    {item.text}
                  </p>
                  {item.sourcePeerName && (
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
                </div>
              </div>
            ))}
          </div>
        )}
      </GlassCard>

      {/* Remote Friend Clipboard */}
      {trustedOnlinePeers.length > 0 && (
        <GlassCard className="p-5 space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-base font-semibold text-slate-900/90 dark:text-white/90">
              {t.clipboard.remoteTitle}
            </h2>
            <div className="flex flex-wrap gap-2">
              {trustedOnlinePeers.map((p) => (
                <GlassButton
                  key={p.peerId}
                  size="sm"
                  variant={selectedPeerId === p.peerId ? "primary" : "glass"}
                  onClick={() => handleFetchRemote(p.peerId)}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                  <span>{p.displayName}</span>
                </GlassButton>
              ))}
            </div>
          </div>

          {remoteItems.length > 0 && (
            <div className="space-y-2">
              {remoteItems.map((item) => (
                <div
                  key={item.id}
                  className="flex items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3"
                >
                  <p className="line-clamp-2 break-all text-sm text-slate-900/90 dark:text-white/90">
                    {item.text}
                  </p>
                  <GlassButton
                    size="sm"
                    variant="ghost"
                    onClick={() => copyText(item.text)}
                  >
                    <Copy className="h-4 w-4" />
                  </GlassButton>
                </div>
              ))}
            </div>
          )}
        </GlassCard>
      )}
    </div>
  );
}
