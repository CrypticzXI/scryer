import { useMemo, useCallback, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import FullCalendar from "@fullcalendar/react";
import dayGridPlugin from "@fullcalendar/daygrid";
import type {
  DatesSetArg,
  DayCellContentArg,
  DayHeaderContentArg,
  EventClickArg,
  EventContentArg,
  EventMountArg,
  MoreLinkContentArg,
} from "@fullcalendar/core";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { CalendarClock } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LibraryRecord } from "@/lib/types";

export type CalendarEpisodeItem = {
  id: string;
  titleId: string;
  libraryId: string;
  libraryName?: string | null;
  librarySlug?: string | null;
  titleName: string;
  titleSlug?: string | null;
  titleFacet: string;
  seasonNumber: string | null;
  episodeNumber: string | null;
  episodeTitle: string | null;
  airDate: string | null;
  monitored: boolean;
};

type CalendarViewProps = {
  episodes: CalendarEpisodeItem[];
  loading: boolean;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (value: string[]) => void;
  onDateRangeChange: (start: string, end: string) => void;
  onEpisodeClick?: (episode: CalendarEpisodeItem) => void;
};

const FACET_COLORS: Record<string, string> = {
  anime: "var(--scry-facet-anime)",
  movie: "var(--scry-facet-movie)",
  series: "var(--scry-facet-series)",
};

const FACET_GRADIENTS: Record<string, string> = {
  anime: "var(--scry-facet-anime-grad)",
  movie: "var(--scry-facet-movie-grad)",
  series: "var(--scry-facet-series-grad)",
};

const FACET_GLOWS: Record<string, string> = {
  anime: "rgba(var(--scry-facet-anime-rgb), .7)",
  movie: "rgba(var(--scry-facet-movie-rgb), .7)",
  series: "rgba(var(--scry-facet-series-rgb), .7)",
};

const FACET_RGB: Record<string, string> = {
  anime: "var(--scry-facet-anime-rgb)",
  movie: "var(--scry-facet-movie-rgb)",
  series: "var(--scry-facet-series-rgb)",
};

// Handoff orders the filter pills Anime · Movie · Series.
const FACET_ORDER = ["anime", "movie", "series"] as const;

const FACET_LABELS: Record<string, string> = {
  anime: "Anime",
  movie: "Movie",
  series: "Series",
};

function formatEpisodeLabel(ep: CalendarEpisodeItem): string {
  const parts: string[] = [ep.titleName];
  if (ep.seasonNumber && ep.episodeNumber) {
    parts.push(`S${ep.seasonNumber}E${ep.episodeNumber}`);
  } else if (ep.episodeNumber) {
    parts.push(`E${ep.episodeNumber}`);
  }
  if (ep.episodeTitle) {
    parts.push(`- ${ep.episodeTitle}`);
  }
  return parts.join(" ");
}

function formatEpisodeBadge(ep: CalendarEpisodeItem): string | null {
  if (ep.seasonNumber && ep.episodeNumber) {
    return `S${ep.seasonNumber}E${ep.episodeNumber}`;
  }
  if (ep.episodeNumber) {
    return `E${ep.episodeNumber}`;
  }
  return ep.titleFacet === "movie" ? "Movie" : null;
}

// The comp chip shows an air time next to the code badge. Only render it when
// the air date actually carries a time component (otherwise it's date-only).
function formatAirTime(airDate: string | null): string | null {
  if (!airDate || !airDate.includes("T")) return null;
  const time = airDate.slice(11, 16);
  return /^\d{2}:\d{2}$/.test(time) ? time : null;
}

function formatDateKey(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatTooltip(ep: CalendarEpisodeItem): string {
  const lines: string[] = [ep.titleName];
  if (ep.seasonNumber && ep.episodeNumber) {
    lines.push(`Season ${ep.seasonNumber}, Episode ${ep.episodeNumber}`);
  } else if (ep.episodeNumber) {
    lines.push(`Episode ${ep.episodeNumber}`);
  }
  if (ep.episodeTitle) {
    lines.push(ep.episodeTitle);
  }
  lines.push(`Library: ${ep.libraryName ?? ep.libraryId}`);
  lines.push(`Type: ${FACET_LABELS[ep.titleFacet] ?? ep.titleFacet}`);
  if (!ep.monitored) {
    lines.push("(Not monitored)");
  }
  return lines.join("\n");
}

export function CalendarView({
  episodes,
  loading,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  onDateRangeChange,
  onEpisodeClick,
}: CalendarViewProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [facetFilter, setFacetFilter] = useState<string[]>(["anime", "movie", "series"]);

  const filteredEpisodes = useMemo(
    () => episodes.filter((ep) => facetFilter.includes(ep.titleFacet)),
    [episodes, facetFilter],
  );

  const dayEventCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const ep of filteredEpisodes) {
      if (!ep.airDate) continue;
      counts.set(ep.airDate, (counts.get(ep.airDate) ?? 0) + 1);
    }
    return counts;
  }, [filteredEpisodes]);

  const events = useMemo(
    () =>
      filteredEpisodes
        .filter((ep) => ep.airDate)
        .map((ep) => ({
          id: ep.id,
          title: formatEpisodeLabel(ep),
          date: ep.airDate!,
          extendedProps: ep,
        })),
    [filteredEpisodes],
  );

  const handleDatesSet = (arg: DatesSetArg) => {
    const start = arg.startStr.slice(0, 10);
    const end = arg.endStr.slice(0, 10);
    onDateRangeChange(start, end);
  };

  const handleEventClick = useCallback(
    (arg: EventClickArg) => {
      const ep = arg.event.extendedProps as CalendarEpisodeItem;
      onEpisodeClick?.(ep);
    },
    [onEpisodeClick],
  );

  const handleEventDidMount = useCallback((arg: EventMountArg) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const facetColor = FACET_COLORS[ep.titleFacet] ?? "#6b7280";
    const facetGradient = FACET_GRADIENTS[ep.titleFacet] ?? facetColor;
    const facetRgb = FACET_RGB[ep.titleFacet] ?? "107, 114, 128";
    arg.el.setAttribute("title", formatTooltip(ep));
    arg.el.style.setProperty("--scryer-event-color", facetColor);
    arg.el.style.setProperty("--scryer-event-accent", facetColor);
    arg.el.style.setProperty("--scryer-event-gradient", facetGradient);
    arg.el.style.setProperty("--scryer-event-rgb", facetRgb);
    arg.el.style.setProperty("--fc-event-text-color", "#f8fbff");
  }, []);

  const renderEventContent = useCallback((arg: EventContentArg) => {
    const ep = arg.event.extendedProps as CalendarEpisodeItem;
    const badge = formatEpisodeBadge(ep);
    const time = formatAirTime(ep.airDate);

    return (
      <div className="fc-scryer-event-card">
        <div className="fc-scryer-event-title">{ep.titleName}</div>
        {badge || time ? (
          <div className="fc-scryer-event-meta">
            {badge ? <span className="fc-scryer-event-badge">{badge}</span> : null}
            {time ? <span className="fc-scryer-event-time">{time}</span> : null}
          </div>
        ) : null}
      </div>
    );
  }, []);

  const renderDayHeaderContent = useCallback((arg: DayHeaderContentArg) => (
    <span className="fc-scryer-header-label">{arg.text}</span>
  ), []);

  const renderDayCellContent = useCallback((arg: DayCellContentArg) => {
    if (arg.view.type !== "dayGridMonth") {
      return (
        <div className="fc-scryer-day-chip">
          <span className="fc-scryer-day-label">{arg.dayNumberText}</span>
        </div>
      );
    }

    return (
      <div className="fc-scryer-day-chip">
        <span className="fc-scryer-day-pill">{arg.dayNumberText}</span>
      </div>
    );
  }, []);

  const renderMoreLinkContent = useCallback((arg: MoreLinkContentArg) => (
    <span className="fc-scryer-more-link-text">+{arg.num} more</span>
  ), []);

  const handleFacetChange = useCallback((values: string[]) => {
    if (values.length > 0) setFacetFilter(values);
  }, []);

  return (
    <Card className="flex min-h-0 flex-1 flex-col rounded-none border-0 bg-transparent shadow-none">
      <CardContent className="flex min-h-0 flex-1 flex-col p-0">
        <div className="mb-4 flex flex-wrap items-center gap-3 max-sm:gap-2">
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
            disabled={librariesLoading || libraries.length === 0}
            triggerClassName="h-10 w-full rounded-[11px] text-[13px] sm:w-[188px]"
          />
          <div className="grid w-full grid-cols-3 items-center gap-2 sm:flex sm:w-auto">
            {FACET_ORDER.map((facet) => {
              const active = facetFilter.includes(facet);
              const color = FACET_COLORS[facet];
              return (
                <button
                  key={facet}
                  type="button"
                  aria-pressed={active}
                  onClick={() =>
                    handleFacetChange(
                      active
                        ? facetFilter.filter((f) => f !== facet)
                        : [...facetFilter, facet],
                    )
                  }
                  className="flex h-9 items-center justify-center gap-2 rounded-[10px] border px-2.5 text-[12.5px] font-semibold transition sm:px-3.5"
                  style={{
                    borderColor: active ? color : "var(--scry-border2)",
                    background: active ? "rgba(255,255,255,0.04)" : "transparent",
                    color: active ? "var(--scry-ink2)" : "var(--scry-faint)",
                  }}
                >
                  <span
                    className="h-[9px] w-[9px] rounded-full"
                    style={{
                      background: color,
                      opacity: active ? 1 : 0.35,
                      boxShadow: active ? `0 0 7px ${FACET_GLOWS[facet]}` : "none",
                    }}
                  />
                  {FACET_LABELS[facet]}
                </button>
              );
            })}
          </div>
          <div className="ml-auto flex w-full items-center justify-end gap-2 text-[12.5px] text-[var(--scry-muted3)] sm:w-auto">
            <CalendarClock className="h-[15px] w-[15px] text-[var(--scry-faint)]" />
            <span className="font-semibold text-[var(--scry-text4)]">
              {filteredEpisodes.length}
            </span>
            airings this month
          </div>
        </div>
        <p
          aria-live="polite"
          className="mb-2 min-h-5 text-sm text-muted-foreground"
        >
          {loading ? t("label.loading") : "\u00a0"}
        </p>
        <div className="fc-scryer min-h-0 flex-1">
          <FullCalendar
            key={isMobile ? "calendar-mobile" : "calendar-desktop"}
            plugins={[dayGridPlugin]}
            initialView="dayGridMonth"
            events={events}
            eventClick={handleEventClick}
            eventDidMount={handleEventDidMount}
            datesSet={handleDatesSet}
            eventContent={renderEventContent}
            eventClassNames={(arg) => {
              const ep = arg.event.extendedProps as CalendarEpisodeItem;
              const classes = [
                "fc-scryer-event",
                `fc-scryer-facet-${ep.titleFacet}`,
              ];
              classes.push(ep.monitored ? "is-monitored" : "is-unmonitored");
              return classes;
            }}
            dayHeaderContent={renderDayHeaderContent}
            dayCellContent={renderDayCellContent}
            dayCellClassNames={(arg) => {
              const classes = ["fc-scryer-day-cell"];
              if (arg.isToday) classes.push("is-today");
              if (arg.isOther) classes.push("is-other");
              if ((dayEventCounts.get(formatDateKey(arg.date)) ?? 0) > 0) {
                classes.push("has-events");
              }
              if (arg.view.type === "dayGridMonth") classes.push("is-month");
              return classes;
            }}
            moreLinkClassNames={() => ["fc-scryer-more-link"]}
            moreLinkContent={renderMoreLinkContent}
            buttonText={{
              today: "Today",
              month: "Month",
              week: "Week",
              dayGridMonth: "Month",
              dayGridWeek: "Week",
            }}
            headerToolbar={
              isMobile
                ? {
                    left: "prev,next",
                    center: "title",
                    right: "today",
                  }
                : {
                    left: "prev,next today",
                    center: "title",
                    right: "dayGridMonth,dayGridWeek",
                  }
            }
            views={{
              dayGridMonth: {
                fixedWeekCount: !isMobile,
                showNonCurrentDates: true,
                dayMaxEvents: isMobile ? 1 : 3,
              },
              dayGridWeek: {
                dayMaxEvents: false,
              },
            }}
            height={isMobile ? "auto" : "100%"}
            contentHeight={isMobile ? "auto" : "100%"}
            expandRows={!isMobile}
            eventDisplay="block"
            displayEventTime={false}
          />
        </div>
      </CardContent>
    </Card>
  );
}
