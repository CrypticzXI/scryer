import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";

import {
  libraryScanDomainEventFeedSubscriptionQuery,
  libraryScanDomainEventsQuery,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type {
  Facet,
  LibraryScanMode,
  LibraryScanProgress,
  LibraryScanStatus,
  LibraryScanSummary,
} from "@/lib/types";

const LIBRARY_SCAN_EVENT_PAGE_SIZE = 500;

type LibraryScanDomainEventType =
  | "library_scan_started"
  | "library_scan_title_discovered"
  | "library_scan_progressed"
  | "library_scan_completed"
  | "library_scan_canceled"
  | "library_scan_failed";

type DomainEventEnvelope = {
  sequence: number;
  occurredAt: string;
  facet: Facet | null;
  eventType: LibraryScanDomainEventType;
  payloadJson: unknown;
};

type LibraryScanStartedPayload = {
  sessionId: string;
  mode: LibraryScanMode;
};

type LibraryScanTitleDiscoveredPayload = {
  sessionId: string;
  facet: Facet;
  discoveredFileCount: number;
};

type LibraryScanPhasePayload = {
  sessionId: string;
  status: LibraryScanStatus;
  foundTitles: number;
  titleMatchCompleted: number;
  titleMatchTotalKnown: boolean;
  titlesCompleted: number;
  titlesTotal: number | null;
  filesCompleted: number;
  filesTotal: number | null;
  summary?: LibraryScanSummary | null;
};

type LibraryScanFailedPayload = {
  sessionId: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isLibraryScanEventType(
  value: unknown,
): value is LibraryScanDomainEventType {
  return (
    value === "library_scan_started" ||
    value === "library_scan_title_discovered" ||
    value === "library_scan_progressed" ||
    value === "library_scan_completed" ||
    value === "library_scan_canceled" ||
    value === "library_scan_failed"
  );
}

function normalizeFacet(value: unknown): Facet {
  if (value === "anime") {
    return "anime";
  }

  if (value === "tv" || value === "series") {
    return "tv";
  }

  return "movie";
}

function normalizeFacetOrNull(value: unknown): Facet | null {
  if (
    value === "movie" ||
    value === "tv" ||
    value === "series" ||
    value === "anime"
  ) {
    return normalizeFacet(value);
  }
  return null;
}

function normalizeStatus(value: unknown): LibraryScanStatus {
  switch (value) {
    case "discovering":
    case "running":
    case "completed":
    case "canceled":
    case "warning":
    case "failed":
      return value;
    default:
      return "running";
  }
}

function normalizeMode(value: unknown): LibraryScanMode {
  return value === "additive" ? "additive" : "full";
}

function normalizeNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function normalizeNonNegativeNumber(value: unknown): number {
  return Math.max(0, normalizeNumber(value));
}

function normalizeSummary(value: unknown): LibraryScanSummary | null {
  if (!isRecord(value)) {
    return null;
  }

  return {
    scanned: normalizeNonNegativeNumber(value.scanned),
    matched: normalizeNonNegativeNumber(value.matched),
    imported: normalizeNonNegativeNumber(value.imported),
    skipped: normalizeNonNegativeNumber(value.skipped),
    unmatched: normalizeNonNegativeNumber(value.unmatched),
  };
}

function isTerminal(status: LibraryScanStatus): boolean {
  return (
    status === "completed" ||
    status === "canceled" ||
    status === "warning" ||
    status === "failed"
  );
}

function emptyPhaseProgress() {
  return {
    total: 0,
    completed: 0,
    failed: 0,
  };
}

function emptySession(
  sessionId: string,
  facet: Facet,
  mode: LibraryScanMode,
  status: LibraryScanStatus,
  occurredAt: string,
): LibraryScanProgress {
  return {
    sessionId,
    facet,
    mode,
    status,
    startedAt: occurredAt,
    updatedAt: occurredAt,
    foundTitles: 0,
    titleMatchTotalKnown: false,
    hydrationTotalKnown: false,
    mediaAnalysisTotalKnown: false,
    titleMatchProgress: emptyPhaseProgress(),
    hydrationProgress: emptyPhaseProgress(),
    mediaAnalysisProgress: emptyPhaseProgress(),
    summary: null,
  };
}

function titleMatchCompletedFromEvent(
  foundTitles: number,
  titleMatchCompleted: number,
  completedEvent: boolean,
): number {
  if (completedEvent && titleMatchCompleted <= 0) {
    return foundTitles;
  }

  return Math.max(0, titleMatchCompleted);
}

function payloadData(event: DomainEventEnvelope): Record<string, unknown> | null {
  if (!isRecord(event.payloadJson)) {
    return null;
  }

  const payloadType = event.payloadJson.type;
  if (typeof payloadType === "string" && payloadType !== event.eventType) {
    return null;
  }

  if (isRecord(event.payloadJson.data)) {
    return event.payloadJson.data;
  }

  return event.payloadJson;
}

function normalizeDomainEventEnvelope(value: unknown): DomainEventEnvelope | null {
  if (!isRecord(value) || !isLibraryScanEventType(value.eventType)) {
    return null;
  }

  const sequence = normalizeNumber(value.sequence);
  if (sequence <= 0 || typeof value.occurredAt !== "string") {
    return null;
  }

  return {
    sequence,
    occurredAt: value.occurredAt,
    facet: normalizeFacetOrNull(value.facet),
    eventType: value.eventType,
    payloadJson: value.payloadJson,
  };
}

function normalizeStartedPayload(
  event: DomainEventEnvelope,
): LibraryScanStartedPayload | null {
  const data = payloadData(event);
  if (!data || typeof data.session_id !== "string") {
    return null;
  }

  return {
    sessionId: data.session_id,
    mode: normalizeMode(data.mode),
  };
}

function normalizeTitleDiscoveredPayload(
  event: DomainEventEnvelope,
): LibraryScanTitleDiscoveredPayload | null {
  const data = payloadData(event);
  if (!data || typeof data.session_id !== "string") {
    return null;
  }

  return {
    sessionId: data.session_id,
    facet: normalizeFacet(data.facet ?? event.facet),
    discoveredFileCount: normalizeNonNegativeNumber(data.discovered_file_count),
  };
}

function normalizePhasePayload(
  event: DomainEventEnvelope,
): LibraryScanPhasePayload | null {
  const data = payloadData(event);
  if (!data || typeof data.session_id !== "string") {
    return null;
  }

  const titlesTotal =
    typeof data.titles_total === "number" && Number.isFinite(data.titles_total)
      ? Math.max(0, data.titles_total)
      : null;
  const filesTotal =
    typeof data.files_total === "number" && Number.isFinite(data.files_total)
      ? Math.max(0, data.files_total)
      : null;

  return {
    sessionId: data.session_id,
    status: normalizeStatus(data.status),
    foundTitles: normalizeNonNegativeNumber(data.found_titles),
    titleMatchCompleted: normalizeNonNegativeNumber(data.title_match_completed),
    titleMatchTotalKnown: data.title_match_total_known === true,
    titlesCompleted: normalizeNonNegativeNumber(data.titles_completed),
    titlesTotal,
    filesCompleted: normalizeNonNegativeNumber(data.files_completed),
    filesTotal,
    summary: normalizeSummary(data.summary),
  };
}

function normalizeFailedPayload(
  event: DomainEventEnvelope,
): LibraryScanFailedPayload | null {
  const data = payloadData(event);
  if (!data || typeof data.session_id !== "string") {
    return null;
  }

  return {
    sessionId: data.session_id,
  };
}

function sessionFromStarted(
  event: DomainEventEnvelope,
  payload: LibraryScanStartedPayload,
): LibraryScanProgress {
  return emptySession(
    payload.sessionId,
    event.facet ?? "movie",
    payload.mode,
    "discovering",
    event.occurredAt,
  );
}

function sessionFromTitleDiscovered(
  event: DomainEventEnvelope,
  payload: LibraryScanTitleDiscoveredPayload,
): LibraryScanProgress {
  return emptySession(
    payload.sessionId,
    payload.facet,
    "full",
    "running",
    event.occurredAt,
  );
}

function sessionFromPhase(
  event: DomainEventEnvelope,
  payload: LibraryScanPhasePayload,
): LibraryScanProgress {
  const session = emptySession(
    payload.sessionId,
    event.facet ?? "movie",
    "full",
    payload.status,
    event.occurredAt,
  );
  applyProgressPayload(session, payload, event, false);
  return session;
}

function sessionFromFailed(
  event: DomainEventEnvelope,
  payload: LibraryScanFailedPayload,
): LibraryScanProgress {
  const session = emptySession(
    payload.sessionId,
    event.facet ?? "movie",
    "full",
    "failed",
    event.occurredAt,
  );
  session.titleMatchTotalKnown = true;
  session.hydrationTotalKnown = true;
  session.mediaAnalysisTotalKnown = true;
  return session;
}

function applyProgressPayload(
  session: LibraryScanProgress,
  payload: LibraryScanPhasePayload,
  event: DomainEventEnvelope,
  completedEvent: boolean,
) {
  session.updatedAt = event.occurredAt;
  session.status = payload.status;
  session.foundTitles = payload.foundTitles;
  session.titleMatchProgress.total = payload.foundTitles;
  session.titleMatchTotalKnown = completedEvent || payload.titleMatchTotalKnown;
  session.titleMatchProgress.completed = titleMatchCompletedFromEvent(
    payload.foundTitles,
    payload.titleMatchCompleted,
    completedEvent,
  );
  if (payload.titlesTotal !== null) {
    session.hydrationProgress.total = payload.titlesTotal;
    session.hydrationTotalKnown = true;
  } else if (completedEvent) {
    session.hydrationTotalKnown = true;
  }
  session.hydrationProgress.completed = payload.titlesCompleted;
  if (payload.filesTotal !== null) {
    session.mediaAnalysisProgress.total = payload.filesTotal;
    session.mediaAnalysisTotalKnown = true;
  } else if (completedEvent) {
    session.mediaAnalysisTotalKnown = true;
  }
  session.mediaAnalysisProgress.completed = payload.filesCompleted;
  if (payload.summary !== undefined) {
    session.summary = payload.summary;
  }
}

function applyLibraryScanEvent(
  sessionsById: Record<string, LibraryScanProgress>,
  event: DomainEventEnvelope,
): Record<string, LibraryScanProgress> {
  switch (event.eventType) {
    case "library_scan_started": {
      const payload = normalizeStartedPayload(event);
      if (!payload) {
        return sessionsById;
      }

      return {
        ...sessionsById,
        [payload.sessionId]: sessionFromStarted(event, payload),
      };
    }

    case "library_scan_title_discovered": {
      const payload = normalizeTitleDiscoveredPayload(event);
      if (!payload) {
        return sessionsById;
      }

      const current =
        sessionsById[payload.sessionId] ?? sessionFromTitleDiscovered(event, payload);
      const next = {
        ...current,
        updatedAt: event.occurredAt,
        facet: payload.facet,
        foundTitles: current.foundTitles + 1,
        mediaAnalysisProgress: {
          ...current.mediaAnalysisProgress,
          total:
            current.mediaAnalysisProgress.total + payload.discoveredFileCount,
        },
      };
      if (next.status === "discovering") {
        next.status = "running";
      }

      return {
        ...sessionsById,
        [payload.sessionId]: next,
      };
    }

    case "library_scan_progressed": {
      const payload = normalizePhasePayload(event);
      if (!payload) {
        return sessionsById;
      }

      const next =
        sessionsById[payload.sessionId] ?? sessionFromPhase(event, payload);
      applyProgressPayload(next, payload, event, false);

      return {
        ...sessionsById,
        [payload.sessionId]: next,
      };
    }

    case "library_scan_completed": {
      const payload = normalizePhasePayload(event);
      if (!payload) {
        return sessionsById;
      }

      const next =
        sessionsById[payload.sessionId] ?? sessionFromPhase(event, payload);
      applyProgressPayload(next, payload, event, true);

      return {
        ...sessionsById,
        [payload.sessionId]: next,
      };
    }

    case "library_scan_canceled": {
      const payload = normalizePhasePayload(event);
      if (!payload) {
        return sessionsById;
      }

      const next =
        sessionsById[payload.sessionId] ?? sessionFromPhase(event, payload);
      applyProgressPayload(next, payload, event, true);

      return {
        ...sessionsById,
        [payload.sessionId]: next,
      };
    }

    case "library_scan_failed": {
      const payload = normalizeFailedPayload(event);
      if (!payload) {
        return sessionsById;
      }

      const current = sessionsById[payload.sessionId] ?? sessionFromFailed(event, payload);
      const next = {
        ...current,
        updatedAt: event.occurredAt,
        status: "failed" as const,
        titleMatchTotalKnown: true,
        hydrationTotalKnown: true,
        mediaAnalysisTotalKnown: true,
      };

      return {
        ...sessionsById,
        [payload.sessionId]: next,
      };
    }
  }
}

async function loadInitialLibraryScanSessions(
  client: ReturnType<typeof useClient>,
): Promise<{
  afterSequence: number;
  sessionsById: Record<string, LibraryScanProgress>;
}> {
  const sessionsById: Record<string, LibraryScanProgress> = {};
  let afterSequence = 0;

  while (true) {
    const { data, error } = await client
      .query(libraryScanDomainEventsQuery, {
        afterSequence,
        limit: LIBRARY_SCAN_EVENT_PAGE_SIZE,
      })
      .toPromise();

    if (error) {
      throw error;
    }

    const rawEvents: unknown[] = Array.isArray(data?.domainEvents)
      ? data.domainEvents
      : [];
    const events = rawEvents
      .map(normalizeDomainEventEnvelope)
      .filter((event): event is DomainEventEnvelope => event !== null);

    if (events.length === 0) {
      break;
    }

    for (const event of events) {
      afterSequence = Math.max(afterSequence, event.sequence);
      Object.assign(sessionsById, applyLibraryScanEvent(sessionsById, event));
    }

    if (events.length < LIBRARY_SCAN_EVENT_PAGE_SIZE) {
      break;
    }
  }

  for (const session of Object.values(sessionsById)) {
    if (isTerminal(session.status)) {
      delete sessionsById[session.sessionId];
    }
  }

  return {
    afterSequence,
    sessionsById,
  };
}

export function useLibraryScanEventStream() {
  const client = useClient();
  const [sessionsById, setSessionsById] = useState<
    Record<string, LibraryScanProgress>
  >({});
  const [subscriptionAfterSequence, setSubscriptionAfterSequence] = useState<
    number | null
  >(null);
  const lastSequenceRef = useRef(0);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const initial = await loadInitialLibraryScanSessions(client);
        if (cancelled) {
          return;
        }

        lastSequenceRef.current = initial.afterSequence;
        setSessionsById(initial.sessionsById);
        setSubscriptionAfterSequence(initial.afterSequence);
      } catch (error) {
        console.error(
          "[library-scan-events] failed to bootstrap scan sessions:",
          error,
        );
        if (!cancelled) {
          lastSequenceRef.current = 0;
          setSessionsById({});
          setSubscriptionAfterSequence(0);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  useDeferredWsSubscription<{ data?: { domainEventFeed?: unknown } }>({
    enabled: subscriptionAfterSequence !== null,
    requestKey:
      subscriptionAfterSequence === null
        ? "libraryScanDomainEventFeed:pending"
        : `libraryScanDomainEventFeed:${subscriptionAfterSequence}`,
    request: {
      query: libraryScanDomainEventFeedSubscriptionQuery,
      variables: {
        afterSequence: subscriptionAfterSequence,
      },
    },
    onNext(result) {
      const event = normalizeDomainEventEnvelope(result.data?.domainEventFeed);
      if (!event || event.sequence <= lastSequenceRef.current) {
        return;
      }

      lastSequenceRef.current = event.sequence;
      setSessionsById((current) => applyLibraryScanEvent(current, event));
    },
    onError(error) {
      console.error("[library-scan-events] subscription error:", error);
    },
  });

  const dismissSession = useCallback((sessionId: string) => {
    setSessionsById((current) => {
      if (!(sessionId in current)) {
        return current;
      }

      const next = { ...current };
      delete next[sessionId];
      return next;
    });
  }, []);

  const sessions = useMemo(
    () =>
      Object.values(sessionsById)
        .filter((session) => session.mode === "full")
        .sort((left, right) => left.startedAt.localeCompare(right.startedAt)),
    [sessionsById],
  );

  const getActiveSession = useCallback(
    (facet: Facet) =>
      sessions.find(
        (session) => session.facet === facet && !isTerminal(session.status),
      ) ?? null,
    [sessions],
  );

  return {
    sessions,
    getActiveSession,
    dismissSession,
  };
}
