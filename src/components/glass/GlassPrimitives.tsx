import React from "react";

export function MeshCanvas() {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 -z-10 overflow-hidden bg-[#eef2ff] dark:bg-[#070913] transition-colors duration-500"
    >
      {/* Top-left luminous orb */}
      <div className="animate-float-slow absolute -left-24 -top-24 h-[460px] w-[460px] rounded-full bg-indigo-400/40 dark:bg-indigo-600/35 blur-[110px]" />
      {/* Top-right cyan/teal refraction orb */}
      <div className="animate-float-reverse absolute -right-20 top-12 h-[420px] w-[420px] rounded-full bg-sky-300/45 dark:bg-cyan-500/25 blur-[110px]" />
      {/* Bottom-center violet/magenta depth orb */}
      <div className="animate-float-slow absolute -bottom-32 left-1/3 h-[500px] w-[500px] rounded-full bg-violet-300/45 dark:bg-fuchsia-600/25 blur-[130px]" />
      {/* Subtle radial vignette */}
      <div className="absolute inset-0 bg-gradient-to-b from-white/20 via-transparent to-black/5 dark:from-white/[0.02] dark:via-transparent dark:to-black/40" />
    </div>
  );
}

interface GlassCardProps extends React.HTMLAttributes<HTMLDivElement> {
  elevated?: boolean;
  children: React.ReactNode;
}

export function GlassCard({
  elevated = false,
  className = "",
  children,
  ...rest
}: GlassCardProps) {
  return (
    <div
      className={`${
        elevated ? "liquid-glass-elevated" : "liquid-glass"
      } rounded-3xl transition-all duration-300 ${className}`}
      {...rest}
    >
      {children}
    </div>
  );
}

interface GlassButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "glass" | "danger" | "ghost";
  size?: "md" | "sm";
  children: React.ReactNode;
}

export function GlassButton({
  variant = "glass",
  size = "md",
  className = "",
  children,
  ...rest
}: GlassButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-2 rounded-full font-medium transition-all duration-200 active:scale-[0.97] disabled:opacity-40 disabled:pointer-events-none select-none";
  const sizing =
    size === "sm"
      ? "min-h-[44px] px-4 text-sm"
      : "min-h-[44px] px-5 text-sm";

  const variants: Record<NonNullable<GlassButtonProps["variant"]>, string> = {
    primary:
      "bg-gradient-to-b from-indigo-400/90 to-indigo-600/90 text-white border border-white/30 shadow-[inset_0_1px_1px_rgba(255,255,255,0.45),0_10px_25px_rgba(79,70,229,0.35)] hover:brightness-110",
    glass:
      "liquid-glass text-slate-900/90 dark:text-white/90 hover:bg-white/25 dark:hover:bg-white/15",
    danger:
      "bg-rose-500/25 text-rose-900 dark:text-rose-100 border border-rose-400/35 shadow-[inset_0_1px_1px_rgba(255,255,255,0.25)] hover:bg-rose-500/35",
    ghost:
      "text-slate-700/80 dark:text-white/70 hover:bg-white/20 dark:hover:bg-white/10",
  };

  return (
    <button
      className={`${base} ${sizing} ${variants[variant]} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

export function GlassBadge({
  tone = "neutral",
  children,
}: {
  tone?: "neutral" | "success" | "accent" | "warning";
  children: React.ReactNode;
}) {
  const tones = {
    neutral:
      "bg-white/30 dark:bg-white/10 text-slate-800/80 dark:text-white/75 border-white/30 dark:border-white/15",
    success:
      "bg-emerald-400/20 text-emerald-900 dark:text-emerald-200 border-emerald-400/35",
    accent:
      "bg-indigo-400/25 text-indigo-950 dark:text-indigo-200 border-indigo-400/35",
    warning:
      "bg-amber-400/20 text-amber-950 dark:text-amber-200 border-amber-400/35",
  };
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium backdrop-blur-md ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

export function GlassInput(
  props: React.InputHTMLAttributes<HTMLInputElement>
) {
  const { className = "", ...rest } = props;
  return (
    <input
      className={`min-h-[44px] w-full rounded-2xl border border-white/30 dark:border-white/15 bg-white/35 dark:bg-white/[0.06] px-4 text-sm text-slate-900/90 dark:text-white/90 placeholder:text-slate-500/70 dark:placeholder:text-white/40 backdrop-blur-xl focus:outline-none focus:ring-2 focus:ring-indigo-400/50 ${className}`}
      {...rest}
    />
  );
}

export function GlassToggle({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex min-h-[44px] w-full items-center justify-between gap-4 rounded-2xl px-2 py-2 text-left transition hover:bg-white/10 dark:hover:bg-white/5"
    >
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-slate-900/90 dark:text-white/90">
          {label}
        </div>
        {description && (
          <div className="text-xs text-slate-700/70 dark:text-white/55">
            {description}
          </div>
        )}
      </div>
      <span
        className={`relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition-colors duration-200 ${
          checked
            ? "border-indigo-300/50 bg-indigo-500/80"
            : "border-white/25 bg-white/20 dark:bg-white/10"
        }`}
      >
        <span
          className={`inline-block h-5 w-5 transform rounded-full bg-white shadow-md transition-transform duration-200 ${
            checked ? "translate-x-6" : "translate-x-1"
          }`}
        />
      </span>
    </button>
  );
}

export interface NavTabItem {
  id: string;
  label: string;
  icon: React.ReactNode;
  badge?: number;
}

export function GlassPillNav({
  items,
  activeId,
  onSelect,
}: {
  items: NavTabItem[];
  activeId: string;
  onSelect: (id: string) => void;
}) {
  return (
    <nav className="liquid-pill inline-flex items-center gap-1 rounded-full p-1.5">
      {items.map((item) => {
        const active = item.id === activeId;
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onSelect(item.id)}
            className={`relative flex min-h-[44px] items-center gap-2 rounded-full px-4 text-sm font-medium transition-all duration-200 ${
              active
                ? "bg-white/65 dark:bg-white/20 text-slate-950 dark:text-white shadow-[inset_0_1px_1px_rgba(255,255,255,0.5)]"
                : "text-slate-700/75 dark:text-white/60 hover:text-slate-950 dark:hover:text-white/90"
            }`}
          >
            {item.icon}
            <span className="hidden sm:inline">{item.label}</span>
            {item.badge !== undefined && item.badge > 0 && (
              <span className="flex h-5 min-w-[20px] items-center justify-center rounded-full bg-indigo-500 px-1.5 text-[11px] font-semibold text-white">
                {item.badge}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

export function GlassModal({
  open,
  onClose,
  title,
  children,
}: {
  open: boolean;
  onClose?: () => void;
  title: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4 backdrop-blur-md">
      <div
        className="fixed inset-0"
        onClick={onClose}
        aria-hidden="true"
      />
      <GlassCard
        elevated
        className="relative z-10 w-full max-w-md p-6"
      >
        <h2 className="mb-4 text-lg font-semibold text-slate-900/95 dark:text-white/95">
          {title}
        </h2>
        {children}
      </GlassCard>
    </div>
  );
}
