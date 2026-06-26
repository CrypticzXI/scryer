import { Eye, EyeOff, Play, Square } from "lucide-react";
import type { ReactNode } from "react";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
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
  value,
  icon,
  label,
  count,
  onClick,
  tone = "neutral",
}: {
  selected: boolean;
  value: string;
  icon?: ReactNode;
  label: string;
  count?: number;
  onClick?: () => void;
  tone?: "neutral" | "success" | "danger" | "muted";
}) {
  return (
    <ToggleGroupItem
      value={value}
      size="sm"
      variant="outline"
      aria-label={label}
      onClick={onClick}
      className={cn(
        "h-10 shrink-0 gap-2 rounded-none border-0 border-b-2 border-transparent bg-transparent px-0 text-[13px] font-semibold tracking-normal text-[var(--scry-muted3)] shadow-none transition-colors hover:bg-transparent hover:text-[var(--scry-ink2)] focus-visible:ring-[var(--scry-focus)] data-[state=on]:border-b-[var(--scry-accent-ring)] data-[state=on]:bg-transparent data-[state=on]:text-[var(--scry-ink2)]",
        selected &&
          "border-b-[var(--scry-accent-ring)] bg-transparent text-[var(--scry-ink2)]",
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
    </ToggleGroupItem>
  );
}

type QuickFilterValue =
  | "all"
  | "monitored"
  | "unmonitored"
  | "continuing"
  | "ended";

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
  const selectedValue: QuickFilterValue = allSelected
    ? "all"
    : filters.monitored
      ? "monitored"
      : filters.unmonitored
        ? "unmonitored"
        : showStatusFilters && filters.continuing
          ? "continuing"
          : showStatusFilters && filters.ended
            ? "ended"
            : "all";
  const handleValueChange = (value: string) => {
    if (!value || value === "all") {
      onClear();
      return;
    }

    if (value === "monitored") {
      onToggleMonitoring("monitored");
      return;
    }
    if (value === "unmonitored") {
      onToggleMonitoring("unmonitored");
      return;
    }
    if (showStatusFilters && value === "continuing") {
      onToggleStatus("continuing");
      return;
    }
    if (showStatusFilters && value === "ended") {
      onToggleStatus("ended");
    }
  };

  return (
    <div className="flex flex-wrap items-end justify-between gap-3">
      <ToggleGroup
        type="single"
        value={selectedValue}
        onValueChange={handleValueChange}
        aria-label={t("label.filters")}
        className="relative top-px flex min-h-10 min-w-0 max-w-full flex-1 flex-wrap items-center justify-start gap-x-5 gap-y-1 border-0 bg-transparent p-0 shadow-none"
      >
        <QuickFilterTab
          selected={allSelected}
          value="all"
          onClick={onClear}
          label={t("activity.historyFilter.all")}
          count={counts?.all}
        />
        <QuickFilterTab
          selected={filters.monitored}
          value="monitored"
          icon={<Eye className="h-3.5 w-3.5" />}
          label={t("title.monitored")}
          count={counts?.monitored}
          tone="success"
        />
        <QuickFilterTab
          selected={filters.unmonitored}
          value="unmonitored"
          icon={<EyeOff className="h-3.5 w-3.5" />}
          label={t("search.monitorType.unmonitored")}
          count={counts?.unmonitored}
          tone="danger"
        />
        {showStatusFilters ? (
          <>
            <QuickFilterTab
              selected={filters.continuing}
              value="continuing"
              icon={<Play className="h-3.5 w-3.5" />}
              label={t("title.continuing")}
              count={counts?.continuing}
              tone="success"
            />
            <QuickFilterTab
              selected={filters.ended}
              value="ended"
              icon={<Square className="h-3.5 w-3.5" />}
              label={t("title.ended")}
              count={counts?.ended}
              tone="muted"
            />
          </>
        ) : null}
      </ToggleGroup>
      {trailingContent ? (
        <div className="w-full shrink-0 sm:w-auto">{trailingContent}</div>
      ) : null}
    </div>
  );
}
