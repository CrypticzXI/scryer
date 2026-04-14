import { lazy, memo, Suspense, useCallback, useState } from "react";
import { useClient } from "urql";
import { Card, CardContent } from "@/components/ui/card";
import { calendarEpisodesQuery } from "@/lib/graphql/queries";
import { FACETS_BY_ID } from "@/lib/facets/registry";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { CalendarEpisodeItem } from "@/components/views/calendar-view";

const CalendarView = lazy(() =>
  import("@/components/views/calendar-view").then((m) => ({ default: m.CalendarView })),
);

type CalendarContainerProps = {
  onOpenOverview?: (targetView: ViewId, titleId: string, episodeId?: string) => void;
};

export const CalendarContainer = memo(function CalendarContainer({
  onOpenOverview,
}: CalendarContainerProps) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [calendarEpisodes, setCalendarEpisodes] = useState<CalendarEpisodeItem[]>([]);
  const [calendarLoading, setCalendarLoading] = useState(false);

  const refreshCalendar = useCallback(
    async (start: string, end: string) => {
      setCalendarLoading(true);
      try {
        const { data, error } = await client
          .query(calendarEpisodesQuery, { startDate: start, endDate: end })
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
    [client, setGlobalStatus, t],
  );

  const handleCalendarEpisodeClick = useCallback(
    (episode: CalendarEpisodeItem) => {
      const facet = FACETS_BY_ID.get(episode.titleFacet as import("@/lib/types/titles").Facet);
      if (!facet || !onOpenOverview) return;
      onOpenOverview(facet.viewId as ViewId, episode.titleId, episode.id);
    },
    [onOpenOverview],
  );

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
          onDateRangeChange={refreshCalendar}
          onEpisodeClick={handleCalendarEpisodeClick}
        />
      </Suspense>
    </div>
  );
});
