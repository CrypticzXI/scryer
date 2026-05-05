import { Eye, EyeOff, Play, Square } from "lucide-react";
import { FilterChipButton } from "@/components/common/filter-chip-button";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";

export type TitleQuickFilters = {
  monitored: boolean;
  unmonitored: boolean;
  continuing: boolean;
  ended: boolean;
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
    filters.monitored
    || filters.unmonitored
    || (view !== "movies" && (filters.continuing || filters.ended))
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
        (filters.monitored && title.monitored)
        || (filters.unmonitored && !title.monitored);
      if (!matchesMonitoring) {
        return false;
      }
    }

    const statusFiltersActive = filters.continuing || filters.ended;
    if (statusFiltersActive) {
      const normalizedStatus = normalizeQuickFilterStatus(title.contentStatus);
      const matchesStatus =
        (filters.continuing && normalizedStatus === "continuing")
        || (filters.ended && normalizedStatus === "ended");
      if (!matchesStatus) {
        return false;
      }
    }

    return true;
  });
}

export function TitleQuickFilterBar({
  view,
  filters,
  onToggleMonitoring,
  onToggleStatus,
  onClear,
}: {
  view: "movies" | "series" | "anime";
  filters: TitleQuickFilters;
  onToggleMonitoring: (filter: "monitored" | "unmonitored") => void;
  onToggleStatus: (filter: "continuing" | "ended") => void;
  onClear: () => void;
}) {
  const t = useTranslate();
  const showStatusFilters = view !== "movies";
  const allSelected = !hasActiveTitleQuickFilters(filters, view);

  return (
    <div className="flex flex-wrap gap-2">
      <FilterChipButton selected={allSelected} onClick={onClear} className="text-xs">
        {t("activity.historyFilter.all")}
      </FilterChipButton>
      <FilterChipButton
        selected={filters.monitored}
        onClick={() => onToggleMonitoring("monitored")}
        icon={<Eye className="h-3.5 w-3.5 shrink-0 text-emerald-500" />}
      >
        {t("title.monitored")}
      </FilterChipButton>
      <FilterChipButton
        selected={filters.unmonitored}
        onClick={() => onToggleMonitoring("unmonitored")}
        icon={<EyeOff className="h-3.5 w-3.5 shrink-0 text-rose-500" />}
      >
        {t("search.monitorType.unmonitored")}
      </FilterChipButton>
      {showStatusFilters ? (
        <>
          <FilterChipButton
            selected={filters.continuing}
            onClick={() => onToggleStatus("continuing")}
            icon={<Play className="h-3.5 w-3.5 shrink-0 text-emerald-500" />}
          >
            {t("title.continuing")}
          </FilterChipButton>
          <FilterChipButton
            selected={filters.ended}
            onClick={() => onToggleStatus("ended")}
            icon={<Square className="h-3.5 w-3.5 shrink-0 text-zinc-400" />}
          >
            {t("title.ended")}
          </FilterChipButton>
        </>
      ) : null}
    </div>
  );
}
