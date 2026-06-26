import { Loader2 } from "lucide-react";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { FilterChipButton } from "@/components/common/filter-chip-button";
import { HistoryEventTable } from "@/components/common/history-event-table";
import type { LibraryRecord, TitleHistoryEvent, TitleRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { HistoryEventIcon } from "@/components/common/history-event-icon";
import { getTitleHistoryFilterLabel } from "@/components/common/title-history-event-meta";

export function TitleHistoryView({
  events,
  totalCount,
  loading,
  error,
  activeFilters,
  availableFilters,
  selectedTitle,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  currentPage,
  pageSize,
  onSelectedTitleChange,
  onSelectedLibraryIdsChange,
  onToggleFilter,
  onClearFilters,
  onPreviousPage,
  onNextPage,
  onRetry,
  hasPreviousPage,
  hasNextPage,
}: {
  events: TitleHistoryEvent[];
  totalCount: number;
  loading: boolean;
  error: string | null;
  activeFilters: string[];
  availableFilters: string[];
  selectedTitle: TitleRecord | null;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  currentPage: number;
  pageSize: number;
  onSelectedTitleChange: (title: TitleRecord | null) => void;
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  onToggleFilter: (eventType: string) => void;
  onClearFilters: () => void;
  onPreviousPage: () => void;
  onNextPage: () => void;
  onRetry?: (importId: string, password?: string) => Promise<void>;
  hasPreviousPage: boolean;
  hasNextPage: boolean;
}) {
  const t = useTranslate();
  const pageStart = totalCount === 0 ? 0 : currentPage * pageSize + 1;
  const pageEnd = totalCount === 0 ? 0 : currentPage * pageSize + events.length;

  return (
    <Card className="flex min-h-0 flex-1 flex-col">
      <CardHeader className="space-y-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
          <div className="space-y-1">
            <CardTitle>{`${t("nav.wanted")} ${t("history.title")}`}</CardTitle>
            <p className="text-sm text-muted-foreground">
              {t("pendingImports.pageRange", {
                start: pageStart,
                end: pageEnd,
                total: totalCount,
              })}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!hasPreviousPage || loading}
              onClick={onPreviousPage}
            >
              {t("pendingImports.prev")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!hasNextPage || loading}
              onClick={onNextPage}
            >
              {t("pendingImports.next")}
            </Button>
          </div>
        </div>
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start">
          <TitleAutocompletePicker
            className="w-full lg:flex-1"
            placeholder={t("title.filterPlaceholder")}
            selectedTitle={selectedTitle}
            selectedTitleId={selectedTitle?.id ?? null}
            onSelectedTitleChange={onSelectedTitleChange}
          />
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
            disabled={librariesLoading}
            triggerClassName="w-full lg:w-72 lg:shrink-0"
          />
        </div>
        <div className="flex flex-wrap gap-2">
          <FilterChipButton
            selected={activeFilters.length === 0}
            onClick={onClearFilters}
            className="text-xs"
          >
            {t("history.allEvents")}
          </FilterChipButton>
          {availableFilters.map((eventType) => {
            const isActive = activeFilters.includes(eventType);
            return (
              <FilterChipButton
                key={eventType}
                selected={isActive}
                onClick={() => onToggleFilter(eventType)}
                icon={<HistoryEventIcon eventType={eventType} size={14} />}
              >
                {getTitleHistoryFilterLabel(eventType, t)}
              </FilterChipButton>
            );
          })}
        </div>
      </CardHeader>
      <CardContent className="min-h-0 flex-1">
        {loading && events.length === 0 ? (
          <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : error ? (
          <p className="py-8 text-sm text-rose-300">{error}</p>
        ) : (
          <HistoryEventTable
            events={events}
            showTitle
            showFacet
            showActor
            onRetry={onRetry}
            emptyMessage={t("history.empty")}
          />
        )}
      </CardContent>
    </Card>
  );
}
