import * as React from "react";
import { Loader2, X } from "lucide-react";
import { cn } from "@/lib/utils";

export function TitleWorkspaceHero({
  backgroundUrl,
  closeLabel,
  onClose,
  children,
}: {
  backgroundUrl?: string | null;
  closeLabel: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <section className="relative mb-3 overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-bg)]">
      {backgroundUrl ? (
        <img
          src={backgroundUrl}
          alt=""
          aria-hidden="true"
          className="absolute inset-0 h-full w-full object-cover opacity-45 saturate-90"
          loading="lazy"
          decoding="async"
        />
      ) : null}
      <div className="absolute inset-0 bg-[linear-gradient(105deg,rgba(8,12,22,0.96)_30%,rgba(8,12,22,0.55)_70%,rgba(8,12,22,0.2))]" />
      <button
        type="button"
        aria-label={closeLabel}
        className="absolute right-2.5 top-2.5 z-10 flex size-8 items-center justify-center rounded-[9px] border border-white/15 bg-slate-950/60 text-[#dde4f5] backdrop-blur-md transition hover:bg-slate-950/75 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
        onClick={onClose}
      >
        <X className="h-4 w-4" />
      </button>
      <div className="relative flex gap-4 p-[18px] pr-14 sm:pr-16">
        {children}
      </div>
    </section>
  );
}

export function TitleWorkspacePosterFrame({
  children,
  label,
}: {
  children: React.ReactNode;
  label: string;
}) {
  return (
    <div className="relative h-[198px] w-[132px] shrink-0 overflow-hidden rounded-[9px] border border-[#2a3556] bg-[var(--scry-inset)] shadow-[0_8px_22px_rgba(0,0,0,0.5)] max-sm:h-44 max-sm:w-[116px]">
      {children}
      <div
        className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_40%,rgba(4,6,12,0.85))]"
        aria-hidden="true"
      />
      <span className="pointer-events-none absolute inset-x-1.5 bottom-2 line-clamp-2 text-[12px] font-bold leading-[1.05] text-white [text-shadow:0_1px_6px_rgba(0,0,0,0.7)]">
        {label}
      </span>
    </div>
  );
}

export function TitleWorkspaceActionGrid({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3 grid grid-cols-7 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-border)] [gap:1px]">
      {children}
    </div>
  );
}

export function TitleWorkspaceActionButton({
  icon: Icon,
  label,
  loading = false,
  destructive = false,
  active = false,
  disabled = false,
  expanded,
  controlsId,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  loading?: boolean;
  destructive?: boolean;
  active?: boolean;
  disabled?: boolean;
  expanded?: boolean;
  controlsId?: string;
  onClick: () => void;
}) {
  const actionDisabled = disabled || loading;

  return (
    <button
      type="button"
      aria-label={label}
      aria-expanded={expanded}
      aria-controls={controlsId}
      className={cn(
        "flex min-h-[88px] min-w-0 flex-col items-center justify-center gap-1.5 bg-[var(--scry-card2)] px-2 py-3 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)] disabled:cursor-not-allowed disabled:opacity-55",
        active &&
          "bg-[rgba(var(--scry-accent-rgb),0.13)] text-[var(--scry-accent-text)] shadow-[inset_0_-2px_0_var(--scry-accent-ring)]",
        destructive && "text-destructive hover:text-destructive",
      )}
      disabled={actionDisabled}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="h-[18px] w-[18px] animate-spin" />
      ) : (
        <Icon className="h-[18px] w-[18px]" />
      )}
      <span className="truncate text-center text-[9.5px] font-bold uppercase tracking-[0.04em]">
        {label}
      </span>
    </button>
  );
}

export function TitleWorkspaceSectionCard({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn(
        "rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4",
        className,
      )}
    >
      {children}
    </section>
  );
}

export function TitleWorkspaceSectionHeader({
  icon: Icon,
  title,
  action,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className="mb-3.5 flex min-w-0 items-center justify-between gap-3">
      <div className="flex min-w-0 items-center gap-2.5">
        <Icon className="h-4 w-4 shrink-0 text-[var(--scry-accent-text)]" />
        <h3 className="truncate text-[14px] font-semibold text-[var(--scry-ink2)]">
          {title}
        </h3>
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  );
}
