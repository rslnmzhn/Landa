import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Copy,
  Eye,
  EyeOff,
  Film,
  Link2,
  Play,
  Square,
} from "lucide-react";
import { api, formatBytes } from "../../core/api";
import type { VideoLinkStatus } from "../../core/types";
import type { TranslationDict } from "../../i18n/translations";
import {
  GlassBadge,
  GlassButton,
  GlassCard,
  GlassInput,
} from "../../components/glass/GlassPrimitives";

interface VideoLinkViewProps {
  t: TranslationDict;
  enabled: boolean;
  onError: (msg: string) => void;
  onNotice: (msg: string) => void;
}

function pathBasename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function VideoLinkView({
  t,
  enabled,
  onError,
  onNotice,
}: VideoLinkViewProps) {
  const [selectedPath, setSelectedPath] = useState<string>("");
  const [password, setPassword] = useState<string>("");
  const [showPassword, setShowPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<VideoLinkStatus>({
    running: false,
    url: null,
    port: null,
    fileName: null,
    sizeBytes: null,
  });

  useEffect(() => {
    if (!enabled) return;
    api
      .videolinkStatus()
      .then(setStatus)
      .catch(() => {});
  }, [enabled]);

  const handlePickVideo = async () => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [
          {
            name: "Video",
            extensions: ["mp4", "mkv", "webm", "mov", "avi", "m4v", "ts", "ogv"],
          },
        ],
      });
      if (!selected || Array.isArray(selected)) return;
      setSelectedPath(selected);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleStart = async () => {
    if (!selectedPath || !password.trim()) return;
    setBusy(true);
    try {
      const next = await api.videolinkStart(selectedPath, password);
      setStatus(next);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleStop = async () => {
    setBusy(true);
    try {
      const next = await api.videolinkStop();
      setStatus(next);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleCopyLink = async () => {
    if (!status.url) return;
    try {
      await navigator.clipboard.writeText(status.url);
      onNotice(t.videolink.copied);
    } catch {
      try {
        await api.clipboardCopyToSystem("", null, status.url);
        onNotice(t.videolink.copied);
      } catch (e) {
        onError(String(e));
      }
    }
  };

  if (!enabled) {
    return (
      <GlassCard className="p-8 text-center text-sm text-slate-700/75 dark:text-white/60">
        {t.videolink.disabledNotice}
      </GlassCard>
    );
  }

  const displayFileName =
    status.running && status.fileName
      ? status.fileName
      : selectedPath
        ? pathBasename(selectedPath)
        : t.videolink.noFile;

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-slate-900/95 dark:text-white/95">
            {t.videolink.title}
          </h1>
          <p className="mt-0.5 text-xs text-slate-700/65 dark:text-white/50">
            {t.videolink.subtitle}
          </p>
        </div>
        <GlassBadge tone={status.running ? "success" : "neutral"}>
          <span
            className={`h-2 w-2 rounded-full ${
              status.running ? "bg-emerald-400" : "bg-slate-400"
            }`}
          />
          <span>{status.running ? t.videolink.running : t.videolink.stopped}</span>
        </GlassBadge>
      </div>

      {/* Controls */}
      <GlassCard className="p-5 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl bg-white/20 dark:bg-white/5 px-4 py-3">
          <div className="flex items-center gap-3 min-w-0 flex-1">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-indigo-500/15 border border-indigo-400/25">
              <Film className="h-5 w-5 text-indigo-600 dark:text-indigo-300" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
                {displayFileName}
              </div>
              {status.running && status.sizeBytes !== null && (
                <div className="font-mono text-xs text-slate-700/65 dark:text-white/45">
                  {formatBytes(status.sizeBytes)}
                </div>
              )}
            </div>
          </div>
          <GlassButton
            size="sm"
            variant="glass"
            disabled={busy || status.running}
            onClick={handlePickVideo}
          >
            <span>{t.videolink.selectFile}</span>
          </GlassButton>
        </div>

        <div className="space-y-1.5">
          <label className="text-xs font-medium text-slate-700/70 dark:text-white/55">
            {t.videolink.passwordLabel}
          </label>
          <div className="flex gap-2">
            <GlassInput
              type={showPassword ? "text" : "password"}
              value={password}
              disabled={busy || status.running}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t.videolink.passwordPlaceholder}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !status.running) {
                  void handleStart();
                }
              }}
            />
            <GlassButton
              type="button"
              variant="ghost"
              onClick={() => setShowPassword((v) => !v)}
              aria-label={t.videolink.passwordLabel}
            >
              {showPassword ? (
                <EyeOff className="h-4 w-4" />
              ) : (
                <Eye className="h-4 w-4" />
              )}
            </GlassButton>
          </div>
        </div>

        <div className="flex justify-end">
          {status.running ? (
            <GlassButton
              variant="danger"
              disabled={busy}
              onClick={handleStop}
            >
              <Square className="h-4 w-4" />
              <span>{t.videolink.stop}</span>
            </GlassButton>
          ) : (
            <GlassButton
              variant="primary"
              disabled={busy || !selectedPath || !password.trim()}
              onClick={handleStart}
            >
              <Play className="h-4 w-4" />
              <span>{t.videolink.start}</span>
            </GlassButton>
          )}
        </div>
      </GlassCard>

      {/* Active Link Card */}
      {status.running && status.url && (
        <GlassCard elevated className="p-5 space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2.5 min-w-0">
              <Link2 className="h-4 w-4 shrink-0 text-indigo-500 dark:text-indigo-300" />
              <span className="truncate font-mono text-sm font-medium text-slate-900/95 dark:text-white/95">
                {status.url}
              </span>
            </div>
            <GlassButton size="sm" variant="primary" onClick={handleCopyLink}>
              <Copy className="h-4 w-4" />
              <span>{t.videolink.copyLink}</span>
            </GlassButton>
          </div>
        </GlassCard>
      )}
    </div>
  );
}
