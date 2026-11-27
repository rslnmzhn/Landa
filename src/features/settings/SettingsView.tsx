import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Copy, FileText, FolderOpen, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type {
  AppSettings,
  LanInterfaceInfo,
  ManualTarget,
  ModuleStatus,
} from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassButton,
  GlassCard,
  GlassInput,
  GlassSegmented,
  GlassToggle,
} from "../../components/glass/GlassPrimitives";

interface SettingsViewProps {
  t: TranslationDict;
  settings: AppSettings;
  modules: ModuleStatus[];
  onSettingsUpdated: (next: AppSettings) => void;
  onModulesUpdated: (next: ModuleStatus[]) => void;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

export function SettingsView({
  t,
  settings,
  modules,
  onSettingsUpdated,
  onModulesUpdated,
  onError,
  onNotice,
}: SettingsViewProps) {
  const [draft, setDraft] = useState<AppSettings>(settings);
  const [targets, setTargets] = useState<ManualTarget[]>([]);
  const [lanInterfaces, setLanInterfaces] = useState<LanInterfaceInfo[]>([]);
  const [newIp, setNewIp] = useState("");
  const [cacheSize, setCacheSize] = useState<number | null>(null);
  const [clearingCache, setClearingCache] = useState(false);
  const isAndroid = navigator.userAgent.toLowerCase().includes("android");

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  const loadTargets = async () => {
    try {
      setTargets(await api.listManualTargets());
    } catch (e) {
      onError(String(e));
    }
  };

  const loadLanInterfaces = async () => {
    try {
      setLanInterfaces(await api.listLanInterfaces());
    } catch (e) {
      onError(String(e));
    }
  };

  const loadCacheSize = async () => {
    try {
      setCacheSize(await api.getCacheSize());
    } catch (e) {
      onError(String(e));
    }
  };

  useEffect(() => {
    loadTargets();
    loadLanInterfaces();
    loadCacheSize();
  }, []);

  const save = async (next: AppSettings, notify = false) => {
    setDraft(next);
    try {
      const saved = await api.updateSettings(next);
      onSettingsUpdated(saved);
      if (notify) onNotice(t.settings.saved);
    } catch (e) {
      onError(String(e));
    }
  };

  const commitDeviceName = () => {
    const trimmed = draft.deviceName.trim();
    if (!trimmed || trimmed === settings.deviceName) return;
    save({ ...draft, deviceName: trimmed }, true);
  };

  const handlePickDownloadDir = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      await save({ ...draft, downloadDir: selected });
    } catch (e) {
      onError(String(e));
    }
  };

  const handleToggleModule = async (id: string, enabled: boolean) => {
    try {
      const updated = await api.setModuleEnabled(id, enabled);
      onModulesUpdated(updated);
      onSettingsUpdated({
        ...draft,
        enabledModules: { ...draft.enabledModules, [id]: enabled },
      });
    } catch (e) {
      onError(String(e));
    }
  };

  const handleAddIp = async () => {
    if (!newIp.trim()) return;
    try {
      await api.addManualTarget(newIp.trim(), null);
      setNewIp("");
      await loadTargets();
    } catch (e) {
      onError(String(e));
    }
  };

  const handleToggleSubnet = async (subnetCidr: string, enabled: boolean) => {
    const current = draft.excludedSubnets ?? [];
    const nextExcluded = enabled
      ? current.filter((c) => c !== subnetCidr)
      : current.includes(subnetCidr)
      ? current
      : [...current, subnetCidr];
    await save({ ...draft, excludedSubnets: nextExcluded });
    await loadLanInterfaces();
  };

  const handleOpenLogs = async () => {
    try {
      await api.openLogs();
    } catch (e) {
      onError(String(e));
    }
  };

  const handleCopyDiagnostics = async () => {
    try {
      await api.copyDiagnostics();
      onNotice(t.settings.diagnosticsCopied);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleClearCache = async () => {
    setClearingCache(true);
    try {
      const res = await api.clearCache();
      setCacheSize(res.remainingBytes);
      if (res.skippedFiles > 0 || res.failedFiles > 0) {
        onNotice(
          t.settings.cacheClearedPartial
            .replace("{skipped}", String(res.skippedFiles))
            .replace("{failed}", String(res.failedFiles))
        );
      } else {
        onNotice(t.settings.cacheCleared);
      }
    } catch (e) {
      onError(String(e));
    } finally {
      setClearingCache(false);
    }
  };

  const activeCidrs = new Set(lanInterfaces.map((i) => i.subnetCidr));
  const offlineExcludedSubnets = (draft.excludedSubnets ?? []).filter(
    (cidr) => !activeCidrs.has(cidr)
  );

  return (
    <div className="space-y-5">
      <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
        {t.settings.title}
      </h1>

      {/* Device & Behavior */}
      <GlassCard className="p-5 space-y-4">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-slate-700/70 dark:text-white/55">
              {t.settings.deviceName}
            </label>
            <GlassInput
              value={draft.deviceName}
              onChange={(e) =>
                setDraft({ ...draft, deviceName: e.target.value })
              }
              onBlur={commitDeviceName}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.currentTarget.blur();
                }
              }}
            />
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-slate-700/70 dark:text-white/55">
              {t.settings.downloadDir}
            </label>
            <div className="flex gap-2">
              <GlassInput
                value={isAndroid ? "Downloads/Landa" : draft.downloadDir}
                readOnly
              />
              {!isAndroid && (
                <GlassButton
                  onClick={handlePickDownloadDir}
                  title={t.settings.chooseDir}
                >
                  <FolderOpen className="h-4 w-4" />
                </GlassButton>
              )}
            </div>
          </div>
        </div>

        <div className="divide-y divide-white/15 dark:divide-white/10">
          <GlassToggle
            checked={draft.autoAcceptFromFriends}
            onChange={(v) => save({ ...draft, autoAcceptFromFriends: v })}
            label={t.settings.autoAccept}
          />
          <GlassToggle
            checked={draft.autoCopyClipboard ?? true}
            onChange={(v) => save({ ...draft, autoCopyClipboard: v })}
            label={t.settings.autoCopyClipboard}
          />
          <GlassToggle
            checked={draft.minimizeToTray}
            onChange={(v) => save({ ...draft, minimizeToTray: v })}
            label={t.settings.minimizeToTray}
          />
          <GlassToggle
            checked={draft.leftHandedMode}
            onChange={(v) => save({ ...draft, leftHandedMode: v })}
            label={t.settings.leftHanded}
          />
        </div>

        {/* Theme & Language with Sliding Liquid Glass Segmented Controls */}
        <div className="grid grid-cols-1 gap-4 pt-1 sm:grid-cols-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium text-slate-900/90 dark:text-white/90">
              {t.settings.theme}
            </span>
            <GlassSegmented
              options={[
                { id: "dark", label: t.settings.themeDark },
                { id: "light", label: t.settings.themeLight },
              ]}
              value={draft.theme === "light" ? "light" : "dark"}
              onChange={(v) => save({ ...draft, theme: v as "dark" | "light" })}
            />
          </div>

          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium text-slate-900/90 dark:text-white/90">
              {t.settings.language}
            </span>
            <GlassSegmented
              options={[
                { id: "ru", label: "RU" },
                { id: "en", label: "EN" },
              ]}
              value={draft.language === "en" ? "en" : "ru"}
              onChange={(v) => save({ ...draft, language: v as "ru" | "en" })}
            />
          </div>
        </div>
      </GlassCard>

      {/* Modular Features */}
      <GlassCard className="p-5 space-y-2">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
          {t.settings.modules}
        </h2>
        <div className="divide-y divide-white/15 dark:divide-white/10">
          {modules.map((mod) => {
            const label =
              mod.id === "shares"
                ? t.settings.modShares
                : mod.id === "clipboard"
                ? t.settings.modClipboard
                : mod.id === "videolink"
                ? t.settings.modVideoLink
                : mod.id;
            return (
              <GlassToggle
                key={mod.id}
                checked={mod.enabled}
                onChange={(v) => handleToggleModule(mod.id, v)}
                label={label}
              />
            );
          })}
        </div>
      </GlassCard>

      {/* Manual IP Targets */}
      <GlassCard className="p-5 space-y-3">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
          {t.settings.manualTargets}
        </h2>
        <div className="flex gap-2">
          <GlassInput
            value={newIp}
            onChange={(e) => setNewIp(e.target.value)}
            placeholder={t.settings.manualIpPlaceholder}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleAddIp();
            }}
          />
          <GlassButton variant="primary" onClick={handleAddIp}>
            <Plus className="h-4 w-4" />
          </GlassButton>
        </div>

        {targets.length > 0 && (
          <div className="space-y-2 pt-1">
            {targets.map((target) => (
              <div
                key={target.ip}
                className="flex items-center justify-between rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2"
              >
                <span className="font-mono text-sm text-slate-900/90 dark:text-white/90">
                  {target.ip}
                </span>
                <GlassButton
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    await api.removeManualTarget(target.ip);
                    await loadTargets();
                  }}
                >
                  <Trash2 className="h-4 w-4" />
                </GlassButton>
              </div>
            ))}
          </div>
        )}
      </GlassCard>

      {/* LAN Interfaces / Subnets */}
      <GlassCard className="p-5 space-y-3">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
              {t.settings.lanInterfaces}
            </h2>
            <p className="mt-0.5 text-xs text-slate-700/60 dark:text-white/45">
              {t.settings.lanInterfacesHint}
            </p>
          </div>
          <GlassButton
            size="sm"
            variant="ghost"
            onClick={loadLanInterfaces}
            title={t.settings.refreshInterfaces}
          >
            <RefreshCw className="h-4 w-4" />
          </GlassButton>
        </div>

        {lanInterfaces.length === 0 ? (
          <p className="text-xs text-slate-700/60 dark:text-white/45">
            {t.settings.noLanInterfaces}
          </p>
        ) : (
          <div className="divide-y divide-white/15 dark:divide-white/10">
            {lanInterfaces.map((iface) => (
              <GlassToggle
                key={`${iface.name}-${iface.ip}`}
                checked={iface.enabled}
                onChange={(v) => handleToggleSubnet(iface.subnetCidr, v)}
                label={iface.name}
                description={`${iface.ip} · ${iface.subnetCidr}`}
              />
            ))}
          </div>
        )}

        {offlineExcludedSubnets.length > 0 && (
          <div className="space-y-2 pt-1">
            <div className="text-xs font-medium text-slate-700/65 dark:text-white/50">
              {t.settings.excludedSubnetsLabel}
            </div>
            {offlineExcludedSubnets.map((cidr) => (
              <div
                key={cidr}
                className="flex items-center justify-between rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2"
              >
                <span className="font-mono text-sm text-slate-900/90 dark:text-white/90">
                  {cidr}
                </span>
                <GlassButton
                  size="sm"
                  variant="ghost"
                  onClick={() => handleToggleSubnet(cidr, true)}
                >
                  <Trash2 className="h-4 w-4" />
                </GlassButton>
              </div>
            ))}
          </div>
        )}
      </GlassCard>

      {/* App Cache */}
      <GlassCard className="p-5 space-y-3">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
          {t.settings.cacheTitle}
        </h2>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="text-sm text-slate-900/90 dark:text-white/90">
            {t.settings.cacheSize}:{" "}
            <span className="font-mono font-medium">
              {cacheSize === null ? "—" : formatBytes(cacheSize)}
            </span>
          </div>
          <GlassButton
            onClick={handleClearCache}
            disabled={clearingCache}
          >
            <Trash2 className="h-4 w-4" />
            <span>{t.settings.clearCache}</span>
          </GlassButton>
        </div>
      </GlassCard>

      {/* Diagnostics & Logs */}
      <GlassCard className="p-5 space-y-3">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-700/65 dark:text-white/50">
          {t.settings.diagnostics}
        </h2>
        <div className="flex flex-wrap gap-2.5">
          <GlassButton onClick={handleOpenLogs}>
            <FileText className="h-4 w-4" />
            <span>{t.settings.openLogs}</span>
          </GlassButton>
          <GlassButton variant="primary" onClick={handleCopyDiagnostics}>
            <Copy className="h-4 w-4" />
            <span>{t.settings.copyDiagnostics}</span>
          </GlassButton>
        </div>
      </GlassCard>
    </div>
  );
}
