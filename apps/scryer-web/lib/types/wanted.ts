export type WantedMediaType = "movie" | "episode" | "series_movie";
export type WantedStatus = "wanted" | "grabbed" | "paused" | "completed";
export type ConvergenceState = "queued" | "searching" | "converged" | "deferred";
export type RecencyLane = "hot" | "cold";
export type WantedKind = "missing" | "cutoff_upgrade";
export type PendingReleaseStatus =
  | "waiting"
  | "standby"
  | "processing"
  | "grabbed"
  | "superseded"
  | "expired"
  | "dismissed";

export type WantedItem = {
  id: string;
  titleId: string;
  titleName: string | null;
  titleSlug: string | null;
  titleFacet: string | null;
  libraryId: string | null;
  libraryName: string | null;
  librarySlug: string | null;
  episodeId: string | null;
  collectionId: string | null;
  seasonNumber: string | null;
  episodeNumber: string | null;
  mediaType: WantedMediaType;
  lastSearchAt: string | null;
  status: WantedStatus;
  grabbedRelease: string | null;
  currentScore: number | null;
  latestReleaseDecision?: {
    decisionCode: string;
    createdAt: string;
  } | null;
  mismatchRecoveryEligible?: boolean;
  convergenceState: ConvergenceState;
  indexersCovered: number;
  indexersRouted: number;
  recencyLane: RecencyLane;
  createdAt: string;
  updatedAt: string;
};

// Progress snapshot of the server-side interactive acquisition-search job
// (RFC 119 §7.3) — survives navigation/refresh, polled by id.
export type AcquisitionSearchJob = {
  id: string;
  state: "running" | "completed" | "cancelled" | "failed";
  total: number;
  processed: number;
  grabbedCount: number;
  failedCount: number;
  currentTitle: string | null;
  startedAt: string;
  finishedAt: string | null;
};

export type PendingReleaseItem = {
  id: string;
  wantedItemId: string;
  titleId: string;
  releaseTitle: string;
  releaseUrl: string | null;
  releaseSizeBytes: number | null;
  releaseScore: number;
  scoringLogJson: string | null;
  indexerSource: string | null;
  addedAt: string;
  delayUntil: string;
  status: PendingReleaseStatus;
};

export type ReleaseDecisionItem = {
  id: string;
  wantedItemId: string;
  titleId: string;
  releaseTitle: string;
  releaseUrl: string | null;
  releaseSizeBytes: number | null;
  decisionCode: string;
  candidateScore: number;
  currentScore: number | null;
  scoreDelta: number | null;
  explanationJson: string | null;
  createdAt: string;
};

export type TitleAcquisitionDiagnostics = {
  recentDecisions: ReleaseDecisionItem[];
  decisionCounts: { code: string; count: number }[];
  wantedStatusCounts: { status: WantedStatus; count: number }[];
  pendingReleaseCounts: { status: PendingReleaseStatus; count: number }[];
  mismatchRecoveryEligibleCount: number;
  latestDecisionAt: string | null;
  latestWantedSearchAt: string | null;
};
