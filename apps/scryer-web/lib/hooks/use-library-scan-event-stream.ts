import { useCallback, useEffect, useMemo, useState } from "react";
import { useClient } from "urql";

import {
  activeLibraryScansQuery,
  libraryScanStateSubscriptionQuery,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type { Facet, LibraryScanProgress, LibraryScanStatus } from "@/lib/types";
import { normalizeLibraryScanProgress } from "@/lib/utils/job-runs";

function isTerminal(status: LibraryScanStatus): boolean {
  return (
    status === "COMPLETED" ||
    status === "CANCELED" ||
    status === "WARNING" ||
    status === "FAILED"
  );
}

function indexSessions(
  sessions: LibraryScanProgress[],
): Record<string, LibraryScanProgress> {
  return sessions.reduce<Record<string, LibraryScanProgress>>((acc, session) => {
    acc[session.sessionId] = session;
    return acc;
  }, {});
}

function scanSessionUpdatedAt(session: LibraryScanProgress): number {
  return (
    Date.parse(session.updatedAt ?? "") ||
    Date.parse(session.startedAt ?? "") ||
    0
  );
}

function preferNewestScanSession(
  current: LibraryScanProgress | undefined,
  incoming: LibraryScanProgress,
): LibraryScanProgress {
  if (!current) {
    return incoming;
  }

  return scanSessionUpdatedAt(incoming) >= scanSessionUpdatedAt(current)
    ? incoming
    : current;
}

function mergeSessionsByNewest(
  current: Record<string, LibraryScanProgress>,
  incoming: LibraryScanProgress[],
): Record<string, LibraryScanProgress> {
  const next = { ...current };
  for (const session of incoming) {
    next[session.sessionId] = preferNewestScanSession(
      next[session.sessionId],
      session,
    );
  }
  return next;
}

function replaceActiveSessions(
  current: Record<string, LibraryScanProgress>,
  incoming: LibraryScanProgress[],
): Record<string, LibraryScanProgress> {
  const retainedTerminalSessions = Object.values(current).filter((session) =>
    isTerminal(session.status),
  );

  return mergeSessionsByNewest(indexSessions(retainedTerminalSessions), incoming);
}

export function useLibraryScanEventStream() {
  const client = useClient();
  const [sessionsById, setSessionsById] = useState<
    Record<string, LibraryScanProgress>
  >({});

  const refreshSessions = useCallback(async () => {
    const { data, error } = await client
      .query(activeLibraryScansQuery, {}, { requestPolicy: "network-only" })
      .toPromise();

    if (error) {
      throw error;
    }

    const rawSessions: unknown[] = Array.isArray(data?.activeLibraryScans)
      ? data.activeLibraryScans
      : [];
    const sessions = rawSessions
      .map(normalizeLibraryScanProgress)
      .filter((session): session is LibraryScanProgress => session !== null);

    setSessionsById((current) => replaceActiveSessions(current, sessions));
    return sessions;
  }, [client]);

  useEffect(() => {
    let cancelled = false;

    void refreshSessions().catch((error) => {
      if (cancelled) {
        return;
      }

      console.error(
        "[library-scan-events] failed to bootstrap active scan sessions:",
        error,
      );
    });

    return () => {
      cancelled = true;
    };
  }, [refreshSessions]);

  useDeferredWsSubscription<{ data?: { libraryScanState?: unknown } }>({
    enabled: true,
    requestKey: "libraryScanState",
    request: {
      query: libraryScanStateSubscriptionQuery,
      variables: {},
    },
    onNext(result) {
      const session = normalizeLibraryScanProgress(result.data?.libraryScanState);
      if (!session) {
        return;
      }

      setSessionsById((current) =>
        mergeSessionsByNewest(current, [session]),
      );
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
        .filter((session) => session.mode === "FULL")
        .sort((left, right) => left.startedAt.localeCompare(right.startedAt)),
    [sessionsById],
  );

  useEffect(() => {
    if (!sessions.some((session) => !isTerminal(session.status))) {
      return;
    }

    const timer = window.setInterval(() => {
      void refreshSessions().catch((error) => {
        console.error(
          "[library-scan-events] failed to reconcile active scan sessions:",
          error,
        );
      });
    }, 5_000);

    return () => {
      window.clearInterval(timer);
    };
  }, [refreshSessions, sessions]);

  const getActiveSession = useCallback(
    (facet: Facet, libraryId?: string | null) =>
      sessions.find(
        (session) =>
          session.facet === facet &&
          (libraryId == null || session.libraryId === libraryId) &&
          !isTerminal(session.status),
      ) ?? null,
    [sessions],
  );

  return {
    sessions,
    getActiveSession,
    refreshSessions,
    dismissSession,
  };
}
