import { lazy, memo, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";
import { Card, CardContent } from "@/components/ui/card";
import { calendarEpisodesQuery, librariesQuery } from "@/lib/graphql/queries";
import { FACETS_BY_ID } from "@/lib/facets/registry";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { CalendarEpisodeItem } from "@/components/views/calendar-view";
import type { LibraryRecord } from "@/lib/types";

const ALL_LIBRARIES_VALUE = "__all__";

const CalendarView = lazy(() =>
  import("@/components/views/calendar-view").then((m) => ({ default: m.CalendarView })),
);

type CalendarContainerProps = {
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

export const CalendarContainer = memo(function CalendarContainer({
  onOpenOverview,
}: CalendarContainerProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [calendarEpisodes, setCalendarEpisodes] = useState<CalendarEpisodeItem[]>([]);
  const [calendarLoading, setCalendarLoading] = useState(false);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = useState(false);
  const [selectedLibraryId, setSelectedLibraryId] = useState(ALL_LIBRARIES_VALUE);
  const lastCalendarRangeRef = useRef<{ start: string; end: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLibrariesLoading(true);
    void client
      .query(
        librariesQuery,
        { facet: null, permission: "view" },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          throw error;
        }
        const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
        setLibraries(nextLibraries);
      })
      .catch((error) => {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : t("status.failedToLoad");
          setGlobalStatus(message);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLibrariesLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  const refreshCalendar = useCallback(
    async (start: string, end: string) => {
      lastCalendarRangeRef.current = { start, end };
      setCalendarLoading(true);
      try {
        const { data, error } = await client
          .query(calendarEpisodesQuery, {
            startDate: start,
            endDate: end,
            libraryId:
              selectedLibraryId === ALL_LIBRARIES_VALUE ? null : selectedLibraryId,
          })
          .toPromise();
        if (error) throw error;
        setCalendarEpisodes(data?.calendarEpisodes ?? []);
      } catch (error) {
        const message = error instanceof Error ? error.message : t("status.failedToLoad");
        setGlobalStatus(message);
      } finally {
        setCalendarLoading(false);
      }
    },
    [client, selectedLibraryId, setGlobalStatus, t],
  );

  useEffect(() => {
    const range = lastCalendarRangeRef.current;
    if (range) {
      void refreshCalendar(range.start, range.end);
    }
  }, [refreshCalendar, selectedLibraryId]);

  const handleCalendarEpisodeClick = useCallback(
    (episode: CalendarEpisodeItem) => {
      const facet = FACETS_BY_ID.get(episode.titleFacet as import("@/lib/types/titles").Facet);
      if (!facet || !onOpenOverview) return;
      onOpenOverview(
        facet.viewId as ViewId,
        {
          id: episode.titleId,
          slug: episode.titleSlug ?? null,
          libraryId: episode.libraryId,
          librarySlug: episode.librarySlug ?? null,
        },
        episode.id,
      );
    },
    [onOpenOverview],
  );

  if (isMobile) {
    return null;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Suspense
        fallback={
          <Card>
            <CardContent className="p-8 text-center text-muted-foreground">
              {t("label.loading")}
            </CardContent>
          </Card>
        }
      >
        <CalendarView
          episodes={calendarEpisodes}
          loading={calendarLoading}
          libraries={libraries}
          librariesLoading={librariesLoading}
          selectedLibraryId={selectedLibraryId}
          allLibrariesValue={ALL_LIBRARIES_VALUE}
          onSelectedLibraryChange={setSelectedLibraryId}
          onDateRangeChange={refreshCalendar}
          onEpisodeClick={handleCalendarEpisodeClick}
        />
      </Suspense>
    </div>
  );
});
