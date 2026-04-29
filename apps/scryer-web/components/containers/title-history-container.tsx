import * as React from "react";
import { useClient } from "urql";
import { titleHistoryQuery } from "@/lib/graphql/queries";
import { retryImportMutation } from "@/lib/graphql/mutations";
import { useActivitySubscription } from "@/lib/hooks/use-activity-subscription";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleHistoryEvent, TitleHistoryPage } from "@/lib/types";
import { WANTED_HISTORY_FILTERS } from "@/components/common/title-history-event-meta";
import { TitleHistoryView } from "@/components/views/title-history-view";

const PAGE_SIZE = 50;
const WANTED_HISTORY_ACTIVITY_KINDS = new Set([
  "acquisition_candidate_accepted",
  "acquisition_download_failed",
  "acquisition_candidate_rejected",
  "movie_downloaded",
  "series_episode_imported",
  "import_rejected",
]);

export function TitleHistoryContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [events, setEvents] = React.useState<TitleHistoryEvent[]>([]);
  const [totalCount, setTotalCount] = React.useState(0);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [activeFilters, setActiveFilters] = React.useState<string[]>([]);
  const [page, setPage] = React.useState(0);
  const [titleFilterInput, setTitleFilterInput] = React.useState("");
  const [titleSearch, setTitleSearch] = React.useState<string | undefined>(undefined);

  const selectedEventTypes = React.useMemo(
    () => (activeFilters.length > 0 ? activeFilters : [...WANTED_HISTORY_FILTERS]),
    [activeFilters],
  );
  const offset = page * PAGE_SIZE;

  const fetchHistory = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await client
        .query<{ titleHistory: TitleHistoryPage }>(titleHistoryQuery, {
          filter: {
            eventTypes: selectedEventTypes,
            titleSearch: titleSearch ?? null,
            groupByEvent: true,
            limit: PAGE_SIZE,
            offset,
          },
        })
        .toPromise();

      if (result.error) {
        throw result.error;
      }

      setEvents(result.data?.titleHistory.records ?? []);
      setTotalCount(result.data?.titleHistory.totalCount ?? 0);
    } catch (fetchError) {
      setError(
        fetchError instanceof Error ? fetchError.message : t("status.failedToLoad"),
      );
      setEvents([]);
      setTotalCount(0);
    } finally {
      setLoading(false);
    }
  }, [client, offset, selectedEventTypes, t, titleSearch]);

  React.useEffect(() => {
    void fetchHistory();
  }, [fetchHistory]);

  React.useEffect(() => {
    const handle = window.setTimeout(() => {
      const normalized = titleFilterInput.trim();
      setPage(0);
      setTitleSearch(normalized.length > 0 ? normalized : undefined);
    }, 250);

    return () => window.clearTimeout(handle);
  }, [titleFilterInput]);

  useActivitySubscription(WANTED_HISTORY_ACTIVITY_KINDS, () => {
    void fetchHistory();
  }, {
    debounceMs: 750,
  });

  const toggleFilter = React.useCallback((eventType: string) => {
    setPage(0);
    setActiveFilters((prev) =>
      prev.includes(eventType)
        ? prev.filter((current) => current !== eventType)
        : [...prev, eventType],
    );
  }, []);

  const clearFilters = React.useCallback(() => {
    setPage(0);
    setActiveFilters([]);
  }, []);

  const handleTitleFilterInputChange = React.useCallback((value: string) => {
    setPage(0);
    setTitleFilterInput(value);
  }, []);

  const handlePreviousPage = React.useCallback(() => {
    setPage((current) => Math.max(0, current - 1));
  }, []);

  const handleNextPage = React.useCallback(() => {
    setPage((current) => current + 1);
  }, []);

  const handleRetry = React.useCallback(
    async (importId: string, password?: string) => {
      try {
        const { error: retryError } = await client
          .mutation(retryImportMutation, {
            input: { importId, password: password || null },
          })
          .toPromise();

        if (retryError) {
          throw retryError;
        }

        setGlobalStatus(t("importHistory.retrySuccess"));
        await fetchHistory();
      } catch (retryError) {
        setGlobalStatus(
          retryError instanceof Error ? retryError.message : t("status.apiError"),
        );
      }
    },
    [client, fetchHistory, setGlobalStatus, t],
  );

  return (
    <TitleHistoryView
      events={events}
      totalCount={totalCount}
      loading={loading}
      error={error}
      activeFilters={activeFilters}
      availableFilters={[...WANTED_HISTORY_FILTERS]}
      titleFilterInput={titleFilterInput}
      currentPage={page}
      pageSize={PAGE_SIZE}
      onTitleFilterInputChange={handleTitleFilterInputChange}
      onToggleFilter={toggleFilter}
      onClearFilters={clearFilters}
      onPreviousPage={handlePreviousPage}
      onNextPage={handleNextPage}
      onRetry={handleRetry}
      hasPreviousPage={page > 0}
      hasNextPage={offset + events.length < totalCount}
    />
  );
}
