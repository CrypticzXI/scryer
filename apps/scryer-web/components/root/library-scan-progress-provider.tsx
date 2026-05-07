import * as React from "react";

import { LibraryScanToast } from "@/components/root/library-scan-toast";
import { Toaster, toast } from "@/components/ui/sonner";
import { LibraryScanProgressContext } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useLibraryScanEventStream } from "@/lib/hooks/use-library-scan-event-stream";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LibraryScanStatus } from "@/lib/types";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";

const TERMINAL_TOAST_DURATION_MS = 6_000;
const MOBILE_TOAST_DURATION_MS = 3_000;
const TOAST_EXIT_GRACE_MS = 200;
const LIBRARY_SCAN_TOASTER_ID = "library-scans";
const MAX_VISIBLE_LIBRARY_SCAN_TOASTS = 3;
const MAX_VISIBLE_GENERAL_TOASTS = 3;

function isTerminal(status: LibraryScanStatus): boolean {
  return (
    status === "completed" ||
    status === "canceled" ||
    status === "warning" ||
    status === "failed"
  );
}

function LiveLibraryScanToast({
  sessionId,
  onRunInBackground,
}: {
  sessionId: string;
  onRunInBackground?: () => void;
}) {
  const t = useTranslate();
  const value = React.useContext(LibraryScanProgressContext);
  if (!value) {
    return null;
  }

  const session = value.getSessionById(sessionId);
  if (!session) {
    return null;
  }

  return (
    <LibraryScanToast
      session={session}
      t={t}
      onRunInBackground={onRunInBackground}
    />
  );
}

export function LibraryScanProgressProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const { sessions, getActiveSession, refreshSessions, dismissSession } =
    useLibraryScanEventStream();
  const isMobile = useIsMobile();
  const dismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});
  const mobileDismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});
  const shownToastIdsRef = React.useRef<Set<string>>(new Set());
  const backgroundedSessionIdsRef = React.useRef<Set<string>>(new Set());
  const refreshedPendingImportSessionsRef = React.useRef<Set<string>>(new Set());

  const hideSessionToast = React.useCallback((sessionId: string) => {
    backgroundedSessionIdsRef.current.add(sessionId);
    const mobileTimer = mobileDismissTimersRef.current[sessionId];
    if (mobileTimer) {
      clearTimeout(mobileTimer);
      delete mobileDismissTimersRef.current[sessionId];
    }
    toast.dismiss(sessionId);
  }, []);

  const getSessionById = React.useCallback(
    (sessionId: string) =>
      sessions.find((session) => session.sessionId === sessionId) ?? null,
    [sessions],
  );

  React.useEffect(() => {
    for (const session of sessions) {
      if (isTerminal(session.status)) {
        if (!refreshedPendingImportSessionsRef.current.has(session.sessionId)) {
          dispatchNavigationBadgesRefresh();
          refreshedPendingImportSessionsRef.current.add(session.sessionId);
        }
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (!existingTimer) {
          const dismissDelay = isMobile
            ? MOBILE_TOAST_DURATION_MS
            : TERMINAL_TOAST_DURATION_MS;
          dismissTimersRef.current[session.sessionId] = setTimeout(() => {
            toast.dismiss(session.sessionId);
            dismissTimersRef.current[session.sessionId] = setTimeout(() => {
              dismissSession(session.sessionId);
              delete dismissTimersRef.current[session.sessionId];
              delete mobileDismissTimersRef.current[session.sessionId];
              shownToastIdsRef.current.delete(session.sessionId);
              backgroundedSessionIdsRef.current.delete(session.sessionId);
            }, TOAST_EXIT_GRACE_MS);
          }, dismissDelay);
        }
      } else {
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (existingTimer) {
          clearTimeout(existingTimer);
          delete dismissTimersRef.current[session.sessionId];
        }
        refreshedPendingImportSessionsRef.current.delete(session.sessionId);

        if (isMobile && !backgroundedSessionIdsRef.current.has(session.sessionId)) {
          if (!mobileDismissTimersRef.current[session.sessionId]) {
            mobileDismissTimersRef.current[session.sessionId] = setTimeout(() => {
              hideSessionToast(session.sessionId);
            }, MOBILE_TOAST_DURATION_MS);
          }
        } else if (!isMobile) {
          const mobileTimer = mobileDismissTimersRef.current[session.sessionId];
          if (mobileTimer) {
            clearTimeout(mobileTimer);
            delete mobileDismissTimersRef.current[session.sessionId];
          }
        }
      }

      if (!shownToastIdsRef.current.has(session.sessionId)) {
        toast.custom(
          () => (
            <LiveLibraryScanToast
              sessionId={session.sessionId}
              onRunInBackground={() => hideSessionToast(session.sessionId)}
            />
          ),
          {
            id: session.sessionId,
            toasterId: LIBRARY_SCAN_TOASTER_ID,
            className: "rounded-lg overflow-hidden p-0",
            duration: Infinity,
          },
        );
        shownToastIdsRef.current.add(session.sessionId);
      }
    }
  }, [dismissSession, hideSessionToast, isMobile, sessions]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
      for (const timer of Object.values(mobileDismissTimersRef.current)) {
        clearTimeout(timer);
      }
      shownToastIdsRef.current.clear();
      backgroundedSessionIdsRef.current.clear();
      refreshedPendingImportSessionsRef.current.clear();
    },
    [],
  );

  const value = React.useMemo(
    () => ({
      sessions: sessions.filter((session) => !isTerminal(session.status)),
      getActiveSession,
      getSessionById,
      refreshSessions,
    }),
    [getActiveSession, getSessionById, refreshSessions, sessions],
  );

  return (
    <LibraryScanProgressContext.Provider value={value}>
      {children}
      <Toaster
        id={LIBRARY_SCAN_TOASTER_ID}
        position="top-right"
        duration={10000}
        expand
        visibleToasts={MAX_VISIBLE_LIBRARY_SCAN_TOASTS}
      />
      <Toaster
        position="bottom-right"
        duration={10000}
        visibleToasts={MAX_VISIBLE_GENERAL_TOASTS}
      />
    </LibraryScanProgressContext.Provider>
  );
}
