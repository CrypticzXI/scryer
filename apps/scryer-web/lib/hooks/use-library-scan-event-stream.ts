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
    status === "completed" ||
    status === "canceled" ||
    status === "warning" ||
    status === "failed"
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

export function useLibraryScanEventStream() {
  const client = useClient();
  const [sessionsById, setSessionsById] = useState<
    Record<string, LibraryScanProgress>
  >({});

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const { data, error } = await client
          .query(activeLibraryScansQuery, {})
          .toPromise();

        if (error) {
          throw error;
        }

        if (cancelled) {
          return;
        }

        const rawSessions: unknown[] = Array.isArray(data?.activeLibraryScans)
          ? data.activeLibraryScans
          : [];
        const sessions = rawSessions
          .map(normalizeLibraryScanProgress)
          .filter((session): session is LibraryScanProgress => session !== null);

        setSessionsById((current) =>
          mergeSessionsByNewest(current, Object.values(indexSessions(sessions))),
        );
      } catch (error) {
        console.error(
          "[library-scan-events] failed to bootstrap active scan sessions:",
          error,
        );
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

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
