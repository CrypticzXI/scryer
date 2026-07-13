import * as React from "react";
import { AlertTriangle, RefreshCw, SlidersHorizontal, X } from "lucide-react";

import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import {
  MultiSelectDropdown,
  type MultiSelectGroup,
} from "@/components/ui/multi-select-dropdown";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LibraryRecord,
  TitleCatalogFilterOptionsRecord,
} from "@/lib/types";
import type { TitleCatalogAdvancedFilters } from "@/lib/utils/title-catalog-query";
import { cn } from "@/lib/utils";

const DEFAULT_MINIMUM_YEAR = 1900;
const FILTER_RANGE_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-transparent accent-[var(--scry-accent)] [&::-moz-range-progress]:h-1.5 [&::-moz-range-progress]:rounded-full [&::-moz-range-progress]:bg-transparent [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)]";
const FILTER_RANGE_THUMB_POINTER_CLASS_NAME =
  "pointer-events-none [&::-moz-range-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:pointer-events-auto";

type CatalogFiltersPanelProps = {
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  filters: TitleCatalogAdvancedFilters;
  options: TitleCatalogFilterOptionsRecord;
  optionsError: boolean;
  onRetryOptions: () => void;
  onFiltersChange: (updates: Partial<TitleCatalogAdvancedFilters>) => void;
  onClear: () => void;
  className?: string;
};

function defaultMaximumYear() {
  return new Date().getFullYear() + 3;
}

function FilterLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
      {children}
    </div>
  );
}

function FilterChips({
  values,
  labels,
  onRemove,
}: {
  values: string[];
  labels: Map<string, string>;
  onRemove: (value: string) => void;
}) {
  if (values.length === 0) return null;
  return (
    <div className="mt-2.5 flex flex-wrap gap-2">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => onRemove(value)}
          className="inline-flex max-w-full items-center gap-2 rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.15)] px-2.5 py-1 text-xs font-semibold text-[var(--scry-accent-text)] transition hover:border-[rgba(var(--scry-accent-rgb),0.48)] hover:bg-[rgba(var(--scry-accent-rgb),0.22)]"
        >
          <span className="truncate">{labels.get(value) ?? value}</span>
          <X className="h-3.5 w-3.5 opacity-75" aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}

export function CatalogFiltersPanel({
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  filters,
  options,
  optionsError,
  onRetryOptions,
  onFiltersChange,
  onClear,
  className,
}: CatalogFiltersPanelProps) {
  const t = useTranslate();
  const eligibleLibraryIds = React.useMemo(() => {
    const explicitLibraryIds = selectedLibraryIds.filter(Boolean);
    return new Set(
        explicitLibraryIds.length > 0
          ? explicitLibraryIds
          : libraries.map((library) => library.id),
    );
  }, [libraries, selectedLibraryIds]);
  const rootGroups = React.useMemo<MultiSelectGroup[]>(
    () =>
      libraries
        .filter((library) => eligibleLibraryIds.has(library.id))
        .map((library) => ({
          label: library.name,
          options: library.roots.map((root) => ({
            value: root.id,
            label: root.path,
            title: root.path,
          })),
        }))
        .filter((group) => group.options.length > 0),
    [eligibleLibraryIds, libraries],
  );
  const rootLabel =
    filters.rootFolderIds.length === 0
      ? t("title.catalogFilters.allRootFolders")
      : filters.rootFolderIds.length === 1
        ? (rootGroups
            .flatMap((group) => group.options)
            .find((option) => option.value === filters.rootFolderIds[0])
            ?.label ?? t("title.rootFolder"))
        : t("title.catalogFilters.selectedCount", {
            count: filters.rootFolderIds.length,
          });
  const genreLabels = React.useMemo(
    () => new Map(options.genres.map((option) => [option.key, option.name])),
    [options.genres],
  );
  const tagLabels = React.useMemo(
    () => new Map(options.tags.map((option) => [option.key, option.name])),
    [options.tags],
  );
  const minimumYearBound = options.minimumYear ?? DEFAULT_MINIMUM_YEAR;
  const maximumYearBound = Math.max(
    options.maximumYear ?? defaultMaximumYear(),
    minimumYearBound,
  );
  const minimumYear = Math.min(
    Math.max(filters.minimumYear ?? minimumYearBound, minimumYearBound),
    maximumYearBound,
  );
  const maximumYear = Math.max(
    Math.min(filters.maximumYear ?? maximumYearBound, maximumYearBound),
    minimumYear,
  );
  const yearSpan = Math.max(1, maximumYearBound - minimumYearBound);
  const minimumYearPercent =
    ((minimumYear - minimumYearBound) / yearSpan) * 100;
  const maximumYearPercent =
    ((maximumYear - minimumYearBound) / yearSpan) * 100;
  const minimumRating = filters.minimumRating ?? 0;
  const hasActiveFilters =
    selectedLibraryIds.length > 0 ||
    filters.rootFolderIds.length > 0 ||
    filters.genreTagKeys.length > 0 ||
    filters.themeTagKeys.length > 0 ||
    filters.minimumYear !== null ||
    filters.maximumYear !== null ||
    minimumRating > 0;

  return (
    <aside
      data-testid="catalog-filters-panel"
      className={cn(
        "relative flex min-h-0 flex-col overflow-y-auto bg-[linear-gradient(180deg,rgba(var(--scry-accent-rgb),0.045),rgba(5,9,18,0.8))] px-[18px] py-4",
        className,
      )}
    >
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-[15px] font-semibold text-[var(--scry-ink2)]">
          <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
          {t("discovery.filters")}
        </div>
        <button
          type="button"
          disabled={!hasActiveFilters}
          onClick={onClear}
          className="text-xs font-medium text-[var(--scry-accent-ring)] transition disabled:cursor-default disabled:opacity-40"
        >
          {t("discovery.clearAll")}
        </button>
      </div>

      {optionsError ? (
        <div
          role="alert"
          className="mb-4 flex items-center gap-2 rounded-[8px] border border-[rgba(255,112,112,0.3)] bg-[rgba(255,80,80,0.08)] px-2.5 py-2 text-[11.5px] text-[var(--scry-danger-text)]"
        >
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            {t("title.catalogFilters.loadError")}
          </span>
          <button
            type="button"
            onClick={onRetryOptions}
            aria-label={t("title.catalogFilters.retry")}
            title={t("title.catalogFilters.retry")}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[6px] text-[var(--scry-danger-text)] transition hover:bg-[rgba(255,255,255,0.08)]"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>
      ) : null}

      <FilterLabel>{t("settings.librariesLabel")}</FilterLabel>
      <div className="mb-4">
        <LibraryMultiSelect
          libraries={libraries}
          selectedLibraryIds={selectedLibraryIds}
          onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
          disabled={librariesLoading || libraries.length === 0}
          triggerClassName="h-9 w-full rounded-[9px] border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[12.5px]"
          contentClassName="max-w-[min(30rem,90vw)]"
        />
      </div>

      <FilterLabel>{t("title.rootFolder")}</FilterLabel>
      <div className="mb-4">
        <MultiSelectDropdown
          groups={rootGroups}
          selectedValues={filters.rootFolderIds}
          onSelectedValuesChange={(rootFolderIds) =>
            onFiltersChange({ rootFolderIds })
          }
          triggerLabel={rootLabel}
          ariaLabel={t("title.rootFolder")}
          disabled={librariesLoading || rootGroups.length === 0}
          size="compact"
          chrome="toolbar"
        />
      </div>

      <FilterLabel>{t("discovery.genres")}</FilterLabel>
      <div className="mb-4">
        <MultiSelectDropdown
          options={options.genres.map((option) => ({
            value: option.key,
            label: option.name,
          }))}
          selectedValues={filters.genreTagKeys}
          onSelectedValuesChange={(genreTagKeys) =>
            onFiltersChange({ genreTagKeys })
          }
          triggerLabel={
            filters.genreTagKeys.length === 0
              ? t("discovery.selectGenres")
              : filters.genreTagKeys.length === 1
                ? (genreLabels.get(filters.genreTagKeys[0]) ??
                  filters.genreTagKeys[0])
                : t("title.catalogFilters.selectedCount", {
                    count: filters.genreTagKeys.length,
                  })
          }
          ariaLabel={t("discovery.genres")}
          size="compact"
          chrome="toolbar"
        />
        <FilterChips
          values={filters.genreTagKeys}
          labels={genreLabels}
          onRemove={(key) =>
            onFiltersChange({
              genreTagKeys: filters.genreTagKeys.filter(
                (candidate) => candidate !== key,
              ),
            })
          }
        />
      </div>

      <FilterLabel>{t("discovery.tags")}</FilterLabel>
      <div className="mb-4">
        <MultiSelectDropdown
          options={options.tags.map((option) => ({
            value: option.key,
            label: option.name,
          }))}
          selectedValues={filters.themeTagKeys}
          onSelectedValuesChange={(themeTagKeys) =>
            onFiltersChange({ themeTagKeys })
          }
          triggerLabel={
            filters.themeTagKeys.length === 0
              ? t("discovery.selectTags")
              : filters.themeTagKeys.length === 1
                ? (tagLabels.get(filters.themeTagKeys[0]) ??
                  filters.themeTagKeys[0])
                : t("title.catalogFilters.selectedCount", {
                    count: filters.themeTagKeys.length,
                  })
          }
          ariaLabel={t("discovery.tags")}
          size="compact"
          chrome="toolbar"
        />
        <FilterChips
          values={filters.themeTagKeys}
          labels={tagLabels}
          onRemove={(key) =>
            onFiltersChange({
              themeTagKeys: filters.themeTagKeys.filter(
                (candidate) => candidate !== key,
              ),
            })
          }
        />
      </div>

      <div className="mb-2.5 flex items-center justify-between">
        <FilterLabel>{t("discovery.releaseYear")}</FilterLabel>
        <span className="mb-2.5 text-[11.5px] text-[var(--scry-faint)]">
          {minimumYear} - {maximumYear}
        </span>
      </div>
      <div className="relative mb-5 h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[#16203a]" />
        <div
          className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{
            left: `${minimumYearPercent}%`,
            right: `${100 - maximumYearPercent}%`,
          }}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={minimumYear}
          aria-label={t("title.catalogFilters.minimumYear")}
          onChange={(event) => {
            const value = Math.min(Number(event.target.value), maximumYear);
            onFiltersChange({
              minimumYear: value === minimumYearBound ? null : value,
            });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={maximumYear}
          aria-label={t("title.catalogFilters.maximumYear")}
          onChange={(event) => {
            const value = Math.max(Number(event.target.value), minimumYear);
            onFiltersChange({
              maximumYear: value === maximumYearBound ? null : value,
            });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
          )}
        />
      </div>

      <div className="mb-2.5 flex items-center justify-between">
        <FilterLabel>{t("discovery.minimumRating")}</FilterLabel>
        <span className="mb-2.5 text-[11.5px] font-bold text-[var(--scry-accent-ring)]">
          {minimumRating.toFixed(1)}+
        </span>
      </div>
      <div className="relative h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[#16203a]" />
        <div
          className="absolute left-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{ width: `${minimumRating * 10}%` }}
        />
        <input
          type="range"
          min={0}
          max={10}
          step={0.5}
          value={minimumRating}
          aria-label={t("discovery.minimumRating")}
          onChange={(event) => {
            const value = Number(event.target.value);
            onFiltersChange({ minimumRating: value > 0 ? value : null });
          }}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
      </div>
    </aside>
  );
}
