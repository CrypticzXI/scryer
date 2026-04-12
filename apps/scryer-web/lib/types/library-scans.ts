import type { Facet, LibraryScanSummary } from "./titles";

export const libraryScanStatusValues = [
  "discovering",
  "running",
  "completed",
  "canceled",
  "warning",
  "failed",
] as const;

export type LibraryScanStatus = (typeof libraryScanStatusValues)[number];

export const libraryScanModeValues = ["full", "additive"] as const;
export type LibraryScanMode = (typeof libraryScanModeValues)[number];

export type LibraryScanPhaseProgress = {
  total: number;
  completed: number;
  failed: number;
};

export type LibraryScanProgress = {
  sessionId: string;
  facet: Facet;
  mode: LibraryScanMode;
  status: LibraryScanStatus;
  startedAt: string;
  updatedAt: string;
  foundTitles: number;
  titleMatchTotalKnown: boolean;
  hydrationTotalKnown: boolean;
  mediaAnalysisTotalKnown: boolean;
  titleMatchProgress: LibraryScanPhaseProgress;
  hydrationProgress: LibraryScanPhaseProgress;
  mediaAnalysisProgress: LibraryScanPhaseProgress;
  summary?: LibraryScanSummary | null;
};
