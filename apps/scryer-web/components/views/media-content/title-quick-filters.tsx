import { Eye, EyeOff, Play, Square } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";
import { cn } from "@/lib/utils";

export type TitleQuickFilters = {
  monitored: boolean;
  unmonitored: boolean;
  continuing: boolean;
  ended: boolean;
};

export type TitleQuickFilterCounts = {
  all: number;
  monitored: number;
  unmonitored: number;
  continuing: number;
  ended: number;
};

function normalizeQuickFilterStatus(
  status: string | null | undefined,
): "continuing" | "ended" | null {
  const normalized = status?.trim().toLowerCase();
  switch (normalized) {
    case "continuing":
    case "returning":
      return "continuing";
    case "ended":
    case "finished":
      return "ended";
    default:
      return null;
  }
}

export function hasActiveTitleQuickFilters(
  filters: TitleQuickFilters,
  view: "movies" | "series" | "anime",
) {
  return (
    filters.monitored ||
    filters.unmonitored ||
    (view !== "movies" && (filters.continuing || filters.ended))
  );
}

export function getTitleQuickFilterCounts(
  titles: TitleRecord[],
): TitleQuickFilterCounts {
  return titles.reduce<TitleQuickFilterCounts>(
    (counts, title) => {
      counts.all += 1;
      if (title.monitored) {
        counts.monitored += 1;
      } else {
        counts.unmonitored += 1;
      }

      const status = normalizeQuickFilterStatus(title.contentStatus);
      if (status === "continuing") {
        counts.continuing += 1;
      } else if (status === "ended") {
        counts.ended += 1;
      }

      return counts;
    },
    {
      all: 0,
      monitored: 0,
      unmonitored: 0,
      continuing: 0,
      ended: 0,
    },
  );
}

export function filterTitlesByQuickFilters(
  titles: TitleRecord[],
  filters: TitleQuickFilters,
): TitleRecord[] {
  return titles.filter((title) => {
    const monitoringFiltersActive = filters.monitored || filters.unmonitored;
    if (monitoringFiltersActive) {
      const matchesMonitoring =
        (filters.monitored && title.monitored) ||
        (filters.unmonitored && !title.monitored);
      if (!matchesMonitoring) {
        return false;
      }
    }

    const statusFiltersActive = filters.continuing || filters.ended;
    if (statusFiltersActive) {
      const normalizedStatus = normalizeQuickFilterStatus(title.contentStatus);
      const matchesStatus =
        (filters.continuing && normalizedStatus === "continuing") ||
        (filters.ended && normalizedStatus === "ended");
      if (!matchesStatus) {
        return false;
      }
    }

    return true;
  });
}

function QuickFilterTab({
  selected,
  onClick,
  icon,
  label,
  count,
  tone = "neutral",
}: {
  selected: boolean;
  onClick: () => void;
  icon?: ReactNode;
  label: string;
  count?: number;
  tone?: "neutral" | "success" | "danger" | "muted";
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        "inline-flex h-[38px] shrink-0 items-center gap-2 border-b-2 px-0.5 text-[13px] font-semibold tracking-normal transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]",
        selected
          ? "border-[var(--scry-accent-ring)] text-[var(--scry-ink2)]"
          : "border-transparent text-[var(--scry-muted3)] hover:text-[var(--scry-ink2)]",
      )}
    >
      {icon ? (
        <span
          className={cn(
            "shrink-0",
            selected
              ? "text-[var(--scry-accent-text)]"
              : tone === "success"
                ? "text-emerald-500"
                : tone === "danger"
                  ? "text-rose-500"
                  : tone === "muted"
                    ? "text-zinc-400"
                    : "text-[var(--scry-muted2)]",
          )}
        >
          {icon}
        </span>
      ) : null}
      <span className="whitespace-nowrap">{label}</span>
      {typeof count === "number" ? (
        <span
          className={cn(
            "rounded-[6px] px-1.5 py-0.5 text-[11px] font-bold leading-none tabular-nums",
            selected
              ? "bg-[rgba(var(--scry-accent-rgb),0.18)] text-[var(--scry-accent-text)]"
              : "bg-[var(--scry-chip)] text-[var(--scry-muted2)]",
          )}
        >
          {count.toLocaleString()}
        </span>
      ) : null}
    </button>
  );
}

export function TitleQuickFilterBar({
  view,
  filters,
  counts,
  onToggleMonitoring,
  onToggleStatus,
  onClear,
  trailingContent,
}: {
  view: "movies" | "series" | "anime";
  filters: TitleQuickFilters;
  counts?: TitleQuickFilterCounts;
  onToggleMonitoring: (filter: "monitored" | "unmonitored") => void;
  onToggleStatus: (filter: "continuing" | "ended") => void;
  onClear: () => void;
  trailingContent?: ReactNode;
}) {
  const t = useTranslate();
  const showStatusFilters = view !== "movies";
  const allSelected = !hasActiveTitleQuickFilters(filters, view);

  return (
    <div className="flex flex-wrap items-end justify-between gap-3">
      <div className="flex min-h-12 min-w-0 max-w-full flex-1 items-end gap-5 overflow-x-auto">
        <QuickFilterTab
          selected={allSelected}
          onClick={onClear}
          label={t("activity.historyFilter.all")}
          count={counts?.all}
        />
        <QuickFilterTab
          selected={filters.monitored}
          onClick={() => onToggleMonitoring("monitored")}
          icon={<Eye className="h-3.5 w-3.5" />}
          label={t("title.monitored")}
          count={counts?.monitored}
          tone="success"
        />
        <QuickFilterTab
          selected={filters.unmonitored}
          onClick={() => onToggleMonitoring("unmonitored")}
          icon={<EyeOff className="h-3.5 w-3.5" />}
          label={t("search.monitorType.unmonitored")}
          count={counts?.unmonitored}
          tone="danger"
        />
        {showStatusFilters ? (
          <>
            <QuickFilterTab
              selected={filters.continuing}
              onClick={() => onToggleStatus("continuing")}
              icon={<Play className="h-3.5 w-3.5" />}
              label={t("title.continuing")}
              count={counts?.continuing}
              tone="success"
            />
            <QuickFilterTab
              selected={filters.ended}
              onClick={() => onToggleStatus("ended")}
              icon={<Square className="h-3.5 w-3.5" />}
              label={t("title.ended")}
              count={counts?.ended}
              tone="muted"
            />
          </>
        ) : null}
      </div>
      {trailingContent ? (
        <div className="w-full shrink-0 sm:w-auto">{trailingContent}</div>
      ) : null}
    </div>
  );
}
