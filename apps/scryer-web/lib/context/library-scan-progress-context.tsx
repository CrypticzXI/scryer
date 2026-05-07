import { createContext, useContext } from "react";

import type { Facet, LibraryScanProgress } from "@/lib/types";

export type LibraryScanProgressContextValue = {
  sessions: LibraryScanProgress[];
  getActiveSession: (
    facet: Facet,
    libraryId?: string | null,
  ) => LibraryScanProgress | null;
  getSessionById: (sessionId: string) => LibraryScanProgress | null;
  refreshSessions: () => Promise<LibraryScanProgress[]>;
};

export const LibraryScanProgressContext =
  createContext<LibraryScanProgressContextValue | null>(null);

export function useLibraryScanProgress(): LibraryScanProgressContextValue {
  const value = useContext(LibraryScanProgressContext);
  if (!value) {
    throw new Error(
      "useLibraryScanProgress must be used within LibraryScanProgressContext.Provider",
    );
  }
  return value;
}
