import * as React from "react";
import { ChevronDown, ChevronUp, Loader2, RefreshCcw } from "lucide-react";
import { useClient } from "urql";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { auditLogQuery } from "@/lib/graphql/queries";
import { CODE_FONT } from "@/lib/fonts";
import type { AuditLogEvent } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";

const PAGE_SIZE = 100;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeAuditEvent(value: unknown): AuditLogEvent | null {
  if (!isRecord(value) || typeof value.eventId !== "string") {
    return null;
  }
  return {
    sequence: typeof value.sequence === "number" ? value.sequence : 0,
    eventId: value.eventId,
    occurredAt: typeof value.occurredAt === "string" ? value.occurredAt : "",
    actorKind: typeof value.actorKind === "string" ? value.actorKind : "system",
    actorUserId: typeof value.actorUserId === "string" ? value.actorUserId : null,
    actorDisplayName:
      typeof value.actorDisplayName === "string" ? value.actorDisplayName : "System",
    titleId: typeof value.titleId === "string" ? value.titleId : null,
    facet: typeof value.facet === "string" ? value.facet : null,
    eventType: typeof value.eventType === "string" ? value.eventType : "unknown",
    streamKind: typeof value.streamKind === "string" ? value.streamKind : "global",
    streamId: typeof value.streamId === "string" ? value.streamId : null,
    payloadJson: value.payloadJson,
  };
}

function formatTimestamp(value: string, dateTimeFormat: UiDateTimeFormat): string {
  return formatUiDateTime(value, dateTimeFormat);
}

function formatLabel(value: string): string {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function payloadText(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export const SystemAuditContainer = React.memo(function SystemAuditContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [events, setEvents] = React.useState<AuditLogEvent[]>([]);
  const [expanded, setExpanded] = React.useState<Record<string, boolean>>({});
  const [loading, setLoading] = React.useState(false);
  const [loadingOlder, setLoadingOlder] = React.useState(false);
  const [hasMore, setHasMore] = React.useState(false);

  const fetchAuditEvents = React.useCallback(
    async (beforeSequence: number | null) => {
      const { data, error } = await client
        .query(auditLogQuery, {
          beforeSequence,
          limit: PAGE_SIZE,
        })
        .toPromise();
      if (error) throw error;
      return ((Array.isArray(data?.auditLog) ? data.auditLog : []) as unknown[])
        .map(normalizeAuditEvent)
        .filter((event): event is AuditLogEvent => event !== null);
    },
    [client],
  );

  const refreshAuditEvents = React.useCallback(async () => {
    setLoading(true);
    try {
      const nextEvents = await fetchAuditEvents(null);
      setHasMore(nextEvents.length === PAGE_SIZE);
      setEvents(nextEvents);
      setGlobalStatus(t("system.auditLoaded"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [fetchAuditEvents, setGlobalStatus, t]);

  const loadOlderAuditEvents = React.useCallback(async () => {
    if (events.length === 0) return;
    setLoadingOlder(true);
    const beforeSequence = Math.min(...events.map((event) => event.sequence));
    try {
      const nextEvents = await fetchAuditEvents(beforeSequence);
      setHasMore(nextEvents.length === PAGE_SIZE);
      setEvents((current) => {
        const seen = new Set(current.map((event) => event.eventId));
        return [
          ...current,
          ...nextEvents.filter((event) => !seen.has(event.eventId)),
        ];
      });
      setGlobalStatus(t("system.auditLoaded"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoadingOlder(false);
    }
  }, [events, fetchAuditEvents, setGlobalStatus, t]);

  React.useEffect(() => {
    void refreshAuditEvents();
  }, [refreshAuditEvents]);

  const toggleExpanded = React.useCallback((eventId: string) => {
    setExpanded((current) => ({
      ...current,
      [eventId]: !current[eventId],
    }));
  }, []);

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">{t("system.auditTitle")}</h1>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void refreshAuditEvents()}
          disabled={loading}
        >
          {loading ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <RefreshCcw className="mr-2 h-4 w-4" />
          )}
          {t("label.refresh")}
        </Button>
      </div>

      <div className="overflow-x-auto rounded-md border border-border">
        <Table className="min-w-[1120px]">
          <TableHeader>
            <TableRow>
              <TableHead className="w-10" />
              <TableHead className="w-48">{t("history.date")}</TableHead>
              <TableHead className="w-52">{t("history.event")}</TableHead>
              <TableHead className="w-40">{t("history.actor")}</TableHead>
              <TableHead className="w-52">{t("system.auditTarget")}</TableHead>
              <TableHead>{t("system.auditStream")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {events.length === 0 && !loading ? (
              <TableRow>
                <TableCell colSpan={6} className="py-6 text-sm text-muted-foreground">
                  {t("system.auditEmpty")}
                </TableCell>
              </TableRow>
            ) : null}
            {events.map((event) => {
              const isExpanded = expanded[event.eventId] ?? false;
              return (
                <React.Fragment key={event.eventId}>
                  <TableRow>
                    <TableCell>
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border/60 bg-card/80 text-muted-foreground transition hover:text-foreground"
                        onClick={() => toggleExpanded(event.eventId)}
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
                    </TableCell>
                    <TableCell className="align-top text-sm text-muted-foreground">
                      {formatTimestamp(event.occurredAt, dateTimeFormat)}
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="text-sm font-medium text-foreground">
                        {formatLabel(event.eventType)}
                      </div>
                      <div className="mt-1 text-xs text-muted-foreground">
                        #{event.sequence}
                      </div>
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="text-sm text-foreground">{event.actorDisplayName}</div>
                      <div className="mt-1 text-xs text-muted-foreground">
                        {formatLabel(event.actorKind)}
                      </div>
                    </TableCell>
                    <TableCell className="align-top text-sm text-muted-foreground">
                      {event.titleId ?? event.facet ?? "\u2014"}
                    </TableCell>
                    <TableCell className="align-top text-sm text-muted-foreground">
                      {event.streamKind}
                      {event.streamId ? ` / ${event.streamId}` : ""}
                    </TableCell>
                  </TableRow>
                  {isExpanded ? (
                    <TableRow>
                      <TableCell colSpan={6} className="bg-card/30">
                        <pre
                          className="max-h-[420px] overflow-auto whitespace-pre-wrap rounded-md border border-border/60 bg-background/60 p-4 text-xs text-foreground"
                          style={{ fontFamily: CODE_FONT }}
                        >
                          {payloadText(event.payloadJson)}
                        </pre>
                      </TableCell>
                    </TableRow>
                  ) : null}
                </React.Fragment>
              );
            })}
          </TableBody>
        </Table>
      </div>

      <div className="flex justify-end">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!hasMore || loadingOlder}
          onClick={() => void loadOlderAuditEvents()}
        >
          {loadingOlder ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
          {t("history.loadMore")}
        </Button>
      </div>
    </div>
  );
});
