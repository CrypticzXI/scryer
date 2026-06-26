import * as React from "react";
import {
  ChevronDown,
  ChevronUp,
  Loader2,
  RotateCcw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { TitleHistoryEvent } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDate, formatUiTime } from "@/lib/utils/date-format";
import { redactHistoryApiKeys } from "@/lib/utils/history-redaction";
import { selectorId } from "@/lib/utils/dom-ids";
import { HistoryEventIcon } from "./history-event-icon";
import {
  HistoryEventDetailContent,
  buildHistoryEventDetail,
} from "./history-event-detail";
import {
  getTitleHistoryEventLabel,
  getTitleHistoryEventMeta,
} from "./title-history-event-meta";

function formatFacetLabel(facet: string | null): string {
  if (!facet) {
    return "\u2014";
  }

  return facet.charAt(0).toUpperCase() + facet.slice(1);
}

function primarySourceLabel(event: TitleHistoryEvent): string {
  return redactHistoryApiKeys(
    event.displayTitle ??
    event.sourceTitle ??
    event.sourcePath ??
    event.destPath ??
    "\u2014",
  );
}

function secondarySourceLabel(event: TitleHistoryEvent): string | null {
  const values = [
    event.sourceSystem,
    event.sourceRef,
    event.sourceHint,
  ]
    .filter((value): value is string => Boolean(value))
    .map((value) => redactHistoryApiKeys(value));
  return values.length > 0 ? values.join(" • ") : null;
}

function actorLabel(event: TitleHistoryEvent): string {
  return event.actorDisplayName ?? event.actorUserId ?? event.actorKind ?? "\u2014";
}

function canRetryEvent(event: TitleHistoryEvent, onRetry?: (importId: string, password?: string) => Promise<void>): boolean {
  return Boolean(
    onRetry &&
      event.importId &&
      (event.eventType === "import_failed" || event.eventType === "import_skipped"),
  );
}

export function HistoryEventTable({
  events,
  showTitle = false,
  showFacet = false,
  showActor = false,
  titleNameMap,
  emptyMessage,
  onRetry,
}: {
  events: TitleHistoryEvent[];
  showTitle?: boolean;
  showFacet?: boolean;
  showActor?: boolean;
  titleNameMap?: Record<string, string>;
  emptyMessage?: string;
  onRetry?: (importId: string, password?: string) => Promise<void>;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [expandedRows, setExpandedRows] = React.useState<Record<string, boolean>>({});
  const [passwordDrafts, setPasswordDrafts] = React.useState<Record<string, string>>({});
  const [retryingId, setRetryingId] = React.useState<string | null>(null);
  const showActions = Boolean(onRetry);
  const columnCount =
    1 + // expander
    1 + // event
    (showTitle ? 1 : 0) +
    1 + // source
    (showFacet ? 1 : 0) +
    (showActor ? 1 : 0) +
    1 + // quality
    1 + // date
    (showActions ? 1 : 0);

  const toggleExpanded = React.useCallback((eventId: string) => {
    setExpandedRows((current) => ({
      ...current,
      [eventId]: !current[eventId],
    }));
  }, []);

  const setPasswordDraft = React.useCallback((eventId: string, value: string) => {
    setPasswordDrafts((current) => ({
      ...current,
      [eventId]: value,
    }));
  }, []);

  const handleRetry = React.useCallback(
    async (event: TitleHistoryEvent) => {
      if (!onRetry || !event.importId) {
        return;
      }

      const password = passwordDrafts[event.id]?.trim();
      if (event.retryRequiresPassword && !password) {
        return;
      }

      setRetryingId(event.id);
      try {
        await onRetry(event.importId, password || undefined);
      } finally {
        setRetryingId(null);
      }
    },
    [onRetry, passwordDrafts],
  );

  if (events.length === 0) {
    return (
      <p className="py-4 text-sm text-muted-foreground">
        {emptyMessage ?? t("history.empty")}
      </p>
    );
  }

  return (
    <div className="overflow-x-auto">
      <Table className={showTitle || showFacet || showActor || showActions ? "min-w-[1080px]" : "min-w-[720px]"}>
        <TableHeader>
          <TableRow>
            <TableHead className="w-10" />
            <TableHead className="w-36">{t("history.event")}</TableHead>
            {showTitle ? (
              <TableHead className="w-52">{t("history.titleColumn")}</TableHead>
            ) : null}
            <TableHead>{t("history.sourceTitle")}</TableHead>
            {showFacet ? (
              <TableHead className="w-28">{t("history.facet")}</TableHead>
            ) : null}
            {showActor ? (
              <TableHead className="w-36">{t("history.actor")}</TableHead>
            ) : null}
            <TableHead className="w-28">{t("history.quality")}</TableHead>
            <TableHead className="w-44">{t("history.date")}</TableHead>
            {showActions ? (
              <TableHead className="w-36 text-right">{t("history.actions")}</TableHead>
            ) : null}
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((event) => {
            const meta = getTitleHistoryEventMeta(event.eventType);
            const isExpanded = expandedRows[event.id] ?? false;
            const detail = buildHistoryEventDetail(event);
            const retryable = canRetryEvent(event, onRetry);
            const hasExpandableContent =
              detail.hasDetail ||
              retryable ||
              event.episodeIds.length > 0 ||
              Boolean(event.collectionId);

            return (
              <React.Fragment key={event.id}>
                <TableRow id={selectorId("history-event-row", event.eventType, event.id)}>
                  <TableCell>
                    {hasExpandableContent ? (
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border/60 bg-card/80 text-muted-foreground transition hover:text-foreground"
                        onClick={() => toggleExpanded(event.id)}
                        aria-label={
                          isExpanded
                            ? t("history.collapseDetails")
                            : t("history.expandDetails")
                        }
                      >
                        {isExpanded ? (
                          <ChevronUp className="h-4 w-4" />
                        ) : (
                          <ChevronDown className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                  </TableCell>
                  <TableCell>
                    <span
                      className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium ${meta.badgeClassName}`}
                    >
                      <HistoryEventIcon eventType={event.eventType} size={14} />
                      <span>{getTitleHistoryEventLabel(event.eventType, t)}</span>
                    </span>
                  </TableCell>
                  {showTitle ? (
                    <TableCell className="align-top">
                      <div className="text-sm font-medium text-foreground">
                        {titleNameMap?.[event.titleId] ?? event.titleName ?? event.titleId}
                      </div>
                      {event.episodeIds.length > 0 ? (
                        <div className="mt-1 text-xs text-muted-foreground">
                          {event.episodeIds.length === 1
                            ? t("history.episodeCountSingle")
                            : t("history.episodeCountMultiple", {
                                count: event.episodeIds.length,
                              })}
                        </div>
                      ) : null}
                    </TableCell>
                  ) : null}
                  <TableCell className="align-top">
                    <div className="text-sm text-foreground">{primarySourceLabel(event)}</div>
                    {secondarySourceLabel(event) ? (
                      <div className="mt-1 text-xs text-muted-foreground">
                        {secondarySourceLabel(event)}
                      </div>
                    ) : null}
                  </TableCell>
                  {showFacet ? (
                    <TableCell className="align-top text-sm text-muted-foreground">
                      {formatFacetLabel(event.facet)}
                    </TableCell>
                  ) : null}
                  {showActor ? (
                    <TableCell
                      id={selectorId("history-event-actor", event.eventType, event.id)}
                      className="align-top text-sm text-muted-foreground"
                    >
                      {actorLabel(event)}
                    </TableCell>
                  ) : null}
                  <TableCell className="align-top text-sm text-muted-foreground">
                    {event.quality ?? "\u2014"}
                  </TableCell>
                  <TableCell className="align-top text-sm text-muted-foreground">
                    <div className="font-medium text-foreground">
                      {formatUiDate(event.occurredAt ?? event.createdAt, dateTimeFormat)}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {formatUiTime(event.occurredAt ?? event.createdAt, dateTimeFormat)}
                    </div>
                  </TableCell>
                  {showActions ? (
                    <TableCell className="align-top">
                      {retryable && !event.retryRequiresPassword && !isExpanded ? (
                        <div className="flex justify-end">
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            disabled={retryingId === event.id}
                            onClick={() => void handleRetry(event)}
                          >
                            {retryingId === event.id ? (
                              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                            ) : (
                              <RotateCcw className="mr-2 h-4 w-4" />
                            )}
                            {t("importHistory.retry")}
                          </Button>
                        </div>
                      ) : null}
                    </TableCell>
                  ) : null}
                </TableRow>
                {isExpanded ? (
                  <TableRow>
                    <TableCell colSpan={columnCount} className="bg-card/30">
                      <div className="space-y-4 rounded-lg border border-border/60 bg-background/40 p-4">
                        {detail.hasDetail ? (
                          <HistoryEventDetailContent event={event} />
                        ) : null}
                        {event.collectionId ? (
                          <div className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
                            <span className="whitespace-nowrap text-muted-foreground">
                              {t("history.collectionId")}
                            </span>
                            <span className="break-all text-foreground">
                              {event.collectionId}
                            </span>
                          </div>
                        ) : null}
                        {retryable ? (
                          <div className="space-y-2 border-t border-border/60 pt-4">
                            {event.retryRequiresPassword ? (
                              <div className="space-y-2">
                                <p className="text-xs text-muted-foreground">
                                  {t("importHistory.passwordRequired")}
                                </p>
                                <div className="flex flex-col gap-2 sm:flex-row">
                                  <Input
                                    type="password"
                                    value={passwordDrafts[event.id] ?? ""}
                                    onChange={(inputEvent) =>
                                      setPasswordDraft(event.id, inputEvent.target.value)
                                    }
                                    placeholder={t("importHistory.passwordPlaceholder")}
                                    className="sm:max-w-xs"
                                  />
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    disabled={
                                      retryingId === event.id ||
                                      !(passwordDrafts[event.id] ?? "").trim()
                                    }
                                    onClick={() => void handleRetry(event)}
                                  >
                                    {retryingId === event.id ? (
                                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                    ) : (
                                      <RotateCcw className="mr-2 h-4 w-4" />
                                    )}
                                    {t("importHistory.retryWithPassword")}
                                  </Button>
                                </div>
                              </div>
                            ) : (
                              <Button
                                type="button"
                                variant="outline"
                                size="sm"
                                disabled={retryingId === event.id}
                                onClick={() => void handleRetry(event)}
                              >
                                {retryingId === event.id ? (
                                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                ) : (
                                  <RotateCcw className="mr-2 h-4 w-4" />
                                )}
                                {t("importHistory.retry")}
                              </Button>
                            )}
                          </div>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                ) : null}
              </React.Fragment>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}
