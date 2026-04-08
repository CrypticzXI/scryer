import * as React from "react";

import { LibraryScanToast } from "@/components/root/library-scan-toast";
import { Toaster, toast } from "@/components/ui/sonner";
import { LibraryScanProgressContext } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useLibraryScanEventStream } from "@/lib/hooks/use-library-scan-event-stream";
import type { LibraryScanStatus } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;

function isTerminal(status: LibraryScanStatus): boolean {
  return status === "completed" || status === "warning" || status === "failed";
}

function LiveLibraryScanToast({
  sessionId,
}: {
  sessionId: string;
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

  return <LibraryScanToast session={session} t={t} />;
}

export function LibraryScanProgressProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const { sessions, getActiveSession, dismissSession } = useLibraryScanEventStream();
  const dismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});
  const shownToastIdsRef = React.useRef<Set<string>>(new Set());

  const getSessionById = React.useCallback(
    (sessionId: string) =>
      sessions.find((session) => session.sessionId === sessionId) ?? null,
    [sessions],
  );

  React.useEffect(() => {
    for (const session of sessions) {
      if (isTerminal(session.status)) {
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (!existingTimer) {
          dismissTimersRef.current[session.sessionId] = setTimeout(() => {
            toast.dismiss(session.sessionId);
            dismissSession(session.sessionId);
            delete dismissTimersRef.current[session.sessionId];
            shownToastIdsRef.current.delete(session.sessionId);
          }, TERMINAL_TOAST_DURATION_MS);
        }
      } else {
        const existingTimer = dismissTimersRef.current[session.sessionId];
        if (existingTimer) {
          clearTimeout(existingTimer);
          delete dismissTimersRef.current[session.sessionId];
        }
      }

      if (!shownToastIdsRef.current.has(session.sessionId)) {
        toast.custom(() => <LiveLibraryScanToast sessionId={session.sessionId} />, {
          id: session.sessionId,
          className: "rounded-lg overflow-hidden p-0",
          duration: Infinity,
        });
        shownToastIdsRef.current.add(session.sessionId);
      }
    }
  }, [dismissSession, sessions]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
      shownToastIdsRef.current.clear();
    },
    [],
  );

  const value = React.useMemo(
    () => ({
      sessions: sessions.filter((session) => !isTerminal(session.status)),
      getActiveSession,
      getSessionById,
    }),
    [getActiveSession, getSessionById, sessions],
  );

  return (
    <LibraryScanProgressContext.Provider value={value}>
      {children}
      <Toaster position="top-right" duration={10000} />
    </LibraryScanProgressContext.Provider>
  );
}
