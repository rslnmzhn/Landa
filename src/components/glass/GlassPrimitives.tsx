import React, { useEffect, useLayoutEffect, useRef, useState } from "react";

export function MeshCanvas() {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 -z-10 overflow-hidden bg-[#eef2ff] dark:bg-[#070913] transition-colors duration-500"
    >
      <div className="animate-float-slow absolute -left-24 -top-24 h-[460px] w-[460px] rounded-full bg-indigo-400/40 dark:bg-indigo-600/35 blur-[110px]" />
      <div className="animate-float-reverse absolute -right-20 top-12 h-[420px] w-[420px] rounded-full bg-sky-300/45 dark:bg-cyan-500/25 blur-[110px]" />
      <div className="animate-float-slow absolute -bottom-32 left-1/3 h-[500px] w-[500px] rounded-full bg-violet-300/45 dark:bg-fuchsia-600/25 blur-[130px]" />
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
    "inline-flex items-center justify-center gap-2 rounded-full font-medium transition-all duration-200 active:scale-[0.96] disabled:opacity-40 disabled:pointer-events-none select-none";
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
      className="flex min-h-[48px] w-full items-center justify-between gap-4 rounded-2xl px-2 py-2 text-left transition hover:bg-white/10 dark:hover:bg-white/5"
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-slate-900/90 dark:text-white/90">
          {label}
        </span>
        {description && (
          <span className="block font-mono text-xs text-slate-700/65 dark:text-white/50">
            {description}
          </span>
        )}
      </span>
      <span
        className={`relative inline-flex h-7 w-12 shrink-0 items-center rounded-full border transition-colors duration-200 ${
          checked
            ? "border-indigo-300/50 bg-indigo-500/80"
            : "border-white/25 bg-white/20 dark:bg-white/10"
        }`}
      >
        <span
          className={`inline-block h-5 w-5 transform rounded-full bg-white shadow-md transition-transform duration-300 ease-[cubic-bezier(0.22,1.28,0.36,1)] ${
            checked ? "translate-x-6" : "translate-x-1"
          }`}
        />
      </span>
    </button>
  );
}

function useSlidingLens(activeKey: string, depsKey: string) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const prevKeyRef = useRef<string>(activeKey);
  const [lens, setLens] = useState<{ left: number; width: number; ready: boolean }>({
    left: 0,
    width: 0,
    ready: false,
  });
  const [stretching, setStretching] = useState(false);

  const measure = () => {
    const el = itemRefs.current[activeKey];
    if (!el) return;
    setLens({
      left: el.offsetLeft,
      width: el.offsetWidth,
      ready: true,
    });
  };

  useLayoutEffect(() => {
    measure();
    if (prevKeyRef.current !== activeKey) {
      prevKeyRef.current = activeKey;
      setStretching(true);
      const timer = window.setTimeout(() => setStretching(false), 240);
      return () => window.clearTimeout(timer);
    }
  }, [activeKey, depsKey]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => measure());
    ro.observe(container);
    return () => ro.disconnect();
  }, [activeKey, depsKey]);

  return { containerRef, itemRefs, lens, stretching };
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
  const depsKey = items.map((i) => `${i.id}:${i.badge ?? 0}`).join("|");
  const { containerRef, itemRefs, lens, stretching } = useSlidingLens(
    activeId,
    depsKey
  );

  return (
    <nav
      ref={containerRef}
      className="liquid-pill relative inline-flex items-center gap-1 rounded-full p-1.5"
    >
      {lens.ready && (
        <div
          aria-hidden="true"
          className={`pointer-events-none absolute bottom-1.5 top-1.5 left-0 rounded-full ${
            stretching ? "liquid-lens-stretching" : ""
          }`}
          style={{
            width: `${lens.width}px`,
            transform: `translate3d(${lens.left}px, 0, 0)`,
            transition:
              "transform 440ms cubic-bezier(0.22, 1.28, 0.36, 1), width 440ms cubic-bezier(0.22, 1.28, 0.36, 1)",
          }}
        >
          <div className="liquid-lens liquid-lens-inner" />
        </div>
      )}

      {items.map((item) => {
        const active = item.id === activeId;
        return (
          <button
            key={item.id}
            ref={(el) => {
              itemRefs.current[item.id] = el;
            }}
            type="button"
            onClick={() => onSelect(item.id)}
            className={`relative z-10 flex min-h-[44px] items-center gap-2 rounded-full px-4 text-sm font-medium transition-all duration-200 active:scale-95 ${
              active
                ? "text-slate-950 dark:text-white"
                : "text-slate-700/75 dark:text-white/60 hover:text-slate-950 dark:hover:text-white/90"
            }`}
          >
            <span
              className={`transition-transform duration-300 ease-[cubic-bezier(0.22,1.28,0.36,1)] ${
                active ? "scale-110" : "scale-100"
              }`}
            >
              {item.icon}
            </span>
            <span className="hidden sm:inline">{item.label}</span>
            {item.badge !== undefined && item.badge > 0 && (
              <span className="flex h-5 min-w-[20px] items-center justify-center rounded-full bg-indigo-500 px-1.5 text-[11px] font-semibold text-white shadow-sm">
                {item.badge}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

export interface SegmentedOption {
  id: string;
  label: string;
}

export function GlassSegmented({
  options,
  value,
  onChange,
  className = "",
}: {
  options: SegmentedOption[];
  value: string;
  onChange: (id: string) => void;
  className?: string;
}) {
  const depsKey = options.map((o) => `${o.id}:${o.label}`).join("|");
  const { containerRef, itemRefs, lens, stretching } = useSlidingLens(
    value,
    depsKey
  );

  return (
    <div
      ref={containerRef}
      className={`relative inline-flex items-center gap-1 rounded-full border border-white/25 dark:border-white/15 bg-white/25 dark:bg-white/[0.05] p-1 backdrop-blur-xl ${className}`}
    >
      {lens.ready && (
        <div
          aria-hidden="true"
          className={`pointer-events-none absolute bottom-1 top-1 left-0 rounded-full ${
            stretching ? "liquid-lens-stretching" : ""
          }`}
          style={{
            width: `${lens.width}px`,
            transform: `translate3d(${lens.left}px, 0, 0)`,
            transition:
              "transform 400ms cubic-bezier(0.22, 1.28, 0.36, 1), width 400ms cubic-bezier(0.22, 1.28, 0.36, 1)",
          }}
        >
          <div className="liquid-lens liquid-lens-inner" />
        </div>
      )}

      {options.map((opt) => {
        const active = opt.id === value;
        return (
          <button
            key={opt.id}
            ref={(el) => {
              itemRefs.current[opt.id] = el;
            }}
            type="button"
            onClick={() => onChange(opt.id)}
            className={`relative z-10 flex min-h-[38px] flex-1 items-center justify-center rounded-full px-3.5 text-xs font-medium transition-colors duration-200 whitespace-nowrap ${
              active
                ? "text-slate-950 dark:text-white"
                : "text-slate-700/75 dark:text-white/60 hover:text-slate-950 dark:hover:text-white/90"
            }`}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
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
        className="animate-glass-modal relative z-10 w-full max-w-md p-6"
      >
        <h2 className="mb-4 text-lg font-semibold text-slate-900/95 dark:text-white/95">
          {title}
        </h2>
        {children}
      </GlassCard>
    </div>
  );
}
