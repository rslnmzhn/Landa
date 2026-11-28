import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Activity,
  Clipboard,
  Film,
  FolderHeart,
  Laptop,
  Settings,
} from "lucide-react";
import { api, formatBytes } from "./core/api";
import { resolveTabSwipe } from "./core/tabSwipe";
import type {
  AppSettings,
  ClipboardItem,
  FriendRequestInfo,
  IncomingTransferOfferInfo,
  LocalIdentityInfo,
  ModuleStatus,
  PeerInfo,
  PendingFocusTarget,
  PreviewDescriptor,
  TransferProgressInfo,
  UpdateCheckResult,
  UpdateProgressEvent,
} from "./core/types";
import { translations } from "./i18n/translations";
import {
  GlassButton,
  GlassModal,
  GlassPillNav,
  MeshCanvas,
  type NavTabItem,
} from "./components/glass/GlassPrimitives";
import { DevicesView } from "./features/devices/DevicesView";

const SharesView = lazy(() =>
  import("./features/shares/SharesView").then((m) => ({ default: m.SharesView }))
);
const ClipboardView = lazy(() =>
  import("./features/clipboard/ClipboardView").then((m) => ({
    default: m.ClipboardView,
  }))
);
const ActivityView = lazy(() =>
  import("./features/activity/ActivityView").then((m) => ({
    default: m.ActivityView,
  }))
);
const SettingsView = lazy(() =>
  import("./features/settings/SettingsView").then((m) => ({
    default: m.SettingsView,
  }))
);
const FilePreviewModal = lazy(() =>
  import("./features/preview/FilePreviewModal").then((m) => ({
    default: m.FilePreviewModal,
  }))
);
const VideoLinkView = lazy(() =>
  import("./features/videolink/VideoLinkView").then((m) => ({
    default: m.VideoLinkView,
  }))
);

type TabId =
  | "devices"
  | "shares"
  | "clipboard"
  | "videolink"
  | "activity"
  | "settings";

export function App() {
  const [activeTab, setActiveTab] = useState<TabId>("devices");
  const [identity, setIdentity] = useState<LocalIdentityInfo | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [friendRequests, setFriendRequests] = useState<FriendRequestInfo[]>([]);
  const [historyTick, setHistoryTick] = useState(0);
  const [activeTransfers, setActiveTransfers] = useState<
    Record<string, TransferProgressInfo>
  >({});
  const [pendingOffers, setPendingOffers] = useState<
    IncomingTransferOfferInfo[]
  >([]);
  const [focusedFriendPeerId, setFocusedFriendPeerId] = useState<string | null>(
    null
  );
  const [pendingSharedPaths, setPendingSharedPaths] = useState<string[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [modules, setModules] = useState<ModuleStatus[]>([]);
  const [clipboardTick, setClipboardTick] = useState(0);
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResult | null>(null);
  const [updateProgress, setUpdateProgress] =
    useState<UpdateProgressEvent | null>(null);
  const [previewData, setPreviewData] = useState<PreviewDescriptor | null>(
    null
  );
  const [previewLoading, setPreviewLoading] = useState(false);
  const [downloadingShareKey, setDownloadingShareKey] = useState<string | null>(
    null
  );
  const [toast, setToast] = useState<{
    kind: "error" | "notice";
    text: string;
  } | null>(null);
  const langRef = useRef<"ru" | "en">("ru");
  langRef.current = settings?.language === "en" ? "en" : "ru";
  const toastTimerRef = useRef<number | null>(null);

  const showToast = (kind: "error" | "notice", text: string) => {
    setToast({ kind, text });
    if (toastTimerRef.current !== null) {
      window.clearTimeout(toastTimerRef.current);
    }
    toastTimerRef.current = window.setTimeout(() => {
      toastTimerRef.current = null;
      setToast((prev) => (prev?.text === text ? null : prev));
    }, 4000);
  };

  useEffect(() => {
    return () => {
      if (toastTimerRef.current !== null) {
        window.clearTimeout(toastTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let unlistenIdentity: (() => void) | undefined;
    let unlistenUpdate: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;

    const applyFocusTarget = (focus: PendingFocusTarget | null) => {
      if (!focus) return;
      if (focus.target === "activity") {
        setActiveTab("activity");
        return;
      }
      if (focus.target === "clipboard") {
        setActiveTab("clipboard");
        return;
      }
      if (focus.target === "transfer_offer") {
        setActiveTab("devices");
        if (focus.transferId) {
          setPendingOffers((prev) => {
            const idx = prev.findIndex((o) => o.transferId === focus.transferId);
            if (idx <= 0) return prev;
            const targetOffer = prev[idx];
            return [targetOffer, ...prev.slice(0, idx), ...prev.slice(idx + 1)];
          });
        }
        return;
      }
      if (focus.target === "friend_requests") {
        setActiveTab("devices");
        api.listFriendRequests().then(setFriendRequests).catch(() => {});
        setFocusedFriendPeerId(focus.peerId ?? "__latest__");
        window.scrollTo({ top: 0, behavior: "smooth" });
      }
    };

    const bootstrap = async () => {
      try {
        await api.ensureStartupPermissions().catch(() => {});
        if (disposed) return;
        const [id, p, reqs, cfg, mods] = await Promise.all([
          api.getLocalIdentity(),
          api.listPeers(),
          api.listFriendRequests(),
          api.getSettings(),
          api.listModules(),
        ]);
        if (disposed) return;
        setIdentity(id);
        setPeers(p);
        setFriendRequests(reqs);
        setSettings(cfg);
        setModules(mods);
        api.triggerScan().catch(() => {});
        api.checkForUpdates().then((info) => {
          if (!disposed) setUpdateInfo(info);
        }).catch(() => {});

        const shared = await api.takePendingSharedFiles().catch(() => []);
        if (disposed) return;
        if (shared.length > 0) {
          setPendingSharedPaths((prev) => [...prev, ...shared]);
        }
        const focus = await api.takePendingFocusTarget().catch(() => null);
        if (disposed) return;
        applyFocusTarget(focus);
      } catch (e) {
        if (!disposed) showToast("error", String(e));
      }

      const uIdentity = await api.onIdentityUpdated(() => {
        api.getLocalIdentity().then(setIdentity).catch(() => {});
        api.getSettings().then(setSettings).catch(() => {});
      });
      if (disposed) {
        uIdentity();
        return;
      }
      unlistenIdentity = uIdentity;

      const uUpdate = await api.onUpdateProgress((ev) => {
        setUpdateProgress(ev);
      });
      if (disposed) {
        uUpdate();
        return;
      }
      unlistenUpdate = uUpdate;

      const uFocus = await api.onFocusTarget((focus) => {
        applyFocusTarget(focus);
      });
      if (disposed) {
        uFocus();
        return;
      }
      unlistenFocus = uFocus;

      const uCore = await api.onCoreEvent((ev) => {
        switch (ev.type) {
          case "peersChanged":
            api.listPeers().then(setPeers).catch(() => {});
            break;
          case "friendRequestsChanged":
            api.listFriendRequests().then(setFriendRequests).catch(() => {});
            break;
          case "historyChanged":
            setHistoryTick((n) => n + 1);
            break;
          case "incomingTransferOffer":
            setPendingOffers((prev) => [...prev, ev.payload]);
            break;
          case "transferProgress": {
            const p = ev.payload;
            setActiveTransfers((prev) => {
              const next = { ...prev };
              if (
                p.status === "completed" ||
                p.status === "failed" ||
                p.status === "cancelled" ||
                p.status === "rejected"
              ) {
                delete next[p.transferId];
              } else {
                next[p.transferId] = p;
              }
              return next;
            });
            break;
          }
          case "moduleEvent":
            if (ev.payload.moduleId === "clipboard") {
              setClipboardTick((n) => n + 1);
              if (
                ev.payload.eventName === "received" &&
                ev.payload.data &&
                typeof ev.payload.data === "object"
              ) {
                const item = ev.payload.data as Partial<ClipboardItem>;
                const dict = translations[langRef.current].clipboard;
                const peerName = item.sourcePeerName || "Peer";
                if (item.kind === "image") {
                  showToast("notice", `${dict.receivedImage} ${peerName}`);
                } else if (typeof item.text === "string" && item.text.trim()) {
                  showToast("notice", `${dict.receivedText} ${peerName}`);
                }
              }
            }
            break;
        }
      });
      if (disposed) {
        uCore();
        return;
      }
      unlisten = uCore;
    };

    bootstrap();

    const checkSharedAndScan = () => {
      if (document.visibilityState === "visible") {
        api.triggerScan().catch(() => {});
      }
      api
        .takePendingSharedFiles()
        .then((shared) => {
          if (shared.length > 0) {
            setPendingSharedPaths((prev) => [...prev, ...shared]);
          }
        })
        .catch(() => {});
      api
        .takePendingFocusTarget()
        .then((focus) => applyFocusTarget(focus))
        .catch(() => {});
    };
    let lastFocusScanAt = { value: 0 };
    const FOCUS_SCAN_COOLDOWN_MS = 400;
    const handleFocusOrVisibility = () => {
      if (document.visibilityState !== "visible") return;
      const now = Date.now();
      if (now - lastFocusScanAt.value < FOCUS_SCAN_COOLDOWN_MS) return;
      lastFocusScanAt.value = now;
      checkSharedAndScan();
    };
    const handleFocusIntentEvent = () => {
      api
        .takePendingFocusTarget()
        .then((focus) => applyFocusTarget(focus))
        .catch(() => {});
    };
    const handleNetworkChanged = () => {
      api.triggerScan().catch(() => {});
    };
    window.addEventListener("focus", handleFocusOrVisibility);
    window.addEventListener("landa-share-intent", checkSharedAndScan);
    window.addEventListener("landa-focus-target", handleFocusIntentEvent);
    window.addEventListener("landa-network-changed", handleNetworkChanged);
    document.addEventListener("visibilitychange", handleFocusOrVisibility);

    return () => {
      disposed = true;
      window.removeEventListener("focus", handleFocusOrVisibility);
      window.removeEventListener("landa-share-intent", checkSharedAndScan);
      window.removeEventListener("landa-focus-target", handleFocusIntentEvent);
      window.removeEventListener("landa-network-changed", handleNetworkChanged);
      document.removeEventListener("visibilitychange", handleFocusOrVisibility);
      if (unlisten) unlisten();
      if (unlistenIdentity) unlistenIdentity();
      if (unlistenUpdate) unlistenUpdate();
      if (unlistenFocus) unlistenFocus();
    };
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    if (settings?.theme === "light") {
      root.classList.remove("dark");
    } else {
      root.classList.add("dark");
    }
  }, [settings?.theme]);

  const lang = settings?.language === "en" ? "en" : "ru";
  const t = translations[lang];

  const isAndroid =
    identity?.os === "android" ||
    navigator.userAgent.toLowerCase().includes("android");

  const isModuleEnabled = (id: string) =>
    modules.find((m) => m.id === id)?.enabled ?? false;

  const sharesEnabled = isModuleEnabled("shares");
  const clipboardEnabled = isModuleEnabled("clipboard");
  const videolinkEnabled = !isAndroid && isModuleEnabled("videolink");

  useEffect(() => {
    if (activeTab === "shares" && !sharesEnabled) setActiveTab("devices");
    if (activeTab === "clipboard" && !clipboardEnabled) setActiveTab("devices");
    if (activeTab === "videolink" && !videolinkEnabled) setActiveTab("devices");
  }, [activeTab, sharesEnabled, clipboardEnabled, videolinkEnabled]);

  const activeTransferList = useMemo(
    () => Object.values(activeTransfers),
    [activeTransfers]
  );

  const incomingFriendCount = useMemo(
    () => friendRequests.filter((r) => r.direction === "incoming").length,
    [friendRequests]
  );

  const navItems: NavTabItem[] = useMemo(() => {
    const items: NavTabItem[] = [
      {
        id: "devices",
        label: t.nav.devices,
        icon: <Laptop className="h-4 w-4" />,
        badge: incomingFriendCount,
      },
    ];
    if (sharesEnabled) {
      items.push({
        id: "shares",
        label: t.nav.shares,
        icon: <FolderHeart className="h-4 w-4" />,
      });
    }
    if (clipboardEnabled) {
      items.push({
        id: "clipboard",
        label: t.nav.clipboard,
        icon: <Clipboard className="h-4 w-4" />,
      });
    }
    if (videolinkEnabled) {
      items.push({
        id: "videolink",
        label: t.nav.videolink,
        icon: <Film className="h-4 w-4" />,
      });
    }
    items.push({
      id: "activity",
      label: t.nav.activity,
      icon: <Activity className="h-4 w-4" />,
      badge: activeTransferList.length,
    });
    items.push({
      id: "settings",
      label: t.nav.settings,
      icon: <Settings className="h-4 w-4" />,
    });
    return items;
  }, [
    t,
    sharesEnabled,
    clipboardEnabled,
    videolinkEnabled,
    incomingFriendCount,
    activeTransferList.length,
  ]);

  const tabIds = useMemo(() => navItems.map((item) => item.id), [navItems]);
  const tabDirectionRef = useRef<-1 | 0 | 1>(0);
  const swipeRef = useRef<{
    x: number;
    y: number;
    time: number;
    locked: "none" | "horizontal" | "vertical";
  } | null>(null);

  const selectTab = useCallback(
    (nextId: string) => {
      if (nextId === activeTab) return;
      const curIdx = tabIds.indexOf(activeTab);
      const nextIdx = tabIds.indexOf(nextId);
      tabDirectionRef.current =
        curIdx >= 0 && nextIdx >= 0 ? (nextIdx > curIdx ? 1 : -1) : 0;
      setActiveTab(nextId as TabId);
    },
    [activeTab, tabIds]
  );

  const currentOffer = pendingOffers[0] ?? null;

  const focusedFriendRequest = useMemo(() => {
    if (!focusedFriendPeerId) return null;
    const incoming = friendRequests.filter((r) => r.direction === "incoming");
    if (focusedFriendPeerId === "__latest__") {
      return incoming[0] ?? null;
    }
    return incoming.find((r) => r.peerId === focusedFriendPeerId) ?? null;
  }, [focusedFriendPeerId, friendRequests]);

  const handleFocusedFriendDecision = async (accept: boolean) => {
    if (!focusedFriendRequest) return;
    const peerId = focusedFriendRequest.peerId;
    setFocusedFriendPeerId(null);
    try {
      await api.respondFriendRequest(peerId, accept);
    } catch (e) {
      showToast("error", String(e));
    }
  };

  const handleOfferDecision = async (accept: boolean) => {
    if (!currentOffer) return;
    const id = currentOffer.transferId;
    setPendingOffers((prev) => prev.slice(1));
    try {
      await api.respondTransferOffer(id, accept);
    } catch (e) {
      showToast("error", String(e));
    }
  };

  const openLocalPreview = async (pathOrUri: string) => {
    setPreviewData(null);
    setPreviewLoading(true);
    try {
      const desc = await api.prepareFilePreview(pathOrUri);
      setPreviewData(desc);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      setPreviewLoading(false);
    }
  };

  const openRemoteSharePreview = async (
    peerId: string,
    rootId: string,
    relPath: string
  ) => {
    setPreviewData(null);
    setPreviewLoading(true);
    try {
      const desc = await api.sharesPreviewRemoteFile(peerId, rootId, relPath);
      setPreviewData(desc);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleTouchStart = (e: React.TouchEvent<HTMLElement>) => {
    if (
      e.touches.length !== 1 ||
      currentOffer !== null ||
      previewData !== null ||
      previewLoading
    ) {
      swipeRef.current = null;
      return;
    }
    let el = e.target as HTMLElement | null;
    while (el && el !== e.currentTarget) {
      const tag = el.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        el.hasAttribute("data-no-swipe")
      ) {
        swipeRef.current = null;
        return;
      }
      if (el.scrollWidth > el.clientWidth + 2) {
        const overflowX = window.getComputedStyle(el).overflowX;
        if (overflowX === "auto" || overflowX === "scroll") {
          swipeRef.current = null;
          return;
        }
      }
      el = el.parentElement;
    }
    const touch = e.touches[0];
    swipeRef.current = {
      x: touch.clientX,
      y: touch.clientY,
      time: Date.now(),
      locked: "none",
    };
  };

  const handleTouchMove = (e: React.TouchEvent<HTMLElement>) => {
    const s = swipeRef.current;
    if (!s || e.touches.length !== 1) return;
    if (s.locked !== "none") return;
    const dx = e.touches[0].clientX - s.x;
    const dy = e.touches[0].clientY - s.y;
    if (Math.abs(dx) < 10 && Math.abs(dy) < 10) return;
    if (Math.abs(dy) >= Math.abs(dx) * 0.85) {
      s.locked = "vertical";
      swipeRef.current = null;
    } else {
      s.locked = "horizontal";
    }
  };

  const handleTouchEnd = (e: React.TouchEvent<HTMLElement>) => {
    const s = swipeRef.current;
    swipeRef.current = null;
    if (!s || s.locked === "vertical" || e.changedTouches.length !== 1) return;
    const touch = e.changedTouches[0];
    const nextTab = resolveTabSwipe(
      tabIds,
      activeTab,
      touch.clientX - s.x,
      touch.clientY - s.y,
      Date.now() - s.time
    );
    if (nextTab) {
      selectTab(nextTab);
    }
  };

  return (
    <div className="relative min-h-screen w-full text-slate-900/90 dark:text-white/90">
      <MeshCanvas />

      {/* Main Scrollable Content */}
      <main
        className="safe-top mx-auto max-w-5xl px-4 pb-28 pt-6 sm:px-6"
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        onTouchEnd={handleTouchEnd}
        onTouchCancel={() => {
          swipeRef.current = null;
        }}
      >
        <div
          key={activeTab}
          className="animate-glass-view"
          style={
            {
              "--tab-enter-x": `${tabDirectionRef.current * 20}px`,
            } as React.CSSProperties
          }
        >
          <Suspense fallback={null}>
            {activeTab === "devices" && (
              <DevicesView
                t={t}
                identity={identity}
                peers={peers}
                friendRequests={friendRequests}
                updateInfo={updateInfo}
                updateProgress={updateProgress}
                onUpdateInfoChange={setUpdateInfo}
                onError={(m) => showToast("error", m)}
                onNotice={(m) => showToast("notice", m)}
              />
            )}
            {activeTab === "shares" && (
              <SharesView
                t={t}
                identity={identity}
                peers={peers}
                enabled={sharesEnabled}
                activeTransfers={activeTransferList}
                downloadingKey={downloadingShareKey}
                onDownloadingKeyChange={setDownloadingShareKey}
                onPreviewRemoteFile={openRemoteSharePreview}
                onPreviewLocalPath={openLocalPreview}
                onError={(m) => showToast("error", m)}
                onNotice={(m) => showToast("notice", m)}
              />
            )}
            {activeTab === "clipboard" && (
              <ClipboardView
                t={t}
                peers={peers}
                enabled={clipboardEnabled}
                refreshTick={clipboardTick}
                onPreviewDescriptor={(desc) => setPreviewData(desc)}
                onError={(m) => showToast("error", m)}
                onNotice={(m) => showToast("notice", m)}
              />
            )}
            {activeTab === "videolink" && !isAndroid && (
              <VideoLinkView
                t={t}
                enabled={videolinkEnabled}
                onError={(m) => showToast("error", m)}
                onNotice={(m) => showToast("notice", m)}
              />
            )}
            {activeTab === "activity" && (
              <ActivityView
                t={t}
                activeTransfers={activeTransferList}
                historyTick={historyTick}
                onPreviewPath={openLocalPreview}
                onError={(m) => showToast("error", m)}
              />
            )}
            {activeTab === "settings" && settings && (
              <SettingsView
                t={t}
                settings={settings}
                modules={modules}
                onSettingsUpdated={(next) => {
                  setSettings(next);
                  setIdentity((prev) =>
                    prev ? { ...prev, deviceName: next.deviceName } : prev
                  );
                }}
                onModulesUpdated={setModules}
                onError={(m) => showToast("error", m)}
                onNotice={(m) => showToast("notice", m)}
              />
            )}
          </Suspense>
        </div>
      </main>

      {/* Floating Liquid Glass Pill Navigation */}
      <div className="safe-bottom pointer-events-none fixed inset-x-0 bottom-0 z-40 flex justify-center px-4 pb-4">
        <div className="pointer-events-auto">
          <GlassPillNav
            items={navItems}
            activeId={activeTab}
            onSelect={selectTab}
          />
        </div>
      </div>

      {/* Toast Notification */}
      {toast && (
        <div className="safe-top fixed right-4 top-4 z-[60] max-w-sm">
          <div
            className={`liquid-glass-elevated rounded-2xl px-4 py-3 text-sm font-medium ${
              toast.kind === "error"
                ? "border-rose-400/40 text-rose-900 dark:text-rose-200"
                : "text-slate-900/90 dark:text-white/90"
            }`}
          >
            {toast.text}
          </div>
        </div>
      )}

      {/* Incoming Transfer Offer Modal */}
      <GlassModal open={currentOffer !== null} title={t.offer.title}>
        {currentOffer && (
          <div className="space-y-4">
            <div className="space-y-1 text-sm">
              <div className="text-slate-700/75 dark:text-white/60">
                {t.offer.from}:{" "}
                <span className="font-semibold text-slate-900/95 dark:text-white/95">
                  {currentOffer.peerName}
                </span>
              </div>
              <div className="font-mono text-xs text-slate-700/75 dark:text-white/60">
                {t.offer.filesCount}: {currentOffer.fileCount} ·{" "}
                {t.offer.totalSize}: {formatBytes(currentOffer.totalBytes)}
              </div>
            </div>

            <div className="max-h-36 space-y-1 overflow-y-auto rounded-2xl bg-white/20 dark:bg-white/5 p-3 font-mono text-xs">
              {currentOffer.fileNames.map((name) => (
                <div key={name} className="truncate">
                  {name}
                </div>
              ))}
            </div>

            <div className="flex justify-end gap-2">
              <GlassButton
                variant="danger"
                onClick={() => handleOfferDecision(false)}
              >
                {t.offer.decline}
              </GlassButton>
              <GlassButton
                variant="primary"
                onClick={() => handleOfferDecision(true)}
              >
                {t.offer.accept}
              </GlassButton>
            </div>
          </div>
        )}
      </GlassModal>

      {/* Focused Friend Request Modal (opened from notification tap) */}
      <GlassModal
        open={focusedFriendRequest !== null}
        onClose={() => setFocusedFriendPeerId(null)}
        title={t.devices.incomingReq}
      >
        {focusedFriendRequest && (
          <div className="space-y-4">
            <div className="space-y-1 text-sm">
              <div className="font-semibold text-slate-900/95 dark:text-white/95">
                {focusedFriendRequest.deviceName}
              </div>
              <div className="font-mono text-xs text-slate-700/75 dark:text-white/60">
                {t.devices.sasLabel}: {focusedFriendRequest.sasCode}
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <GlassButton
                variant="danger"
                onClick={() => handleFocusedFriendDecision(false)}
              >
                {t.devices.decline}
              </GlassButton>
              <GlassButton
                variant="primary"
                onClick={() => handleFocusedFriendDecision(true)}
              >
                {t.devices.accept}
              </GlassButton>
            </div>
          </div>
        )}
      </GlassModal>

      {/* Android Share Intent Modal */}
      <GlassModal
        open={pendingSharedPaths.length > 0}
        onClose={() => setPendingSharedPaths([])}
        title={t.shareIntent.title}
      >
        <div className="space-y-4">
          <div className="text-xs font-mono text-slate-700/75 dark:text-white/60">
            {t.shareIntent.filesReady}: {pendingSharedPaths.length}
          </div>
          <div className="text-sm text-slate-800/85 dark:text-white/80">
            {t.shareIntent.pickDevice}
          </div>
          {peers.filter((p) => p.isOnline).length === 0 ? (
            <div className="rounded-2xl bg-white/20 dark:bg-white/5 p-3 text-xs text-slate-700/75 dark:text-white/60">
              {t.shareIntent.noOnline}
            </div>
          ) : (
            <div className="max-h-52 space-y-2 overflow-y-auto">
              {peers
                .filter((p) => p.isOnline)
                .map((peer) => (
                  <button
                    key={peer.peerId}
                    type="button"
                    onClick={async () => {
                      const paths = pendingSharedPaths;
                      setPendingSharedPaths([]);
                      try {
                        await api.sendFiles(peer.peerId, paths);
                        setActiveTab("activity");
                      } catch (e) {
                        showToast("error", String(e));
                      }
                    }}
                    className="flex w-full items-center justify-between rounded-2xl bg-white/25 dark:bg-white/10 px-4 py-3 text-left transition hover:bg-white/35 dark:hover:bg-white/15"
                  >
                    <span className="font-medium text-slate-900/95 dark:text-white/95">
                      {peer.displayName}
                    </span>
                    <span className="font-mono text-xs text-slate-700/70 dark:text-white/55">
                      {peer.ip ?? ""}
                    </span>
                  </button>
                ))}
            </div>
          )}
          <div className="flex justify-end">
            <GlassButton
              variant="ghost"
              onClick={() => setPendingSharedPaths([])}
            >
              {t.shareIntent.cancel}
            </GlassButton>
          </div>
        </div>
      </GlassModal>

      {/* In-App File Preview Modal */}
      {(previewData !== null || previewLoading) && (
        <Suspense fallback={null}>
          <FilePreviewModal
            t={t}
            preview={previewData}
            loading={previewLoading}
            onClose={() => {
              setPreviewData(null);
              setPreviewLoading(false);
            }}
            onError={(m) => showToast("error", m)}
            onNotice={(m) => showToast("notice", m)}
          />
        </Suspense>
      )}
    </div>
  );
}
