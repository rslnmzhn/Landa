import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus, Trash2 } from "lucide-react";
import { api } from "../../core/api";
import type { AppSettings, ManualTarget, ModuleStatus } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassButton,
  GlassCard,
  GlassInput,
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
  const [newIp, setNewIp] = useState("");

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

  useEffect(() => {
    loadTargets();
  }, []);

  const save = async (next: AppSettings) => {
    setDraft(next);
    try {
      const saved = await api.updateSettings(next);
      onSettingsUpdated(saved);
    } catch (e) {
      onError(String(e));
    }
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

  return (
    <div className="space-y-5">
      <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
        {t.settings.title}
      </h1>

      {/* Device & Network */}
      <GlassCard className="p-5 space-y-4">
        <h2 className="text-base font-semibold text-slate-900/90 dark:text-white/90">
          {t.settings.identity}
        </h2>

        <div className="space-y-1.5">
          <label className="text-xs font-medium text-slate-700/75 dark:text-white/60">
            {t.settings.deviceName}
          </label>
          <div className="flex gap-2">
            <GlassInput
              value={draft.deviceName}
              onChange={(e) =>
                setDraft({ ...draft, deviceName: e.target.value })
              }
            />
            <GlassButton
              variant="primary"
              onClick={async () => {
                await save(draft);
                onNotice(t.settings.saved);
              }}
            >
              {t.settings.save}
            </GlassButton>
          </div>
        </div>

        <div className="space-y-1.5">
          <label className="text-xs font-medium text-slate-700/75 dark:text-white/60">
            {t.settings.downloadDir}
          </label>
          <div className="flex gap-2">
            <GlassInput value={draft.downloadDir} readOnly />
            <GlassButton onClick={handlePickDownloadDir}>
              {t.settings.chooseDir}
            </GlassButton>
          </div>
        </div>

        <div className="divide-y divide-white/15 dark:divide-white/10">
          <GlassToggle
            checked={draft.autoAcceptFromFriends}
            onChange={(v) => save({ ...draft, autoAcceptFromFriends: v })}
            label={t.settings.autoAccept}
            description={t.settings.autoAcceptDesc}
          />
          <GlassToggle
            checked={draft.minimizeToTray}
            onChange={(v) => save({ ...draft, minimizeToTray: v })}
            label={t.settings.minimizeToTray}
            description={t.settings.minimizeToTrayDesc}
          />
          <GlassToggle
            checked={draft.leftHandedMode}
            onChange={(v) => save({ ...draft, leftHandedMode: v })}
            label={t.settings.leftHanded}
            description={t.settings.leftHandedDesc}
          />
        </div>

        {/* Theme & Language */}
        <div className="grid grid-cols-1 gap-4 pt-2 sm:grid-cols-2">
          <div className="space-y-2">
            <div className="text-xs font-medium text-slate-700/75 dark:text-white/60">
              {t.settings.theme}
            </div>
            <div className="flex gap-2">
              <GlassButton
                size="sm"
                variant={draft.theme === "dark" ? "primary" : "glass"}
                onClick={() => save({ ...draft, theme: "dark" })}
                className="flex-1"
              >
                {t.settings.themeDark}
              </GlassButton>
              <GlassButton
                size="sm"
                variant={draft.theme === "light" ? "primary" : "glass"}
                onClick={() => save({ ...draft, theme: "light" })}
                className="flex-1"
              >
                {t.settings.themeLight}
              </GlassButton>
            </div>
          </div>

          <div className="space-y-2">
            <div className="text-xs font-medium text-slate-700/75 dark:text-white/60">
              {t.settings.language}
            </div>
            <div className="flex gap-2">
              <GlassButton
                size="sm"
                variant={draft.language === "ru" ? "primary" : "glass"}
                onClick={() => save({ ...draft, language: "ru" })}
                className="flex-1"
              >
                Русский
              </GlassButton>
              <GlassButton
                size="sm"
                variant={draft.language === "en" ? "primary" : "glass"}
                onClick={() => save({ ...draft, language: "en" })}
                className="flex-1"
              >
                English
              </GlassButton>
            </div>
          </div>
        </div>
      </GlassCard>

      {/* Modular Features */}
      <GlassCard className="p-5 space-y-3">
        <div>
          <h2 className="text-base font-semibold text-slate-900/90 dark:text-white/90">
            {t.settings.modules}
          </h2>
          <p className="text-xs text-slate-700/70 dark:text-white/55">
            {t.settings.modulesDesc}
          </p>
        </div>

        <div className="divide-y divide-white/15 dark:divide-white/10">
          {modules.map((mod) => {
            const label =
              mod.id === "shares"
                ? t.settings.modShares
                : mod.id === "clipboard"
                ? t.settings.modClipboard
                : mod.id;
            const desc =
              mod.id === "shares"
                ? t.settings.modSharesDesc
                : mod.id === "clipboard"
                ? t.settings.modClipboardDesc
                : undefined;
            return (
              <GlassToggle
                key={mod.id}
                checked={mod.enabled}
                onChange={(v) => handleToggleModule(mod.id, v)}
                label={label}
                description={desc}
              />
            );
          })}
        </div>
      </GlassCard>

      {/* Manual IP Targets */}
      <GlassCard className="p-5 space-y-3">
        <h2 className="text-base font-semibold text-slate-900/90 dark:text-white/90">
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
            <span>{t.settings.addIp}</span>
          </GlassButton>
        </div>

        {targets.length > 0 && (
          <div className="space-y-2 pt-1">
            {targets.map((target) => (
              <div
                key={target.ip}
                className="flex items-center justify-between rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-2.5"
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
    </div>
  );
}
